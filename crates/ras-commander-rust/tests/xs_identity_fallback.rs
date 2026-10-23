#![cfg(feature = "hdf")]
//! Steady XS identity selection with the geometry-table fallback of Python
//! `HdfResultsPlan._get_steady_cross_section_attributes` (synthetic fixtures from
//! tests/fixtures/xs_identity_fallback/make_fixtures.py, SHA256 manifest).
use ras_commander_rust::hdf::xs1d::{FamilyStatus, ResultFamily, XsVariable};
use ras_commander_rust::hdf::{
    NumericArray, NumericValues, ReadOnlyHdfSession, ResultMode, Slice, DEFAULT_MAX_BYTES,
};
use ras_commander_rust::{Capability, Error, ErrorKind, UnsupportedCode};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn open(name: &str) -> ReadOnlyHdfSession {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/xs_identity_fallback");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let path = dir.join(name);
    let got = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
    assert_eq!(got, m["fixtures"][name].as_str().unwrap(), "{name} hash");
    ReadOnlyHdfSession::open(path).unwrap()
}
fn read(
    s: &ReadOnlyHdfSession,
    mode: ResultMode,
    variable: XsVariable,
) -> Result<NumericArray, Error> {
    s.read_cross_section_result(
        mode,
        variable,
        Slice::new(0, 2, 1),
        Slice::new(0, 2, 1),
        DEFAULT_MAX_BYTES,
        1 << 20,
    )
}
fn steady_family(s: &ReadOnlyHdfSession) -> FamilyStatus {
    s.plan_capabilities(DEFAULT_MAX_BYTES)
        .unwrap()
        .families
        .into_iter()
        .find(|f| f.family == ResultFamily::SteadyProfiles)
        .unwrap()
        .status
}
/// The read, the capability report and the result inventory agree on one refusal.
fn refused(name: &str, kind: ErrorKind, code: Option<UnsupportedCode>, text: &str) {
    let s = open(name);
    let e = read(&s, ResultMode::Steady, XsVariable::WaterSurface).unwrap_err();
    assert_eq!((e.kind, e.code), (kind, code), "{name}: {e:?}");
    assert!(e.message.contains(text), "{name}: {}", e.message);
    let status = steady_family(&s);
    assert!(
        status.reason().unwrap_or_default().contains(text),
        "{name}: {status:?}"
    );
    match kind {
        ErrorKind::Corrupt => assert!(matches!(status, FamilyStatus::Corrupt { .. })),
        ErrorKind::Unsupported => assert_eq!(status.code(), code, "{name}"),
        _ => assert!(!status.is_supported(), "{name}"),
    }
    // The inventory refuses at the identity stage, or per variable for a result-axis
    // mismatch found after the identity is validated.
    let inv = s
        .xs_result_inventory(ResultMode::Steady, DEFAULT_MAX_BYTES)
        .unwrap();
    assert!(
        !inv.capability.is_supported()
            || inv.variables.iter().all(|v| !v.capability.is_supported()),
        "{name}: {:?}",
        inv.capability
    );
}
fn reads_exactly(name: &str) {
    let s = open(name);
    let ws = read(&s, ResultMode::Steady, XsVariable::WaterSurface).unwrap();
    assert_eq!(ws.shape, vec![2, 2]);
    assert_eq!(ws.values, NumericValues::F32(vec![100., 101., 102., 103.]));
    let flow = read(&s, ResultMode::Steady, XsVariable::Flow).unwrap();
    assert_eq!(
        flow.values,
        NumericValues::F64(vec![1000., 1001., 1002., 1003.])
    );
    assert_eq!(steady_family(&s), FamilyStatus::Supported, "{name}");
    let inv = s
        .xs_result_inventory(ResultMode::Steady, DEFAULT_MAX_BYTES)
        .unwrap();
    assert_eq!(inv.capability, Capability::Supported, "{name}");
    assert_eq!(inv.rows, 2);
    // Identities are the geometry rows, in result-column order.
    let rows: Vec<_> = inv
        .sections
        .iter()
        .map(|i| (i.table.as_str(), i.row))
        .collect();
    assert_eq!(
        rows,
        [
            ("Geometry/Cross Sections/Attributes", 0),
            ("Geometry/Cross Sections/Attributes", 1)
        ]
    );
    let g = s.cross_sections(DEFAULT_MAX_BYTES).unwrap();
    let ids: Vec<_> = g.sections.iter().map(|x| x.station.as_str()).collect();
    assert_eq!(ids, ["1000", "900"]);
}

#[test]
fn results_side_table_present_is_unchanged() {
    reads_exactly("results_present.h5");
    reads_exactly("packed_only.h5");
    refused(
        "results_present_mismatch.h5",
        ErrorKind::Corrupt,
        None,
        "results-side XS identity mismatch at column 0",
    );
    // Python's case-insensitive station/rs lookup would accept this results-side table; the
    // engine keeps requiring `Station` there (present-table validation is unchanged).
    refused(
        "results_present_rs_field.h5",
        ErrorKind::Unsupported,
        Some(UnsupportedCode::IdentityLayout),
        "results-side identity field Station missing",
    );
}

#[test]
fn absent_results_side_table_falls_back_to_geometry_rs() {
    reads_exactly("geometry_fallback.h5");
}

#[test]
fn unsteady_has_no_geometry_fallback() {
    let s = open("geometry_fallback_unsteady.h5");
    let e = read(&s, ResultMode::Unsteady, XsVariable::WaterSurface).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    assert_eq!(e.code, Some(UnsupportedCode::IdentityLayout));
    assert_eq!(e.message, "results-side identity table absent");
    let inv = s
        .xs_result_inventory(ResultMode::Unsteady, DEFAULT_MAX_BYTES)
        .unwrap();
    assert_eq!(inv.capability.code(), Some(UnsupportedCode::IdentityLayout));
}

#[test]
fn both_tables_absent_still_refuse() {
    let s = open("both_absent.h5");
    let e = read(&s, ResultMode::Steady, XsVariable::WaterSurface).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound, "{e:?}");
    assert!(e.message.contains("Attributes"), "{e:?}");
    assert!(!steady_family(&s).is_supported());
    let inv = s
        .xs_result_inventory(ResultMode::Steady, DEFAULT_MAX_BYTES)
        .unwrap();
    assert!(!inv.capability.is_supported());
}

#[test]
fn count_mismatch_refuses() {
    refused(
        "count_mismatch.h5",
        ErrorKind::Corrupt,
        None,
        "XS results must have (time/profile, cross-section) shape",
    );
}

#[test]
fn unproven_or_contradictory_geometry_identity_refuses() {
    // The geometry table itself requires proven fixed-text River/Reach/RS.
    refused(
        "unproven_river.h5",
        ErrorKind::Unsupported,
        Some(UnsupportedCode::Encoding),
        "nested/array/vlen/reference metadata encoding",
    );
    // Python prefers `Station` over `RS`; a numeric `Station` is not proven text.
    refused(
        "unproven_station.h5",
        ErrorKind::Unsupported,
        Some(UnsupportedCode::Encoding),
        "geometry-fallback identity field Station is not proven fixed text",
    );
    // Python would align by `Station`; it contradicts the RS identity, so refuse.
    refused(
        "station_and_rs_disagree.h5",
        ErrorKind::Corrupt,
        None,
        "geometry-fallback XS identity mismatch at column 0",
    );
    // Python's lower-cased lookup keeps the last of `RS`/`rs`; the engine refuses.
    refused(
        "case_ambiguous_rs.h5",
        ErrorKind::Unsupported,
        Some(UnsupportedCode::IdentityLayout),
        "geometry-fallback identity field rs is ambiguous by case",
    );
    // Python accepts lower-case river/reach/rs; the engine geometry table needs River/Reach/RS.
    refused(
        "lowercase_fields.h5",
        ErrorKind::Unsupported,
        Some(UnsupportedCode::Schema),
        "missing cross-section identity field River",
    );
}
