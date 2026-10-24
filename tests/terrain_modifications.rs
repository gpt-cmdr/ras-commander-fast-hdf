//! RASMapper terrain-modification attribute tables (`ReadFamily::TerrainModifications`).
//! Synthetic fixtures reproduce the observed stored schema with invented values:
//! `tests/generate_terrain_modification_fixtures.py`. Qualified tables must return the exact
//! stored compound bytes, from which h5py's `dataset[()]` bytes follow by HDF5's fixed-string
//! conversion; every malformed or unobserved layout is refused, never guessed.
#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::{
    hdf::{
        families::ReadFamily, stored::StoredValues, AttributeType, ReadOnlyHdfSession,
        StringPadding,
    },
    ErrorKind, UnsupportedCode,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const MAX: usize = 1 << 20;
const FAMILY: ReadFamily = ReadFamily::TerrainModifications;

fn dir() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/terrain_modifications");
    if d.is_dir() {
        Some(d)
    } else {
        eprintln!("skipping: {} is absent", d.display());
        None
    }
}

fn manifest(base: &std::path::Path) -> Vec<serde_json::Value> {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    m.as_array().unwrap().clone()
}

fn open(base: &std::path::Path, entry: &serde_json::Value) -> PathBuf {
    let path = base.join(entry["name"].as_str().unwrap());
    assert_eq!(
        format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())),
        entry["sha256"].as_str().unwrap(),
        "{}",
        path.display()
    );
    path
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// HDF5's conversion into h5py's NULLPAD memory string: NULLPAD is copied, NULLTERM is cut at
/// the first NUL and SPACEPAD loses trailing spaces; the rest of the field is zero-filled.
fn h5py_bytes(raw: &[u8], rows: usize, columns: &[(u32, AttributeType)]) -> Vec<u8> {
    let mut out = raw.to_vec();
    let width = raw.len().checked_div(rows).unwrap_or(0);
    for row in 0..rows {
        for (offset, kind) in columns {
            let AttributeType::FixedString {
                padding, width: w, ..
            } = kind
            else {
                continue;
            };
            let at = row * width + *offset as usize;
            let field = &mut out[at..at + w];
            let keep = match padding {
                StringPadding::NullPad => *w,
                StringPadding::NullTerminate => field.iter().position(|&b| b == 0).unwrap_or(*w),
                StringPadding::SpacePad => {
                    field.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1)
                }
                _ => panic!("unexpected padding"),
            };
            field[keep..].fill(0);
        }
    }
    out
}

#[test]
fn qualified_tables_are_exact_and_refusals_explicit() {
    let Some(base) = dir() else { return };
    let mut supported = 0;
    let mut refused = 0;
    for entry in manifest(&base) {
        let name = entry["name"].as_str().unwrap();
        if name == "truncated.h5" {
            continue;
        }
        let path = open(&base, &entry);
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        let inv = s.read_family_inventory(FAMILY, MAX).unwrap();
        assert!(
            inv.capability.is_supported(),
            "{name}: {:?}",
            inv.capability
        );
        let tables = entry["tables"].as_array().unwrap();
        let listed: Vec<&str> = inv
            .datasets
            .iter()
            .map(|d| d.stored.path.as_str())
            .collect();
        let want: Vec<&str> = tables.iter().map(|t| t["path"].as_str().unwrap()).collect();
        assert_eq!(listed, want, "{name}: exact Attributes table set");
        // Only Attributes tables belong to the family; geometry arrays stay generic.
        assert!(listed.iter().all(|p| p.ends_with("/Attributes")));
        assert!(inv.groups.iter().any(|g| g.path == "Modifications"));
        for (t, d) in tables.iter().zip(&inv.datasets) {
            let p = t["path"].as_str().unwrap();
            let status = t["status"].as_str().unwrap();
            assert_eq!(
                d.stored.capability.as_str(),
                status,
                "{name} {p}: {:?}",
                d.stored.capability
            );
            let read = s.read_family_dataset(FAMILY, p, None, MAX, MAX);
            if status != "supported" {
                refused += 1;
                if let Some(code) = t["code"].as_str() {
                    assert_eq!(
                        d.stored.capability.code().unwrap().as_str(),
                        code,
                        "{name} {p}"
                    );
                }
                let e = read.expect_err(p);
                let kind = if status == "corrupt" {
                    ErrorKind::Corrupt
                } else {
                    ErrorKind::Unsupported
                };
                assert_eq!(e.kind, kind, "{name} {p}: {e:?}");
                continue;
            }
            supported += 1;
            assert_eq!(d.role, t["role"].as_str().unwrap(), "{name} {p}");
            assert_eq!(d.domain, "terrain_modification");
            assert_eq!(
                d.identity_table.as_deref(),
                t["identity_table"].as_str(),
                "{name} {p}"
            );
            assert_eq!(d.stored.storage, "compound");
            let rows = t["rows"].as_u64().unwrap();
            assert_eq!(d.stored.shape, vec![rows]);
            let StoredValues::Compound(c) = read.unwrap() else {
                panic!("{name} {p}: compound storage expected");
            };
            let columns: Vec<(String, u32)> = c
                .table
                .columns
                .iter()
                .map(|c| (c.name.clone(), c.byte_offset))
                .collect();
            let want: Vec<(String, u32)> = t["columns"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    (
                        c[0].as_str().unwrap().to_owned(),
                        c[1].as_u64().unwrap() as u32,
                    )
                })
                .collect();
            assert_eq!(columns, want, "{name} {p}: member names, order and offsets");
            assert_eq!(
                hex(&c.raw_bytes),
                t["stored_hex"].as_str().unwrap(),
                "{name} {p}"
            );
            let kinds: Vec<(u32, AttributeType)> = c
                .table
                .columns
                .iter()
                .map(|c| (c.byte_offset, c.attribute_type.clone()))
                .collect();
            assert_eq!(
                hex(&h5py_bytes(&c.raw_bytes, rows as usize, &kinds)),
                t["h5py_hex"].as_str().unwrap(),
                "{name} {p}: h5py dataset[()] bytes"
            );
            // Text members keep evidence; floats are little-endian f4, flags unsigned bytes.
            for col in &c.table.columns {
                match &col.attribute_type {
                    AttributeType::FixedString { width, .. } => assert!(*width > 0),
                    AttributeType::Float { bytes, order } => {
                        assert_eq!((*bytes, order.as_str()), (4, "little"), "{p} {}", col.name)
                    }
                    AttributeType::Int { signed, bytes, .. } => {
                        assert_eq!((*signed, *bytes), (false, 1), "{p} {}", col.name)
                    }
                    other => panic!("{p} {}: {other:?}", col.name),
                }
            }
        }
    }
    assert_eq!((supported, refused), (16, 25));
}

#[test]
fn transition_pier_and_rectangle_refusals_name_the_defect() {
    let Some(base) = dir() else { return };
    let entry = manifest(&base)
        .into_iter()
        .find(|e| e["name"] == "refusals.h5")
        .unwrap();
    let s = ReadOnlyHdfSession::open(open(&base, &entry)).unwrap();
    let inv = s.read_family_inventory(FAMILY, MAX).unwrap();
    let reason = |group: &str| {
        let p = format!("Modifications/{group}/Attributes");
        let d = inv.datasets.iter().find(|d| d.stored.path == p).unwrap();
        d.stored.capability.reason().unwrap_or_default().to_owned()
    };
    for (group, want) in [
        (
            "Transition Missing Percent",
            "member count differs from every observed layout of its Type",
        ),
        (
            "Transition Reordered",
            "member \"Elev Pt Tolerance\" where \"Transition Percent\" is expected",
        ),
        (
            "Transition Renamed",
            "member \"Sys Name\" where \"System Name\" is expected",
        ),
        (
            "Transition Wide",
            "member \"Transition Percent\" has an unqualified datatype",
        ),
        ("Transition Padded", "unqualified compound padding"),
        ("Transition Row Mismatch", "rows differ from Polyline Info"),
        // The writer-only Type=Channel is qualified in its observed 10-member layout only.
        (
            "Channel Type Short Layout",
            "member count differs from every observed layout of its Type",
        ),
        ("Pier Missing Feature", "feature table Piers absent"),
        (
            "Pier Wide",
            "member \"Elevation Value\" has an unqualified datatype",
        ),
        ("Rectangle Row Mismatch", "rows differ from Rectangles"),
        (
            "Rectangle Polygon Layout",
            "member count differs from every observed layout of its Type",
        ),
        (
            "Unknown Type",
            "unqualified terrain modification Type \"Spline\"",
        ),
    ] {
        let r = reason(group);
        assert!(r.contains(want), "{group}: {r:?}");
    }
}

#[test]
fn requests_are_exact_whole_table_and_bounded() {
    let Some(base) = dir() else { return };
    let entry = manifest(&base)
        .into_iter()
        .find(|e| e["name"] == "valid.h5")
        .unwrap();
    let s = ReadOnlyHdfSession::open(open(&base, &entry)).unwrap();
    let good = "Modifications/Synthetic Channel/Attributes";
    let slice = [ras_commander_fast_hdf::hdf::Slice::new(0, 1, 1)];
    for (path, slices, kind) in [
        (good, Some(&slice[..]), ErrorKind::InvalidRequest),
        (
            "Modifications/Synthetic Channel/Polyline Info",
            None,
            ErrorKind::InvalidRequest,
        ),
        (
            "Modifications/Synthetic Channel/../Synthetic Levee/Attributes",
            None,
            ErrorKind::InvalidRequest,
        ),
        ("Modifications//Attributes", None, ErrorKind::InvalidRequest),
        (
            "Terrain/Synthetic Channel/Attributes",
            None,
            ErrorKind::InvalidRequest,
        ),
        ("Modifications/Absent/Attributes", None, ErrorKind::NotFound),
    ] {
        let e = s
            .read_family_dataset(FAMILY, path, slices, MAX, MAX)
            .expect_err(path);
        assert_eq!(e.kind, kind, "{path}: {e:?}");
    }
    // Read budgets refuse before decoding; the inventory isolates an over-budget table.
    let e = s
        .read_family_dataset(FAMILY, good, None, MAX, 16)
        .expect_err("budget");
    assert_eq!(e.code, Some(UnsupportedCode::ResourceLimit), "{e:?}");
    let inv = s.read_family_inventory(FAMILY, 64).unwrap();
    assert!(inv
        .datasets
        .iter()
        .any(|d| d.stored.capability.code() == Some(UnsupportedCode::ResourceLimit)));
    // Other read families never list the modification tables.
    for &f in ras_commander_fast_hdf::hdf::families::READ_FAMILIES {
        if f == FAMILY {
            continue;
        }
        if let Ok(inv) = s.read_family_inventory(f, MAX) {
            assert!(inv
                .datasets
                .iter()
                .all(|d| !d.stored.path.starts_with("Modifications/")));
        }
    }
}

#[test]
fn absent_modifications_are_unavailable() {
    let Some(base) = dir() else { return };
    // A non-terrain file from another fixture set has no Modifications group.
    let other = base.join("../generic_strings/fixed_nullterm.h5");
    let s = ReadOnlyHdfSession::open(other).unwrap();
    let inv = s.read_family_inventory(FAMILY, MAX).unwrap();
    assert_eq!(inv.capability.as_str(), "unavailable");
    assert!(inv.datasets.is_empty());
}

#[test]
fn truncated_payload_is_refused() {
    let Some(base) = dir() else { return };
    let entry = manifest(&base)
        .into_iter()
        .find(|e| e["name"] == "truncated.h5")
        .unwrap();
    // h5py/HDF5 refuses to open the truncated file at all.
    assert_ne!(entry["h5py"].as_str().unwrap(), "read");
    let s = ReadOnlyHdfSession::open(open(&base, &entry)).unwrap();
    let p = "Modifications/Cut Short/Attributes";
    let inv = s.read_family_inventory(FAMILY, MAX).unwrap();
    let d = inv.datasets.iter().find(|d| d.stored.path == p).unwrap();
    assert_eq!(
        d.stored.capability.as_str(),
        "corrupt",
        "{:?}",
        d.stored.capability
    );
    let e = s
        .read_family_dataset(FAMILY, p, None, MAX, MAX)
        .expect_err(p);
    assert_eq!(e.kind, ErrorKind::Corrupt, "{e:?}");
}
