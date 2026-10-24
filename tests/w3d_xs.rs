#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{hdf::ReadOnlyHdfSession, Capability, ErrorKind};
fn file(n: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/w3c")
            .join(n),
    )
    .unwrap()
}
#[test]
fn htab_identity_probe_rejects_malformed_station_rank_without_panicking() {
    use ras_commander_fast_hdf::hdf::xs_geometry::XsGeometryField;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    for name in ["scalar", "vector", "rank3"] {
        let s = file(&format!("htab_station_{name}.h5"));
        let inv = catch_unwind(AssertUnwindSafe(|| s.xs_geometry_inventory(1 << 20)))
            .expect("XS inventory must not unwind")
            .unwrap();
        let station = inv
            .fields
            .iter()
            .find(|d| d.field == XsGeometryField::StationElevation)
            .unwrap();
        assert!(matches!(station.capability, Capability::Corrupt { .. }));
        let read = catch_unwind(AssertUnwindSafe(|| {
            s.read_xs_geometry(XsGeometryField::StationElevation, None, 1 << 20, 1 << 20)
        }))
        .expect("requested malformed station read must not unwind")
        .unwrap_err();
        assert_eq!(read.kind, ErrorKind::Corrupt);
        assert!(s.read_numeric(&station.values_path, None).is_ok());
        let htab = inv
            .fields
            .iter()
            .find(|d| d.field == XsGeometryField::PropertyTable)
            .unwrap();
        assert!(htab.capability.is_supported());
        assert_eq!(htab.section_records, 0);
        assert!(!htab.records_are_sections);
        assert!(s
            .read_xs_geometry(XsGeometryField::PropertyTable, None, 1 << 20, 1 << 20)
            .is_ok());
    }
}
