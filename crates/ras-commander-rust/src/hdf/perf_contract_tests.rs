use super::*;
#[test]
#[ignore = "RAS_PERF2_PROBE supplies immutable local fixtures; counters only, no timing"]
fn sparse_cache_counter_probe() {
    let manifest = std::env::var("RAS_PERF2_PROBE").expect("RAS_PERF2_PROBE required");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    for case in cases.as_array().unwrap() {
        let s = ReadOnlyHdfSession::with_options(case["path"].as_str().unwrap(), 8, true).unwrap();
        let area = case["area"].as_str().unwrap();
        let n = case["columns"].as_u64().unwrap();
        let rows = case["rows"].as_u64().unwrap().min(24);
        let ids = (0..500).map(|i| i * (n - 1) / 499).collect::<Vec<_>>();
        let mut expected = None;
        for run in 0..3 {
            let a = s.file.chunk_cache_stats();
            let r = s
                .read_mesh_series(
                    area,
                    "Water Surface",
                    Slice::new(0, rows, 1),
                    Some(&ids),
                    DEFAULT_MAX_BYTES,
                    DEFAULT_MAX_BYTES,
                )
                .unwrap();
            let NumericValues::F32(v) = r.values.values else {
                panic!("expected f32")
            };
            let bits = v.into_iter().map(f32::to_bits).collect::<Vec<_>>();
            if let Some(ref e) = expected {
                assert_eq!(&bits, e);
            } else {
                expected = Some(bits);
            }
            let z = s.file.chunk_cache_stats();
            println!("CACHE {} run={} hits={} misses={} inserts={} evictions={} retained={} entries={} bounding_bytes={} output_bytes={}",area,run,z.hits-a.hits,z.misses-a.misses,z.inserts-a.inserts,z.evictions-a.evictions,z.current_bytes,z.entries,rows*n*4,rows*500*4);
        }
    }
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/perf_contract")
        .join(name)
}
fn bits(a: NumericArray) -> Vec<u64> {
    let NumericValues::F64(v) = a.values else {
        panic!("expected f64")
    };
    v.into_iter().map(f64::to_bits).collect()
}
#[test]
fn corrupt_child_listing_retains_original_verdict() {
    let s = ReadOnlyHdfSession::open(fixture("dense_bad.h5")).unwrap();
    let expected = s
        .file
        .group("G")
        .unwrap()
        .members()
        .err()
        .map(map_error)
        .unwrap();
    assert_eq!(expected.kind, ErrorKind::Corrupt);
    assert!(s.file.group("G").unwrap().member_names().is_ok());
    for _ in 0..2 {
        assert_eq!(s.group_members("G").unwrap_err(), expected);
        assert!(s.read_numeric("G/ok", None).is_ok());
    }
}
#[test]
fn numeric_errors_keep_complete_original_error() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture("manifest.json")).unwrap()).unwrap();
    let addr = m["header_address"].as_u64().unwrap() as usize;
    let mut data = std::fs::read(fixture("header_base.h5")).unwrap();
    assert_eq!(data[addr], 1);
    data[addr + 8..addr + 12].copy_from_slice(&((1u32 << 24) + 8).to_le_bytes());
    let path = std::env::temp_dir().join(format!("perf-header-{}.h5", std::process::id()));
    std::fs::write(&path, data).unwrap();
    let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    f.set_len((addr + 16 + (1 << 24) + 8) as u64).unwrap();
    drop(f);
    {
        let s = ReadOnlyHdfSession::open(&path).unwrap();
        let ds = s.file.dataset("data").unwrap();
        let expected = encoding::encoding(&s.file, &ds).unwrap_err();
        assert_eq!(expected.code, Some(UnsupportedCode::ResourceLimit));
        for _ in 0..2 {
            assert_eq!(s.read_numeric("data", None).unwrap_err(), expected);
        }
        let missing = s.file.dataset("absent").err().map(map_error).unwrap();
        assert_eq!(s.read_numeric("absent", None).unwrap_err(), missing);
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn large_parallel_reads_are_bit_exact_concurrent_and_nested() {
    let path = fixture("parallel.h5");
    let serial = ReadOnlyHdfSession::with_options(&path, 1, false).unwrap();
    let selection = [Slice::new(3, 400, 2), Slice::new(1, 500, 3)];
    let full = bits(serial.read_numeric("data", None).unwrap());
    let slab = bits(serial.read_numeric("data", Some(&selection)).unwrap());
    assert!(slab.len() * 8 >= 64 * 1024);
    let s = ReadOnlyHdfSession::with_options(&path, 4, true).unwrap();
    let check = |s: &ReadOnlyHdfSession| {
        assert_eq!(bits(s.read_numeric("data", None).unwrap()), full);
        assert_eq!(
            bits(s.read_numeric("data", Some(&selection)).unwrap()),
            slab
        );
    };
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| check(&s));
        }
    });
    let foreign = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    foreign.install(|| {
        use rayon::prelude::*;
        (0..8).into_par_iter().for_each(|_| check(&s));
    });
    s.pool.install(|| {
        use rayon::prelude::*;
        (0..8).into_par_iter().for_each(|_| {
            let nested = ReadOnlyHdfSession::with_options(&path, 4, true).unwrap();
            check(&nested);
        });
    });
}
#[test]
fn real_budget_keys_match_fresh_sessions_after_shrinking() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/xs_time/xs_time_ok.h5");
    let reused = ReadOnlyHdfSession::open(&path).unwrap();
    for budget in [64 << 20, 64, 1, 64 << 20] {
        let fresh = ReadOnlyHdfSession::open(&path).unwrap();
        assert_eq!(
            format!("{:?}", reused.cross_sections(budget)),
            format!("{:?}", fresh.cross_sections(budget))
        );
        assert_eq!(
            format!("{:?}", reused.plan_capabilities(budget)),
            format!("{:?}", fresh.plan_capabilities(budget))
        );
    }
}

#[test]
fn gathered_chunks_match_serial_with_strides_duplicates_and_edges() {
    let s = ReadOnlyHdfSession::open(fixture("parallel.h5")).unwrap();
    let full = bits(s.read_numeric("data", None).unwrap());
    let columns = [499, 0, 62, 62, 121, 3];
    for rows in [
        Slice::new(0, 400, 1),
        Slice::new(31, 398, 7),
        Slice::new(399, 400, 9),
        Slice::new(17, 17, 1),
    ] {
        let got = bits(
            s.gather_numeric("data", NumericKind::F64, rows, &columns)
                .unwrap()
                .unwrap(),
        );
        let expected = (rows.start..rows.end)
            .step_by(rows.step as usize)
            .flat_map(|row| columns.iter().map(move |&col| (row * 500 + col) as usize))
            .map(|i| full[i])
            .collect::<Vec<_>>();
        assert_eq!(got, expected);
    }
    assert!(bits(
        s.gather_numeric("data", NumericKind::F64, Slice::new(0, 400, 1), &[])
            .unwrap()
            .unwrap()
    )
    .is_empty());
    assert!(s
        .gather_numeric("data", NumericKind::F64, Slice::new(0, 401, 1), &columns)
        .is_err());
    assert!(s
        .gather_numeric("data", NumericKind::F64, Slice::new(0, 400, 0), &columns)
        .is_err());
    assert!(s
        .gather_numeric("data", NumericKind::F64, Slice::new(0, 400, 1), &[500])
        .is_err());
}
#[test]
fn bulk_bypass_preserves_small_cache_contents_and_exact_bits() {
    let path = fixture("parallel.h5");
    let expected = bits(
        ReadOnlyHdfSession::open(&path)
            .unwrap()
            .read_numeric("data", None)
            .unwrap(),
    );
    for parallel in [false, true] {
        let s = ReadOnlyHdfSession::with_cache_options(&path, 4, parallel, 32768, 7).unwrap();
        s.read_numeric("data", Some(&[Slice::new(0, 1, 1), Slice::new(0, 1, 1)]))
            .unwrap();
        let before = s.chunk_cache_stats();
        for _ in 0..2 {
            assert_eq!(bits(s.read_numeric("data", None).unwrap()), expected);
        }
        assert_eq!(s.chunk_cache_stats(), before);
        assert!(before.retained_bytes <= 32768 && before.entries <= 7);
    }
    assert!(ReadOnlyHdfSession::with_cache_options(path, 1, false, 0, 0).is_err());
}

#[test]
fn requested_bc_ignores_corrupt_sibling_but_inventory_and_all_batch_refuse() {
    let s = ReadOnlyHdfSession::open(fixture("bc_sibling.h5")).unwrap();
    assert!(s
        .read_mesh_boundary_series(
            "Area",
            "good",
            Slice::new(0, 2, 1),
            None,
            DEFAULT_MAX_BYTES,
            DEFAULT_MAX_BYTES
        )
        .is_ok());
    assert_eq!(
        s.mesh_boundary_inventory("Area", DEFAULT_MAX_BYTES)
            .unwrap_err()
            .kind,
        ErrorKind::Corrupt
    );
    assert_eq!(
        s.read_mesh_boundary_batch(
            "Area",
            None,
            Slice::new(0, 2, 1),
            DEFAULT_MAX_BYTES,
            DEFAULT_MAX_BYTES
        )
        .unwrap_err()
        .kind,
        ErrorKind::Corrupt
    );
}

#[test]
#[ignore = "RAS_PERF3_FIXTURES real files; rough attribution only"]
fn perf3_small_read_attribution() {
    use std::time::Instant;
    let root = std::env::var("RAS_PERF3_FIXTURES").expect("fixture directory required");
    let s =
        ReadOnlyHdfSession::with_options(Path::new(&root).join("AustinOyster.p01.hdf"), 8, true)
            .unwrap();
    let area = String::from("Perimeter 1");
    let path=format!("Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas/{area}/Maximum Water Surface");
    s.read_numeric(&path, None).unwrap();
    let ds = s.file.dataset(&path).unwrap();
    for stage in [
        "reader_dataset_lookup",
        "reader_array",
        "engine_numeric",
        "typed_summary",
    ] {
        let before = s.chunk_cache_stats();
        let t = Instant::now();
        for _ in 0..2000 {
            match stage {
                "reader_dataset_lookup" => {
                    std::hint::black_box(s.file.dataset(&path).unwrap());
                }
                "reader_array" => {
                    std::hint::black_box(ds.read_array::<f32>().unwrap());
                }
                "engine_numeric" => {
                    std::hint::black_box(s.read_numeric(&path, None).unwrap());
                }
                _ => {
                    std::hint::black_box(
                        s.read_mesh_summary_values(
                            &area,
                            "Maximum Water Surface",
                            None,
                            DEFAULT_MAX_BYTES,
                            DEFAULT_MAX_BYTES,
                        )
                        .unwrap(),
                    );
                }
            }
        }
        println!(
            "ATTRIBUTION {}",
            serde_json::json!({"stage":stage,"calls":2000,"rough_elapsed_ns":t.elapsed().as_nanos().to_string(),"cache_before":format!("{before:?}"),"cache_after":format!("{:?}",s.chunk_cache_stats())})
        );
    }
}

#[test]
fn full_slice_reuses_hot_array_without_changing_bits_or_bypass() {
    let s = ReadOnlyHdfSession::with_options(fixture("parallel.h5"), 4, true).unwrap();
    let full = [Slice::new(0, 400, 1), Slice::new(0, 500, 1)];
    // Independent reference: the previous vendor slice path, on a separate file/cache.
    let reference = ReadOnlyHdfSession::open(fixture("parallel.h5")).unwrap();
    let expected = reference
        .file
        .dataset("data")
        .unwrap()
        .read_slice::<f64>(&SliceInfo {
            selections: vec![
                SliceInfoElem::Slice {
                    start: 0,
                    end: 400,
                    step: 1,
                },
                SliceInfoElem::Slice {
                    start: 0,
                    end: 500,
                    step: 1,
                },
            ],
        })
        .unwrap()
        .iter()
        .map(|v| v.to_bits())
        .collect::<Vec<_>>();
    assert_eq!(bits(s.read_numeric("data", Some(&full)).unwrap()), expected);
    let before = s.chunk_cache_stats();
    assert_eq!(bits(s.read_numeric("data", Some(&full)).unwrap()), expected);
    assert_eq!(bits(s.read_numeric("data", None).unwrap()), expected);
    assert_eq!(s.chunk_cache_stats(), before);
    let tiny =
        ReadOnlyHdfSession::with_cache_options(fixture("parallel.h5"), 4, true, 32768, 7).unwrap();
    let before = tiny.chunk_cache_stats();
    assert_eq!(
        bits(tiny.read_numeric("data", Some(&full)).unwrap()),
        expected
    );
    assert_eq!(tiny.chunk_cache_stats(), before);
}
