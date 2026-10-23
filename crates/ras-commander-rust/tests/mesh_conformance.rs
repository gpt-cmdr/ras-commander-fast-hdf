//! Full polygon/composite-identity oracle uses independent Python polygonize.
//! Canonicalization removes only adjacent duplicates and chooses orientation/start;
//! no tolerance, snapping, repair or collinear vertex removal.
#![cfg(feature = "mesh")]
use ras_commander_rust::hdf::ReadOnlyHdfSession;
use ras_commander_rust::mesh::{GeometryPolicy, OmissionKind};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
fn hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn fixture(name: &str, expected: &str) -> PathBuf {
    let p = PathBuf::from(
        std::env::var_os("RAS_NATIVE_FIXTURES")
            .expect("set RAS_NATIVE_FIXTURES to immutable5/6/7 fixture directory"),
    )
    .join(name);
    assert_eq!(
        hex(&std::fs::read(&p).expect("real fixture required")),
        expected
    );
    p
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
        "JUL2020_D100RAS.p04.hdf",
        "37d91b9f957f78067cb0a8ae5db7cf3355f9c5d7172cb40e7931c35be78b9683",
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
            assert!(
                matches!(named.topology.assemble(GeometryPolicy::Strict,threads),Err(e) if e.kind==ras_commander_rust::ErrorKind::InvalidGeometry&&e.message.contains("317422"))
            );
        }
        assert_eq!(count, 340246);
        assert_eq!(
            format!("{:x}", hash.finalize()),
            "cf885b8038e4d26c0a92a0a9f2686349cbe7571deb26a9365f647990fa27fa67"
        );
        assert_eq!(invalid, vec![317422, 339176]);
    }
    assert_eq!(
        hex(&std::fs::read(&p).unwrap()),
        "37d91b9f957f78067cb0a8ae5db7cf3355f9c5d7172cb40e7931c35be78b9683"
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
        hex(&std::fs::read(&p).unwrap()),
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
        hex(&std::fs::read(&p).unwrap()),
        "1d0063a84fd131ac95ffaa2b3f6647f2ef51c7f38f23c43e8ae6c698bfc95f0b"
    );
}
