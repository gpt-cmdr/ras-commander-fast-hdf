//! Reference-line result columns linked to their geometry rows (`FamilyDataset::geometry_link`),
//! on excerpts of HEC's public Chippewa_2D p02 plan (`generate_reference_line_excerpts.py`) and,
//! opt-in, on whole real plans against an independent h5py oracle.
#![cfg(feature = "hdf")]
use ras_commander_rust::{
    hdf::{families::ReadFamily, stored::StoredValues, NumericValues, ReadOnlyHdfSession},
    Capability, ErrorKind, UnsupportedCode,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const ROOT: &str =
    "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/Reference Lines";
const TABLE: &str = "Geometry/Reference Lines/Attributes";

fn local(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/reference_lines")
            .join(name),
    )
    .unwrap()
}

/// Every dataset's (path, link status, linked geometry rows).
fn links(s: &ReadOnlyHdfSession) -> Vec<(String, Capability, Vec<u64>)> {
    let inv = s
        .read_family_inventory(ReadFamily::ReferenceLines, 1 << 20)
        .unwrap();
    assert!(!inv.datasets.is_empty());
    inv.datasets
        .iter()
        .map(|d| {
            assert!(
                d.stored.capability.is_supported(),
                "{}: the link never changes read capability: {:?}",
                d.stored.path,
                d.stored.capability
            );
            for id in &d.geometry_locations {
                assert_eq!(id.table, TABLE);
            }
            (
                d.stored.path.clone(),
                d.geometry_link.clone(),
                d.geometry_locations.iter().map(|id| id.row).collect(),
            )
        })
        .collect()
}

fn floats(s: &ReadOnlyHdfSession, var: &str) -> Vec<f32> {
    let StoredValues::Numeric(a) = s
        .read_family_dataset(
            ReadFamily::ReferenceLines,
            &format!("{ROOT}/{var}"),
            None,
            1 << 20,
            1 << 20,
        )
        .unwrap()
    else {
        panic!("numeric")
    };
    let NumericValues::F32(v) = a.values else {
        panic!("f32")
    };
    v
}

#[test]
fn columns_link_to_geometry_rows_by_name() {
    for (path, link, rows) in links(&local("valid.h5")) {
        assert_eq!(link, Capability::Supported, "{path}");
        assert_eq!(rows, [0, 1, 2], "{path}");
    }
}

#[test]
fn a_column_permutation_is_linked_not_assumed() {
    for (path, link, rows) in links(&local("reordered.h5")) {
        assert_eq!(link, Capability::Supported, "{path}");
        assert_eq!(rows, [2, 0, 1], "{path}");
    }
    // The stored columns are read as stored; only the link follows the names.
    let (valid, reordered) = (
        floats(&local("valid.h5"), "Flow"),
        floats(&local("reordered.h5"), "Flow"),
    );
    for (a, b) in valid.chunks(3).zip(reordered.chunks(3)) {
        assert_eq!([a[2], a[0], a[1]], b);
    }
}

#[test]
fn unprovable_identities_refuse_the_link_and_keep_the_read() {
    for (name, kind) in [
        ("bad_name.h5", ErrorKind::Unsupported),
        ("duplicate_geometry.h5", ErrorKind::Unsupported),
        ("no_internal_faces.h5", ErrorKind::Unsupported),
        ("bad_faces.h5", ErrorKind::Corrupt),
    ] {
        let s = local(name);
        for (path, link, rows) in links(&s) {
            assert!(rows.is_empty(), "{name} {path}");
            match (&link, kind) {
                (Capability::Unsupported { code, .. }, ErrorKind::Unsupported) => {
                    assert_eq!(*code, UnsupportedCode::IdentityLayout, "{name} {path}")
                }
                (Capability::Corrupt { .. }, ErrorKind::Corrupt) => {}
                other => panic!("{name} {path}: {other:?}"),
            }
        }
        assert_eq!(
            floats(&s, "Flow"),
            floats(&local("valid.h5"), "Flow"),
            "{name}"
        );
    }
    // A wrong area ordinal is stored on Flow only: Flow's link is Corrupt, the others link.
    for (path, link, rows) in links(&local("bad_areas.h5")) {
        if path.ends_with("/Flow") {
            assert!(
                matches!(link, Capability::Corrupt { .. }),
                "{path}: {link:?}"
            );
            assert!(rows.is_empty());
        } else {
            assert_eq!(link, Capability::Supported, "{path}");
        }
    }
    // No geometry: no link, not a refusal of the stored results.
    for (path, link, rows) in links(&local("no_geometry.h5")) {
        assert!(
            matches!(link, Capability::Unavailable { .. }),
            "{path}: {link:?}"
        );
        assert!(rows.is_empty());
    }
}

fn sha(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `RAS_REFERENCE_LINES_ORACLE` is the JSON written by `reference_lines_oracle.py` (h5py only)
/// for the plans in `RAS_REFERENCE_LINES_FIXTURES`: per plan, per time-series variable, the
/// geometry row of each column (linked by exact Name|SA-2D and Internal Faces) and the digest of
/// the stored values.
#[test]
#[ignore = "requires RAS_REFERENCE_LINES_ORACLE and RAS_REFERENCE_LINES_FIXTURES"]
fn reference_lines_match_real_h5py_links() {
    let oracle: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RAS_REFERENCE_LINES_ORACLE").unwrap()).unwrap(),
    )
    .unwrap();
    let root = PathBuf::from(std::env::var("RAS_REFERENCE_LINES_FIXTURES").unwrap());
    let (mut columns, mut datasets) = (0, 0);
    for (name, want) in oracle.as_object().unwrap() {
        let path = root.join(name);
        assert_eq!(
            sha(std::fs::read(&path).unwrap()),
            want["sha256"].as_str().unwrap()
        );
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        let inv = s
            .read_family_inventory(ReadFamily::ReferenceLines, 64 << 20)
            .unwrap();
        for (var, w) in want["series"].as_object().unwrap() {
            let d = inv
                .datasets
                .iter()
                .find(|d| d.stored.path == format!("{ROOT}/{var}"))
                .unwrap_or_else(|| panic!("{name} {var}"));
            assert_eq!(d.geometry_link, Capability::Supported, "{name} {var}");
            let rows: Vec<u64> = d.geometry_locations.iter().map(|id| id.row).collect();
            let want_rows: Vec<u64> = w["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_u64().unwrap())
                .collect();
            assert_eq!(rows, want_rows, "{name} {var}");
            let StoredValues::Numeric(a) = s
                .read_family_dataset(
                    ReadFamily::ReferenceLines,
                    &d.stored.path,
                    None,
                    64 << 20,
                    1 << 30,
                )
                .unwrap()
            else {
                panic!("numeric")
            };
            let bytes: Vec<u8> = match a.values {
                NumericValues::F32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
                NumericValues::F64(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
                other => panic!("{other:?}"),
            };
            assert_eq!(
                sha(bytes),
                w["values_sha256"].as_str().unwrap(),
                "{name} {var}"
            );
            columns += rows.len();
            datasets += 1;
        }
    }
    println!("reference-line links checked: {columns} columns in {datasets} datasets");
}
