//! Canonical Arrow layers (feature `arrow`) on the synthetic fixture written by
//! `tests/generate_arrow_layers_fixture.py`. Equivalence with ras-commander/ras2cng on real
//! HEC-RAS models is checked outside CI (see docs/reference/arrow-layers.md).
#![cfg(feature = "arrow")]
use arrow_array::cast::AsArray;
use arrow_array::types::{
    Float32Type, Int64Type, TimestampMicrosecondType, TimestampNanosecondType,
};
use arrow_array::Array;
use arrow_schema::{DataType, TimeUnit};
use ras_commander_fast_hdf::arrow_layers::{self, LayerOptions};
use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;
use ras_commander_fast_hdf::{ErrorKind, UnsupportedCode};
use sha2::{Digest, Sha256};

const SHA256: &str = "70c1bad4f2866b9ff7ddcaa127eee18c33ba7160c0469f8939e332911c15f037";
const DAY_NS: i64 = 86_400_000_000_000;
const START_NS: i64 = 18_262 * DAY_NS; // 2020-01-01

fn session() -> ReadOnlyHdfSession {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arrow_layers/arrow_layers.h5");
    let bytes = std::fs::read(&path).unwrap();
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        digest, SHA256,
        "regenerate with tests/generate_arrow_layers_fixture.py"
    );
    ReadOnlyHdfSession::open(path).unwrap()
}

fn coords(wkb: &[u8]) -> (u32, Vec<[f64; 2]>) {
    assert_eq!(wkb[0], 1, "little endian");
    let kind = u32::from_le_bytes(wkb[1..5].try_into().unwrap());
    let (count_at, start) = if kind == 3 {
        assert_eq!(
            u32::from_le_bytes(wkb[5..9].try_into().unwrap()),
            1,
            "one ring"
        );
        (9, 13)
    } else {
        (5, 9)
    };
    let n = u32::from_le_bytes(wkb[count_at..count_at + 4].try_into().unwrap()) as usize;
    assert_eq!(wkb.len(), start + 16 * n, "no trailing bytes");
    let f = |o: usize| f64::from_le_bytes(wkb[o..o + 8].try_into().unwrap());
    (
        kind,
        (0..n)
            .map(|i| [f(start + 16 * i), f(start + 16 * i + 8)])
            .collect(),
    )
}

#[test]
fn mesh_cells_are_closed_engine_rings_without_ghosts() {
    let layer = session()
        .mesh_cells_layer(&LayerOptions::default())
        .unwrap();
    assert_eq!(layer.name, "mesh_cells");
    assert_eq!(layer.identity, ("mesh_name".into(), Some("cell_id".into())));
    let b = &layer.batches[0];
    assert_eq!(b.num_rows(), 2, "the one-face ghost cell is not a polygon");
    assert_eq!(b.column(1).as_primitive::<Int64Type>().values(), &[0, 1]);
    let field = layer.schema.field_with_name("geometry").unwrap();
    assert_eq!(field.metadata()["ARROW:extension:name"], "geoarrow.wkb");
    for row in 0..2 {
        let (kind, ring) = coords(b.column(2).as_binary::<i32>().value(row));
        assert_eq!(kind, 3);
        assert_eq!(ring.first(), ring.last(), "closed");
    }
    let (_, curved) = coords(b.column(2).as_binary::<i32>().value(1));
    assert!(
        curved.contains(&[1.5, -0.25]),
        "perimeter point of the curved face kept"
    );
}

#[test]
fn faces_and_perimeters_follow_stored_order() {
    let s = session();
    let faces = s.mesh_faces_layer(&LayerOptions::default()).unwrap();
    let b = &faces.batches[0];
    assert_eq!(b.num_rows(), 7);
    let (kind, line) = coords(b.column(2).as_binary::<i32>().value(1));
    assert_eq!(kind, 2);
    assert_eq!(line, vec![[1.0, 0.0], [1.5, -0.25], [2.0, 0.0]]);
    let areas = s.mesh_perimeters_layer(&LayerOptions::default()).unwrap();
    let (_, ring) = coords(areas.batches[0].column(1).as_binary::<i32>().value(0));
    assert_eq!(ring, vec![[0., 0.], [2., 0.], [2., 1.], [0., 1.], [0., 0.]]);
    assert_eq!(
        arrow_layers::lookup("mesh_areas").unwrap().name,
        "mesh_perimeters"
    );
}

#[test]
fn small_batches_split_rows() {
    let o = LayerOptions::default().with_batch_rows(3);
    let faces = session().mesh_faces_layer(&o).unwrap();
    let sizes: Vec<_> = faces.batches.iter().map(|b| b.num_rows()).collect();
    assert_eq!(sizes, vec![3, 3, 1]);
}

#[test]
fn cell_summary_drops_ghosts_nulls_nan_and_times_values() {
    let layer = session()
        .mesh_summary_layer("Maximum Water Surface", &LayerOptions::default())
        .unwrap();
    assert_eq!(layer.name, "maximum_water_surface");
    let names: Vec<_> = layer
        .schema
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert_eq!(
        names,
        [
            "mesh_name",
            "cell_id",
            "maximum_water_surface",
            "maximum_water_surface_time",
            "is_ghost_cell"
        ]
    );
    let b = &layer.batches[0];
    assert_eq!(b.num_rows(), 2, "ghost cell 2 (Cell Count 2) left out");
    let v = b.column(2).as_primitive::<Float32Type>();
    assert_eq!((v.value(0), v.is_null(1)), (10.0, true), "NaN is null");
    assert_eq!(
        b.schema().field(3).data_type(),
        &DataType::Timestamp(TimeUnit::Nanosecond, None)
    );
    let t = b.column(3).as_primitive::<TimestampNanosecondType>();
    assert_eq!(
        t.values(),
        &[START_NS + DAY_NS / 2, START_NS + DAY_NS + DAY_NS / 4]
    );
    assert_eq!(b.column(4).as_boolean().true_count(), 0);
}

#[test]
fn whole_day_times_are_microseconds_like_pandas() {
    let layer = session()
        .mesh_summary_layer("Maximum Face Velocity", &LayerOptions::default())
        .unwrap();
    let b = &layer.batches[0];
    assert_eq!(b.num_rows(), 7);
    assert_eq!(layer.identity.1.as_deref(), Some("face_id"));
    assert_eq!(
        b.schema().field(3).data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, None)
    );
    let t = b.column(3).as_primitive::<TimestampMicrosecondType>();
    assert_eq!(t.value(0), (START_NS + DAY_NS) / 1000);
    assert_eq!(b.schema().fields().len(), 4, "faces carry no ghost flag");
}

#[test]
fn untimed_summary_and_refusals() {
    let s = session();
    let it = s
        .mesh_summary_layer("Cell Cumulative Iteration", &LayerOptions::default())
        .unwrap();
    assert_eq!(it.schema.fields().len(), 4);
    let e = s
        .mesh_summary_layer("Starting Differences WSE", &LayerOptions::default())
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    let e = s
        .mesh_summary_layer("Not Stored", &LayerOptions::default())
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    let e = s
        .mesh_summary_layer("Tiefe ü", &LayerOptions::default())
        .unwrap_err();
    assert_eq!(e.code, Some(UnsupportedCode::Encoding));
    assert_eq!(s.simulation_start_ns().unwrap(), START_NS);
}

const FEATURES_SHA256: &str = "d525ad01411278a61c2937c093032d6fc9dc1ec3f6896df175105a60dcfc9138";

fn features() -> ReadOnlyHdfSession {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arrow_layers/arrow_features.h5");
    let digest: String = Sha256::digest(std::fs::read(&path).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        digest, FEATURES_SHA256,
        "regenerate with tests/generate_arrow_features_fixture.py"
    );
    ReadOnlyHdfSession::open(path).unwrap()
}

fn field_names(layer: &arrow_layers::ArrowLayer) -> Vec<String> {
    layer
        .schema
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect()
}

#[test]
fn bc_lines_keep_stored_columns_and_multipart_lines() {
    let layer = features()
        .read_layer("bc_lines", None, &LayerOptions::default())
        .unwrap();
    assert_eq!(layer.identity, ("source_row".into(), None));
    assert_eq!(
        field_names(&layer),
        ["source_row", "Name", "SA-2D", "Type", "geometry"]
    );
    let b = &layer.batches[0];
    assert_eq!(b.column(0).as_primitive::<Int64Type>().values(), &[0, 1]);
    assert_eq!(b.column(1).as_string::<i32>().value(1), "Split");
    let g = b.column(4).as_binary::<i32>();
    assert_eq!(coords(g.value(0)), (2, vec![[0.0, 0.0], [1.0, 0.0]]));
    // Two parts: MultiLineString of two LineStrings.
    let m = g.value(1);
    assert_eq!(&m[..9], &[1, 5, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(coords(&m[9..9 + 41]), (2, vec![[5.0, 5.0], [6.0, 5.0]]));
    assert_eq!(
        coords(&m[50..]),
        (2, vec![[7.0, 7.0], [8.0, 7.0], [9.0, 8.0]])
    );
}

#[test]
fn storage_areas_close_rings_keep_stored_types_and_null_empty_features() {
    let layer = features()
        .read_layer("storage_areas", None, &LayerOptions::default())
        .unwrap();
    assert_eq!(
        field_names(&layer),
        [
            "source_row",
            "Name",
            "Mode",
            "Avg Area",
            "Min Elev",
            "Elev Vol Count",
            "geometry"
        ]
    );
    let b = &layer.batches[0];
    assert_eq!(b.schema().field(3).data_type(), &DataType::Float32);
    assert_eq!(b.schema().field(5).data_type(), &DataType::Int64);
    assert!(b.column(4).is_null(0), "NaN is null");
    let g = b.column(6).as_binary::<i32>();
    assert_eq!(
        coords(g.value(0)),
        (3, vec![[0., 0.], [4., 0.], [0., 3.], [0., 0.]])
    );
    assert!(g.is_null(1), "no points, one empty part: null geometry");
}

#[test]
fn points_aliases_and_refusals_of_feature_layers() {
    let s = features();
    let o = LayerOptions::default();
    let pumps = s.read_layer("pump_stations", None, &o).unwrap();
    let b = &pumps.batches[0];
    assert_eq!(b.schema().field(2).data_type(), &DataType::UInt64);
    let w = b.column(3).as_binary::<i32>().value(1);
    assert_eq!(&w[..5], &[1, 1, 0, 0, 0]);
    assert_eq!(f64::from_le_bytes(w[5..13].try_into().unwrap()), 30.0);
    let e = s.read_layer("refinement_regions", None, &o).unwrap_err();
    assert_eq!(e.code, Some(UnsupportedCode::NotImplemented), "two rings");
    let e = s.read_layer("breaklines", None, &o).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    let e = s.read_layer("centerlines", None, &o).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound, "alias resolves; not stored");
    let e = s.read_layer("nope", None, &o).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    let aliases: Vec<_> = arrow_layers::field_aliases("ras2cng", "bc_lines")
        .map(|a| (a.canonical, a.alias))
        .collect();
    assert_eq!(aliases, [("source_row", "bc_line_id")]);
    assert_eq!(
        arrow_layers::lookup("river_reaches").unwrap().name,
        "river_centerlines"
    );
}

#[test]
fn cell_points_are_physical_cell_centres() {
    let layer = session()
        .read_layer("mesh_cell_points", None, &LayerOptions::default())
        .unwrap();
    let b = &layer.batches[0];
    assert_eq!(b.num_rows(), 2, "ghost centre left out");
    let w = b.column(2).as_binary::<i32>().value(1);
    assert_eq!(f64::from_le_bytes(w[5..13].try_into().unwrap()), 1.5);
}

#[test]
fn summary_geometry_joins_cells_and_faces() {
    let s = session();
    let o = LayerOptions::default().with_summary_geometry(true);
    let cells = s
        .read_layer("mesh_summary", Some("Maximum Water Surface"), &o)
        .unwrap();
    assert_eq!(field_names(&cells).last().unwrap(), "geometry");
    let plain = s.mesh_cells_layer(&LayerOptions::default()).unwrap();
    assert_eq!(
        cells.batches[0].column(5).as_binary::<i32>(),
        plain.batches[0].column(2).as_binary::<i32>()
    );
    let faces = s
        .read_layer("mesh_summary", Some("Maximum Face Velocity"), &o)
        .unwrap();
    let (_, line) = coords(faces.batches[0].column(4).as_binary::<i32>().value(1));
    assert_eq!(line, vec![[1.0, 0.0], [1.5, -0.25], [2.0, 0.0]]);
    assert!(s.read_layer("mesh_summary", None, &o).is_err());
}

#[test]
fn ghost_cell_points_are_kept_and_flagged_on_request() {
    let o = LayerOptions::default().with_ghost_cells(true);
    let layer = session().read_layer("mesh_cell_points", None, &o).unwrap();
    assert_eq!(
        field_names(&layer),
        ["mesh_name", "cell_id", "is_ghost_cell", "geometry"]
    );
    let b = &layer.batches[0];
    assert_eq!(b.num_rows(), 3, "the ghost centre is a stored row");
    let ghost = b.column(2).as_boolean();
    assert_eq!(
        (0..3).map(|i| ghost.value(i)).collect::<Vec<_>>(),
        [false, false, true]
    );
    let w = b.column(3).as_binary::<i32>().value(2);
    assert_eq!(f64::from_le_bytes(w[13..21].try_into().unwrap()), -0.5);
}

#[test]
fn stored_time_offsets_on_request() {
    let o = LayerOptions::default().with_summary_time_days(true);
    let layer = session()
        .mesh_summary_layer("Maximum Water Surface", &o)
        .unwrap();
    let names = field_names(&layer);
    assert_eq!(names[4], "maximum_water_surface_time_days");
    let b = &layer.batches[0];
    let d = b.column(4).as_primitive::<Float32Type>();
    assert_eq!(
        d.values(),
        &[0.5, 1.25],
        "the stored days, not the rounded times"
    );
    // Untimed summaries are unchanged.
    let plain = session()
        .mesh_summary_layer("Maximum Water Surface", &LayerOptions::default())
        .unwrap();
    assert!(!field_names(&plain)
        .iter()
        .any(|n| n.ends_with("_time_days")));
}

#[test]
fn omitted_cells_are_reported() {
    let layer = session()
        .mesh_cells_layer(&LayerOptions::default())
        .unwrap();
    assert_eq!(layer.omitted.len(), 1, "{:?}", layer.omitted);
    assert_eq!(layer.omitted[0].1.cell_id, 2);
}

#[test]
fn consumer_names_come_from_the_catalog() {
    use arrow_layers::{apply_naming_rule, consumer_field_name, NamingRule};
    assert_eq!(
        apply_naming_rule(NamingRule::SnakeCase, "US SA/2D"),
        "us_sa_2d"
    );
    assert_eq!(apply_naming_rule(NamingRule::SnakeCase, "SA-2D"), "sa_2d");
    assert_eq!(apply_naming_rule(NamingRule::Canonical, "SA-2D"), "SA-2D");
    assert_eq!(consumer_field_name("terrio", "bc_lines", "SA-2D"), "sa_2d");
    assert_eq!(
        consumer_field_name("terrio", "bc_lines", "source_row"),
        "source_row"
    );
    assert_eq!(
        consumer_field_name("terrio", "mesh_cell_points", "is_ghost_cell"),
        "is_ghost"
    );
    assert_eq!(
        consumer_field_name("ras2cng", "bc_lines", "source_row"),
        "bc_line_id"
    );
    assert_eq!(consumer_field_name("ras2cng", "bc_lines", "SA-2D"), "SA-2D");

    let s = features();
    let layer = s
        .feature_layer("bc_lines", &LayerOptions::default())
        .unwrap();
    let before = layer.batches[0].column(0).clone();
    let renamed = layer.with_consumer_names("ras2cng", "bc_lines").unwrap();
    assert_eq!(field_names(&renamed)[0], "bc_line_id");
    assert_eq!(renamed.identity.0, "bc_line_id");
    assert!(
        std::sync::Arc::ptr_eq(&before, renamed.batches[0].column(0)),
        "zero copy"
    );
    let geometry = renamed.schema.field_with_name("geometry").unwrap();
    assert_eq!(geometry.metadata()["ARROW:extension:name"], "geoarrow.wkb");
    let terrio = s
        .feature_layer("bc_lines", &LayerOptions::default())
        .unwrap()
        .with_consumer_names("terrio", "bc_lines")
        .unwrap();
    assert!(field_names(&terrio).iter().all(|n| n
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')));
}
