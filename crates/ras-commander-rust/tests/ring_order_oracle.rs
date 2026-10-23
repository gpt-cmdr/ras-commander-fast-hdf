//! Fixture-level oracle for the public ring-order contract (clockwise, start at the first
//! listed face, consecutive duplicates dropped): the native packed rings must equal
//! ras-commander's Python output *exactly* (coordinate bits, ring offsets, source cell IDs and
//! omitted cells), on real HEC-RAS fixtures. Equal coordinates and offsets give equal WKB.
//!
//! RAS_RING_ORDER_ORACLE = directory written by `generate_ring_order_reference.py`.
//! RAS_RING_ORDER_NATIVE_DUMP (optional) = directory that receives the native rings in the
//! same raw layout plus `native_manifest.json`, for the independent numpy/shapely checks.
//! Fixtures are opened read-only and verified by sha256.
#![cfg(feature = "mesh")]
use ras_commander_rust::hdf::ReadOnlyHdfSession;
use ras_commander_rust::mesh::{GeometryPolicy, OmissionKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const REQUIRED: [&str; 4] = [
    "BaldEagleDamBrk.p07",
    "DavisStormSystem.p02",
    "JUL2020_D100RAS.p04",
    "AustinOyster.p01",
];

fn sha256_file(path: &Path) -> String {
    use std::io::Read;
    let mut file = std::fs::File::open(path).expect("fixture");
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1 << 22];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    format!("{:x}", hash.finalize())
}
fn word(bytes: &[u8], index: usize) -> [u8; 8] {
    bytes[index * 8..index * 8 + 8].try_into().unwrap()
}
fn read_f64(path: &Path) -> Vec<[f64; 2]> {
    let bytes = std::fs::read(path).unwrap();
    (0..bytes.len() / 16)
        .map(|i| {
            [
                f64::from_le_bytes(word(&bytes, 2 * i)),
                f64::from_le_bytes(word(&bytes, 2 * i + 1)),
            ]
        })
        .collect()
}
fn read_u64(path: &Path) -> Vec<u64> {
    let bytes = std::fs::read(path).unwrap();
    (0..bytes.len() / 8)
        .map(|i| u64::from_le_bytes(word(&bytes, i)))
        .collect()
}
fn read_i64(path: &Path) -> Vec<i64> {
    let bytes = std::fs::read(path).unwrap();
    (0..bytes.len() / 8)
        .map(|i| i64::from_le_bytes(word(&bytes, i)))
        .collect()
}
fn bytes_of<T: Copy>(v: &[T], f: impl Fn(T) -> Vec<u8>) -> Vec<u8> {
    v.iter().flat_map(|&x| f(x)).collect()
}
/// Python's reason code for an omitted cell, as the native kind spells it.
fn native_reason(kind: OmissionKind) -> &'static str {
    match kind {
        OmissionKind::ZeroFaces => "zero_faces",
        OmissionKind::InsufficientFaces => "insufficient_faces",
        OmissionKind::InvalidGeometry => "no_polygon",
        _ => "other",
    }
}

#[test]
#[ignore = "requires RAS_RING_ORDER_ORACLE and the real fixtures"]
fn native_rings_equal_the_python_reference_on_real_fixtures() {
    let dir = PathBuf::from(
        std::env::var_os("RAS_RING_ORDER_ORACLE").expect("set RAS_RING_ORDER_ORACLE"),
    );
    let dump = std::env::var_os("RAS_RING_ORDER_NATIVE_DUMP").map(PathBuf::from);
    if let Some(d) = &dump {
        std::fs::create_dir_all(d).unwrap();
    }
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let entries = manifest["entries"].as_array().unwrap();
    for required in REQUIRED {
        assert!(
            entries.iter().any(|e| e["fixture"] == required),
            "reference lacks {required}"
        );
    }
    let mut hashes: std::collections::HashMap<String, String> = Default::default();
    let mut native_entries = Vec::new();
    let (mut rings_total, mut mismatches_total) = (0usize, 0usize);
    for entry in entries {
        let fixture = entry["fixture"].as_str().unwrap();
        let path = PathBuf::from(entry["path"].as_str().unwrap());
        let sha = hashes
            .entry(path.to_string_lossy().into_owned())
            .or_insert_with(|| sha256_file(&path));
        assert_eq!(sha, entry["sha256"].as_str().unwrap(), "{fixture} changed");
        let mesh = entry["mesh"].as_str().unwrap();
        let stem = entry["stem"].as_str().unwrap();
        let ref_coords = read_f64(&dir.join(format!("{stem}.coords.f64")));
        let ref_offsets = read_u64(&dir.join(format!("{stem}.offsets.u64")));
        let ref_cells = read_i64(&dir.join(format!("{stem}.cells.i64")));
        assert_eq!(ref_cells.len() as u64, entry["rings"].as_u64().unwrap());

        let session = ReadOnlyHdfSession::with_options(&path, 4, true).unwrap();
        let packed = session
            .read_topology(mesh)
            .unwrap()
            .topology
            .assemble(GeometryPolicy::QuarantineInvalid, 4)
            .unwrap();

        // Source cell IDs (kept cells and therefore omitted cells), offsets, coordinates.
        assert_eq!(
            packed.cell_ids, ref_cells,
            "{fixture}/{mesh}: cell sets differ"
        );
        assert_eq!(
            packed.offsets, ref_offsets,
            "{fixture}/{mesh}: ring offsets differ"
        );
        let mut mismatched_rings = 0usize;
        assert_eq!(packed.coordinates.len(), ref_coords.len());
        for row in 0..packed.cell_ids.len() {
            let range = packed.offsets[row] as usize..packed.offsets[row + 1] as usize;
            let same = packed.coordinates[range.clone()]
                .iter()
                .zip(&ref_coords[range])
                .all(|(a, b)| a[0].to_bits() == b[0].to_bits() && a[1].to_bits() == b[1].to_bits());
            if !same {
                mismatched_rings += 1;
            }
        }
        println!(
            "ring-order oracle {fixture}/{mesh}: rings {} mismatches {mismatched_rings}",
            packed.cell_ids.len()
        );
        rings_total += packed.cell_ids.len();
        mismatches_total += mismatched_rings;
        assert_eq!(
            mismatched_rings, 0,
            "{fixture}/{mesh}: ring coordinates differ"
        );

        // Omitted cells and their reasons. Python's `ambiguous` cells keep polygon[0] and are
        // not omitted; every omitted Python cell must be omitted here with the same reason.
        let mut python_omitted: Vec<(i64, String)> = entry["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["status"] != "ambiguous")
            .map(|d| {
                (
                    d["cell_id"].as_i64().unwrap(),
                    d["reason_code"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        python_omitted.sort();
        let mut native_omitted: Vec<(i64, String)> = packed
            .diagnostics
            .iter()
            .map(|d| (d.cell_id, native_reason(d.kind).to_string()))
            .collect();
        native_omitted.sort();
        assert_eq!(
            native_omitted, python_omitted,
            "{fixture}/{mesh}: omitted cells differ"
        );

        if let Some(out) = &dump {
            let coords = bytes_of(&packed.coordinates, |p| {
                [p[0].to_le_bytes(), p[1].to_le_bytes()].concat()
            });
            std::fs::write(out.join(format!("{stem}.coords.f64")), coords).unwrap();
            std::fs::write(
                out.join(format!("{stem}.offsets.u64")),
                bytes_of(&packed.offsets, |v| v.to_le_bytes().to_vec()),
            )
            .unwrap();
            std::fs::write(
                out.join(format!("{stem}.cells.i64")),
                bytes_of(&packed.cell_ids, |v| v.to_le_bytes().to_vec()),
            )
            .unwrap();
            native_entries.push(json!({
                "fixture": fixture, "path": entry["path"], "mesh": mesh, "stem": stem,
                "rings": packed.cell_ids.len(),
                "diagnostics": packed.diagnostics.iter().map(|d| json!({
                    "cell_id": d.cell_id, "kind": format!("{:?}", d.kind),
                    "reason_code": native_reason(d.kind), "reason": d.reason,
                })).collect::<Vec<_>>(),
            }));
        }
    }
    println!("ring-order oracle total: rings {rings_total} mismatches {mismatches_total}");
    if let Some(out) = &dump {
        std::fs::write(
            out.join("native_manifest.json"),
            serde_json::to_vec_pretty(&json!({"entries": native_entries})).unwrap(),
        )
        .unwrap();
    }
}
