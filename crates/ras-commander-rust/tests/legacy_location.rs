#![cfg(feature = "hdf")]
//! Absent-Location legacy mesh-series rule on real JUL2020_D100RAS.p04 (HEC-RAS 5.0.7) D601
//! excerpts (`generate_legacy_location_excerpts.py`).
use ras_commander_rust::{
    hdf::{
        mesh_results::{DomainRule, ElementDomain, MeshResultVariable},
        ReadOnlyHdfSession, Slice,
    },
    Capability, ErrorKind, UnsupportedCode,
};
const AREA: &str = "D601";
const ROOT: &str =
    "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/2D Flow Areas/D601";
fn fixture(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy_location")
            .join(name),
    )
    .unwrap()
}
fn variable(s: &ReadOnlyHdfSession, name: &str) -> MeshResultVariable {
    s.mesh_series_inventory(AREA, 1 << 20)
        .unwrap()
        .variables
        .into_iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("{name} absent"))
}
fn read_err(s: &ReadOnlyHdfSession, name: &str) -> (ErrorKind, Option<UnsupportedCode>) {
    let e = s
        .read_mesh_series(AREA, name, Slice::new(0, 2, 1), None, 1 << 20, 1 << 20)
        .unwrap_err();
    (e.kind, e.code)
}
const IDENTITY: (ErrorKind, Option<UnsupportedCode>) = (
    ErrorKind::Unsupported,
    Some(UnsupportedCode::IdentityLayout),
);

#[test]
fn legacy_exact_names_qualify_their_domain_and_read_the_stored_values() {
    let s = fixture("valid.h5");
    for (name, domain, columns) in [
        ("Depth", ElementDomain::Cell, 3),
        ("Water Surface", ElementDomain::Cell, 3),
        ("Face Velocity", ElementDomain::Face, 4),
        ("Face Shear Stress", ElementDomain::Face, 4),
        ("Face Area", ElementDomain::Face, 4),
        ("Face Manning's n", ElementDomain::Face, 4),
    ] {
        let v = variable(&s, name);
        assert_eq!(v.capability, Capability::Supported, "{name}");
        assert_eq!(v.domain, domain, "{name}");
        assert_eq!(v.domain_rule, DomainRule::LegacyExactName, "{name}");
        assert_eq!(v.domain_rule.as_str(), "legacy_exact_name");
        assert!(v.attributes.iter().all(|a| a.name != "Location"));
        let r = s
            .read_mesh_series(AREA, name, Slice::new(0, 2, 1), None, 1 << 20, 1 << 20)
            .unwrap();
        assert_eq!(r.elements, (0..columns).collect::<Vec<_>>());
        // Identical to the literal generic read: the rule qualifies, it never transforms.
        let literal = s.read_numeric(&format!("{ROOT}/{name}"), None).unwrap();
        assert_eq!(r.values, literal, "{name}");
        assert!(s
            .reduce_mesh_series(AREA, name, Slice::new(0, 2, 1), None, 1 << 20, 1 << 20)
            .is_ok());
    }
}

#[test]
fn unknown_names_time_only_and_summary_arrays_never_use_the_legacy_table() {
    let s = fixture("valid.h5");
    for name in ["Cell Hydraulic Depth", "depth", "Face Velocity X"] {
        let v = variable(&s, name);
        assert_eq!(v.domain, ElementDomain::Other, "{name}");
        assert_eq!(v.domain_rule, DomainRule::Unqualified, "{name}");
        assert_eq!(v.capability.code(), Some(UnsupportedCode::IdentityLayout));
        assert_eq!(read_err(&s, name), IDENTITY, "{name}");
        assert!(s.read_numeric(&format!("{ROOT}/{name}"), None).is_ok());
    }
    // Rank-one time-only series keep the orientation refusal.
    let t = variable(&s, "Time Step");
    assert_eq!(t.domain_rule, DomainRule::Unqualified);
    assert_eq!(t.capability.code(), Some(UnsupportedCode::Schema));
    // Summary Output keeps its own rule even for an exact legacy name.
    let summary = s
        .mesh_summary_inventory(AREA, 1 << 20)
        .unwrap()
        .variables
        .into_iter()
        .find(|v| v.name == "Depth")
        .unwrap();
    assert_eq!(summary.domain_rule, DomainRule::Unqualified);
    assert_eq!(
        summary.capability.code(),
        Some(UnsupportedCode::IdentityLayout)
    );
}

#[test]
fn legacy_domain_axis_must_equal_the_geometry_count() {
    let s = fixture("wrong_count.h5");
    for name in ["Depth", "Face Velocity"] {
        let v = variable(&s, name);
        assert_eq!(v.domain_rule, DomainRule::LegacyExactName);
        assert!(matches!(v.capability, Capability::Corrupt { .. }), "{name}");
        assert_eq!(read_err(&s, name).0, ErrorKind::Corrupt, "{name}");
    }
    assert!(variable(&s, "Water Surface").capability.is_supported());
}

#[test]
fn legacy_rows_are_checked_against_the_block_time() {
    let s = fixture("rows_mismatch.h5");
    let v = variable(&s, "Depth");
    assert_eq!(v.domain_rule, DomainRule::LegacyExactName);
    assert_eq!(
        v.capability,
        Capability::corrupt("series rows differ from Time axis")
    );
    assert!(variable(&s, "Face Velocity").capability.is_supported());
}

#[test]
fn a_present_location_stays_authoritative_and_never_falls_back_to_the_name() {
    let s = fixture("invalid_location.h5");
    // Unknown, non-scalar and non-text stored Location refuse despite legacy names.
    for name in ["Depth", "Face Velocity", "Face Shear Stress"] {
        let v = variable(&s, name);
        assert_eq!(v.domain, ElementDomain::Other, "{name}");
        assert_eq!(v.domain_rule, DomainRule::Unqualified, "{name}");
        assert_eq!(read_err(&s, name), IDENTITY, "{name}");
    }
    // Location=Faces on Water Surface wins over the cell name: 3 columns != 4 faces.
    let ws = variable(&s, "Water Surface");
    assert_eq!(
        (ws.domain, ws.domain_rule),
        (ElementDomain::Face, DomainRule::StoredLocation)
    );
    assert_eq!(read_err(&s, "Water Surface").0, ErrorKind::Corrupt);
    let area = variable(&s, "Face Area");
    assert_eq!(
        (area.domain, area.domain_rule, area.capability),
        (
            ElementDomain::Face,
            DomainRule::StoredLocation,
            Capability::Supported
        )
    );
}

#[test]
fn legacy_cell_series_refuse_a_physical_cell_count_above_the_stored_rows() {
    // PR #565 parity: real JUL Attributes Cell Count 340248 exceeds the 3 excerpt cell rows.
    let s = fixture("physical_count.h5");
    for name in ["Depth", "Water Surface"] {
        let v = variable(&s, name);
        assert_eq!(
            (v.domain, v.domain_rule),
            (ElementDomain::Cell, DomainRule::LegacyExactName)
        );
        assert_eq!(
            v.capability,
            Capability::corrupt("physical Cell Count exceeds stored cell geometry rows"),
            "{name}"
        );
        assert_eq!(read_err(&s, name).0, ErrorKind::Corrupt, "{name}");
        assert!(s.read_numeric(&format!("{ROOT}/{name}"), None).is_ok());
    }
    // Face series have no physical-cell bound.
    assert!(variable(&s, "Face Velocity").capability.is_supported());
    // Both domain paths share the bound (PR #565): see the stored-Location test below.
    // A physical count within the stored rows (2 <= 3) qualifies.
    let s = fixture("physical_count_ok.h5");
    for name in ["Depth", "Water Surface", "Face Velocity", "Stored Cells"] {
        assert!(variable(&s, name).capability.is_supported(), "{name}");
    }
}

#[test]
fn stored_location_cell_series_share_the_physical_cell_count_bound() {
    // PR #565 applies the bound on both domain paths. `Stored Cells` carries Location=Cells
    // over the same 3 cell rows, with the real JUL Cell Count 340248.
    let s = fixture("physical_count.h5");
    let stored = variable(&s, "Stored Cells");
    assert_eq!(
        (stored.domain, stored.domain_rule),
        (ElementDomain::Cell, DomainRule::StoredLocation)
    );
    assert_eq!(
        stored.capability,
        Capability::corrupt("physical Cell Count exceeds stored cell geometry rows")
    );
    assert_eq!(read_err(&s, "Stored Cells").0, ErrorKind::Corrupt);
    assert!(s
        .read_numeric(&format!("{ROOT}/Stored Cells"), None)
        .is_ok());
    // Within the stored rows (Cell Count 2 <= 3) the stored-Location series qualifies.
    let ok = fixture("physical_count_ok.h5");
    let stored = variable(&ok, "Stored Cells");
    assert_eq!(stored.domain_rule, DomainRule::StoredLocation);
    assert_eq!(stored.capability, Capability::Supported);
    // Excerpts without an Attributes table record no physical count: stored Location stays
    // qualified (the invalid_location excerpt's valid stored Faces series is unaffected too).
    let none = fixture("invalid_location.h5");
    assert!(variable(&none, "Face Area").capability.is_supported());
}

#[test]
fn series_need_the_base_output_time_dataset_not_only_date_stamps() {
    // ras-commander PR #565 parity: `time_missing.h5` keeps the real JUL date stamps but drops
    // `.../Unsteady Time Series/Time`. Both domain paths refuse; literal reads remain.
    let s = fixture("time_missing.h5");
    for (name, rule) in [
        ("Depth", DomainRule::LegacyExactName),
        ("Water Surface", DomainRule::LegacyExactName),
        ("Face Velocity", DomainRule::LegacyExactName),
        ("Stored Cells", DomainRule::StoredLocation),
        ("Stored Faces", DomainRule::StoredLocation),
    ] {
        let v = variable(&s, name);
        assert_eq!(v.domain_rule, rule, "{name}");
        assert_eq!(
            v.capability,
            Capability::unsupported(
                UnsupportedCode::Schema,
                "Base Output Time dataset absent; date stamps alone do not qualify mesh series"
            ),
            "{name}"
        );
        assert_eq!(
            read_err(&s, name),
            (ErrorKind::Unsupported, Some(UnsupportedCode::Schema)),
            "{name}"
        );
        assert!(s.read_numeric(&format!("{ROOT}/{name}"), None).is_ok());
    }
}

#[test]
fn absent_geometry_count_refusals_name_the_rule_that_chose_the_domain() {
    // Review P3-7: `cell_geometry_absent.h5` drops Cells Center Coordinate.
    let s = fixture("cell_geometry_absent.h5");
    for (name, rule, reason) in [
        (
            "Depth",
            DomainRule::LegacyExactName,
            "geometry count for legacy exact-name domain absent",
        ),
        (
            "Stored Cells",
            DomainRule::StoredLocation,
            "geometry count for stored Location absent",
        ),
    ] {
        let v = variable(&s, name);
        assert_eq!(v.domain_rule, rule, "{name}");
        assert_eq!(
            v.capability,
            Capability::unsupported(UnsupportedCode::IdentityLayout, reason),
            "{name}"
        );
    }
    // Face series keep their own count and qualify.
    assert!(variable(&s, "Face Velocity").capability.is_supported());
}
