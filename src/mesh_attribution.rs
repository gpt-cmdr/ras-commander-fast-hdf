// Measurement and reference code only; compiled exclusively under cfg(test).
mod attribution {
    use super::*;
    use std::{hint::black_box, time::Instant};
    type Rings = Vec<Result<Option<Vec<Point>>, CellError>>;

    // Exact packing reference from accepted engine 1c3a9ce mesh.rs.
    fn pack_reference(
        top: &Topology,
        rings: Rings,
        policy: GeometryPolicy,
    ) -> crate::Result<PackedMesh> {
        let mut out = PackedMesh {
            coordinates: Vec::new(),
            offsets: vec![0],
            cell_ids: Vec::new(),
            diagnostics: Vec::new(),
        };
        for (cell, ring) in rings.into_iter().enumerate() {
            let id = i64::try_from(cell)
                .map_err(|_| Error::new(ErrorKind::StructuralTopology, "cell ID overflow"))?;
            match ring {
                Ok(Some(ring)) => {
                    out.coordinates.extend(ring);
                    out.offsets.push(out.coordinates.len() as u64);
                    out.cell_ids.push(id);
                }
                Ok(None) => {
                    let count = top.cell_info[[cell, 1]];
                    out.diagnostics.push(CellDiagnostic {
                        cell_id: id,
                        kind: if count == 0 {
                            OmissionKind::ZeroFaces
                        } else {
                            OmissionKind::InsufficientFaces
                        },
                        reason: format!("{count} faces"),
                    });
                }
                Err(CellError::Geometry(reason)) if policy == GeometryPolicy::QuarantineInvalid => {
                    out.diagnostics.push(CellDiagnostic {
                        cell_id: id,
                        kind: OmissionKind::InvalidGeometry,
                        reason,
                    })
                }
                Err(CellError::Geometry(reason)) => {
                    return Err(Error::new(
                        ErrorKind::InvalidGeometry,
                        format!("cell {id}: {reason}"),
                    ))
                }
                Err(CellError::Structural(reason)) => {
                    return Err(Error::new(
                        ErrorKind::StructuralTopology,
                        format!("cell {id}: {reason}"),
                    ))
                }
            }
        }
        Ok(out)
    }
    fn reference(
        top: &Topology,
        policy: GeometryPolicy,
        threads: usize,
    ) -> crate::Result<PackedMesh> {
        top.validate()
            .map_err(|e| Error::new(ErrorKind::StructuralTopology, e))?;
        let pool = crate::pool(threads)?;
        let rings = pool.install(|| {
            (0..top.cell_info.nrows())
                .into_par_iter()
                .map(|c| top.ring(c))
                .collect()
        });
        pack_reference(top, rings, policy)
    }
    fn synthetic(cells: usize, vertices: usize) -> Topology {
        let polygon = if vertices == 4 {
            vec![[0., 0.], [2., 0.], [2., 2.], [0., 2.]]
        } else {
            vec![
                [0., 0.],
                [1., 0.],
                [2., 0.],
                [2., 1.],
                [2., 2.],
                [1., 2.],
                [0., 2.],
                [0., 1.],
            ]
        };
        let mut points = Vec::new();
        let mut ends = Vec::new();
        let mut info = Vec::new();
        let mut faces = Vec::new();
        for cell in 0..cells {
            for (v, p) in polygon.iter().enumerate() {
                points.extend([
                    p[0] + (cell % 512) as f64 * 4.,
                    p[1] + (cell / 512) as f64 * 4.,
                ]);
                ends.extend([
                    (cell * vertices + v) as i64,
                    (cell * vertices + (v + 1) % vertices) as i64,
                ]);
                faces.extend([(cell * vertices + v) as i64, 0]);
            }
            info.extend([(cell * vertices) as i64, vertices as i64]);
        }
        Topology {
            points: Array2::from_shape_vec((cells * vertices, 2), points).unwrap(),
            ends: Array2::from_shape_vec((cells * vertices, 2), ends).unwrap(),
            perim_info: Array2::zeros((cells * vertices, 2)),
            perim: Array2::zeros((0, 2)),
            cell_info: Array2::from_shape_vec((cells, 2), info).unwrap(),
            cell_faces: Array2::from_shape_vec((cells * vertices, 2), faces).unwrap(),
        }
    }
    fn bits(out: &PackedMesh) -> Vec<u8> {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        for p in &out.coordinates {
            for v in p {
                h.update(v.to_bits().to_le_bytes());
            }
        }
        for n in &out.offsets {
            h.update(n.to_le_bytes());
        }
        for n in &out.cell_ids {
            h.update(n.to_le_bytes());
        }
        for d in &out.diagnostics {
            h.update(d.cell_id.to_le_bytes());
            h.update(format!("{:?}", d.kind));
            h.update(&d.reason);
        }
        h.finalize().to_vec()
    }
    #[test]
    fn outputs_match_accepted_reference() {
        for vertices in [4, 8] {
            let top = synthetic(64, vertices);
            for threads in [1, 4] {
                for policy in [GeometryPolicy::Strict, GeometryPolicy::QuarantineInvalid] {
                    assert_eq!(
                        bits(&top.assemble(policy, threads).unwrap()),
                        bits(&reference(&top, policy, threads).unwrap())
                    );
                }
            }
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/coverage/coverage_mesh.h5");
        let session = crate::hdf::ReadOnlyHdfSession::open(&path).unwrap();
        for name in ["Good", "Bowtie", "Empty"] {
            let top = session.read_topology(name).unwrap().topology;
            for threads in [1, 4] {
                for policy in [GeometryPolicy::Strict, GeometryPolicy::QuarantineInvalid] {
                    match (
                        top.assemble(policy, threads),
                        reference(&top, policy, threads),
                    ) {
                        (Ok(a), Ok(b)) => assert_eq!(bits(&a), bits(&b)),
                        (Err(a), Err(b)) => {
                            assert_eq!(a.kind, b.kind);
                            assert_eq!(a.message, b.message);
                        }
                        _ => panic!("reference outcome differs"),
                    }
                }
            }
        }
    }
    fn stages(top: &Topology, threads: usize) -> serde_json::Value {
        let now = Instant::now();
        top.validate().unwrap();
        let validation = now.elapsed().as_secs_f64();
        let now = Instant::now();
        let pool = crate::pool(threads).unwrap();
        let pool_create = now.elapsed().as_secs_f64();
        let now = Instant::now();
        let rings = pool.install(|| {
            (0..top.cell_info.nrows())
                .into_par_iter()
                .map(|c| top.ring(c))
                .collect()
        });
        let ring = now.elapsed().as_secs_f64();
        let now = Instant::now();
        let out = pack_reference(top, rings, GeometryPolicy::Strict).unwrap();
        let packing = now.elapsed().as_secs_f64();
        black_box(&out);
        let now = Instant::now();
        drop(pool);
        let pool_drop = now.elapsed().as_secs_f64();
        serde_json::json!({"validation":validation,"pool_create":pool_create,"rings":ring,"packing_reference":packing,"pool_drop":pool_drop})
    }
    #[test]
    #[ignore = "release-only synthetic attribution; no timing assertions"]
    fn synthetic_stage_benchmark() {
        if cfg!(debug_assertions) {
            panic!("run --release");
        }
        let cells =
            std::env::var("RAS_ASSEMBLE_BENCH_CELLS").unwrap_or_else(|_| "1000,100000".into());
        let repeats: usize = std::env::var("RAS_ASSEMBLE_BENCH_REPEATS")
            .unwrap_or_else(|_| "7".into())
            .parse()
            .unwrap();
        for count in cells.split(',').map(|v| v.parse::<usize>().unwrap()) {
            for vertices in [4, 8] {
                for threads in [1, 4] {
                    let top = synthetic(count, vertices);
                    let expected = bits(&reference(&top, GeometryPolicy::Strict, threads).unwrap());
                    for repeat in 0..repeats {
                        let total = || {
                            let now = Instant::now();
                            let out = top.assemble(GeometryPolicy::Strict, threads).unwrap();
                            let seconds = now.elapsed().as_secs_f64();
                            assert_eq!(bits(&out), expected);
                            black_box(out);
                            seconds
                        };
                        let (public, parts) = if repeat % 2 == 0 {
                            (total(), stages(&top, threads))
                        } else {
                            let parts = stages(&top, threads);
                            (total(), parts)
                        };
                        println!(
                            "ATTRIBUTION {}",
                            serde_json::json!({"host":std::env::var("COMPUTERNAME").unwrap_or_default(),"label":std::env::var("RAS_ASSEMBLE_BENCH_LABEL").unwrap_or_default(),"cells":count,"vertices":vertices,"threads":threads,"repeat":repeat,"public_total":public,"stage_seconds":parts})
                        );
                    }
                }
            }
        }
    }
}
