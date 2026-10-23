#![cfg(feature = "hdf")]
use ras_commander_rust::{
    hdf::{
        families::ReadFamily,
        mesh_results::{DomainRule, ElementDomain},
        NumericValues, ReadOnlyHdfSession, Slice,
    },
    ErrorKind, UnsupportedCode,
};
fn fixture(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/location")
            .join(name),
    )
    .unwrap()
}
const PIPE:&str="Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Pipe Networks/N/Face Velocity";
#[test]
fn pipe_location_wins_over_names_and_coincident_counts_and_ids_stay_exact() {
    let s = fixture("pipe_valid.h5");
    let inv = s
        .read_family_inventory(ReadFamily::PipeResults, 1 << 20)
        .unwrap();
    let d = inv.datasets.iter().find(|d| d.stored.path == PIPE).unwrap();
    assert!(d.stored.capability.is_supported());
    assert_eq!(d.domain, "node");
    assert_eq!(d.locations.len(), 133);
    assert_eq!(d.locations[0].row, 0);
    assert_eq!(
        d.locations[0].table,
        "Geometry/Pipe Networks/N/Node Indices"
    );
    let ids = d.stored_location_ids.as_ref().unwrap();
    let NumericValues::I32(ids) = &ids.values else {
        panic!("exact stored i32 required")
    };
    assert_eq!(&ids[..3], &[121, 85, 119]);
    let conduit = inv
        .datasets
        .iter()
        .find(|d| d.stored.path.ends_with("Unrelated Quantity"))
        .unwrap();
    assert_eq!(conduit.domain, "conduit");
    assert!(conduit.stored_location_ids.is_some());
    assert!(s
        .read_family_dataset(ReadFamily::PipeResults, PIPE, None, 1 << 20, 1 << 20)
        .is_ok());
}
#[test]
fn pipe_missing_unknown_and_contradictory_location_never_guess_a_domain() {
    for name in [
        "pipe_missing.h5",
        "pipe_unknown.h5",
        "pipe_float_ids.h5",
        "pipe_missing_table.h5",
        "pipe_short_ids.h5",
        "pipe_mismatch.h5",
    ] {
        let s = fixture(name);
        let e = s
            .read_family_dataset(ReadFamily::PipeResults, PIPE, None, 1 << 20, 1 << 20)
            .unwrap_err();
        if matches!(name, "pipe_short_ids.h5" | "pipe_mismatch.h5") {
            assert_eq!(e.kind, ErrorKind::Corrupt, "{name}");
        } else {
            assert_eq!(
                (e.kind, e.code),
                (
                    ErrorKind::Unsupported,
                    Some(UnsupportedCode::IdentityLayout)
                ),
                "{name}"
            );
        }
        assert!(s.read_numeric(PIPE, None).is_ok());
    }
}
#[test]
fn mesh_location_selects_the_domain_even_when_all_counts_coincide() {
    for name in ["mesh_valid.h5", "mesh_singular.h5"] {
        let s = fixture(name);
        let inv = s.mesh_series_inventory("Perimeter 1", 1 << 20).unwrap();
        assert_eq!(
            inv.variables
                .iter()
                .find(|v| v.name == "Face Velocity")
                .unwrap()
                .domain,
            ElementDomain::Cell
        );
        assert_eq!(
            inv.variables
                .iter()
                .find(|v| v.name == "Cell Level")
                .unwrap()
                .domain,
            ElementDomain::Face
        );
        assert!(s
            .read_mesh_series(
                "Perimeter 1",
                "Face Velocity",
                Slice::new(0, 2, 1),
                None,
                1 << 20,
                1 << 20
            )
            .is_ok());
        let summary = s
            .read_mesh_summary(
                "Perimeter 1",
                "Maximum Water Surface",
                None,
                1 << 20,
                1 << 20,
            )
            .unwrap();
        assert_eq!(summary.values.shape[0], 2);
        assert_eq!(
            s.mesh_summary_inventory("Perimeter 1", 1 << 20)
                .unwrap()
                .variables
                .iter()
                .find(|d| d.name == "Maximum Water Surface")
                .unwrap()
                .domain,
            ElementDomain::Cell
        );
    }
}
#[test]
fn absent_location_on_an_exact_legacy_name_uses_the_api9_legacy_rule() {
    // API 9: Face Velocity without Location at the canonical series path is a face series
    // (stable-core legacy table). Counts coincide in this excerpt, so only the rule decides.
    let s = fixture("mesh_missing.h5");
    let inv = s.mesh_series_inventory("Perimeter 1", 1 << 20).unwrap();
    let d = inv
        .variables
        .iter()
        .find(|v| v.name == "Face Velocity")
        .unwrap();
    assert!(d.capability.is_supported());
    assert_eq!(d.domain, ElementDomain::Face);
    assert_eq!(d.domain_rule, DomainRule::LegacyExactName);
}
#[test]
fn mesh_location_refusals_are_per_entry_and_literal_reads_remain_available() {
    for name in ["mesh_unknown.h5", "mesh_mismatch.h5"] {
        let s = fixture(name);
        let inv = s.mesh_series_inventory("Perimeter 1", 1 << 20).unwrap();
        let d = inv
            .variables
            .iter()
            .find(|v| v.name == "Face Velocity")
            .unwrap();
        assert!(!d.capability.is_supported());
        let e = s
            .read_mesh_series(
                "Perimeter 1",
                "Face Velocity",
                Slice::new(0, 2, 1),
                None,
                1 << 20,
                1 << 20,
            )
            .unwrap_err();
        if name == "mesh_mismatch.h5" {
            assert_eq!(e.kind, ErrorKind::Corrupt);
        } else {
            assert_eq!(
                (e.kind, e.code),
                (
                    ErrorKind::Unsupported,
                    Some(UnsupportedCode::IdentityLayout)
                )
            );
        }
        if name != "mesh_mismatch.h5" {
            assert!(s
                .read_mesh_series(
                    "Perimeter 1",
                    "Cell Level",
                    Slice::new(0, 2, 1),
                    None,
                    1 << 20,
                    1 << 20
                )
                .is_ok());
        }
        assert!(s.read_numeric("Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/2D Flow Areas/Perimeter 1/Face Velocity",None).is_ok());
    }
}
