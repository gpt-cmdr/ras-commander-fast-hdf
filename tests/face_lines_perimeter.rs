//! Face polylines and stored perimeters (see `Topology::face_lines`,
//! `ReadOnlyHdfSession::read_mesh_perimeter`). Synthetic cases use the frozen coverage
//! fixtures; the opt-in real-file test compares with ras-commander's Python readers.
#![cfg(feature = "mesh")]
use ras_commander_fast_hdf::hdf::{ReadOnlyHdfSession, DEFAULT_READ_MAX_BYTES};
use ras_commander_fast_hdf::ErrorKind;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/coverage")
        .join(name)
}

#[test]
fn face_lines_follow_the_stored_tables_exactly() {
    let s = ReadOnlyHdfSession::with_options(fixture("coverage_mesh.h5"), 1, true).unwrap();
    let t = s.read_topology("Good").unwrap().topology;
    let lines = t.face_lines().unwrap();
    let faces = t.ends.nrows();
    assert_eq!(lines.face_ids, (0..faces as i64).collect::<Vec<_>>());
    assert_eq!(lines.offsets.len(), faces + 1);
    for f in 0..faces {
        let line = &lines.coordinates[lines.offsets[f] as usize..lines.offsets[f + 1] as usize];
        let (a, b) = (t.ends[[f, 0]] as usize, t.ends[[f, 1]] as usize);
        let (start, count) = (t.perim_info[[f, 0]] as usize, t.perim_info[[f, 1]] as usize);
        assert_eq!(line.len(), count + 2, "face {f}");
        assert_eq!(line[0], [t.points[[a, 0]], t.points[[a, 1]]]);
        assert_eq!(line[line.len() - 1], [t.points[[b, 0]], t.points[[b, 1]]]);
        for (k, row) in (start..start + count).enumerate() {
            assert_eq!(line[k + 1], [t.perim[[row, 0]], t.perim[[row, 1]]]);
        }
    }
    // Same result whatever the coordinate precision or connectivity width.
    for name in ["F32Coords", "U16Conn"] {
        let other = s
            .read_topology(name)
            .unwrap()
            .topology
            .face_lines()
            .unwrap();
        assert_eq!(other, lines, "{name}");
    }
}

#[test]
fn absent_perimeter_is_not_found_and_names_are_validated() {
    let s = ReadOnlyHdfSession::with_options(fixture("coverage_mesh.h5"), 1, true).unwrap();
    let e = s
        .read_mesh_perimeter("Good", DEFAULT_READ_MAX_BYTES)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound, "{e}");
    for bad in ["", "Good/Sub", r"Good\Sub"] {
        assert_eq!(
            s.read_mesh_perimeter(bad, DEFAULT_READ_MAX_BYTES)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidRequest
        );
    }
}

/// `RAS_NATIVE_FIXTURES` (folder of HDF files) and `RAS_FACE_PERIMETER_ORACLE` (JSON from
/// `tests/fixtures/face_perimeter_oracle.py`, which calls ras-commander's Python readers with
/// h5py) must both be set.
#[test]
#[ignore]
fn real_face_lines_and_perimeters_match_ras_commander() {
    let root = PathBuf::from(std::env::var("RAS_NATIVE_FIXTURES").expect("RAS_NATIVE_FIXTURES"));
    let oracle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::env::var("RAS_FACE_PERIMETER_ORACLE").expect("RAS_FACE_PERIMETER_ORACLE"),
        )
        .unwrap(),
    )
    .unwrap();
    let digest = |coords: &[[f64; 2]], offsets: &[u64]| -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        for w in offsets.windows(2) {
            h.update((w[1] - w[0]).to_le_bytes());
            for c in &coords[w[0] as usize..w[1] as usize] {
                h.update(c[0].to_le_bytes());
                h.update(c[1].to_le_bytes());
            }
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    };
    let files = oracle.as_object().unwrap();
    assert!(!files.is_empty());
    for (file, meshes) in files {
        let s = ReadOnlyHdfSession::open(root.join(file)).unwrap();
        for (mesh, want) in meshes.as_object().unwrap() {
            let lines = s
                .read_topology(mesh)
                .unwrap()
                .topology
                .face_lines()
                .unwrap();
            assert_eq!(
                lines.face_ids.len() as u64,
                want["faces"].as_u64().unwrap(),
                "{file} {mesh}"
            );
            assert_eq!(
                digest(&lines.coordinates, &lines.offsets),
                want["faces_sha256"].as_str().unwrap(),
                "{file} {mesh} faces"
            );
            match want["perimeter_sha256"].as_str() {
                Some(sha) => {
                    let p = s.read_mesh_perimeter(mesh, DEFAULT_READ_MAX_BYTES).unwrap();
                    assert_eq!(
                        digest(&p.ring, &[0, p.ring.len() as u64]),
                        sha,
                        "{file} {mesh} perimeter"
                    );
                }
                None => assert_eq!(
                    s.read_mesh_perimeter(mesh, DEFAULT_READ_MAX_BYTES)
                        .unwrap_err()
                        .kind,
                    ErrorKind::NotFound
                ),
            }
        }
    }
}
