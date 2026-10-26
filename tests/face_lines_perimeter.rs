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

#[test]
fn session_face_lines_read_only_the_face_tables_and_match_topology() {
    let s = ReadOnlyHdfSession::with_options(fixture("coverage_mesh.h5"), 1, true).unwrap();
    for name in ["Good", "F32Coords", "U16Conn"] {
        let want = s
            .read_topology(name)
            .unwrap()
            .topology
            .face_lines()
            .unwrap();
        let got = s
            .read_mesh_face_lines(name, DEFAULT_READ_MAX_BYTES)
            .unwrap();
        assert_eq!(got, want, "{name}");
    }
    // Cell topology is not read, so a mesh whose cells cannot be assembled (a bowtie), whose
    // cell tables are malformed or whose cell values are absent still has exact face lines.
    for name in [
        "Bowtie",
        "OverrunCellInfo",
        "NegativeOffset",
        "FaceIndexOutOfRange",
        "MissingValues",
    ] {
        let got = s
            .read_mesh_face_lines(name, DEFAULT_READ_MAX_BYTES)
            .unwrap();
        assert_eq!(got.face_ids.len(), 7, "{name}");
    }
    let empty = s
        .read_mesh_face_lines("Empty", DEFAULT_READ_MAX_BYTES)
        .unwrap();
    assert!(empty.face_ids.is_empty() && empty.coordinates.is_empty());
    assert_eq!(empty.offsets, vec![0]);
}

#[test]
fn session_face_lines_refuse_layouts_the_python_loop_reads_differently() {
    let s = ReadOnlyHdfSession::with_options(fixture("coverage_mesh.h5"), 1, true).unwrap();
    // Python would unpack, build 3D lines or fail differently: not (rows, 2).
    for name in ["ThreeColumnCoords", "OneColumnEnds", "OneDimCoords"] {
        let e = s
            .read_mesh_face_lines(name, DEFAULT_READ_MAX_BYTES)
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unsupported, "{name}: {e}");
    }
    // Python would wrap a negative index, truncate a slice or zip unequal tables.
    for name in [
        "PerimInfoMismatch",
        "PointIndexOutOfRange",
        "PerimOverrun",
        "NaNCoord",
        "FloatConn",
        "IntCoords",
        "U64Huge",
    ] {
        let e = s
            .read_mesh_face_lines(name, DEFAULT_READ_MAX_BYTES)
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::StructuralTopology, "{name}: {e}");
    }
    assert_eq!(
        s.read_mesh_face_lines("NoSuchMesh", DEFAULT_READ_MAX_BYTES)
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    for bad in ["", "Good/Sub", r"Good\Sub"] {
        assert_eq!(
            s.read_mesh_face_lines(bad, DEFAULT_READ_MAX_BYTES)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidRequest
        );
    }
}

#[test]
fn session_face_lines_budget_the_packed_output() {
    let s = ReadOnlyHdfSession::with_options(fixture("coverage_mesh.h5"), 1, true).unwrap();
    // "Good": 208 bytes of face tables, 7 faces of 2 points each: 14 * 16 + 7 * 16 = 336.
    s.read_mesh_face_lines("Good", 336).unwrap();
    let e = s.read_mesh_face_lines("Good", 335).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported, "{e}");
    assert_eq!(
        e.code,
        Some(ras_commander_fast_hdf::UnsupportedCode::ResourceLimit)
    );
}
