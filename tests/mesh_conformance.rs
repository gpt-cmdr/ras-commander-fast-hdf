//! Full polygon/composite-identity oracle uses independent Python polygonize.
//! Canonicalization removes only adjacent duplicates and chooses orientation/start;
//! no tolerance, snapping, repair or collinear vertex removal.
#![cfg(feature = "mesh")]
use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;
use ras_commander_fast_hdf::mesh::{GeometryPolicy, OmissionKind};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
fn fixture(name: &str, expected: &str) -> PathBuf {
    let p = PathBuf::from(
        std::env::var_os("RAS_NATIVE_FIXTURES")
            .expect("set RAS_NATIVE_FIXTURES to immutable5/6/7 fixture directory"),
    )
    .join(name);
    assert_eq!(file_sha256(&p), expected);
    p
}
/// Streamed SHA-256, so multi-GB fixtures are not read into memory.
fn file_sha256(path: &std::path::Path) -> String {
    use std::io::Read;
    let mut file = std::fs::File::open(path).expect("real fixture required");
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

fn canonical(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut p = Vec::new();
    for &v in points {
        if p.last() != Some(&v) {
            p.push(v);
        }
    }
    if p.last() == p.first() {
        p.pop();
    }
    let cmp = |a: &[f64; 2], b: &[f64; 2]| {
        a[0].partial_cmp(&b[0])
            .unwrap()
            .then(a[1].partial_cmp(&b[1]).unwrap())
    };
    let i = (0..p.len()).min_by(|&i, &j| cmp(&p[i], &p[j])).unwrap();
    p.rotate_left(i);
    let mut q = p.clone();
    q.reverse();
    let j = (0..q.len()).min_by(|&i, &j| cmp(&q[i], &q[j])).unwrap();
    q.rotate_left(j);
    for (a, b) in p.iter().zip(&q) {
        let order = cmp(a, b);
        if order.is_lt() {
            return p;
        }
        if order.is_gt() {
            return q;
        }
    }
    p
}

#[test]
#[ignore = "requires immutable real5/6/7 fixtures"]
fn real_mesh_0() {
    let p = fixture(
        "AustinOyster.p03.hdf",
        "fdaff5045ad36bf5d424489ff64b3d86a1e0e3ef92a628554488f009047cad92",
    );
    for threads in [1, 4] {
        let s = ReadOnlyHdfSession::with_options(&p, threads, true).unwrap();
        let mut names = s.mesh_names().unwrap();
        names.sort();
        let mut hash = Sha256::new();
        let mut count = 0;
        let mut invalid = Vec::new();
        for name in names {
            let named = s.read_topology(&name).unwrap();
            assert_eq!(named.source, p);
            assert_eq!(named.mesh_name, name);
            let packed = named
                .topology
                .assemble(GeometryPolicy::QuarantineInvalid, threads)
                .unwrap();
            for d in &packed.diagnostics {
                if d.kind == OmissionKind::InvalidGeometry {
                    invalid.push(d.cell_id);
                }
            }
            for (row, id) in packed.cell_ids.iter().enumerate() {
                let ring = canonical(
                    &packed.coordinates
                        [packed.offsets[row] as usize..packed.offsets[row + 1] as usize],
                );
                hash.update((name.len() as u64).to_le_bytes());
                hash.update(name.as_bytes());
                hash.update(id.to_le_bytes());
                hash.update((ring.len() as u64).to_le_bytes());
                for v in ring {
                    for x in v {
                        hash.update(x.to_le_bytes());
                    }
                }
                count += 1;
            }
            // The public 5.0.7 eBFE mesh has no invalid cells: strict assembly succeeds too.
            assert!(named
                .topology
                .assemble(GeometryPolicy::Strict, threads)
                .is_ok());
        }
        assert_eq!(count, 1872746);
        assert_eq!(
            format!("{:x}", hash.finalize()),
            "6641444a93bb83a9f6b7a8d9ed586f6b4f56e63775c0f01e834cbd306f1ca392"
        );
        assert!(invalid.is_empty());
    }
    assert_eq!(
        file_sha256(&p),
        "fdaff5045ad36bf5d424489ff64b3d86a1e0e3ef92a628554488f009047cad92"
    );
}

#[test]
#[ignore = "requires immutable real5/6/7 fixtures"]
fn real_mesh_1() {
    let p = fixture(
        "BaldEagleDamBrk.p07.hdf",
        "07cad3930a679891f87bbface1ab749b82fd7d7eae6aaac3b5ff041c44280984",
    );
    for threads in [1, 4] {
        let s = ReadOnlyHdfSession::with_options(&p, threads, true).unwrap();
        let mut names = s.mesh_names().unwrap();
        names.sort();
        let mut hash = Sha256::new();
        let mut count = 0;
        let mut invalid = Vec::new();
        for name in names {
            let named = s.read_topology(&name).unwrap();
            assert_eq!(named.source, p);
            assert_eq!(named.mesh_name, name);
            let packed = named
                .topology
                .assemble(GeometryPolicy::QuarantineInvalid, threads)
                .unwrap();
            for d in &packed.diagnostics {
                if d.kind == OmissionKind::InvalidGeometry {
                    invalid.push(d.cell_id);
                }
            }
            for (row, id) in packed.cell_ids.iter().enumerate() {
                let ring = canonical(
                    &packed.coordinates
                        [packed.offsets[row] as usize..packed.offsets[row + 1] as usize],
                );
                hash.update((name.len() as u64).to_le_bytes());
                hash.update(name.as_bytes());
                hash.update(id.to_le_bytes());
                hash.update((ring.len() as u64).to_le_bytes());
                for v in ring {
                    for x in v {
                        hash.update(x.to_le_bytes());
                    }
                }
                count += 1;
            }
        }
        assert_eq!(count, 87039);
        assert_eq!(
            format!("{:x}", hash.finalize()),
            "184d129f464bd73912a37ffd6ead7d3fc770e63a2749023ee3b8592de14a0d8f"
        );
        assert!(invalid.is_empty());
    }
    assert_eq!(
        file_sha256(&p),
        "07cad3930a679891f87bbface1ab749b82fd7d7eae6aaac3b5ff041c44280984"
    );
}

#[test]
#[ignore = "requires immutable real5/6/7 fixtures"]
fn real_mesh_2() {
    let p = fixture(
        "BaldEagleDamBrk.p18.hdf",
        "1d0063a84fd131ac95ffaa2b3f6647f2ef51c7f38f23c43e8ae6c698bfc95f0b",
    );
    for threads in [1, 4] {
        let s = ReadOnlyHdfSession::with_options(&p, threads, true).unwrap();
        let mut names = s.mesh_names().unwrap();
        names.sort();
        let mut hash = Sha256::new();
        let mut count = 0;
        let mut invalid = Vec::new();
        for name in names {
            let named = s.read_topology(&name).unwrap();
            assert_eq!(named.source, p);
            assert_eq!(named.mesh_name, name);
            let packed = named
                .topology
                .assemble(GeometryPolicy::QuarantineInvalid, threads)
                .unwrap();
            for d in &packed.diagnostics {
                if d.kind == OmissionKind::InvalidGeometry {
                    invalid.push(d.cell_id);
                }
            }
            for (row, id) in packed.cell_ids.iter().enumerate() {
                let ring = canonical(
                    &packed.coordinates
                        [packed.offsets[row] as usize..packed.offsets[row + 1] as usize],
                );
                hash.update((name.len() as u64).to_le_bytes());
                hash.update(name.as_bytes());
                hash.update(id.to_le_bytes());
                hash.update((ring.len() as u64).to_le_bytes());
                for v in ring {
                    for x in v {
                        hash.update(x.to_le_bytes());
                    }
                }
                count += 1;
            }
        }
        assert_eq!(count, 4425);
        assert_eq!(
            format!("{:x}", hash.finalize()),
            "f0449ef7cef8b077911125f68de7dae137e466504267471c53d22fffaaaa6bae"
        );
        assert!(invalid.is_empty());
    }
    assert_eq!(
        file_sha256(&p),
        "1d0063a84fd131ac95ffaa2b3f6647f2ef51c7f38f23c43e8ae6c698bfc95f0b"
    );
}
