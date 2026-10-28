//! Per-format handling of the archive attribute types: float32 is stored
//! exactly by every writer; `Null`, lists, structs and non-millisecond
//! timestamps refuse up front in FlatGeobuf, GeoPackage and GeoJSON.
use super::*;
use arrow_array::builder::{Float64Builder, ListBuilder};
use arrow_array::{
    ArrayRef, BinaryArray, Float32Array, Int64Array, NullArray, StringArray,
    TimestampNanosecondArray,
};
use arrow_schema::{DataType, Field, Schema};
use std::{collections::HashMap, sync::Arc};

fn batch(name: &str, values: ArrayRef) -> RecordBatch {
    let json = serde_json::json!({"type":"GeographicCRS","name":"WGS 84"});
    let crs = contract::Crs {
        wkt: "WGS 84 WKT".into(),
        projjson: json.clone(),
        units: "degree".into(),
        axis_order: "xy".into(),
        resolved: true,
        geographic_wgs84: true,
    };
    let geometry = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
        ("ARROW:extension:name".into(), "geoarrow.wkb".into()),
        (
            "ARROW:extension:metadata".into(),
            serde_json::json!({"crs":json,"crs_type":"projjson"}).to_string(),
        ),
    ]));
    let rows = values.len();
    let schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("mesh_name", DataType::Utf8, false),
            Field::new("cell_id", DataType::Int64, false),
            Field::new(name, values.data_type().clone(), true),
            geometry,
        ],
        HashMap::from([(
            contract::CONTRACT_KEY.into(),
            serde_json::to_string(&crs).unwrap(),
        )]),
    ));
    let point = |x: f64| {
        let mut b = vec![1, 1, 0, 0, 0];
        b.extend(x.to_le_bytes());
        b.extend(40f64.to_le_bytes());
        b
    };
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from_iter_values(std::iter::repeat_n(
                "m", rows,
            ))) as ArrayRef,
            Arc::new(Int64Array::from_iter_values(0..rows as i64)),
            values,
            Arc::new(BinaryArray::from_iter(
                (0..rows).map(|r| Some(point(-77. + r as f64))),
            )),
        ],
    )
    .unwrap()
}

fn export(batch: &RecordBatch, output: &Path, format: Format) -> Result<Metrics> {
    export_batches(
        batch.schema(),
        vec![Ok(batch.clone())],
        output,
        format,
        1024,
    )
}

#[test]
fn float32_is_exact_in_every_format() {
    let temp = tempfile::tempdir().unwrap();
    let values = batch(
        "weir_coef",
        Arc::new(Float32Array::from(vec![Some(2.6), None])),
    );
    for (format, ext) in [
        (Format::Arrow, "arrow"),
        (Format::GeoParquet, "parquet"),
        (Format::FlatGeobuf, "fgb"),
        (Format::GeoJson, "geojson"),
        (Format::GeoJsonLines, "jsonl"),
        (Format::GeoPackage, "gpkg"),
    ] {
        let output = temp.path().join(format!("f32.{ext}"));
        let metrics = export(&values, &output, format).unwrap();
        assert_eq!(metrics.rows, 2, "{format:?}");
    }
    // GeoJSON writes the float32's exact value, not a rounded decimal.
    let text = std::fs::read_to_string(temp.path().join("f32.geojson")).unwrap();
    assert!(text.contains(r#""weir_coef":2.5999999046325684"#), "{text}");
    assert!(text.contains(r#""weir_coef":null"#), "{text}");
    // GeoPackage declares FLOAT and stores the exact value as SQLite REAL.
    let gpkg = geopackage::GeoPackage::open(temp.path().join("f32.gpkg")).unwrap();
    let declared: String = gpkg
        .connection()
        .query_row(
            "SELECT type FROM pragma_table_info('features') WHERE name = 'weir_coef'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(declared, "FLOAT");
    let stored: f64 = gpkg
        .connection()
        .query_row(
            "SELECT weir_coef FROM features WHERE cell_id = 0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, f64::from(2.6f32));
    // FlatGeobuf keeps a native Float column.
    let mut file = std::io::BufReader::new(File::open(temp.path().join("f32.fgb")).unwrap());
    let reader = flatgeobuf::FgbReader::open(&mut file).unwrap();
    let column = reader.header().columns().unwrap().get(2);
    assert_eq!(column.name(), "weir_coef");
    assert_eq!(column.type_(), flatgeobuf::ColumnType::Float);
}

#[test]
fn names_differing_only_in_case_are_kept_where_names_are_case_sensitive() {
    let temp = tempfile::tempdir().unwrap();
    let b = batch("River", Arc::new(StringArray::from(vec!["Bald Eagle"])));
    let mut fields: Vec<Field> = b
        .schema()
        .fields()
        .iter()
        .map(|f| f.as_ref().clone())
        .collect();
    fields.insert(3, Field::new("river", DataType::Utf8, true));
    let mut columns = b.columns().to_vec();
    columns.insert(3, Arc::new(StringArray::from(vec!["Spring Creek"])));
    let b = RecordBatch::try_new(
        Arc::new(Schema::new_with_metadata(
            fields,
            b.schema().metadata().clone(),
        )),
        columns,
    )
    .unwrap();
    for (format, ext) in [(Format::Arrow, "arrow"), (Format::GeoParquet, "parquet")] {
        export(&b, &temp.path().join(format!("case.{ext}")), format).unwrap();
    }
    for (format, ext) in [
        (Format::FlatGeobuf, "fgb"),
        (Format::GeoJson, "geojson"),
        (Format::GeoPackage, "gpkg"),
    ] {
        let error = export(&b, &temp.path().join(format!("case.{ext}")), format).unwrap_err();
        assert!(
            error.to_string().contains("differing only in case"),
            "{format:?}: {error}"
        );
    }
}

#[test]
fn nested_and_other_new_types_refuse_outside_arrow_and_parquet() {
    let temp = tempfile::tempdir().unwrap();
    let list: ArrayRef = {
        let mut b = ListBuilder::new(Float64Builder::new());
        b.values().append_value(1.);
        b.append(true);
        Arc::new(b.finish())
    };
    let cases: Vec<ArrayRef> = vec![
        list,
        Arc::new(NullArray::new(1)),
        Arc::new(TimestampNanosecondArray::from(vec![1])),
    ];
    for values in cases {
        let b = batch("v", values);
        for (format, ext) in [(Format::Arrow, "arrow"), (Format::GeoParquet, "parquet")] {
            let output = temp
                .path()
                .join(format!("{}.{ext}", b.column(2).data_type()));
            export(&b, &output, format).unwrap();
            std::fs::remove_file(output).unwrap();
        }
        for (format, ext) in [
            (Format::FlatGeobuf, "fgb"),
            (Format::GeoJson, "geojson"),
            (Format::GeoJsonLines, "jsonl"),
            (Format::GeoPackage, "gpkg"),
        ] {
            let output = temp.path().join(format!("refused.{ext}"));
            let error = export(&b, &output, format).unwrap_err();
            assert!(
                error.to_string().contains("cannot store exactly"),
                "{format:?}: {error}"
            );
            assert!(!output.exists());
        }
        assert_eq!(temp.path().read_dir().unwrap().count(), 0);
    }
}
