#![cfg(feature = "hdf")]
//! HEC-RAS 5.0 summaries labelled by `Row 0:  WSEL` / `Row 1:  Time` attributes, on excerpts of
//! HEC's public BaldEagleDamBrk.p07.hdf (HEC-RAS 5.0.3; `fixtures/legacy_row_labels`,
//! `generate_legacy_row_labels.py`).
use ras_commander_fast_hdf::hdf::mesh_results::{DomainRule, ElementDomain, MeshResultVariable};
use ras_commander_fast_hdf::hdf::{NumericValues, ReadOnlyHdfSession};
use ras_commander_fast_hdf::{Capability, UnsupportedCode};

const AREA: &str = "BaldEagleCr";

fn session(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy_row_labels")
            .join(name),
    )
    .unwrap()
}

fn variable(s: &ReadOnlyHdfSession, name: &str) -> MeshResultVariable {
    s.mesh_summary_inventory(AREA, 1 << 20)
        .unwrap()
        .variables
        .into_iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("{name} absent"))
}

#[test]
fn the_observed_5_0_layout_qualifies_cells_and_reads_literal_values() {
    let s = session("valid.h5");
    let v = variable(&s, "Maximum Water Surface");
    assert_eq!(v.capability, Capability::Supported);
    assert_eq!(
        (v.domain, v.domain_rule),
        (ElementDomain::Cell, DomainRule::LegacyRowLabels)
    );
    assert_eq!(v.domain_rule.as_str(), "legacy_row_labels");
    assert_eq!(v.time_of_extreme_row, Some(1));
    let typed = s
        .read_mesh_summary_values(AREA, "Maximum Water Surface", None, 1 << 20, 1 << 20)
        .unwrap();
    let literal = s.read_numeric(&v.path, None).unwrap();
    assert_eq!(typed, literal);
    assert_eq!(typed.shape, vec![2, 6]);
    assert!(matches!(typed.values, NumericValues::F32(_)));
}

#[test]
fn any_other_layout_stays_refused() {
    for file in [
        "units_meters.h5",    // SI label never observed in a 5.0 file
        "extra_attribute.h5", // more than the two row labels
        "three_rows.h5",      // not a value/time pair
        "label_case.h5",      // label spelling differs (one space)
    ] {
        let v = variable(&session(file), "Maximum Water Surface");
        assert_eq!(v.domain_rule, DomainRule::Unqualified, "{file}");
        assert_eq!(
            v.capability.code(),
            Some(UnsupportedCode::IdentityLayout),
            "{file}"
        );
        assert_eq!(v.time_of_extreme_row, None, "{file}");
    }
    // Face summaries are not covered by this rule.
    let v = variable(&session("face_velocity.h5"), "Maximum Face Velocity");
    assert_eq!(v.domain_rule, DomainRule::Unqualified);
    assert_eq!(v.capability.code(), Some(UnsupportedCode::IdentityLayout));
}

#[test]
fn a_count_contradiction_is_corrupt() {
    let v = variable(&session("wrong_count.h5"), "Maximum Water Surface");
    assert_eq!(v.domain_rule, DomainRule::LegacyRowLabels);
    assert!(
        matches!(v.capability, Capability::Corrupt { .. }),
        "{:?}",
        v.capability
    );
}

/// Opt-in: `RAS_NATIVE_FIXTURES` holding HEC's BaldEagleDamBrk p07/p08/p19 (HEC-RAS 5.0.3).
/// Values equal the literal dataset, and the stored cell-domain claim holds physically: for every
/// drawn cell the maximum water surface is at or above `Cells Minimum Elevation`, with exact
/// equality on dry cells.
#[cfg(feature = "mesh")]
#[test]
#[ignore]
fn real_5_0_summaries_are_literal_and_physically_cell_aligned() {
    let root = std::path::PathBuf::from(std::env::var("RAS_NATIVE_FIXTURES").unwrap());
    for file in [
        "BaldEagleDamBrk.p07.hdf",
        "BaldEagleDamBrk.p08.hdf",
        "BaldEagleDamBrk.p19.hdf",
    ] {
        let s = ReadOnlyHdfSession::open(root.join(file)).unwrap();
        for wse in ["Maximum Water Surface", "Minimum Water Surface"] {
            let v = variable(&s, wse);
            assert_eq!(
                (v.capability.clone(), v.domain_rule),
                (Capability::Supported, DomainRule::LegacyRowLabels),
                "{file} {wse}"
            );
            let typed = s
                .read_mesh_summary_values(AREA, wse, None, 1 << 26, 1 << 26)
                .unwrap();
            assert_eq!(
                typed,
                s.read_numeric(&v.path, None).unwrap(),
                "{file} {wse}"
            );
        }
        let NumericValues::F32(max) = s
            .read_mesh_summary_values(AREA, "Maximum Water Surface", None, 1 << 26, 1 << 26)
            .unwrap()
            .values
        else {
            panic!()
        };
        let min_elev = s
            .read_numeric(
                &format!("Geometry/2D Flow Areas/{AREA}/Cells Minimum Elevation"),
                None,
            )
            .unwrap();
        let NumericValues::F32(min_elev) = min_elev.values else {
            panic!()
        };
        let drawn: Vec<i64> = s
            .read_topology(AREA)
            .unwrap()
            .topology
            .assemble(ras_commander_fast_hdf::mesh::GeometryPolicy::Strict, 1)
            .unwrap()
            .cell_ids;
        let (mut dry, mut wet) = (0, 0);
        for &c in &drawn {
            let (w, z) = (max[c as usize], min_elev[c as usize]);
            assert!(w >= z, "{file} cell {c}: {w} < {z}");
            if w == z {
                dry += 1
            } else {
                wet += 1
            }
        }
        assert!(dry > 1000 && wet > 1000, "{file}: {dry} dry, {wet} wet");
    }
}
