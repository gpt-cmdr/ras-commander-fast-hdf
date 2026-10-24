#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::hdf::ResultMode;
use ras_commander_fast_hdf::hdf::DEFAULT_MAX_BYTES;
use ras_commander_fast_hdf::schema::MeshNameIssue;
use ras_commander_fast_hdf::{hdf::ReadOnlyHdfSession, Capability, ErrorKind};
fn session(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/deferred")
            .join(format!("{name}.h5")),
    )
    .unwrap()
}
#[test]
fn structured_names_fail_closed_before_counts() {
    for (case, issue) in [
        ("missing", MeshNameIssue::MissingGroup),
        ("extra", MeshNameIssue::ExtraGroup),
        ("duplicate", MeshNameIssue::DuplicateAttributeName),
    ] {
        let s = session(case);
        let d = s.mesh_name_diagnostics(DEFAULT_MAX_BYTES).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].issue, issue);
        assert_eq!(
            s.mesh_inventory(DEFAULT_MAX_BYTES).unwrap_err().kind,
            ErrorKind::Corrupt
        );
    }
}
#[test]
fn every_block_and_explicit_steady_state_is_reported() {
    let meshes = session("blocks").mesh_inventory(DEFAULT_MAX_BYTES).unwrap();
    let m = &meshes[0];
    assert_eq!((m.physical_cells, m.stored_cells, m.ghost_cells), (2, 3, 1));
    let blocks = m
        .output_blocks
        .iter()
        .filter(|b| b.mode == ResultMode::Unsteady)
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 2);
    assert!(blocks
        .iter()
        .all(|b| b.results.capability.is_supported() && b.results.variables.len() == 1));
    assert!(m.results.capability.is_supported());
    let m = session("steady")
        .mesh_inventory(DEFAULT_MAX_BYTES)
        .unwrap()
        .remove(0);
    assert!(matches!(
        m.results.capability,
        Capability::Unavailable { .. }
    ));
    assert!(m
        .output_blocks
        .iter()
        .any(|b| b.mode == ResultMode::Steady && b.results.capability.is_supported()));
    let m = session("steady-unknown")
        .mesh_inventory(DEFAULT_MAX_BYTES)
        .unwrap()
        .remove(0);
    assert!(m.output_blocks.iter().any(|b| b.mode == ResultMode::Steady
        && matches!(b.results.capability, Capability::Unsupported { .. })));
    let m = session("wrong-block")
        .mesh_inventory(DEFAULT_MAX_BYTES)
        .unwrap()
        .remove(0);
    assert!(matches!(
        m.output_blocks[0].results.capability,
        Capability::Corrupt { .. }
    ));
    assert_eq!(
        session("wrong-family")
            .mesh_inventory(DEFAULT_MAX_BYTES)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
}
#[test]
#[allow(deprecated)] // Explicit alias-compatibility check only.
fn deprecated_alias_is_exact() {
    let s = session("blocks");
    assert_eq!(
        s.initial_2d_inventory().unwrap(),
        s.mesh_inventory(DEFAULT_MAX_BYTES).unwrap()
    );
}
#[test]
#[cfg(feature = "results")]
fn reduction_returns_source_chunk_shape_and_request_ieee_equality() {
    use ras_commander_fast_hdf::results::{Accumulator, ReductionRequest};
    let s = session("numeric");
    for (path, shape) in [("chunked", Some(vec![2, 4])), ("contiguous", None)] {
        let r = s.reduce_temporal(path, ReductionRequest::new()).unwrap();
        assert_eq!(r.chunk_shape, shape);
        assert_eq!(r.values.maximum, vec![8., 9., 10., 11.]);
    }
    let a = ReductionRequest::new();
    assert_eq!(a, a.clone());
    let nan = ReductionRequest::new().with_offset(Some(vec![f32::NAN]));
    assert_ne!(nan, nan.clone());
    assert!(format!("{:?}", Accumulator::new(1, None, None).unwrap()).contains("Accumulator"));
}

#[test]
fn absent_unsteady_blocks_and_empty_inventory_are_explicit() {
    let m = session("unsteady-missing")
        .mesh_inventory(DEFAULT_MAX_BYTES)
        .unwrap()
        .remove(0);
    assert!(m
        .output_blocks
        .iter()
        .filter(|b| b.mode == ResultMode::Unsteady)
        .all(|b| matches!(b.results.capability, Capability::Unavailable { .. })));
    assert_eq!(
        session("empty")
            .mesh_inventory(DEFAULT_MAX_BYTES)
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    assert_eq!(
        session("empty").mesh_names().unwrap_err().kind,
        ErrorKind::Unsupported
    );
    assert_eq!(
        session("wrong-root")
            .mesh_inventory(DEFAULT_MAX_BYTES)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
}
