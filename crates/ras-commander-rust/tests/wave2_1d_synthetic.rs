#![cfg(feature = "hdf")]
//! Synthetic 1D (API 7) fixtures (generator tests/fixtures/wave2_1d/make_fixtures.py,
//! SHA256 manifest): absent, unsupported and corrupt layouts map to distinct outcomes.
use ras_commander_rust::hdf::structures_1d::OneDStructureKind as K;
use ras_commander_rust::hdf::xs_geometry::{XsGeometryField as F, XsGeometryValues};
use ras_commander_rust::hdf::{NumericValues, ReadOnlyHdfSession, ResultMode, Slice};
use ras_commander_rust::{Capability, ErrorKind, UnsupportedCode};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const MAX: usize = 1 << 20;
fn open(name: &str) -> ReadOnlyHdfSession {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wave2_1d");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let path = dir.join(name);
    let got = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
    assert_eq!(got, m["fixtures"][name].as_str().unwrap(), "{name} hash");
    ReadOnlyHdfSession::open(path).unwrap()
}
fn status(c: &Capability) -> (&'static str, Option<UnsupportedCode>) {
    (c.as_str(), c.code())
}

#[test]
fn xs_fields_distinguish_absent_unsupported_and_corrupt() {
    let s = open("xs_fields.h5");
    let inv = s.xs_geometry_inventory(MAX).unwrap();
    let cap = |f| &inv.fields.iter().find(|d| d.field == f).unwrap().capability;
    assert_eq!(status(cap(F::StationElevation)).0, "corrupt");
    assert_eq!(status(cap(F::ManningsN)).0, "corrupt");
    assert_eq!(
        status(cap(F::DeckHigh)),
        ("unsupported", Some(UnsupportedCode::Encoding))
    );
    assert_eq!(status(cap(F::IneffectiveAreas)).0, "unavailable");
    assert_eq!(status(cap(F::PropertyTable)).0, "unavailable");
    assert!(cap(F::DeckLow).is_supported());
    assert_eq!(inv.unqualified.len(), 1);
    assert_eq!(
        inv.unqualified[0].0,
        "Geometry/Cross Sections/Orthogonal Vectors"
    );
    assert_eq!(
        s.read_xs_geometry(F::StationElevation, None, MAX, MAX)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
    assert_eq!(
        s.read_xs_geometry(F::IneffectiveAreas, None, MAX, MAX)
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        s.read_xs_geometry(F::DeckHigh, None, MAX, MAX)
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    // Exact f64 values (NaN and signed zero bits kept), unsorted/repeated selection packed.
    let sel = s
        .read_xs_geometry(F::DeckLow, Some(&[1, 0, 1]), MAX, MAX)
        .unwrap();
    let XsGeometryValues::Numeric(a) = &sel.values else {
        panic!("numeric")
    };
    assert_eq!(a.shape, vec![4, 2]);
    let NumericValues::F64(v) = &a.values else {
        panic!("f64 kept")
    };
    assert!(v[0].is_nan() && v[1] == 7.25 && v[2] == 1.5 && v[5].to_bits() == (-0.0f64).to_bits());
    assert_eq!(
        sel.records.iter().map(|r| r.offset).collect::<Vec<_>>(),
        [0, 1, 3]
    );
    assert_eq!(sel.records[0].id.row, 1);
    let NumericValues::I32(info) = &sel.info.values else {
        panic!("i32 info kept")
    };
    assert_eq!(info, &[2, 1, 0, 2, 2, 1]);
}

#[test]
fn multi_part_lines_and_bad_ranges() {
    use ras_commander_rust::hdf::xs_geometry::OneDLineFamily as L;
    let s = open("lines.h5");
    let r = s.one_d_line_family(L::RiverBankLines, MAX).unwrap();
    let d = r.value().unwrap();
    let parts: Vec<Vec<(u64, u64)>> = d
        .features
        .iter()
        .map(|f| f.parts.iter().map(|p| (p.start, p.end)).collect())
        .collect();
    // Stored Parts offsets are relative to each feature; returned ranges are absolute.
    assert_eq!(parts, vec![vec![(0, 3), (3, 5)], vec![(5, 7)]]);
    assert_eq!(d.features[1].point_range, 5..7);
    assert_eq!(
        d.features[0].id.table,
        "Geometry/River Bank Lines/Polyline Info"
    );
    let e = s.one_d_line_family(L::RiverEdgeLines, MAX).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Corrupt);
    let e = s.xs_interpolation_surfaces(MAX).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Corrupt);
    assert!(s
        .one_d_line_family(L::RiverFlowPaths, MAX)
        .unwrap()
        .is_unavailable());
}

#[test]
fn separated_xs_layout_is_unsupported_identity_layout() {
    let s = open("xs_legacy.h5");
    let e = s.xs_geometry_inventory(MAX).unwrap_err();
    assert_eq!(
        (e.kind, e.code),
        (
            ErrorKind::Unsupported,
            Some(UnsupportedCode::IdentityLayout)
        )
    );
}

#[test]
fn xs_results_and_summary_isolate_variables() {
    let s = open("results.h5");
    let inv = s.xs_result_inventory(ResultMode::Unsteady, MAX).unwrap();
    assert!(inv.capability.is_supported());
    let cap = |n: &str| {
        inv.variables
            .iter()
            .find(|v| v.name == n)
            .unwrap()
            .capability
            .clone()
    };
    assert!(cap("Water Surface").is_supported());
    assert_eq!(status(&cap("Velocity Total")).0, "corrupt");
    assert_eq!(
        status(&cap("Velocity Channel")),
        ("unsupported", Some(UnsupportedCode::IdentityLayout))
    );
    let r = s
        .read_xs_result(
            ResultMode::Unsteady,
            "Water Surface",
            Slice::new(0, 3, 2),
            Some(&[1, 1, 0]),
            MAX,
            MAX,
        )
        .unwrap();
    let NumericValues::F32(v) = &r.values.values else {
        panic!("f32 kept")
    };
    assert_eq!(v, &[1.0, 1.0, 0.0, 5.0, 5.0, 4.0]);
    assert_eq!(r.section_ids[2].row, 0);
    assert_eq!(
        s.read_xs_result(
            ResultMode::Unsteady,
            "Velocity Total",
            Slice::new(0, 1, 1),
            None,
            MAX,
            MAX
        )
        .unwrap_err()
        .kind,
        ErrorKind::Corrupt
    );
    assert_eq!(
        status(&cap("Components")),
        ("unsupported", Some(UnsupportedCode::Schema))
    );
    // Steady rank 3 with Dimension [Profiles, Variables, Cross Sections] and 2 profiles: the
    // (profile, component, section) gather runs with more than one row, on both read paths.
    let steady = s.xs_result_inventory(ResultMode::Steady, MAX).unwrap();
    assert!(steady.capability.is_supported(), "{:?}", steady.capability);
    let r = s
        .read_xs_result(
            ResultMode::Steady,
            "Cross Section Variables",
            Slice::new(0, 2, 1),
            Some(&[1, 0]),
            MAX,
            MAX,
        )
        .unwrap();
    assert_eq!(r.values.shape, vec![2, 3, 2]);
    let NumericValues::F32(v) = &r.values.values else {
        panic!("f32 kept")
    };
    assert_eq!(
        v,
        &[1.0, 0.0, 3.0, 2.0, 5.0, 4.0, 7.0, 6.0, 9.0, 8.0, 11.0, 10.0]
    );
    let r = s
        .read_xs_result(
            ResultMode::Steady,
            "Cross Section Variables",
            Slice::new(1, 2, 1),
            Some(&[1, 0]),
            MAX,
            40,
        )
        .unwrap();
    let NumericValues::F32(v) = &r.values.values else {
        panic!("f32 kept")
    };
    assert_eq!(v, &[7.0, 6.0, 9.0, 8.0, 11.0, 10.0], "per-column path");
    let sum = s.one_d_summary_inventory(MAX).unwrap();
    let mf = sum
        .variables
        .iter()
        .find(|v| v.name == "Maximum Flow")
        .unwrap();
    assert_eq!(mf.element_axis, Some(1));
    assert_eq!(mf.time_of_extreme_rows, vec![2]);
    let bad = sum.variables.iter().find(|v| v.name == "Bad Rank").unwrap();
    assert_eq!(bad.capability.as_str(), "unsupported");
    let unq = sum
        .variables
        .iter()
        .find(|v| v.family == "Inline Structures")
        .unwrap();
    assert_eq!(
        status(&unq.capability),
        ("unsupported", Some(UnsupportedCode::Schema))
    );
    assert_eq!(
        s.read_one_d_summary("Inline Structures", "Maximum Flow", None, MAX, MAX)
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    let r = s
        .read_one_d_summary("Cross Sections", "Maximum Flow", Some(&[0]), MAX, MAX)
        .unwrap();
    let NumericValues::F32(v) = &r.values.values else {
        panic!("f32 kept")
    };
    assert_eq!(v, &[0.0, 2.0, 4.0]);
}

#[test]
fn one_d_structure_families_are_isolated() {
    let s = open("structures.h5");
    let caps = s.one_d_structure_capabilities(MAX).unwrap();
    let get = |k: K| {
        caps.iter()
            .find(|c| c.kind == k && c.mode == ResultMode::Unsteady)
            .unwrap()
            .status
            .clone()
    };
    assert_eq!(
        status(&get(K::InlineStructure)),
        ("unsupported", Some(UnsupportedCode::IdentityLayout))
    );
    assert_eq!(status(&get(K::Bridge)).0, "corrupt");
    assert_eq!(
        status(&get(K::Culvert)),
        ("unsupported", Some(UnsupportedCode::IdentityLayout))
    );
    assert_eq!(status(&get(K::PumpStation)).0, "unavailable");
    let steady = caps
        .iter()
        .find(|c| c.kind == K::Bridge && c.mode == ResultMode::Steady)
        .unwrap();
    assert_eq!(
        status(&steady.status),
        ("unsupported", Some(UnsupportedCode::IdentityLayout))
    );
    assert!(steady
        .status
        .reason()
        .unwrap()
        .contains("unqualified layout"));
    assert_eq!(
        s.one_d_structure_inventory(K::LateralStructure, ResultMode::Unsteady, MAX)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    let g = s.structure_geometry(MAX).unwrap();
    let g = g.value().unwrap();
    assert_eq!(g.records.len(), 2);
    assert!(g.records[0].profile_ranges.is_empty());
    assert!(s.layer_inventory(MAX).is_ok());
    // One budget policy: an over-budget compound and an over-budget array are both listed
    // with Unsupported(resource_limit); the group itself is still reported.
    use ras_commander_rust::hdf::structures_1d::OneDGeometryGroup;
    let grp = s
        .one_d_geometry_group(OneDGeometryGroup::Structures, 100)
        .unwrap();
    let grp = grp.value().unwrap();
    let listed: Vec<(&str, Option<UnsupportedCode>)> = grp
        .unsupported
        .iter()
        .map(|(n, c)| (n.as_str(), c.code()))
        .collect();
    assert_eq!(
        listed,
        [
            ("Attributes", Some(UnsupportedCode::ResourceLimit)),
            ("Profile Data", Some(UnsupportedCode::ResourceLimit))
        ]
    );
    assert!(grp.tables.is_empty() && grp.arrays.is_empty());
}

#[test]
fn malformed_connection_column_is_surfaced() {
    let s = open("bad_connection.h5");
    let caps = s.one_d_structure_capabilities(MAX).unwrap();
    let inline = caps
        .iter()
        .find(|c| c.kind == K::InlineStructure && c.mode == ResultMode::Unsteady)
        .unwrap();
    // The integer Connection column is surfaced, not replaced by empty text.
    assert_eq!(
        status(&inline.status),
        ("unsupported", Some(UnsupportedCode::IdentityLayout))
    );
    assert!(inline.status.reason().unwrap().contains("Connection"));
}
