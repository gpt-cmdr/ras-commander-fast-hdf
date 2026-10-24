//! Sessions opened from bytes (`ReadOnlyHdfSession::from_bytes`) read exactly what sessions
//! opened by path read, on the frozen coverage fixtures, and refuse the same way.
#![cfg(feature = "mesh")]
use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;
use ras_commander_fast_hdf::mesh::GeometryPolicy;
use ras_commander_fast_hdf::ErrorKind;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/coverage")
        .join(name)
}

fn both(name: &str) -> (ReadOnlyHdfSession, ReadOnlyHdfSession) {
    let path = fixture(name);
    let by_path = ReadOnlyHdfSession::with_options(&path, 1, false).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let in_memory = ReadOnlyHdfSession::from_bytes(bytes, name, 1, false).unwrap();
    (by_path, in_memory)
}

#[test]
fn in_memory_sessions_read_what_path_sessions_read() {
    let (a, b) = both("coverage_mesh.h5");
    assert_eq!(b.path(), std::path::Path::new("coverage_mesh.h5"));
    assert_eq!(a.mesh_names().unwrap(), b.mesh_names().unwrap());
    assert_eq!(
        a.typed_group_attributes("/").unwrap(),
        b.typed_group_attributes("/").unwrap()
    );
    for mesh in a.mesh_names().unwrap() {
        let (ta, tb) = (
            a.read_topology(&mesh).map(|t| t.topology),
            b.read_topology(&mesh).map(|t| t.topology),
        );
        match (ta, tb) {
            (Ok(ta), Ok(tb)) => {
                let pa = ta.assemble_with_session(GeometryPolicy::Strict, &a);
                let pb = tb.assemble_with_session(GeometryPolicy::Strict, &b);
                match (pa, pb) {
                    (Ok(pa), Ok(pb)) => {
                        assert_eq!(pa.cell_ids, pb.cell_ids, "{mesh}");
                        assert_eq!(pa.offsets, pb.offsets, "{mesh}");
                        assert_eq!(
                            pa.coordinates
                                .iter()
                                .map(|c| c.map(f64::to_bits))
                                .collect::<Vec<_>>(),
                            pb.coordinates
                                .iter()
                                .map(|c| c.map(f64::to_bits))
                                .collect::<Vec<_>>(),
                            "{mesh}"
                        );
                    }
                    (Err(ea), Err(eb)) => assert_eq!((ea.kind, ea.code), (eb.kind, eb.code)),
                    (pa, pb) => panic!("{mesh}: {:?} vs {:?}", pa.err(), pb.err()),
                }
            }
            (Err(ea), Err(eb)) => assert_eq!((ea.kind, ea.code), (eb.kind, eb.code), "{mesh}"),
            (ta, tb) => panic!("{mesh}: {:?} vs {:?}", ta.err(), tb.err()),
        }
    }
}

#[test]
fn bytes_that_are_not_hdf5_are_refused() {
    let e = ReadOnlyHdfSession::from_bytes(b"not an HDF5 file".to_vec(), "x.hdf", 1, false)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Corrupt, "{e}");
    assert_eq!(e.path.as_deref(), Some(std::path::Path::new("x.hdf")));
    let mut truncated = std::fs::read(fixture("coverage_mesh.h5")).unwrap();
    truncated.truncate(truncated.len() / 3);
    if let Ok(s) = ReadOnlyHdfSession::from_bytes(truncated, "cut.hdf", 1, false) {
        // A cut file may still open; its reads must then fail, never return data.
        for mesh in s.mesh_names().unwrap_or_default() {
            assert!(s
                .read_topology(&mesh)
                .and_then(|t| t.topology.assemble_with_session(GeometryPolicy::Strict, &s))
                .is_err());
        }
    }
}
