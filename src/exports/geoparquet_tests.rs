use super::*;
use crate::exports::contract::{Crs, CONTRACT_KEY};
use arrow_array::{BooleanArray, Int64Array, ListArray};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// Recommended archive options: ZSTD, bbox struct covering, Hilbert order
/// within each layer and a `layer` column.
fn archive() -> Options {
    Options {
        compression: Codec::Zstd,
        bbox_covering: Covering::Struct,
        order: Order::Hilbert,
        layer_column: Some("layer".into()),
        ..Options::default()
    }
}

fn crs(name: &str) -> HashMap<String, String> {
    let json = serde_json::json!({"type":"ProjectedCRS","name":name});
    let c = Crs {
        wkt: format!("{name} WKT"),
        projjson: json,
        units: "US survey foot".into(),
        axis_order: "xy".into(),
        resolved: true,
        geographic_wgs84: false,
    };
    HashMap::from([(CONTRACT_KEY.into(), serde_json::to_string(&c).unwrap())])
}

fn geometry_field(name: &str) -> Field {
    let json = serde_json::json!({"type":"ProjectedCRS","name":name});
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

/// Square with a square hole inside it, so its bounds are the shell's.
fn square(x: f64, y: f64, size: f64) -> Vec<u8> {
    let mut b = vec![1, 3, 0, 0, 0, 2, 0, 0, 0];
    for (x0, y0, s) in [(x, y, size), (x + size / 4., y + size / 4., size / 2.)] {
        b.extend(5u32.to_le_bytes());
        for (px, py) in [
            (x0, y0),
            (x0 + s, y0),
            (x0 + s, y0 + s),
            (x0, y0 + s),
            (x0, y0),
        ] {
            b.extend(px.to_le_bytes());
            b.extend(py.to_le_bytes());
        }
    }
    b
}

/// Layer with identities, an optional extra column and the given WKB rows.
fn layer(
    crs_name: &str,
    extra: Option<(&str, ArrayRef)>,
    geometry: Vec<Option<Vec<u8>>>,
) -> RecordBatch {
    let rows = geometry.len();
    let mut fields = vec![
        Field::new("mesh_name", DataType::Utf8, false),
        Field::new("cell_id", DataType::Int64, false),
    ];
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(std::iter::repeat_n(
            "m", rows,
        ))),
        Arc::new(Int64Array::from_iter_values(0..rows as i64)),
    ];
    if let Some((name, array)) = extra {
        fields.push(Field::new(name, array.data_type().clone(), true));
        columns.push(array);
    }
    fields.push(geometry_field(crs_name));
    columns.push(Arc::new(BinaryArray::from_iter(geometry)));
    RecordBatch::try_new(
        Arc::new(Schema::new_with_metadata(fields, crs(crs_name))),
        columns,
    )
    .unwrap()
}

fn input(name: &str, batch: RecordBatch) -> LayerInput<Vec<Result<RecordBatch>>> {
    LayerInput {
        name: Some(name.into()),
        schema: batch.schema(),
        batches: vec![Ok(batch)],
    }
}

fn read(path: &Path) -> (RecordBatch, serde_json::Value, usize) {
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap()).unwrap();
    let geo = builder
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
    let groups = builder.metadata().num_row_groups();
    let schema = builder.schema().clone();
    let batches: Vec<_> = builder.build().unwrap().map(|b| b.unwrap()).collect();
    (
        arrow_select::concat::concat_batches(&schema, &batches).unwrap(),
        serde_json::from_str(&geo).unwrap(),
        groups,
    )
}

fn strings(batch: &RecordBatch, name: &str) -> Vec<Option<String>> {
    let a = batch.column_by_name(name).unwrap();
    let a = a.as_any().downcast_ref::<StringArray>().unwrap();
    a.iter().map(|v| v.map(str::to_string)).collect()
}

fn floats(batch: &RecordBatch, name: &str) -> Vec<Option<f64>> {
    let a = batch.column_by_name(name).unwrap();
    let a = a.as_any().downcast_ref::<Float64Array>().unwrap();
    a.iter().collect()
}

fn ids(batch: &RecordBatch) -> Vec<i64> {
    let a = batch.column_by_name("cell_id").unwrap();
    a.as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .values()
        .to_vec()
}

#[test]
fn hilbert_matches_duckdb_spatial() {
    // Golden values from DuckDB 2.0 spatial:
    // ST_Hilbert(x, y, {'min_x':0,'min_y':0,'max_x':10,'max_y':10}::BOX_2D)
    let extent = [0., 0., 10., 10.];
    for (x, y, expected) in [
        (0., 0., 0u32),
        (10., 10., 2863311530),
        (5., 5., 715827882),
        (1.5, 2.5, 157903209),
        (9.99, 0.01, 4294959096),
        (3.3, 7.7, 1629721655),
        (0., 10., 1431655765),
        (10., 0., 4294967295),
        (2.5, 2.5, 178956970),
    ] {
        assert_eq!(hilbert_index(x, y, extent), expected, "({x}, {y})");
    }
    // A degenerate axis maps to cell 0 instead of dividing by zero.
    assert_eq!(hilbert_index(3., 3., [3., 3., 3., 3.]), 0);
}

#[test]
fn hilbert_permutation_is_stable_with_empty_rows_last() {
    let b = |x: f64, y: f64| Some([x, y, x, y]);
    let order = hilbert_permutation(&[None, b(10., 0.), b(0., 0.), b(0., 0.), None, b(5., 5.)]);
    assert_eq!(order, vec![2, 3, 5, 1, 0, 4]);
}

#[test]
fn archive_profile_unifies_layers_with_covering_and_layer_groups() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("model.g01.parquet");
    let cells = layer(
        "crs",
        Some((
            "min_elevation",
            Arc::new(Float64Array::from(vec![Some(1.5), None, Some(3.0)])),
        )),
        vec![
            Some(square(100., 100., 8.)),
            Some(square(0., 0., 4.)),
            Some(square(50., 50., 2.)),
        ],
    );
    let points = layer(
        "crs",
        Some(("active", Arc::new(BooleanArray::from(vec![true, false])))),
        vec![Some(point(7., 9.)), None],
    );
    let options = Options {
        compression: Codec::Zstd,
        compression_level: Some(3),
        row_group_rows: 2,
        bbox_covering: Covering::Columns,
        ..archive()
    };
    let metrics = export_layers(
        vec![
            input("mesh_cells", cells),
            input("reference_points", points),
        ],
        &output,
        &options,
    )
    .unwrap();
    assert_eq!((metrics.rows, metrics.layers), (5, 2));
    // Three cells in groups of 2 rows, then a new group for the next layer.
    assert_eq!(metrics.row_groups, 3);
    let (batch, geo, groups) = read(&output);
    assert_eq!(groups, 3);
    let names: Vec<_> = batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    assert_eq!(
        names,
        [
            "mesh_name",
            "cell_id",
            "min_elevation",
            "geometry",
            "active",
            "layer",
            "bbox_xmin",
            "bbox_ymin",
            "bbox_xmax",
            "bbox_ymax"
        ]
    );
    assert_eq!(
        strings(&batch, "layer"),
        [
            "mesh_cells",
            "mesh_cells",
            "mesh_cells",
            "reference_points",
            "reference_points"
        ]
        .map(|s| Some(s.to_string()))
    );
    // Hilbert order within the cell layer over its extent [0, 0, 108, 108];
    // the null geometry sorts last in the point layer.
    assert_eq!(ids(&batch), vec![1, 2, 0, 0, 1]);
    assert_eq!(
        floats(&batch, "min_elevation"),
        vec![None, Some(3.0), Some(1.5), None, None]
    );
    assert_eq!(
        floats(&batch, "bbox_xmin"),
        vec![Some(0.), Some(50.), Some(100.), Some(7.), None]
    );
    assert_eq!(
        floats(&batch, "bbox_ymax"),
        vec![Some(4.), Some(52.), Some(108.), Some(9.), None]
    );
    assert_eq!(batch.column_by_name("active").unwrap().null_count(), 3);
    let column = &geo["columns"]["geometry"];
    assert_eq!(
        column["covering"]["bbox"],
        serde_json::json!({"xmin":["bbox_xmin"],"ymin":["bbox_ymin"],
                           "xmax":["bbox_xmax"],"ymax":["bbox_ymax"]})
    );
    assert_eq!(column["encoding"], "WKB");
    assert_eq!(column["crs"]["name"], "crs");
    let bbox: Vec<f64> = serde_json::from_value(column["bbox"].clone()).unwrap();
    assert_eq!(bbox, vec![0., 0., 108., 108.]);
    assert!(!batch
        .schema()
        .field_with_name("layer")
        .unwrap()
        .is_nullable());
}

#[test]
fn input_order_streams_and_keeps_rows() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("plain.parquet");
    let cells = layer(
        "crs",
        None,
        vec![Some(square(100., 100., 8.)), Some(square(0., 0., 4.))],
    );
    let options = Options {
        bbox_covering: Covering::Columns,
        ..Options::default()
    };
    export_layers(vec![input("unused", cells)], &output, &options).unwrap();
    let (batch, _, _) = read(&output);
    assert_eq!(ids(&batch), vec![0, 1]);
    assert!(batch.column_by_name("layer").is_none());
}

#[test]
fn refusals_leave_no_output() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("refused.parquet");
    let cells = || layer("crs", None, vec![Some(point(1., 1.))]);
    let typed =
        |name: &str, array: ArrayRef| layer("crs", Some((name, array)), vec![Some(point(1., 1.))]);
    let cases: Vec<(Vec<LayerInput<_>>, Options, &str)> = vec![
        (
            vec![input("a", cells()), input("b", cells())],
            Options::default(),
            "requires layer_column",
        ),
        (
            vec![input("a", cells()), input("a", cells())],
            archive(),
            "unique",
        ),
        (
            vec![
                input("a", typed("v", Arc::new(Float64Array::from(vec![1.])))),
                input("b", typed("v", Arc::new(Int64Array::from(vec![1])))),
            ],
            archive(),
            "different types",
        ),
        (
            vec![
                input("a", cells()),
                input("b", layer("other", None, vec![Some(point(1., 1.))])),
            ],
            archive(),
            "different source CRS",
        ),
        (
            vec![input(
                "a",
                typed("bbox_xmin", Arc::new(Float64Array::from(vec![1.]))),
            )],
            Options {
                bbox_covering: Covering::Columns,
                ..archive()
            },
            "collides",
        ),
        (
            vec![input(
                "a",
                typed("Layer", Arc::new(StringArray::from(vec!["x"]))),
            )],
            archive(),
            "collides",
        ),
        (
            vec![input(
                "a",
                typed("BBOX", Arc::new(Float64Array::from(vec![1.]))),
            )],
            archive(),
            "collides",
        ),
        (
            vec![input("a", cells())],
            Options {
                compression_level: Some(3),
                ..Options::default()
            },
            "only to zstd",
        ),
        (
            vec![input("a", cells())],
            Options {
                row_group_rows: 0,
                ..Options::default()
            },
            "must be positive",
        ),
    ];
    for (layers, options, message) in cases {
        let error = export_layers(layers, &output, &options).unwrap_err();
        assert!(error.to_string().contains(message), "{error} vs {message}");
        assert!(!output.exists());
        assert_eq!(temp.path().read_dir().unwrap().count(), 0);
    }
    // Invalid geometry in a later layer fails the whole file atomically.
    let bad = layer("crs", None, vec![Some(vec![1, 3, 0, 0, 0, 1, 0, 0, 0])]);
    assert!(export_layers(
        vec![input("a", cells()), input("b", bad)],
        &output,
        &archive()
    )
    .is_err());
    assert_eq!(temp.path().read_dir().unwrap().count(), 0);
    // An existing destination is never replaced.
    std::fs::write(&output, b"keep").unwrap();
    assert!(export_layers(vec![input("a", cells())], &output, &Options::default()).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"keep");
    // Unknown options are rejected rather than ignored.
    assert!(serde_json::from_str::<Options>(r#"{"order":"hilbert","sort":true}"#).is_err());
    let parsed: Options = serde_json::from_str(r#"{"compression":"zstd"}"#).unwrap();
    assert_eq!(parsed.compression, Codec::Zstd);
}

#[test]
fn per_layer_identity_covers_faces_and_named_areas() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("identity.parquet");
    // Faces: (mesh_name, face_id); areas: mesh_name alone.
    let rename = |batch: RecordBatch, keep_local: Option<&str>| {
        let schema = batch.schema();
        let mut fields = Vec::new();
        let mut columns = Vec::new();
        for (f, c) in schema.fields().iter().zip(batch.columns()) {
            match (f.name().as_str(), keep_local) {
                ("cell_id", None) => continue,
                ("cell_id", Some(name)) => fields.push(f.as_ref().clone().with_name(name)),
                _ => fields.push(f.as_ref().clone()),
            }
            columns.push(c.clone());
        }
        RecordBatch::try_new(
            Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone())),
            columns,
        )
        .unwrap()
    };
    let faces = rename(
        layer("crs", None, vec![Some(point(1., 1.)), Some(point(2., 2.))]),
        Some("face_id"),
    );
    let areas = rename(layer("crs", None, vec![Some(square(0., 0., 4.))]), None);
    let mut options = archive();
    options.identity.insert(
        "mesh_faces".into(),
        Identity {
            namespace: "mesh_name".into(),
            local: Some("face_id".into()),
        },
    );
    options.identity.insert(
        "mesh_areas".into(),
        Identity {
            namespace: "mesh_name".into(),
            local: None,
        },
    );
    // The default identity refuses a layer without cell_id.
    assert!(export_layers(
        vec![input("mesh_faces", faces.clone())],
        &output,
        &archive()
    )
    .is_err());
    export_layers(
        vec![
            input("mesh_faces", faces.clone()),
            input("mesh_areas", areas.clone()),
        ],
        &output,
        &options,
    )
    .unwrap();
    let (batch, _, _) = read(&output);
    assert_eq!(batch.num_rows(), 3);
    assert_eq!(batch.column_by_name("face_id").unwrap().null_count(), 1);
    std::fs::remove_file(&output).unwrap();
    // A namespace-only identity must be unique per row.
    let twice = rename(
        layer("crs", None, vec![Some(point(1., 1.)), Some(point(2., 2.))]),
        None,
    );
    let error = export_layers(vec![input("mesh_areas", twice)], &output, &options).unwrap_err();
    assert!(error.to_string().contains("identity"), "{error}");
    // Identity entries for layers that are not supplied are refused, not ignored.
    let error = export_layers(vec![input("mesh_faces", faces)], &output, &options).unwrap_err();
    assert!(error.to_string().contains("unknown layer"), "{error}");
    assert!(!output.exists());
}

#[test]
fn struct_covering_follows_the_specification_layout() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("struct.parquet");
    let cells = layer(
        "crs",
        None,
        vec![Some(square(10., 20., 4.)), None, Some(point(1., 2.))],
    );
    export_layers(vec![input("mesh_cells", cells)], &output, &archive()).unwrap();
    let (batch, geo, _) = read(&output);
    assert_eq!(
        geo["columns"]["geometry"]["covering"]["bbox"],
        serde_json::json!({"xmin":["bbox","xmin"],"ymin":["bbox","ymin"],
                           "xmax":["bbox","xmax"],"ymax":["bbox","ymax"]})
    );
    let bbox = batch.column_by_name("bbox").unwrap();
    let bbox = bbox
        .as_any()
        .downcast_ref::<arrow_array::StructArray>()
        .unwrap();
    // Hilbert order puts the point (centre 1, 2) first and the null last.
    assert_eq!(ids(&batch), vec![2, 0, 1]);
    assert!(bbox.is_null(2) && bbox.is_valid(0));
    let xmax = bbox.column_by_name("xmax").unwrap();
    let xmax = xmax.as_any().downcast_ref::<Float64Array>().unwrap();
    assert_eq!((xmax.value(0), xmax.value(1)), (1., 14.));
    assert!(batch.column_by_name("bbox_xmin").is_none());
}

fn line(points: &[(f64, f64)]) -> Vec<u8> {
    let mut b = vec![1, 2, 0, 0, 0];
    b.extend((points.len() as u32).to_le_bytes());
    for (x, y) in points {
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
    }
    b
}

fn multipolygon(polygons: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![1, 6, 0, 0, 0];
    b.extend((polygons.len() as u32).to_le_bytes());
    for p in polygons {
        b.extend(p);
    }
    b
}

/// Single layer of three points carrying the given extra columns.
fn typed_layer(extra: Vec<(&str, ArrayRef)>) -> RecordBatch {
    let mut batch = layer(
        "crs",
        None,
        vec![
            Some(point(0., 0.)),
            Some(point(1., 1.)),
            Some(point(2., 2.)),
        ],
    );
    for (name, array) in extra {
        let mut fields: Vec<Field> = batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.as_ref().clone())
            .collect();
        let mut columns = batch.columns().to_vec();
        fields.push(Field::new(name, array.data_type().clone(), true));
        columns.push(array);
        batch = RecordBatch::try_new(
            Arc::new(Schema::new_with_metadata(fields, crs("crs"))),
            columns,
        )
        .unwrap();
    }
    batch
}

/// `struct<Mann n: list<double>, Station: list<double>>` whose null row hides
/// a NaN, and a `list<double>` whose null row references a NaN.
fn hidden_nonfinite() -> (ArrayRef, ArrayRef) {
    use arrow_array::types::Float64Type;
    let list = |rows: Vec<Option<Vec<Option<f64>>>>| -> ArrayRef {
        Arc::new(ListArray::from_iter_primitive::<Float64Type, _, _>(rows))
    };
    let fields = vec![
        Field::new("Mann n", DataType::new_list(DataType::Float64, true), true),
        Field::new("Station", DataType::new_list(DataType::Float64, true), true),
    ];
    let mannings_n = arrow_array::StructArray::try_new(
        fields.into(),
        vec![
            list(vec![
                Some(vec![Some(0.035), Some(0.06)]),
                Some(vec![Some(f64::NAN)]),
                Some(vec![]),
            ]),
            list(vec![Some(vec![Some(0.), Some(120.5)]), None, Some(vec![])]),
        ],
        Some(arrow_buffer::NullBuffer::from(vec![true, false, true])),
    )
    .unwrap();
    // Offsets 0..1, 1..2 (null slot over the NaN), 2..3.
    let masked = ListArray::try_new(
        Arc::new(Field::new("item", DataType::Float64, true)),
        arrow_buffer::OffsetBuffer::new(vec![0, 1, 2, 3].into()),
        Arc::new(Float64Array::from(vec![1., f64::NAN, 3.])),
        Some(arrow_buffer::NullBuffer::from(vec![true, false, true])),
    )
    .unwrap();
    (Arc::new(mannings_n), Arc::new(masked))
}

#[test]
fn nonfinite_under_null_slots_is_not_data_and_refusals_name_the_field() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("hidden.parquet");
    let (mannings_n, masked) = hidden_nonfinite();
    let batch = typed_layer(vec![("mannings_n", mannings_n), ("masked", masked)]);
    export_layers(vec![input("xs", batch.clone())], &output, &archive()).unwrap();
    let (read_back, _, _) = read(&output);
    for name in ["mannings_n", "masked"] {
        let actual = read_back.column_by_name(name).unwrap();
        let expected = batch.column_by_name(name).unwrap();
        assert_eq!(actual.nulls(), expected.nulls(), "{name}");
    }
    // The same NaN in a valid slot refuses.
    let output = temp.path().join("refused.parquet");
    let visible: ArrayRef = Arc::new(ListArray::from_iter_primitive::<
        arrow_array::types::Float64Type,
        _,
        _,
    >(vec![
        Some(vec![Some(1.)]),
        Some(vec![Some(f64::NAN)]),
        None,
    ]));
    let error = export_layers(
        vec![input("xs", typed_layer(vec![("v", visible)]))],
        &output,
        &archive(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("nonfinite"), "{error}");
    let error = export_layers(
        vec![input(
            "xs",
            typed_layer(vec![(
                "v",
                Arc::new(arrow_array::Int32Array::from(vec![1, 2, 3])),
            )]),
        )],
        &output,
        &archive(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported field type for field v: Int32"),
        "{error}"
    );
    assert_eq!(temp.path().read_dir().unwrap().count(), 1);
}

#[test]
fn geometry_types_are_sorted_and_output_is_byte_reproducible() {
    let temp = tempfile::tempdir().unwrap();
    let make = || {
        vec![
            input(
                "mixed",
                layer(
                    "crs",
                    None,
                    vec![
                        Some(square(0., 0., 4.)),
                        Some(point(1., 1.)),
                        Some(line(&[(0., 0.), (3., 3.)])),
                        Some(multipolygon(&[square(10., 10., 2.)])),
                    ],
                ),
            ),
            input("points", layer("crs", None, vec![Some(point(7., 9.))])),
        ]
    };
    let mut first: Option<Vec<u8>> = None;
    for run in 0..8 {
        let output = temp.path().join(format!("run{run}.parquet"));
        export_layers(make(), &output, &archive()).unwrap();
        let (_, geo, _) = read(&output);
        assert_eq!(
            geo["columns"]["geometry"]["geometry_types"],
            serde_json::json!(["LineString", "MultiPolygon", "Point", "Polygon"])
        );
        let bytes = std::fs::read(&output).unwrap();
        match &first {
            None => first = Some(bytes),
            Some(first) => assert!(first == &bytes, "run {run} differs"),
        }
    }
}

#[test]
fn contract_metadata_is_dropped_unless_requested() {
    let temp = tempfile::tempdir().unwrap();
    let cells = || layer("crs", None, vec![Some(point(1., 1.))]);
    let metadata = |path: &Path| {
        ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap())
            .unwrap()
            .schema()
            .metadata()
            .clone()
    };
    let plain = temp.path().join("plain.parquet");
    export_layers(vec![input("a", cells())], &plain, &archive()).unwrap();
    assert!(!metadata(&plain).contains_key(CONTRACT_KEY));
    let kept = temp.path().join("kept.parquet");
    let options = Options {
        contract_metadata: true,
        ..archive()
    };
    export_layers(vec![input("a", cells())], &kept, &options).unwrap();
    assert_eq!(
        metadata(&kept).get(CONTRACT_KEY),
        crs("crs").get(CONTRACT_KEY)
    );
    let parsed: Options = serde_json::from_str(r#"{"contract_metadata":true}"#).unwrap();
    assert!(parsed.contract_metadata);
}

#[test]
fn layer_row_groups_can_span_layers() {
    let temp = tempfile::tempdir().unwrap();
    let layers = || {
        vec![
            input(
                "cells",
                layer(
                    "crs",
                    None,
                    vec![
                        Some(square(100., 100., 8.)),
                        Some(square(0., 0., 4.)),
                        Some(square(50., 50., 2.)),
                    ],
                ),
            ),
            input(
                "points",
                layer("crs", None, vec![Some(point(7., 9.)), None]),
            ),
        ]
    };
    let group_rows = |path: &Path| {
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap()).unwrap();
        builder
            .metadata()
            .row_groups()
            .iter()
            .map(|g| g.num_rows())
            .collect::<Vec<_>>()
    };
    let split = temp.path().join("split.parquet");
    let options = Options {
        row_group_rows: 4,
        ..archive()
    };
    let metrics = export_layers(layers(), &split, &options).unwrap();
    assert_eq!(metrics.row_groups, 2);
    assert_eq!(group_rows(&split), vec![3, 2]);
    let packed = temp.path().join("packed.parquet");
    let options = Options {
        row_group_rows: 4,
        layer_row_groups: false,
        ..archive()
    };
    let metrics = export_layers(layers(), &packed, &options).unwrap();
    assert_eq!(metrics.row_groups, 2);
    assert_eq!(group_rows(&packed), vec![4, 1]);
    // Same rows in the same order either way.
    assert_eq!(read(&split).0, read(&packed).0);
    assert!(Options::default().layer_row_groups);
    let parsed: Options = serde_json::from_str(r#"{"layer_row_groups":false}"#).unwrap();
    assert!(!parsed.layer_row_groups);
}

#[test]
fn hilbert_column_holds_the_sort_key() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("keys.parquet");
    let cells = layer(
        "crs",
        None,
        vec![
            Some(square(100., 100., 8.)),
            None,
            Some(square(0., 0., 4.)),
            Some(square(50., 50., 2.)),
        ],
    );
    let points = layer("crs", None, vec![Some(point(7., 9.))]);
    let options = Options {
        row_group_rows: 2,
        hilbert_column: Some("hilbert".into()),
        ..archive()
    };
    export_layers(
        vec![input("cells", cells), input("points", points)],
        &output,
        &options,
    )
    .unwrap();
    let (batch, _, _) = read(&output);
    let names: Vec<_> = batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    assert_eq!(names.last().unwrap(), "hilbert");
    assert_eq!(names[names.len() - 2], "bbox");
    let field = batch.schema().field_with_name("hilbert").unwrap().clone();
    assert_eq!(field.data_type(), &DataType::UInt64);
    assert!(field.is_nullable());
    let keys: Vec<Option<u64>> = batch
        .column_by_name("hilbert")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::UInt64Array>()
        .unwrap()
        .iter()
        .collect();
    let extent = [0., 0., 108., 108.];
    let key = |x: f64, y: f64| Some(u64::from(hilbert_index(x, y, extent)));
    // Hilbert order of the cells, the null geometry last, then the single
    // point keyed over its own (degenerate) extent.
    assert_eq!(ids(&batch), vec![2, 3, 0, 1, 0]);
    assert_eq!(
        keys,
        vec![
            key(2., 2.),
            key(51., 51.),
            key(104., 104.),
            None,
            Some(u64::from(hilbert_index(7., 9., [7., 9., 7., 9.])))
        ]
    );
    assert!(keys[..3].windows(2).all(|w| w[0] <= w[1]));
    // An empty name refuses (order and collision refusals:
    // archive_types_tests.rs).
    let target = temp.path().join("refused.parquet");
    let options = Options {
        hilbert_column: Some(String::new()),
        ..archive()
    };
    let error = export_layers(
        vec![input("a", layer("crs", None, vec![Some(point(1., 1.))]))],
        &target,
        &options,
    )
    .unwrap_err();
    assert!(error.to_string().contains("nonempty"), "{error}");
    assert!(!target.exists());
}

#[test]
fn file_bbox_ignores_empty_geometry_and_schema_carries_geo() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("empty.parquet");
    let empty_polygon = vec![1, 3, 0, 0, 0, 0, 0, 0, 0];
    let cells = layer(
        "crs",
        None,
        vec![Some(square(0., 0., 4.)), Some(empty_polygon), None],
    );
    let points = layer("crs", None, vec![Some(point(-2., 9.))]);
    // Input order, no covering: the file bbox is still computed.
    let options = Options {
        layer_column: Some("layer".into()),
        ..Options::default()
    };
    export_layers(
        vec![input("cells", cells), input("points", points)],
        &output,
        &options,
    )
    .unwrap();
    let (_, geo, _) = read(&output);
    assert_eq!(
        geo["columns"]["geometry"]["bbox"],
        serde_json::json!([-2., 0., 4., 9.])
    );
    // The embedded Arrow schema carries the same `geo` metadata.
    let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&output).unwrap()).unwrap();
    let embedded: serde_json::Value =
        serde_json::from_str(&builder.schema().metadata()["geo"]).unwrap();
    assert_eq!(embedded, geo);
    // No geometry at all: no file bbox rather than NaN.
    let nothing = temp.path().join("nothing.parquet");
    export_layers(
        vec![input("a", layer("crs", None, vec![None]))],
        &nothing,
        &Options::default(),
    )
    .unwrap();
    assert!(read(&nothing).1["columns"]["geometry"]
        .get("bbox")
        .is_none());
}
