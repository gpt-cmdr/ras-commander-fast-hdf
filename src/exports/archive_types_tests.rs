//! Attribute types beyond the v1 scalars (float32, Null, nanosecond
//! timestamps, lists and structs), as ras2cng archives carry them: exact round
//! trips through the GeoParquet archive profile and explicit refusals.
use super::*;
use crate::exports::contract::{Crs, CONTRACT_KEY};
use arrow_array::builder::{
    BooleanBuilder, Float32Builder, Float64Builder, ListBuilder, StructBuilder,
};
use arrow_array::{
    Float32Array, Int64Array, ListArray, NullArray, StructArray, TimestampNanosecondArray,
    UInt32Array, UInt64Array,
};
use arrow_schema::Fields;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::reader::{FileReader, SerializedFileReader};

fn archive() -> Options {
    Options {
        compression: Codec::Zstd,
        bbox_covering: Covering::Struct,
        order: Order::Hilbert,
        layer_column: Some("layer".into()),
        ..Options::default()
    }
}

fn crs() -> HashMap<String, String> {
    let c = Crs {
        wkt: "crs WKT".into(),
        projjson: serde_json::json!({"type":"ProjectedCRS","name":"crs"}),
        units: "US survey foot".into(),
        axis_order: "xy".into(),
        resolved: true,
        geographic_wgs84: false,
    };
    HashMap::from([(CONTRACT_KEY.into(), serde_json::to_string(&c).unwrap())])
}

fn geometry_field() -> Field {
    let json = serde_json::json!({"type":"ProjectedCRS","name":"crs"});
    Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
        ("ARROW:extension:name".into(), "geoarrow.wkb".into()),
        (
            "ARROW:extension:metadata".into(),
            serde_json::json!({"crs":json,"crs_type":"projjson"}).to_string(),
        ),
    ]))
}

fn point(x: f64, y: f64) -> Vec<u8> {
    let mut b = vec![1, 1, 0, 0, 0];
    b.extend(x.to_le_bytes());
    b.extend(y.to_le_bytes());
    b
}

/// A layer: identities, the given extra columns, then point geometry, null
/// every 7th row.
fn batch(start: i64, rows: usize, extra: Vec<(&str, ArrayRef)>) -> RecordBatch {
    let mut fields = vec![
        Field::new("mesh_name", DataType::Utf8, false),
        Field::new("cell_id", DataType::Int64, false),
    ];
    let mut arrays: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(std::iter::repeat_n(
            "m", rows,
        ))),
        Arc::new(Int64Array::from_iter_values(start..start + rows as i64)),
    ];
    for (name, array) in extra {
        fields.push(Field::new(name, array.data_type().clone(), true));
        arrays.push(array);
    }
    fields.push(geometry_field());
    arrays.push(Arc::new(BinaryArray::from_iter((0..rows).map(|r| {
        let r = start as usize + r;
        (!r.is_multiple_of(7)).then(|| point(r as f64, (r % 13) as f64))
    }))));
    RecordBatch::try_new(Arc::new(Schema::new_with_metadata(fields, crs())), arrays).unwrap()
}

fn input(name: &str, batch: RecordBatch) -> LayerInput<Vec<Result<RecordBatch>>> {
    LayerInput {
        name: Some(name.into()),
        schema: batch.schema(),
        batches: vec![Ok(batch)],
    }
}

/// `list<list<double>>`: null rows, empty lists, a null inner list, null
/// values, empty inner lists.
fn nested_lists(rows: usize) -> ArrayRef {
    let mut b = ListBuilder::new(ListBuilder::new(Float64Builder::new()));
    for r in 0..rows {
        match r % 4 {
            0 => b.append(false),
            1 => b.append(true),
            2 => {
                b.values().append(false);
                b.values().values().append_value(r as f64);
                b.values().values().append_null();
                b.values().append(true);
                b.append(true);
            }
            _ => {
                b.values().values().append_value(r as f64 + 0.5);
                b.values().append(true);
                b.values().append(true);
                b.append(true);
            }
        }
    }
    Arc::new(b.finish())
}

/// `list<struct<Left Sta: double, Permanent: bool>>` with null and empty rows.
fn list_of_struct(rows: usize) -> ArrayRef {
    let fields = Fields::from(vec![
        Field::new("Left Sta", DataType::Float64, true),
        Field::new("Permanent", DataType::Boolean, true),
    ]);
    let mut b = ListBuilder::new(StructBuilder::new(
        fields,
        vec![
            Box::new(Float64Builder::new()),
            Box::new(BooleanBuilder::new()),
        ],
    ));
    for r in 0..rows {
        match r % 3 {
            0 => b.append(false),
            1 => b.append(true),
            _ => {
                let s = b.values();
                s.field_builder::<Float64Builder>(0)
                    .unwrap()
                    .append_value(r as f64);
                s.field_builder::<BooleanBuilder>(1).unwrap().append_null();
                s.append(true);
                let s = b.values();
                s.field_builder::<Float64Builder>(0).unwrap().append_null();
                s.field_builder::<BooleanBuilder>(1)
                    .unwrap()
                    .append_value(true);
                s.append(true);
                b.append(true);
            }
        }
    }
    Arc::new(b.finish())
}

/// `struct<Station: list<double>, Mann n: list<double>>`, null every 5th row.
fn struct_of_lists(rows: usize) -> ArrayRef {
    let list = |offset: f64| {
        let mut b = ListBuilder::new(Float64Builder::new());
        for r in 0..rows {
            b.values().append_value(r as f64 + offset);
            if r.is_multiple_of(2) {
                b.values().append_null();
            }
            b.append(true);
        }
        Arc::new(b.finish()) as ArrayRef
    };
    let (station, mann) = (list(0.), list(0.5));
    let fields = Fields::from(vec![
        Field::new("Station", station.data_type().clone(), true),
        Field::new("Mann n", mann.data_type().clone(), true),
    ]);
    let validity = arrow_buffer::NullBuffer::from_iter((0..rows).map(|r| !r.is_multiple_of(5)));
    Arc::new(StructArray::new(
        fields,
        vec![station, mann],
        Some(validity),
    ))
}

/// Every archive type ras2cng writes, `rows` rows, ids from `start`.
fn typed(start: i64, rows: usize) -> RecordBatch {
    batch(
        start,
        rows,
        vec![
            (
                "Len Left",
                Arc::new(Float32Array::from_iter(
                    (0..rows).map(|r| (!r.is_multiple_of(3)).then_some(r as f32 * 0.1)),
                )),
            ),
            ("Left Levee Sta", Arc::new(NullArray::new(rows))),
            (
                "max_time",
                Arc::new(TimestampNanosecondArray::from_iter((0..rows).map(|r| {
                    (!r.is_multiple_of(4)).then_some(r as i64 * 1_000_000_007)
                }))),
            ),
            ("station_elevation", nested_lists(rows)),
            ("ineffective_blocks", list_of_struct(rows)),
            ("mannings_n", struct_of_lists(rows)),
            (
                "all_null_list",
                Arc::new(ListArray::new_null(
                    Arc::new(Field::new_list_field(DataType::Null, true)),
                    rows,
                )),
            ),
        ],
    )
}

fn read(path: &Path) -> (RecordBatch, Vec<i64>) {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap()).unwrap();
    let sizes = builder
        .metadata()
        .row_groups()
        .iter()
        .map(|g| g.num_rows())
        .collect();
    let schema = builder.schema().clone();
    let batches: Vec<_> = builder.build().unwrap().map(|b| b.unwrap()).collect();
    (
        arrow_select::concat::concat_batches(&schema, &batches).unwrap(),
        sizes,
    )
}

/// The arrow-rs reader names list children `element` after the Parquet
/// schema, where the input (like pyarrow) used `item`. Give `array` the
/// expected type after checking the two differ only in list child names.
fn retyped(array: &dyn Array, expected: &DataType) -> ArrayRef {
    fn unnamed(dtype: &DataType) -> DataType {
        match dtype {
            DataType::List(f) => DataType::List(Arc::new(
                Field::new("item", unnamed(f.data_type()), f.is_nullable())
                    .with_metadata(f.metadata().clone()),
            )),
            DataType::Struct(fields) => DataType::Struct(
                fields
                    .iter()
                    .map(|f| f.as_ref().clone().with_data_type(unnamed(f.data_type())))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    fn rebuild(data: arrow_data::ArrayData, dtype: &DataType) -> arrow_data::ArrayData {
        let children = match dtype {
            DataType::List(f) => vec![rebuild(data.child_data()[0].clone(), f.data_type())],
            DataType::Struct(fields) => data
                .child_data()
                .iter()
                .zip(fields)
                .map(|(c, f)| rebuild(c.clone(), f.data_type()))
                .collect(),
            _ => data.child_data().to_vec(),
        };
        data.into_builder()
            .data_type(dtype.clone())
            .child_data(children)
            .build()
            .unwrap()
    }
    assert_eq!(unnamed(array.data_type()), unnamed(expected));
    arrow_array::make_array(rebuild(array.to_data(), expected))
}

fn physical_schema(path: &Path) -> String {
    let reader = SerializedFileReader::new(File::open(path).unwrap()).unwrap();
    let mut out = Vec::new();
    parquet::schema::printer::print_schema(&mut out, reader.metadata().file_metadata().schema());
    String::from_utf8(out).unwrap()
}

#[test]
fn archive_types_round_trip_exactly_across_batches_and_row_groups() {
    let temp = tempfile::tempdir().unwrap();
    // 65,537 rows in three batches, the last of one row: two row groups,
    // 65,536 rows and 1 row.
    let batches = [typed(0, 40_000), typed(40_000, 25_536), typed(65_536, 1)];
    let schema = batches[0].schema();
    let expected = arrow_select::concat::concat_batches(&schema, &batches).unwrap();
    for order in [Order::Input, Order::Hilbert] {
        let output = temp.path().join(format!("{order:?}.parquet"));
        let options = Options {
            order,
            hilbert_column: (order == Order::Hilbert).then(|| "hilbert_index".into()),
            dictionary: Dictionary::Utf8,
            ..archive()
        };
        let layers = vec![LayerInput {
            name: Some("cross_sections".into()),
            schema: schema.clone(),
            batches: batches.iter().cloned().map(Ok).collect::<Vec<_>>(),
        }];
        let metrics = export_layers(layers, &output, &options).unwrap();
        assert_eq!(metrics.row_groups, 2);
        let (read_back, sizes) = read(&output);
        assert_eq!(sizes, vec![65_536, 1]);
        // Restore input order by cell_id, then compare each input column
        // exactly: field (type, nullability, metadata), values, null and
        // offset buffers.
        let cell_ids = read_back
            .column_by_name("cell_id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .clone();
        let mut by_id: Vec<(i64, u32)> = cell_ids
            .values()
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i as u32))
            .collect();
        by_id.sort_unstable();
        let take = UInt32Array::from_iter_values(by_id.into_iter().map(|(_, i)| i));
        for (i, field) in schema.fields().iter().enumerate() {
            let out_field = read_back
                .schema()
                .field_with_name(field.name())
                .unwrap()
                .clone();
            assert_eq!(out_field.name(), field.name());
            assert_eq!(out_field.is_nullable(), field.is_nullable());
            if field.name() != "geometry" {
                // GeoParquet moves the geometry CRS into the `geo` metadata.
                assert_eq!(out_field.metadata(), field.metadata(), "{}", field.name());
            }
            let column = read_back.column_by_name(field.name()).unwrap();
            let column = arrow_select::take::take(column.as_ref(), &take, None).unwrap();
            let column = retyped(column.as_ref(), field.data_type());
            assert_eq!(
                column.as_ref(),
                expected.column(i).as_ref(),
                "{}",
                field.name()
            );
        }
        // The specification's 3-level list layout with `element`, as pyarrow
        // writes it, and exact physical types.
        let physical = physical_schema(&output);
        for needle in [
            "OPTIONAL group station_elevation (LIST)",
            "REPEATED group list",
            "OPTIONAL group element (LIST)",
            "OPTIONAL DOUBLE element;",
            "OPTIONAL FLOAT Len Left;",
            "OPTIONAL INT64 max_time (TIMESTAMP(NANOS,false));",
            "OPTIONAL INT32 Left Levee Sta (UNKNOWN);",
            "OPTIONAL group mannings_n {",
            "message schema {",
        ] {
            assert!(physical.contains(needle), "{needle}\n{physical}");
        }
        assert!(!physical.contains(" item"), "{physical}");
        let keys = read_back.column_by_name("hilbert_index");
        if order == Order::Hilbert {
            // The stored key is the sort key; null exactly for null geometry.
            let keys = keys.unwrap();
            let keys = keys.as_any().downcast_ref::<UInt64Array>().unwrap();
            assert_eq!(
                keys.nulls(),
                read_back.column_by_name("geometry").unwrap().nulls()
            );
            let present: Vec<u64> = keys.iter().flatten().collect();
            assert!(present.windows(2).all(|w| w[0] <= w[1]));
            assert!(keys.iter().skip(present.len()).all(|k| k.is_none()));
        } else {
            assert!(keys.is_none());
        }
    }
}

#[test]
fn layers_with_missing_nested_fields_get_typed_nulls() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("mixed.parquet");
    let plain = batch(0, 5, vec![]);
    let nested = typed(5, 9);
    export_layers(
        vec![
            input("cells", plain),
            input("cross_sections", nested.clone()),
        ],
        &output,
        &archive(),
    )
    .unwrap();
    let (read_back, sizes) = read(&output);
    assert_eq!(sizes, vec![5, 9]);
    for field in nested.schema().fields() {
        let column = read_back.column_by_name(field.name()).unwrap();
        let column = retyped(column.as_ref(), field.data_type());
        if !["mesh_name", "cell_id", "geometry"].contains(&field.name().as_str()) {
            assert_eq!(
                column.slice(0, 5).logical_null_count(),
                5,
                "{}",
                field.name()
            );
        }
    }
}

fn one(name: &str, array: ArrayRef) -> RecordBatch {
    batch(1, 1, vec![(name, array)])
}

fn float_list(v: f64) -> ArrayRef {
    let mut b = ListBuilder::new(Float64Builder::new());
    b.values().append_value(v);
    b.append(true);
    Arc::new(b.finish())
}

#[test]
fn archive_types_refuse_what_they_cannot_store_exactly() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("refused.parquet");
    let list_field =
        |dtype: DataType, nullable: bool| Arc::new(Field::new_list_field(dtype, nullable));
    let inf_struct: ArrayRef = Arc::new(StructArray::new(
        Fields::from(vec![Field::new("x", DataType::Float64, true)]),
        vec![Arc::new(Float64Array::from(vec![f64::NEG_INFINITY]))],
        None,
    ));
    let cases: Vec<(RecordBatch, &str)> = vec![
        (
            one("f", Arc::new(Float32Array::from(vec![f32::NAN]))),
            "nonfinite",
        ),
        (
            one("f", Arc::new(Float32Array::from(vec![f32::INFINITY]))),
            "nonfinite",
        ),
        (one("l", float_list(f64::NAN)), "nonfinite"),
        (one("s", inf_struct), "nonfinite"),
        (
            one("i", Arc::new(arrow_array::Int32Array::from(vec![1]))),
            "unsupported field type",
        ),
        (
            one(
                "big",
                Arc::new(arrow_array::LargeListArray::new_null(
                    list_field(DataType::Float64, true),
                    1,
                )),
            ),
            "unsupported field type",
        ),
        (
            one(
                "fixed",
                Arc::new(arrow_array::FixedSizeListArray::new_null(
                    list_field(DataType::Float64, true),
                    2,
                    1,
                )),
            ),
            "unsupported field type",
        ),
        (
            one(
                "dict",
                Arc::new(
                    arrow_array::DictionaryArray::<arrow_array::types::Int32Type>::from_iter(["a"]),
                ),
            ),
            "unsupported field type",
        ),
        (
            one(
                "tz",
                Arc::new(TimestampNanosecondArray::from(vec![1]).with_timezone("UTC")),
            ),
            "unsupported field type",
        ),
        (
            one(
                "strict",
                Arc::new(ListArray::new_null(list_field(DataType::Float64, false), 1)),
            ),
            "unsupported field type",
        ),
        (
            one(
                "s",
                Arc::new(StructArray::new_null(
                    Fields::from(vec![Field::new("x", DataType::Int32, true)]),
                    1,
                )),
            ),
            "unsupported field type",
        ),
    ];
    for (batch, message) in cases {
        let error = export_layers(vec![input("a", batch)], &output, &archive()).unwrap_err();
        assert!(error.to_string().contains(message), "{error} vs {message}");
        assert!(!output.exists());
        assert_eq!(temp.path().read_dir().unwrap().count(), 0);
    }

    // A later batch whose nested type differs from the schema refuses the file.
    let first = one("l", float_list(1.));
    let f32_list = {
        let mut b = ListBuilder::new(Float32Builder::new());
        b.values().append_value(1.);
        b.append(true);
        Arc::new(b.finish()) as ArrayRef
    };
    let second = one("l", f32_list);
    let layers = vec![LayerInput {
        name: Some("a".into()),
        schema: first.schema(),
        batches: vec![Ok(first.clone()), Ok(second.clone())],
    }];
    let error = export_layers(layers, &output, &archive()).unwrap_err();
    assert!(error.to_string().contains("schema mismatch"), "{error}");
    assert_eq!(temp.path().read_dir().unwrap().count(), 0);
    // ... and two layers with different nested types in one field refuse.
    let error = export_layers(
        vec![input("a", first), input("b", second)],
        &output,
        &archive(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("different types"), "{error}");

    // hilbert_column needs Hilbert order and a free name.
    for (options, message) in [
        (
            Options {
                order: Order::Input,
                hilbert_column: Some("h".into()),
                ..archive()
            },
            "requires order",
        ),
        (
            Options {
                hilbert_column: Some("CELL_ID".into()),
                ..archive()
            },
            "collides",
        ),
    ] {
        let error =
            export_layers(vec![input("a", batch(0, 1, vec![]))], &output, &options).unwrap_err();
        assert!(error.to_string().contains(message), "{error} vs {message}");
    }
    assert_eq!(temp.path().read_dir().unwrap().count(), 0);
    let parsed: Options =
        serde_json::from_str(r#"{"dictionary":"utf8","hilbert_column":"h"}"#).unwrap();
    assert_eq!(parsed.dictionary, Dictionary::Utf8);
    assert!(serde_json::from_str::<Options>(r#"{"dictionary":"numeric"}"#).is_err());
}

#[test]
fn utf8_dictionary_option_limits_dictionaries_to_string_columns() {
    let temp = tempfile::tempdir().unwrap();
    let dictionaries = |dictionary: Dictionary| {
        let output = temp.path().join(format!("{dictionary:?}.parquet"));
        let options = Options {
            dictionary,
            ..archive()
        };
        export_layers(vec![input("a", typed(0, 300))], &output, &options).unwrap();
        let reader = SerializedFileReader::new(File::open(&output).unwrap()).unwrap();
        reader
            .metadata()
            .row_group(0)
            .columns()
            .iter()
            .filter(|c| c.dictionary_page_offset().is_some())
            .map(|c| c.column_path().string())
            .collect::<Vec<_>>()
    };
    assert_eq!(dictionaries(Dictionary::Utf8), vec!["mesh_name", "layer"]);
    assert!(dictionaries(Dictionary::None).is_empty());
    assert!(dictionaries(Dictionary::All).len() > 2);
}

#[test]
fn chunk_statistics_option_writes_untruncated_chunk_statistics_without_page_index() {
    let temp = tempfile::tempdir().unwrap();
    let long = "x".repeat(200);
    let strings: ArrayRef = Arc::new(StringArray::from(vec![long.as_str(); 3]));
    let write = |name: &str, options: Options| {
        let output = temp.path().join(name);
        let layer = batch(0, 3, vec![("description", strings.clone())]);
        export_layers(vec![input("a", layer)], &output, &options).unwrap();
        let reader = SerializedFileReader::new(File::open(&output).unwrap()).unwrap();
        let metadata = reader.metadata().clone();
        let group = metadata.row_group(0);
        let column = |path: &str| {
            group
                .columns()
                .iter()
                .find(|c| c.column_path().string() == path)
                .unwrap()
                .clone()
        };
        let description = column("description");
        let geometry = column("geometry");
        (
            description.column_index_offset().is_some(),
            description.offset_index_offset().is_some(),
            description
                .statistics()
                .and_then(|s| s.max_bytes_opt().map(<[u8]>::len)),
            geometry
                .statistics()
                .is_some_and(|s| s.min_bytes_opt().is_some()),
        )
    };
    assert_eq!(
        write("page.parquet", archive()),
        (true, true, Some(64), true)
    );
    let chunk = Options {
        statistics: Statistics::Chunk,
        geometry_statistics: false,
        ..archive()
    };
    assert_eq!(
        write("chunk.parquet", chunk.clone()),
        (false, false, Some(200), false)
    );
    // No statistics for all-null columns, top-level or nested (as pyarrow),
    // nor, with `geometry_statistics = false`, for the WKB.
    let output = temp.path().join("nulls.parquet");
    export_layers(vec![input("a", typed(0, 20))], &output, &chunk).unwrap();
    let reader = SerializedFileReader::new(File::open(&output).unwrap()).unwrap();
    for column in reader.metadata().row_group(0).columns() {
        let path = column.column_path().string();
        let unknown =
            ["Left Levee Sta", "all_null_list.list.element", "geometry"].contains(&path.as_str());
        assert_eq!(column.statistics().is_none(), unknown, "{path}");
    }
}

#[test]
fn file_bbox_ignores_empty_geometries() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("empty.parquet");
    // Rows 1..=3: two points and an empty LineString (row 0 is null).
    let mut layer = batch(0, 4, vec![]);
    let mut columns = layer.columns().to_vec();
    let empty_line = vec![1u8, 2, 0, 0, 0, 0, 0, 0, 0];
    columns[2] = Arc::new(BinaryArray::from_iter(vec![
        None,
        Some(point(1., 2.)),
        Some(empty_line),
        Some(point(5., -3.)),
    ]));
    layer = RecordBatch::try_new(layer.schema(), columns).unwrap();
    for options in [Options::default(), archive()] {
        let _ = std::fs::remove_file(&output);
        export_layers(vec![input("a", layer.clone())], &output, &options).unwrap();
        let reader = SerializedFileReader::new(File::open(&output).unwrap()).unwrap();
        let geo = reader
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .unwrap()
            .iter()
            .find(|kv| kv.key == "geo")
            .unwrap()
            .value
            .clone()
            .unwrap();
        let geo: serde_json::Value = serde_json::from_str(&geo).unwrap();
        assert_eq!(
            geo["columns"]["geometry"]["bbox"],
            serde_json::json!([1.0, -3.0, 5.0, 2.0])
        );
    }
}
