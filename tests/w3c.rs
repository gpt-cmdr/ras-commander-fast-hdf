#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{
    hdf::{families::ReadFamily, mesh_results::ElementDomain, ReadOnlyHdfSession},
    Capability, ErrorKind, UnsupportedCode,
};
fn file(n: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/w3c")
            .join(n),
    )
    .unwrap()
}

#[test]
fn unexpected_summary_ranks_refuse_without_panicking() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    for (name, shape) in [
        ("scalar", vec![]),
        ("rank3", vec![1, 1, 1]),
        ("rank4", vec![1, 1, 1, 1]),
    ] {
        let s = file(&format!("summary_{name}.h5"));
        let inv = catch_unwind(AssertUnwindSafe(|| {
            s.mesh_summary_inventory("Perimeter 1", 1 << 20)
        }))
        .expect("inventory must not unwind")
        .unwrap();
        let d = inv
            .variables
            .iter()
            .find(|v| v.name == "Renamed Evidence")
            .unwrap();
        assert_eq!(d.shape, shape);
        assert!(matches!(
            d.capability,
            Capability::Unsupported {
                code: UnsupportedCode::Schema,
                ..
            }
        ));
        let selection = catch_unwind(AssertUnwindSafe(|| {
            s.read_mesh_summary("Perimeter 1", "Renamed Evidence", None, 1 << 20, 1 << 20)
        }))
        .expect("summary selection must not unwind")
        .unwrap_err();
        let values = catch_unwind(AssertUnwindSafe(|| {
            s.read_mesh_summary_values("Perimeter 1", "Renamed Evidence", None, 1 << 20, 1 << 20)
        }))
        .expect("values-only summary must not unwind")
        .unwrap_err();
        for err in [selection, values] {
            assert_eq!(err.kind, ErrorKind::Unsupported);
            assert_eq!(err.code, Some(UnsupportedCode::Schema));
            assert_eq!(err.message, d.capability.reason().unwrap());
        }
        let raw = s.read_numeric(&d.path, None).unwrap();
        assert_eq!(raw.shape, shape);
        assert_eq!(
            raw.values,
            ras_commander_fast_hdf::hdf::NumericValues::F32(vec![0.0])
        );
        let sibling = s
            .read_mesh_summary("Perimeter 1", "Sibling", None, 1 << 20, 1 << 20)
            .unwrap();
        assert_eq!(
            sibling.values,
            s.read_mesh_summary_values("Perimeter 1", "Sibling", None, 1 << 20, 1 << 20)
                .unwrap()
        );
        let sibling_path = inv.variables.iter().find(|v| v.name == "Sibling").unwrap();
        assert_eq!(
            sibling.values,
            s.read_numeric(&sibling_path.path, None).unwrap()
        );
    }
}
#[test]
fn unexpected_root_polygon_never_panics() {
    let s = file("landcover_root_polygon.h5");
    let inv = s
        .read_family_inventory(ReadFamily::LandCover, 1 << 20)
        .unwrap();
    let d = inv
        .datasets
        .iter()
        .find(|d| d.stored.path == "Polygon Info")
        .unwrap();
    assert!(!d.stored.capability.is_supported());
    assert!(s
        .read_family_dataset(
            ReadFamily::LandCover,
            "Polygon Info",
            None,
            1 << 20,
            1 << 20
        )
        .is_err());
    assert!(s.read_numeric("Polygon Info", None).is_ok());
    assert!(inv
        .datasets
        .iter()
        .find(|d| d.stored.path == "Variables")
        .unwrap()
        .stored
        .capability
        .is_supported());
}

#[test]
fn dynamic_scalar_vector_need_time_orientation() {
    for n in ["sediment_scalar.h5", "sediment_vector.h5"] {
        let s = file(n);
        let inv = s
            .read_family_inventory(ReadFamily::Sediment, 1 << 20)
            .unwrap();
        let p = "Results/Sediment/Output Blocks/Sediment/Sediment Time Series/Cross Sections/Flow";
        assert!(!inv
            .datasets
            .iter()
            .find(|d| d.stored.path == p)
            .unwrap()
            .stored
            .capability
            .is_supported());
        assert_eq!(
            s.read_family_dataset(ReadFamily::Sediment, p, None, 1 << 20, 1 << 20)
                .unwrap_err()
                .kind,
            ErrorKind::Unsupported
        );
        assert!(s.read_numeric(p, None).is_ok());
    }
}
#[test]
fn labelled_summary_requires_exact_evidence_and_preserves_siblings() {
    for n in ["valid", "labels", "missing", "units", "count"] {
        let s = file(&format!("summary_{n}.h5"));
        let inv = s.mesh_summary_inventory("Perimeter 1", 1 << 20).unwrap();
        let d = inv
            .variables
            .iter()
            .find(|v| v.name == "Renamed Evidence")
            .unwrap();
        assert_eq!(d.capability.is_supported(), n == "valid", "{n}");
        assert_eq!(
            s.read_mesh_summary_values("Perimeter 1", "Renamed Evidence", None, 1 << 20, 1 << 20)
                .is_ok(),
            n == "valid"
        );
        if n == "valid" {
            assert_eq!(d.domain, ElementDomain::Cell);
            assert_eq!(d.time_of_extreme_row, Some(1));
        }
        assert!(inv
            .variables
            .iter()
            .find(|v| v.name == "Sibling")
            .unwrap()
            .capability
            .is_supported());
        let p="Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas/Perimeter 1/Renamed Evidence";
        assert!(s.read_numeric(p, None).is_ok());
    }
}
