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
