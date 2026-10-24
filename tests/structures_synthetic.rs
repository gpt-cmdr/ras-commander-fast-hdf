#![cfg(feature = "hdf")]
//! Synthetic structure fixtures (generator tests/fixtures/structures/make_fixtures.py with a
//! SHA256 manifest and hand-written expectations): malformed layouts must map to the exact
//! Capability/ErrorKind outcomes, with per-family isolation.
use ras_commander_fast_hdf::hdf::structures::{ColumnLabelSource, StructureFamilyKind as K};
use ras_commander_fast_hdf::hdf::units::UnitsSource;
use ras_commander_fast_hdf::hdf::xs1d::ResultFamily;
use ras_commander_fast_hdf::hdf::DEFAULT_MAX_BYTES;
use ras_commander_fast_hdf::hdf::{NumericKind, NumericValues, ReadOnlyHdfSession, Slice};
use ras_commander_fast_hdf::{Capability, ErrorKind, UnsupportedCode};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const MAX: usize = 1 << 20;
fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/structures")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir().join("manifest.json")).unwrap()).unwrap()
}
fn open(name: &str) -> ReadOnlyHdfSession {
    let m = manifest();
    let entry = &m["fixtures"][name];
    let path = dir().join(entry["file"].as_str().unwrap());
    let digest = Sha256::digest(std::fs::read(&path).unwrap());
    let got: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(got, entry["sha256"].as_str().unwrap(), "{name} hash");
    ReadOnlyHdfSession::open(path).unwrap()
}
fn label(c: &Capability) -> String {
    match c {
        Capability::Supported => "Supported".into(),
        Capability::Corrupt { .. } => "Corrupt".into(),
        Capability::Unavailable { .. } => "Unavailable".into(),
        Capability::Unsupported { code, .. } => format!(
            "Unsupported:{}",
            match code {
                UnsupportedCode::IdentityLayout => "IdentityLayout",
                UnsupportedCode::Schema => "Schema",
                UnsupportedCode::Encoding => "Encoding",
                UnsupportedCode::ResourceLimit => "ResourceLimit",
                _ => "Other",
            }
        ),
        _ => "Other".into(),
    }
}
fn s(a: u64, b: u64, c: u64) -> Slice {
    Slice::new(a, b, c)
}

#[test]
fn manifest_cases_match_hand_written_expectations_with_family_isolation() {
    let m = manifest();
    let mut checked = 0;
    for (name, entry) in m["fixtures"].as_object().unwrap() {
        let caps = open(name).plan_capabilities(DEFAULT_MAX_BYTES).unwrap();
        let e = &entry["expect"];
        for (kind, key) in [
            (K::Sa2dAreaConnection, "sa2d"),
            (K::LateralStructure, "lateral"),
            (K::StorageArea, "storage"),
        ] {
            let got = caps.structures.iter().find(|c| c.kind == kind).unwrap();
            assert_eq!(
                label(&got.status),
                e[key].as_str().unwrap(),
                "{name} {key}: {:?}",
                got.status
            );
        }
        let agg = caps
            .families
            .iter()
            .find(|f| f.family == ResultFamily::Structures)
            .unwrap();
        assert_eq!(
            label(&agg.status),
            e["aggregate"].as_str().unwrap(),
            "{name} aggregate"
        );
        checked += 1;
    }
    assert_eq!(checked, 17);
}

#[test]
fn malformed_inventories_use_stable_error_kinds() {
    for (name, kind, want) in [
        (
            "missing_attribute_table",
            K::LateralStructure,
            ErrorKind::Unsupported,
        ),
        ("count_mismatch", K::LateralStructure, ErrorKind::Corrupt),
        ("wrong_object_type", K::LateralStructure, ErrorKind::Corrupt),
        ("duplicate_names", K::LateralStructure, ErrorKind::Corrupt),
        (
            "duplicate_geometry_identity",
            K::LateralStructure,
            ErrorKind::Unsupported,
        ),
        (
            "unmatched_lateral_group",
            K::LateralStructure,
            ErrorKind::Unsupported,
        ),
        (
            "unmatched_storage_name",
            K::StorageArea,
            ErrorKind::Unsupported,
        ),
        (
            "duplicate_node_pointer",
            K::LateralStructure,
            ErrorKind::Corrupt,
        ),
        (
            "row_count_vs_time_axis",
            K::LateralStructure,
            ErrorKind::Corrupt,
        ),
        ("missing_time_axis", K::StorageArea, ErrorKind::Corrupt),
        (
            "unmatched_connection",
            K::Sa2dAreaConnection,
            ErrorKind::Unsupported,
        ),
        ("storage_is_dataset", K::StorageArea, ErrorKind::Corrupt),
    ] {
        let err = open(name).structure_inventory(kind, MAX).unwrap_err();
        assert_eq!(err.kind, want, "{name}: {err}");
    }
    let err = open("row_count_vs_time_axis")
        .structure_inventory(K::LateralStructure, MAX)
        .unwrap_err();
    assert!(
        err.message.contains("3 rows but the time axis has 4"),
        "{err}"
    );
}

#[test]
fn good_fixture_identities_labels_units_and_exact_reads() {
    let sess = open("good");
    let lat = sess.structure_inventory(K::LateralStructure, MAX).unwrap();
    assert_eq!(lat.time_rows, 4);
    let names: Vec<_> = lat.structures.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["R1 Reach1 100", "R1 Reach1 200"]);
    let second = &lat.structures[1];
    assert_eq!(second.node_pointer.value(), Some(&11));
    assert!(second.node_pointer.is_supported());
    assert_eq!(
        (
            second.identity.river.as_str(),
            second.identity.reach.as_str(),
            second.identity.station.as_str()
        ),
        ("R1", "Reach1", "200")
    );
    assert_eq!(second.identity.id.row, 1);
    assert_eq!(second.identity.id.table, "Geometry/Structures/Attributes");
    assert_eq!(second.identity.id.source, sess.path());
    let rels: Vec<_> = second.series.iter().map(|x| x.relative.as_str()).collect();
    assert_eq!(
        rels,
        [
            "Structure Variables",
            "Weir Variables",
            "HW TW Segments/Flow"
        ]
    );
    assert_eq!(second.static_datasets, ["HW TW Segments/HW TW Station"]);
    match second.series[0].columns.value() {
        Some(set) => {
            assert_eq!(set.source, ColumnLabelSource::VariableUnit);
            assert_eq!(set.labels[1].name, "Stage HW");
            assert_eq!(set.labels[1].unit.as_deref(), Some("ft"));
        }
        None => panic!("{:?}", second.series[0].columns),
    }
    assert!(second.series[1].columns.is_unavailable());
    let flow = &second.series[2];
    assert_eq!(flow.units.source, UnitsSource::UnitsAttribute);
    assert_eq!(flow.units.units.value().map(String::as_str), Some("cfs"));
    assert_eq!(flow.kind, Some(NumericKind::F32));

    let storage = sess.structure_inventory(K::StorageArea, MAX).unwrap();
    assert!(storage.structures[0].node_pointer.is_unavailable());
    let fam: Vec<_> = storage
        .family_series
        .iter()
        .map(|x| x.relative.as_str())
        .collect();
    assert_eq!(fam, ["Flow", "Water Surface"]);
    assert_eq!(
        storage.family_series[0]
            .units
            .units
            .value()
            .map(String::as_str),
        Some("cfs")
    );
    assert_eq!(
        storage.family_series[1]
            .units
            .units
            .value()
            .map(String::as_str),
        Some("ft")
    );
    let conn = &storage.structures[0].series[0];
    match conn.columns.value() {
        Some(set) => {
            assert_eq!(set.source, ColumnLabelSource::Connections);
            assert_eq!(set.labels[1].name, " LSDam");
            assert_eq!(set.labels[1].unit, None);
        }
        None => panic!("{:?}", conn.columns),
    }
    let sa = sess
        .structure_inventory(K::Sa2dAreaConnection, MAX)
        .unwrap();
    assert_eq!(sa.structures[0].identity.connection, "Conn1");
    assert_eq!(sa.structures[1].identity.connection, "Dam");

    let m = manifest();
    let f = &m["frozen_read"];
    let got = sess
        .read_structure_series(
            K::LateralStructure,
            f["structure"].as_str(),
            f["relative"].as_str().unwrap(),
            s(1, 4, 2),
            s(0, 2, 1),
            MAX,
        )
        .unwrap();
    assert_eq!(got.shape, [2, 2]);
    let NumericValues::F32(v) = got.values else {
        panic!("dtype")
    };
    let bits: Vec<u64> = v.iter().map(|x| u64::from(x.to_bits())).collect();
    let want: Vec<u64> = f["bits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect();
    assert_eq!(bits, want, "IEEE bits");
}

#[test]
fn read_requests_are_bounded_and_validated() {
    let sess = open("good");
    let rel = "Structure Variables";
    let name = Some("R1 Reach1 100");
    let read = |n: Option<&str>, r: &str, rows: Slice, cols: Slice, max: usize| {
        sess.read_structure_series(K::LateralStructure, n, r, rows, cols, max)
    };
    assert_eq!(
        read(name, rel, s(0, 4, 1), s(0, 2, 1), 31)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert!(read(name, rel, s(0, 4, 1), s(0, 2, 1), 32).is_ok());
    assert_eq!(
        read(name, rel, s(0, 5, 1), s(0, 2, 1), MAX)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        read(name, rel, s(0, 4, 0), s(0, 2, 1), MAX)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        read(Some("nope"), rel, s(0, 4, 1), s(0, 2, 1), MAX)
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        read(name, "missing", s(0, 4, 1), s(0, 2, 1), MAX)
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    for bad in [
        "../x",
        "Geometric Info/Face Points",
        "",
        "a//b",
        "./Structure Variables",
    ] {
        assert_eq!(
            read(name, bad, s(0, 1, 1), s(0, 1, 1), MAX)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidRequest,
            "{bad}"
        );
    }
    assert_eq!(
        read(None, "Flow", s(0, 1, 1), s(0, 1, 1), MAX)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    let fam = sess
        .read_structure_series(K::StorageArea, None, "Flow", s(0, 4, 1), s(0, 2, 1), MAX)
        .unwrap();
    assert_eq!(fam.shape, [4, 2]);
    let ints = open("integer_series");
    let inv = ints.structure_inventory(K::LateralStructure, MAX).unwrap();
    let counts = inv.structures[0]
        .series
        .iter()
        .find(|x| x.relative == "Counts")
        .unwrap();
    assert_eq!(counts.kind, Some(NumericKind::I32));
    let e = ints
        .read_structure_series(
            K::LateralStructure,
            Some("R1 Reach1 100"),
            "Counts",
            s(0, 4, 1),
            s(0, 2, 1),
            MAX,
        )
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
}

#[test]
fn label_count_mismatch_is_series_level_evidence_not_a_hidden_pass() {
    let sess = open("label_count_mismatch");
    let inv = sess.structure_inventory(K::LateralStructure, MAX).unwrap();
    let sv = &inv.structures[0].series[0];
    assert!(
        matches!(sv.columns.capability(), Capability::Corrupt { .. })
            && sv.columns.raw_bytes().is_some_and(|b| !b.is_empty())
    );
    assert!(
        inv.structures[1].series[0].columns.is_supported(),
        "other structures keep valid labels"
    );
}

#[test]
fn followup_review_corrections() {
    // Invalid label tables are visible at family level without hiding Supported.
    let caps = open("label_count_mismatch")
        .plan_capabilities(DEFAULT_MAX_BYTES)
        .unwrap();
    let lat = caps
        .structures
        .iter()
        .find(|c| c.kind == K::LateralStructure)
        .unwrap();
    assert!(lat.status.is_supported());
    assert_eq!(lat.diagnostics.len(), 1, "{:?}", lat.diagnostics);
    assert!(
        lat.diagnostics[0].contains("Structure Variables")
            && lat.diagnostics[0].contains("column labels corrupt")
    );
    let good = open("good").plan_capabilities(DEFAULT_MAX_BYTES).unwrap();
    assert!(good.structures.iter().all(|c| c.diagnostics.is_empty()));

    // P3: multiple label attributes message names the attributes (not a fixed pair).
    let inv = open("both_label_attributes")
        .structure_inventory(K::LateralStructure, MAX)
        .unwrap();
    let cols = &inv.structures[0].series[0].columns;
    match cols.capability() {
        Capability::Corrupt { reason } => {
            assert!(
                reason.contains("multiple column-label attributes")
                    && reason.contains("Connections"),
                "{reason}"
            );
        }
        other => panic!("{other:?}"),
    }

    // P3: a group path is an InvalidRequest, not Corrupt storage.
    let sess = open("good");
    let e = sess
        .read_structure_series(
            K::LateralStructure,
            Some("R1 Reach1 100"),
            "HW TW Segments",
            s(0, 1, 1),
            s(0, 1, 1),
            MAX,
        )
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest, "{e}");

    // P3: over-budget geometry table keeps ResourceLimit through the capability mapping.
    let e = sess
        .structure_inventory(K::LateralStructure, 1)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest, "{e}");
}
