#![cfg(feature = "hdf")]
//! Exact stored observations, IC points, obstruction blocks and steady XS values.
use ras_commander_fast_hdf::hdf::{
    families::ReadFamily,
    xs_geometry::{XsGeometryField, XsGeometryValues},
    NumericValues, ReadOnlyHdfSession, ResultMode, Slice,
};
use std::path::PathBuf;
const MAX: usize = 64 * 1024 * 1024;
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/data_readers")
        .join(name)
}
#[test]
fn ic_points_keep_source_rows_and_signed_zero() {
    let s = ReadOnlyHdfSession::open(fixture("valid.h5")).unwrap();
    let r = s.initial_condition_point_geometry(MAX).unwrap();
    let v = r.value().unwrap();
    assert_eq!(v.features.len(), 2);
    assert_eq!(v.features[1].id.row, 1);
    assert_eq!(v.features[1].cell_index, 1);
    assert!(v.features[0].id.table.ends_with("IC Points/Attributes"));
    let NumericValues::F64(points) = &v.points.values else {
        panic!("f64")
    };
    assert_eq!(points[1].to_bits(), (-0.0f64).to_bits());
    assert!(s.initial_condition_point_geometry(1).is_err());
    let partial = ReadOnlyHdfSession::open(fixture("partial.h5")).unwrap();
    assert!(partial.initial_condition_point_geometry(MAX).is_err());
}
#[test]
fn obstruction_blocks_are_exact_and_budgeted() {
    let s = ReadOnlyHdfSession::open(fixture("valid.h5")).unwrap();
    let r = s
        .read_xs_geometry(
            XsGeometryField::BlockedObstructions,
            Some(&[1, 0, 1]),
            MAX,
            MAX,
        )
        .unwrap();
    assert_eq!(r.records.len(), 3);
    let XsGeometryValues::Compound(values) = r.values else {
        panic!("compound")
    };
    assert_eq!(values.table.shape, [3]);
    assert!(s
        .read_xs_geometry(XsGeometryField::BlockedObstructions, None, MAX, 1)
        .is_err());
}
#[test]
fn existing_observation_and_steady_readers_cover_new_consumers() {
    let s = ReadOnlyHdfSession::open(fixture("valid.h5")).unwrap();
    let path = "Event Conditions/Observed Data/Flow/Ref Line: Gauge";
    let v = s
        .read_family_dataset(ReadFamily::EventConditions, path, None, MAX, MAX)
        .unwrap();
    assert!(matches!(
        v,
        ras_commander_fast_hdf::hdf::stored::StoredValues::Compound(_)
    ));
    assert!(s
        .read_family_dataset(ReadFamily::EventConditions, path, None, MAX, 1)
        .is_err());
    for name in ["Velocity Total", "Area including Ineffective Total"] {
        let r = s
            .read_xs_result(
                ResultMode::Steady,
                &format!("Additional Variables/{name}"),
                Slice::new(0, 2, 1),
                Some(&[1, 0]),
                MAX,
                MAX,
            )
            .unwrap();
        assert_eq!(r.values.shape, [2, 2]);
        let NumericValues::F32(values) = r.values.values else {
            panic!("f32")
        };
        assert_eq!(values[0].to_bits(), (-0.0f32).to_bits());
    }
}
