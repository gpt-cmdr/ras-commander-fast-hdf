//! Topology edge cases for mesh.rs, driven by frozen synthetic meshes in
//! tests/fixtures/coverage/coverage_mesh*.h5 (see make_coverage_fixtures.py).
#![cfg(feature = "mesh")]
use ras_commander_fast_hdf::hdf::ReadOnlyHdfSession;
use ras_commander_fast_hdf::mesh::{validate_ring_geometry, GeometryPolicy, OmissionKind, PackedMesh};
use ras_commander_fast_hdf::ErrorKind;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/coverage")
        .join(name)
}
fn session(file: &str, threads: usize) -> ReadOnlyHdfSession {
    ReadOnlyHdfSession::with_options(fixture(file), threads, true).unwrap()
}
fn assemble(
    file: &str,
    mesh: &str,
    policy: GeometryPolicy,
    threads: usize,
) -> ras_commander_fast_hdf::Result<PackedMesh> {
    session(file, threads)
        .read_topology(mesh)
        .unwrap()
        .topology
        .assemble(policy, threads)
}
fn ring(p: &PackedMesh, row: usize) -> &[[f64; 2]] {
    &p.coordinates[p.offsets[row] as usize..p.offsets[row + 1] as usize]
}

#[test]
fn mesh_inventory_is_group_based_and_typed() {
    let s = session("coverage_mesh.h5", 1);
    let mut names = s.mesh_names().unwrap();
    names.sort();
    assert_eq!(names.len(), 19);
    assert!(names.contains(&"Good".to_string()) && names.contains(&"Empty".to_string()));
    assert!(!names.iter().any(|n| n.contains("Unrelated")));
    // A Names-only legacy layout is an explicit Unsupported, never an empty inventory.
    let e = session("coverage_mesh_nogroups.h5", 1)
        .mesh_names()
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    let e = session("coverage_mesh_nogeometry.h5", 1)
        .mesh_names()
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}

#[test]
fn mesh_name_validation_and_missing_mesh() {
    let s = session("coverage_mesh.h5", 1);
    for bad in [
        "",
        "Good/Sub",
        r"Good\Sub",
        "/Good",
        "Geometry/2D Flow Areas/Good",
    ] {
        assert_eq!(
            s.read_topology(bad).unwrap_err().kind,
            ErrorKind::InvalidRequest,
            "{bad:?}"
        );
    }
    assert_eq!(
        s.read_topology("NoSuchMesh").unwrap_err().kind,
        ErrorKind::NotFound
    );
    // A mesh group lacking one required dataset is NotFound, not an empty mesh.
    let e = s.read_topology("MissingValues").unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound, "{e}");
}

#[test]
fn good_mesh_assembles_with_ghost_diagnostic_and_stable_ids() {
    let named = session("coverage_mesh.h5", 1)
        .read_topology("Good")
        .unwrap();
    assert_eq!(named.mesh_name, "Good");
    assert_eq!(named.source, fixture("coverage_mesh.h5"));
    let dbg = format!("{:?}", named.topology);
    assert!(dbg.contains("Topology"), "{dbg}");
    let copy = named.topology.clone();
    assert_eq!(copy, named.topology);
    for policy in [GeometryPolicy::Strict, GeometryPolicy::QuarantineInvalid] {
        let mut previous: Option<PackedMesh> = None;
        for threads in [1, 2, 4] {
            let p = named.topology.assemble(policy, threads).unwrap();
            assert_eq!(p.cell_ids, vec![0, 1]);
            assert_eq!(p.offsets, vec![0, 5, 10]);
            // Orientation +1 on the first face: start at its B point, run clockwise.
            assert_eq!(
                ring(&p, 0),
                &[[1., 0.], [0., 0.], [0., 1.], [1., 1.], [1., 0.]]
            );
            assert_eq!(p.diagnostics.len(), 1);
            assert_eq!(p.diagnostics[0].cell_id, 2);
            assert_eq!(p.diagnostics[0].kind, OmissionKind::InsufficientFaces);
            assert_eq!(p.diagnostics[0].reason, "1 faces");
            if let Some(prev) = &previous {
                assert_eq!(prev, &p, "threads {threads} {policy:?}");
            }
            previous = Some(p);
        }
    }
    assert_eq!(
        named
            .topology
            .assemble(GeometryPolicy::Strict, 0)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        named
            .topology
            .assemble(GeometryPolicy::Strict, 1025)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
}

#[test]
fn connectivity_width_and_coordinate_precision_variants() {
    let want = assemble("coverage_mesh.h5", "Good", GeometryPolicy::Strict, 1).unwrap();
    for name in ["F32Coords", "U16Conn"] {
        let got = assemble("coverage_mesh.h5", name, GeometryPolicy::Strict, 2).unwrap();
        assert_eq!(got, want, "{name}");
    }
    let s = session("coverage_mesh.h5", 1);
    // Integer coordinates and floating connectivity are refused instead of coerced.
    for (name, needle) in [
        ("IntCoords", "coordinates must be floats"),
        ("FloatConn", "connectivity must contain integers"),
        ("U64Huge", "exceeds int64"),
    ] {
        let e = s.read_topology(name).unwrap_err();
        assert_eq!(e.kind, ErrorKind::StructuralTopology, "{name}: {e}");
        assert!(e.message.contains(needle), "{name}: {e}");
    }
}

#[test]
fn mismatched_offsets_and_shapes_are_typed_structural_errors() {
    let s = session("coverage_mesh.h5", 1);
    let cases = [
        ("OverrunCellInfo", "cell 1"),
        ("NegativeOffset", "negative cell faces offset"),
        ("FaceIndexOutOfRange", "out of range"),
        ("PointIndexOutOfRange", "face endpoint 17 out of range"),
        ("PerimInfoMismatch", "face/perimeter row mismatch"),
        ("PerimOverrun", "face perimeter"),
        ("NaNCoord", "nonfinite"),
        ("OneColumnEnds", "at least 2 columns"),
        ("ThreeColumnCoords", "two columns"),
        ("OneDimCoords", "must be 2D"),
    ];
    for (name, needle) in cases {
        let e = s.read_topology(name).unwrap_err();
        assert_eq!(e.kind, ErrorKind::StructuralTopology, "{name}: {e}");
        assert!(
            e.message.contains(needle),
            "{name}: expected {needle:?} in {e}"
        );
    }
}

const TOPO: &str = "coverage_mesh_topology.h5";

#[test]
fn structural_chain_errors_are_fatal_under_both_policies() {
    let cases = [
        ("SelfLoop", "self-loop face"),
        ("DegreeThree", "not a degree-two closed chain"),
        ("OpenChain", "not a degree-two closed chain"),
        ("DuplicateFace", "duplicate face"),
        ("TwoTriangles", "disconnected"),
    ];
    for (name, needle) in cases {
        for policy in [GeometryPolicy::Strict, GeometryPolicy::QuarantineInvalid] {
            for threads in [1, 3] {
                let e = assemble(TOPO, name, policy, threads).unwrap_err();
                assert_eq!(
                    e.kind,
                    ErrorKind::StructuralTopology,
                    "{name} {policy:?}: {e}"
                );
                assert!(e.message.starts_with("cell 0:"), "{name}: {e}");
                assert!(
                    e.message.contains(needle),
                    "{name}: expected {needle:?} in {e}"
                );
            }
        }
    }
}

#[test]
fn proven_invalid_geometry_is_quarantined_only_on_request() {
    for name in ["CollinearCell", "CoincidentPointsAll"] {
        let e = assemble(TOPO, name, GeometryPolicy::Strict, 1).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidGeometry, "{name}: {e}");
        assert!(e.message.starts_with("cell 0:"), "{e}");
        let p = assemble(TOPO, name, GeometryPolicy::QuarantineInvalid, 1).unwrap();
        assert!(p.cell_ids.is_empty() && p.coordinates.is_empty());
        assert_eq!(p.offsets, vec![0]);
        assert_eq!(p.diagnostics.len(), 1);
        assert_eq!(p.diagnostics[0].kind, OmissionKind::InvalidGeometry);
        assert_eq!(p.diagnostics[0].cell_id, 0);
    }
    // Self-intersecting cell 0 is omitted; the valid square after it keeps its source ID 1,
    // and the trailing ghost is reported separately with its own kind.
    let strict = assemble(TOPO, "BowtieThenSquare", GeometryPolicy::Strict, 2).unwrap_err();
    assert_eq!(strict.kind, ErrorKind::InvalidGeometry);
    assert!(
        strict.message.contains("cell 0") && strict.message.contains("self-intersection"),
        "{strict}"
    );
    for threads in [1, 2, 4] {
        let p = assemble(
            TOPO,
            "BowtieThenSquare",
            GeometryPolicy::QuarantineInvalid,
            threads,
        )
        .unwrap();
        assert_eq!(p.cell_ids, vec![1]);
        assert_eq!(p.offsets, vec![0, 5]);
        let kinds: Vec<_> = p.diagnostics.iter().map(|d| (d.cell_id, d.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                (0, OmissionKind::InvalidGeometry),
                (2, OmissionKind::InsufficientFaces)
            ]
        );
        assert!(p.diagnostics[0].reason.contains("self-intersection"));
    }
}

#[test]
fn zero_face_and_ghost_only_meshes_report_every_omission() {
    let p = assemble(TOPO, "ZeroAndGhostOnly", GeometryPolicy::Strict, 1).unwrap();
    assert!(p.cell_ids.is_empty());
    let got: Vec<_> = p
        .diagnostics
        .iter()
        .map(|d| (d.cell_id, d.kind, d.reason.clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, OmissionKind::ZeroFaces, "0 faces".to_string()),
            (1, OmissionKind::InsufficientFaces, "1 faces".to_string()),
            (2, OmissionKind::ZeroFaces, "0 faces".to_string()),
            (3, OmissionKind::InsufficientFaces, "1 faces".to_string()),
        ]
    );
    let p = assemble(TOPO, "AllGhost", GeometryPolicy::QuarantineInvalid, 4).unwrap();
    assert_eq!(
        (p.cell_ids.len(), p.diagnostics.len(), p.offsets.clone()),
        (0, 3, vec![0])
    );
    assert!(p
        .diagnostics
        .iter()
        .all(|d| d.kind == OmissionKind::InsufficientFaces));
    let p = assemble("coverage_mesh.h5", "Empty", GeometryPolicy::Strict, 1).unwrap();
    assert_eq!(
        (p.offsets.clone(), p.cell_ids.len(), p.diagnostics.len()),
        (vec![0], 0, 0)
    );
}

#[test]
fn two_face_cells_are_insufficient_faces_not_structural_errors() {
    for policy in [GeometryPolicy::Strict, GeometryPolicy::QuarantineInvalid] {
        let p = assemble(TOPO, "TwoFaceCell", policy, 1).unwrap();
        assert!(p.cell_ids.is_empty() && p.coordinates.is_empty());
        assert_eq!(p.diagnostics.len(), 1);
        assert_eq!(p.diagnostics[0].kind, OmissionKind::InsufficientFaces);
        assert_eq!(p.diagnostics[0].reason, "2 faces");
    }
}

#[test]
fn consecutive_duplicate_coordinates_are_dropped_from_the_ring() {
    // Face points 1 and 2 coincide at (1,0). Ring-order contract: consecutive duplicate
    // coordinates are dropped (as GEOS polygonize does), then the ring is closed.
    let p = assemble(TOPO, "DuplicatePoints", GeometryPolicy::Strict, 1).unwrap();
    assert_eq!(p.cell_ids, vec![0]);
    assert_eq!(
        ring(&p, 0),
        &[[1., 0.], [0., 0.], [1., 1.], [1., 0.]],
        "clockwise from B of the first face, duplicates dropped"
    );
}

#[test]
fn shared_curved_face_is_reversed_for_the_opposite_cell() {
    let p = assemble(TOPO, "CurvedShared", GeometryPolicy::Strict, 2).unwrap();
    assert_eq!(p.cell_ids, vec![0, 1]);
    // The shared face is stored (1,0) to (1,1); cell 0 is on its left, cell 1 on its right.
    // Rings run clockwise, so the left cell walks it B to A and the right cell A to B.
    let a = ring(&p, 0);
    let ia = a.iter().position(|v| *v == [1.2, 0.7]).unwrap();
    assert_eq!(
        a[ia + 1],
        [1.2, 0.3],
        "the left cell walks the shared face B to A and flips interior vertices"
    );
    let b = ring(&p, 1);
    let ib = b.iter().position(|v| *v == [1.2, 0.3]).unwrap();
    assert_eq!(
        b[ib + 1],
        [1.2, 0.7],
        "the right cell walks the shared face A to B and keeps stored order"
    );
    for r in [a, b] {
        assert_eq!(r.first(), r.last());
    }
}

fn shoelace(r: &[[f64; 2]]) -> f64 {
    let mut s = 0.0;
    for w in r.windows(2) {
        s += (w[0][0] - 1.0e6) * (w[1][1] - 2.0e6) - (w[1][0] - 1.0e6) * (w[0][1] - 2.0e6);
    }
    s / 2.0
}

#[test]
fn grid_mesh_is_thread_invariant_with_original_ids_and_trailing_ghost() {
    let (nx, ny) = (24usize, 17usize);
    let base = assemble(TOPO, "Grid", GeometryPolicy::Strict, 1).unwrap();
    assert_eq!(base.cell_ids, (0..(nx * ny) as i64).collect::<Vec<_>>());
    assert_eq!(base.offsets.len(), nx * ny + 1);
    assert!(base.offsets.windows(2).all(|w| w[1] - w[0] == 5));
    assert_eq!(base.diagnostics.len(), 1);
    assert_eq!(base.diagnostics[0].cell_id, (nx * ny) as i64);
    assert_eq!(base.diagnostics[0].kind, OmissionKind::InsufficientFaces);
    for row in 0..nx * ny {
        // Cells are 2 x 3 map units; the large origin must not disturb exact corner coordinates.
        assert!(
            (shoelace(ring(&base, row)).abs() - 6.0).abs() < 1e-9,
            "row {row}"
        );
        let (i, j) = (row % nx, row / nx);
        let corner = [1.0e6 + 2.0 * i as f64, 2.0e6 + 3.0 * j as f64];
        assert!(ring(&base, row).contains(&corner), "row {row}");
    }
    for threads in [2, 4, 8] {
        for policy in [GeometryPolicy::Strict, GeometryPolicy::QuarantineInvalid] {
            assert_eq!(
                base,
                assemble(TOPO, "Grid", policy, threads).unwrap(),
                "threads {threads} {policy:?}"
            );
        }
    }
}

#[test]
fn separate_sessions_yield_equal_independent_topologies() {
    let a = session(TOPO, 1).read_topology("Grid").unwrap();
    let b = session(TOPO, 4).read_topology("Grid").unwrap();
    assert_eq!(a, b);
    assert_eq!(a.topology, b.topology);
    let other = session(TOPO, 1).read_topology("DuplicatePoints").unwrap();
    assert_ne!(a.topology, other.topology);
}

#[test]
fn public_ring_validation_categories() {
    let ok: &[&[[f64; 2]]] = &[
        &[[0., 0.], [1., 0.], [0., 1.], [0., 0.]],
        &[[0., 0.], [0., 1.], [1., 0.], [0., 0.]],
        &[[0., 0.], [0., 0.], [1., 0.], [1., 0.], [0., 1.], [0., 0.]],
        &[[0., 0.], [1., 0.], [2., 0.], [2., 2.], [0., 2.], [0., 0.]],
        &[
            [1e9, 1e9],
            [1e9 + 1., 1e9],
            [1e9 + 1., 1e9 + 1e-6],
            [1e9, 1e9 + 1e-6],
            [1e9, 1e9],
        ],
    ];
    for r in ok {
        validate_ring_geometry(r.iter().copied()).unwrap_or_else(|e| panic!("{r:?}: {e}"));
    }
    let bad: &[(&[[f64; 2]], &str)] = &[
        (&[], "empty"),
        (&[[0., 0.]], "zero-area"),
        (&[[0., 0.], [1., 0.]], "unclosed"),
        (&[[0., 0.], [1., 0.], [0., 1.]], "unclosed"),
        (&[[0., 0.], [1., 0.], [0., 0.]], "zero-area"),
        (&[[0., 0.], [1., 0.], [2., 0.], [0., 0.]], "zero-area"),
        (&[[0., 0.], [f64::NAN, 0.], [0., 1.], [0., 0.]], "nonfinite"),
        (
            &[[0., 0.], [f64::INFINITY, 0.], [0., 1.], [0., 0.]],
            "nonfinite",
        ),
        (
            &[[0., 0.], [1., 1.], [0., 1.], [1., 0.], [0., 0.]],
            "self-intersection",
        ),
        (
            &[[0., 0.], [2., 0.], [1., 0.], [1., 2.], [0., 2.], [0., 0.]],
            "backtracks",
        ),
    ];
    for (r, needle) in bad {
        let e = validate_ring_geometry(r.iter().copied()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidGeometry, "{r:?}");
        assert!(
            e.message.contains(needle),
            "{r:?}: expected {needle:?} in {e}"
        );
    }
}
