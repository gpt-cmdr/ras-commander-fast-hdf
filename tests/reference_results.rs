#![cfg(all(feature = "hdf", feature = "mesh", feature = "results"))]
//! Reference results joined to their geometry features (`reference_line_results`,
//! `reference_point_results`) on the frozen reference-line and reference-point fixtures:
//! proven links join, unproven links refuse the whole join with their status.
use ras_commander_fast_hdf::hdf::families::{ReadFamily, READ_FAMILIES};
use ras_commander_fast_hdf::hdf::reference_results::ReferenceResultDataset;
use ras_commander_fast_hdf::hdf::stored::StoredValues;
use ras_commander_fast_hdf::hdf::{NumericValues, ReadOnlyHdfSession, Slice, DEFAULT_MAX_BYTES};
use ras_commander_fast_hdf::{Capability, UnsupportedCode};
use std::path::PathBuf;

fn open(dir: &str, name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(dir)
            .join(name),
    )
    .unwrap()
}
fn family(name: &str) -> ReadFamily {
    *READ_FAMILIES.iter().find(|f| f.as_str() == name).unwrap()
}
fn label(c: &Capability) -> &'static str {
    match c {
        Capability::Supported => "Supported",
        Capability::Unavailable { .. } => "Unavailable",
        Capability::Corrupt { .. } => "Corrupt",
        Capability::Unsupported {
            code: UnsupportedCode::IdentityLayout,
            ..
        } => "Unsupported:IdentityLayout",
        Capability::Unsupported { .. } => "Unsupported:Other",
        _ => "Other",
    }
}
fn bytes(v: &NumericValues) -> Vec<u8> {
    match v {
        NumericValues::F32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        NumericValues::F64(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        other => panic!("unexpected reference result kind {other:?}"),
    }
}
/// Column `column` of a joined dataset equals the family reader's single-column read.
fn check_column(s: &ReadOnlyHdfSession, fam: ReadFamily, d: &ReferenceResultDataset, column: u64) {
    let rows = d.values.shape[0];
    let StoredValues::Numeric(one) = s
        .read_family_dataset(
            fam,
            &d.dataset.stored.path,
            Some(&[Slice::new(0, rows, 1), Slice::new(column, column + 1, 1)]),
            DEFAULT_MAX_BYTES,
            DEFAULT_MAX_BYTES,
        )
        .unwrap()
    else {
        panic!("numeric")
    };
    let all = bytes(&d.values.values);
    let width = all.len() / (rows as usize * d.values.shape[1] as usize);
    let cols = d.values.shape[1] as usize;
    let mut picked = Vec::new();
    for r in 0..rows as usize {
        let at = (r * cols + column as usize) * width;
        picked.extend_from_slice(&all[at..at + width]);
    }
    assert_eq!(
        picked,
        bytes(&one.values),
        "{} column {column}",
        d.dataset.stored.path
    );
}
/// The result's own Name axis entry for a linked column names the feature: `<name>|<area>`
/// for global series, bare `<name>` for per-area series under that area.
fn names_feature(d: &ReferenceResultDataset, column: u64, name: &str, area: &str) -> bool {
    let entry = &d.dataset.names[column as usize];
    let path = &d.dataset.stored.path;
    entry == &format!("{name}|{area}")
        || (entry == name && path.contains(&format!("/2D Flow Areas/{area}/")))
}

fn line_area(s: &ReadOnlyHdfSession, row: u64) -> String {
    let g = s
        .geometry_family(
            ras_commander_fast_hdf::hdf::lines::GeometryFamily::ReferenceLines,
            DEFAULT_MAX_BYTES,
        )
        .unwrap();
    let col = g
        .value()
        .unwrap()
        .attributes
        .table
        .columns
        .iter()
        .find(|c| c.name == "SA-2D")
        .unwrap();
    match &col.value {
        ras_commander_fast_hdf::hdf::metadata::MetadataValue::Strings(v) => v[row as usize].clone(),
        _ => panic!("SA-2D text"),
    }
}

#[test]
fn reference_line_joins_follow_the_proven_links() {
    let fam = family("reference_line_results");
    for (name, expected) in [
        ("valid.h5", "Supported"),
        ("reordered.h5", "Supported"),
        ("per_area_valid.h5", "Supported"),
        ("per_area_reordered.h5", "Supported"),
        ("bad_name.h5", "Unsupported:IdentityLayout"),
        ("duplicate_geometry.h5", "Unsupported:IdentityLayout"),
        ("no_internal_faces.h5", "Unsupported:IdentityLayout"),
        ("per_area_bad_name.h5", "Unsupported:IdentityLayout"),
        ("per_area_no_block_name.h5", "Unsupported:IdentityLayout"),
        ("bad_faces.h5", "Corrupt"),
        ("bad_areas.h5", "Corrupt"),
        ("per_area_bad_faces.h5", "Corrupt"),
        ("no_geometry.h5", "Unavailable"),
    ] {
        let s = open("reference_lines", name);
        let r = s.reference_line_results(DEFAULT_MAX_BYTES).unwrap();
        assert_eq!(
            label(r.capability()),
            expected,
            "{name}: {:?}",
            r.capability()
        );
        let Some(j) = r.value() else { continue };
        assert!(!j.datasets.is_empty(), "{name}");
        let mut joined = 0;
        for f in &j.features {
            let area = line_area(&s, f.feature.id.row);
            for l in &f.series {
                let d = &j.datasets[l.dataset];
                assert!(
                    names_feature(d, l.column, &f.feature.name, &area),
                    "{name} {}",
                    f.feature.name
                );
                check_column(&s, fam, d, l.column);
                joined += 1;
            }
        }
        // Every column of every joined dataset belongs to exactly one feature.
        let columns: u64 = j.datasets.iter().map(|d| d.values.shape[1]).sum();
        assert_eq!(joined, columns, "{name}");
    }
}

#[test]
fn reference_point_joins_follow_the_proven_links() {
    let fam = family("reference_point_results");
    let mut refused = 0;
    for name in [
        "valid.h5",
        "reordered.h5",
        "bad_areas.h5",
        "bad_cells.h5",
        "bad_name.h5",
        "bad_points.h5",
        "bad_time.h5",
        "bad_units.h5",
        "duplicate_geometry.h5",
        "unknown_variable.h5",
    ] {
        let s = open("reference_points", name);
        let r = s.reference_point_results(DEFAULT_MAX_BYTES).unwrap();
        let expected = match name {
            "valid.h5" | "reordered.h5" => "Supported",
            "bad_name.h5" | "duplicate_geometry.h5" => "Unsupported:IdentityLayout",
            "bad_areas.h5" | "bad_cells.h5" | "bad_points.h5" => "Corrupt",
            _ => "any",
        };
        if expected != "any" {
            assert_eq!(
                label(r.capability()),
                expected,
                "{name}: {:?}",
                r.capability()
            );
        }
        let Some(j) = r.value() else {
            refused += 1;
            continue;
        };
        // A layout the family does not qualify yet is listed, never silently dropped.
        if matches!(name, "bad_time.h5" | "bad_units.h5" | "unknown_variable.h5") {
            assert!(!j.omitted.is_empty(), "{name}");
        }
        for f in &j.features {
            for l in &f.series {
                let d = &j.datasets[l.dataset];
                assert!(
                    names_feature(d, l.column, &f.feature.name, &f.feature.area),
                    "{name}"
                );
                check_column(&s, fam, d, l.column);
            }
        }
    }
    assert!(refused >= 5);
}

#[test]
fn joins_repeat_identically_in_one_session() {
    let s = open("reference_lines", "per_area_valid.h5");
    assert_eq!(
        s.reference_line_results(DEFAULT_MAX_BYTES).unwrap(),
        s.reference_line_results(DEFAULT_MAX_BYTES).unwrap()
    );
    let s = open("reference_points", "valid.h5");
    assert_eq!(
        s.reference_point_results(DEFAULT_MAX_BYTES).unwrap(),
        s.reference_point_results(DEFAULT_MAX_BYTES).unwrap()
    );
}

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
/// Little-endian bytes of `rows` rows (of `width` columns) from a numeric array, starting at `start`.
fn row_bytes(a: &ras_commander_fast_hdf::hdf::NumericArray, start: u64, rows: u64) -> Vec<u8> {
    let all = bytes(&a.values);
    let row = all.len() / a.shape[0] as usize;
    all[start as usize * row..(start + rows) as usize * row].to_vec()
}
fn column_sha(d: &ReferenceResultDataset, column: u64) -> String {
    let all = bytes(&d.values.values);
    let (rows, cols) = (d.values.shape[0] as usize, d.values.shape[1] as usize);
    let width = all.len() / (rows * cols);
    let mut out = Vec::new();
    for r in 0..rows {
        let at = (r * cols + column as usize) * width;
        out.extend_from_slice(&all[at..at + width]);
    }
    sha(&out)
}
type Series = std::collections::BTreeSet<(String, u64, String)>;
/// Engine join versus the h5py oracle for one geometry row: same name and geometry digest;
/// every engine series is an oracle series; every oracle series is joined or its dataset omitted.
fn compare_row(
    what: &str,
    want: &serde_json::Value,
    name: &str,
    geometry_sha: String,
    got: Series,
    omitted: &std::collections::BTreeSet<String>,
) -> usize {
    assert_eq!(want["name"].as_str().unwrap(), name, "{what}");
    assert_eq!(
        want["geometry_sha256"].as_str().unwrap(),
        geometry_sha,
        "{what} geometry"
    );
    let oracle: Series = want["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s[0].as_str().unwrap().to_owned(),
                s[1].as_u64().unwrap(),
                s[2].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for s in &got {
        assert!(
            oracle.contains(s),
            "{what}: engine series not in the h5py join {s:?}"
        );
    }
    for s in &oracle {
        assert!(
            got.contains(s) || omitted.contains(&s.0),
            "{what}: h5py series neither joined nor omitted {s:?}"
        );
    }
    got.len()
}

/// `RAS_REFERENCE_RESULTS_ORACLE` is the JSON written by `reference_results_oracle.py` (h5py
/// only) for the plans in `RAS_REFERENCE_RESULTS_FIXTURES`. Each supported engine join must
/// equal the oracle's Name-axis join row by row; a refused join must be one the oracle also
/// cannot make, or be listed in `REFUSED` with its reason.
///
/// Expected refusals (plan, kind): the eBFE 6.x features plan stores reference-line results
/// whose Name entries match by name, but its `Internal Faces` table has no integer columns
/// to cross-check the link, so the engine refuses (IdentityLayout).
const REFUSED: &[(&str, &str)] = &[("1101001306.p01.features.hdf", "lines")];
#[test]
#[ignore = "requires RAS_REFERENCE_RESULTS_ORACLE and RAS_REFERENCE_RESULTS_FIXTURES"]
fn reference_results_match_the_h5py_join() {
    let oracle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("RAS_REFERENCE_RESULTS_ORACLE").unwrap()).unwrap(),
    )
    .unwrap();
    let root = PathBuf::from(std::env::var("RAS_REFERENCE_RESULTS_FIXTURES").unwrap());
    let (mut joined, mut refused) = (0, Vec::new());
    for (plan, want) in oracle.as_object().unwrap() {
        let path = root.join(plan);
        assert_eq!(
            sha(&std::fs::read(&path).unwrap()),
            want["sha256"].as_str().unwrap()
        );
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        let lines = s.reference_line_results(DEFAULT_MAX_BYTES).unwrap();
        match (lines.value(), want["lines"]["rows"].as_array()) {
            (Some(j), Some(rows)) => {
                assert_eq!(j.features.len(), rows.len(), "{plan} lines");
                let omitted = j.omitted.iter().map(|o| o.0.clone()).collect();
                for (f, w) in j.features.iter().zip(rows) {
                    let r = &f.feature.point_range;
                    let got = f
                        .series
                        .iter()
                        .map(|l| {
                            let d = &j.datasets[l.dataset];
                            (
                                d.dataset.stored.path.clone(),
                                l.column,
                                column_sha(d, l.column),
                            )
                        })
                        .collect();
                    let geometry = sha(&row_bytes(&j.geometry.points, r.start, r.end - r.start));
                    joined += compare_row(
                        &format!("{plan} line {}", f.feature.name),
                        w,
                        &f.feature.name,
                        geometry,
                        got,
                        &omitted,
                    );
                }
            }
            (None, None) => assert!(lines.is_unavailable(), "{plan} lines"),
            (None, Some(_)) => {
                println!("refused: {plan} lines: {:?}", lines.capability());
                refused.push((plan.clone(), "lines"));
            }
            (Some(_), None) => panic!("{plan}: engine joined lines the oracle has no geometry for"),
        }
        let points = s.reference_point_results(DEFAULT_MAX_BYTES).unwrap();
        match (points.value(), want["points"]["rows"].as_array()) {
            (Some(j), Some(rows)) => {
                assert_eq!(j.features.len(), rows.len(), "{plan} points");
                let omitted = j.omitted.iter().map(|o| o.0.clone()).collect();
                for (f, w) in j.features.iter().zip(rows) {
                    let got = f
                        .series
                        .iter()
                        .map(|l| {
                            let d = &j.datasets[l.dataset];
                            (
                                d.dataset.stored.path.clone(),
                                l.column,
                                column_sha(d, l.column),
                            )
                        })
                        .collect();
                    let geometry = sha(&row_bytes(&j.geometry.points, f.feature.point_row, 1));
                    joined += compare_row(
                        &format!("{plan} point {}", f.feature.name),
                        w,
                        &f.feature.name,
                        geometry,
                        got,
                        &omitted,
                    );
                }
            }
            (None, None) => assert!(points.is_unavailable(), "{plan} points"),
            (None, Some(_)) => {
                println!("refused: {plan} points: {:?}", points.capability());
                refused.push((plan.clone(), "points"));
            }
            (Some(_), None) => {
                panic!("{plan}: engine joined points the oracle has no geometry for")
            }
        }
    }
    println!("joined columns checked against h5py: {joined}");
    let expected: Vec<(String, &str)> = REFUSED
        .iter()
        .filter(|(p, _)| oracle.get(*p).is_some())
        .map(|(p, k)| (p.to_string(), *k))
        .collect();
    assert_eq!(refused, expected);
    assert!(joined > 0);
}
