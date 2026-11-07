#![cfg(feature = "hdf")]
use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;
use ras_commander_fast_hdf::{ErrorKind, UnsupportedCode};

fn session(name: &str) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy_mesh")
            .join(format!("{name}.h5")),
    )
    .unwrap()
}

#[test]
fn legacy_names_keep_stored_order_and_counts_ignore_stale_attributes() {
    let s = session("valid");
    assert!(s.mesh_name_diagnostics(1 << 20).unwrap().is_empty());
    let meshes = s.mesh_inventory(1 << 20).unwrap();
    assert_eq!(
        meshes
            .iter()
            .map(|m| m.mesh_name.as_str())
            .collect::<Vec<_>>(),
        ["Zulu", "Alpha"]
    );
    for (row, m) in meshes.iter().enumerate() {
        assert_eq!((m.physical_cells, m.stored_cells, m.ghost_cells), (2, 4, 2));
        assert_eq!(m.id.table, "Geometry/2D Flow Areas/Names");
        assert_eq!(m.id.row, row as u64);
    }
    let m = session("no_ghosts").mesh_inventory(1 << 20).unwrap();
    assert_eq!((m[0].physical_cells, m[0].ghost_cells), (2, 0));
}

#[test]
fn present_modern_metadata_takes_precedence() {
    let m = session("modern").mesh_inventory(1 << 20).unwrap();
    assert_eq!(m[0].mesh_name, "Alpha");
    assert_eq!(m[0].physical_cells, 3);
    assert_eq!(m[0].id.table, "Geometry/2D Flow Areas/Attributes");
}

#[test]
fn ambiguous_or_damaged_legacy_layouts_are_errors() {
    for name in [
        "two_faces",
        "interleaved",
        "gap",
        "duplicates",
        "missing_group",
        "invalid_name",
        "rank_two",
    ] {
        let e = session(name).mesh_inventory(1 << 20).unwrap_err();
        assert!(
            matches!(
                e.kind,
                ErrorKind::Corrupt | ErrorKind::Unsupported | ErrorKind::NotFound
            ),
            "{name}: {e:?}"
        );
    }
    assert_eq!(
        session("valid").mesh_inventory(1).unwrap_err().code,
        Some(UnsupportedCode::ResourceLimit)
    );
}

#[test]
#[ignore = "requires an immutable local WA2 geometry copy in RAS_LEGACY_MESH_HDF"]
fn real_legacy_mesh_metadata() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let path = std::env::var_os("RAS_LEGACY_MESH_HDF").expect("set RAS_LEGACY_MESH_HDF");
    let mut file = std::fs::File::open(&path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    assert_eq!(
        format!("{:x}", hash.finalize()),
        "60ecd5c354beba7bfeff3378867a3d070f9c2eb4415e098fa33a44f42e45d9f2"
    );
    let s = ReadOnlyHdfSession::open(&path).unwrap();
    let m = s.mesh_inventory(16 << 20).unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].mesh_name, "WA2");
    assert_eq!(
        (m[0].physical_cells, m[0].stored_cells, m[0].ghost_cells),
        (138005, 140875, 2870)
    );
    assert!(s.mesh_name_diagnostics(16 << 20).unwrap().is_empty());
}
