//! Behavioural coverage for ReadOnlyHdfSession numeric reads.
//!
//! Fixtures under tests/fixtures/coverage are frozen synthetic HDF5 files produced by
//! make_coverage_fixtures.py (recorded in coverage_manifest.json). Expected values are analytic
//! (v = i * 10 + j, floats v / 7), so these tests never invoke Python.
#![cfg(feature = "hdf")]
use ras_commander_rust::hdf::{
    ByteOrder, NumericArray, NumericKind, NumericValues, ReadOnlyHdfSession, Slice,
};
use ras_commander_rust::ErrorKind;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const FILES: [&str; 2] = ["coverage_numeric_v1.h5", "coverage_numeric_v2.h5"];

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/coverage")
        .join(name)
}
fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/coverage/coverage_manifest.json")).unwrap()
}
fn sl(start: u64, end: u64, step: u64) -> Slice {
    Slice::new(start, end, step)
}
fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ras-coverage-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Analytic oracle: value at grid position (i, j) for the dtype named by `kind`.
fn cell(kind: &str, i: usize, j: usize) -> f64 {
    let v = (i * 10 + j) as f64;
    match kind {
        "f4" => f64::from((v / 7.0) as f32),
        "f8" => v / 7.0,
        _ => v,
    }
}
fn expected(kind: &str, rows: &[usize], cols: &[usize]) -> NumericValues {
    let mut it = Vec::new();
    for &i in rows {
        for &j in cols {
            it.push(cell(kind, i, j));
        }
    }
    match kind {
        "i1" => NumericValues::I8(it.iter().map(|&v| v as i8).collect()),
        "u1" => NumericValues::U8(it.iter().map(|&v| v as u8).collect()),
        "i2" => NumericValues::I16(it.iter().map(|&v| v as i16).collect()),
        "u2" => NumericValues::U16(it.iter().map(|&v| v as u16).collect()),
        "i4" => NumericValues::I32(it.iter().map(|&v| v as i32).collect()),
        "u4" => NumericValues::U32(it.iter().map(|&v| v as u32).collect()),
        "i8" => NumericValues::I64(it.iter().map(|&v| v as i64).collect()),
        "u8" => NumericValues::U64(it.iter().map(|&v| v as u64).collect()),
        "u8_offset" => NumericValues::U64(it.iter().map(|&v| v as u64 + (1u64 << 60)).collect()),
        "f4" => NumericValues::F32(it.iter().map(|&v| v as f32).collect()),
        "f8" => NumericValues::F64(it),
        other => panic!("kind {other}"),
    }
}
fn all(n: usize) -> Vec<usize> {
    (0..n).collect()
}
fn strided(start: usize, end: usize, step: usize) -> Vec<usize> {
    (start..end).step_by(step).collect()
}
fn len_of(v: &NumericValues) -> usize {
    match v {
        NumericValues::I8(v) => v.len(),
        NumericValues::I16(v) => v.len(),
        NumericValues::I32(v) => v.len(),
        NumericValues::I64(v) => v.len(),
        NumericValues::U8(v) => v.len(),
        NumericValues::U16(v) => v.len(),
        NumericValues::U32(v) => v.len(),
        NumericValues::U64(v) => v.len(),
        NumericValues::F32(v) => v.len(),
        NumericValues::F64(v) => v.len(),
        _ => panic!("new numeric variant requires qualification"),
    }
}
fn bytes_of(a: &NumericArray) -> Vec<u8> {
    macro_rules! b {
        ($v:expr) => {
            $v.iter().flat_map(|x| x.to_le_bytes()).collect()
        };
    }
    match &a.values {
        NumericValues::I8(v) => b!(v),
        NumericValues::I16(v) => b!(v),
        NumericValues::I32(v) => b!(v),
        NumericValues::I64(v) => b!(v),
        NumericValues::U8(v) => v.clone(),
        NumericValues::U16(v) => b!(v),
        NumericValues::U32(v) => b!(v),
        NumericValues::U64(v) => b!(v),
        NumericValues::F32(v) => b!(v),
        NumericValues::F64(v) => b!(v),
        _ => panic!("new numeric variant requires qualification"),
    }
}

#[test]
fn fixture_hashes_match_manifest() {
    let m = manifest();
    let files = m["files"].as_array().unwrap();
    assert!(files.len() >= 5);
    for f in files {
        let bytes = std::fs::read(fixture(f["file"].as_str().unwrap())).unwrap();
        assert!(
            bytes.len() < 200_000,
            "{} exceeds fixture budget",
            f["file"]
        );
        assert_eq!(bytes.len() as u64, f["bytes"].as_u64().unwrap());
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            f["sha256"].as_str().unwrap()
        );
    }
}

#[test]
fn filtered_and_chunked_layouts_match_analytic_oracle() {
    let cases: [(&str, &str); 6] = [
        ("filt/gzip_i4", "i4"),
        ("filt/shuffle_gzip_f8", "f8"),
        ("filt/fletcher32_f4", "f4"),
        ("filt/shuffle_fletcher_gzip_i2", "i2"),
        ("filt/shuffle_u8", "u8_offset"),
        ("filt/contig_f4", "f4"),
    ];
    for file in FILES {
        let s = ReadOnlyHdfSession::open(fixture(file)).unwrap();
        for (path, kind) in cases {
            let a = s.read_numeric(path, None).unwrap();
            assert_eq!(a.shape, vec![12, 10], "{file} {path}");
            assert_eq!(
                a.values,
                expected(kind, &all(12), &all(10)),
                "{file} {path}"
            );
            // Selections that straddle chunk edges (chunk shapes 5x4, 4x10, 3x5, 6x5, 12x2).
            let b = s
                .read_numeric(path, Some(&[sl(2, 11, 3), sl(1, 10, 4)]))
                .unwrap();
            assert_eq!(b.shape, vec![3, 3]);
            assert_eq!(
                b.values,
                expected(kind, &strided(2, 11, 3), &strided(1, 10, 4)),
                "{file} {path} strided"
            );
        }
    }
}

#[test]
fn capability_reports_chunks_and_byte_order() {
    let s = ReadOnlyHdfSession::open(fixture(FILES[0])).unwrap();
    let c = s.numeric_capability("filt/gzip_i4").unwrap();
    assert_eq!(c.shape, vec![12, 10]);
    assert_eq!(c.chunks, Some(vec![5, 4]));
    assert_eq!(c.kind, Some(NumericKind::I32));
    assert_eq!(c.byte_order, Some(ByteOrder::Little));
    assert_eq!(c.signed, Some(true));
    assert!(c.capability.is_supported());
    let contiguous = s.numeric_capability("filt/contig_f4").unwrap();
    assert_eq!(contiguous.chunks, None);
    assert_eq!(contiguous.kind, Some(NumericKind::F32));
    let le = s.numeric_capability("endian/le_u8").unwrap();
    assert_eq!(le.byte_order, Some(ByteOrder::Little));
    assert_eq!(le.signed, Some(false));
    assert_eq!(le.kind, Some(NumericKind::U64));
    let empty = s.numeric_capability("shape/empty_chunked_f4").unwrap();
    assert_eq!(empty.shape, vec![0]);
    assert!(empty.max_shape.is_some());
}

#[test]
fn fill_value_only_and_partially_allocated_datasets_return_fill() {
    for file in FILES {
        let s = ReadOnlyHdfSession::open(fixture(file)).unwrap();
        let a = s.read_numeric("fill/chunked_unalloc_f4", None).unwrap();
        assert_eq!(a.shape, vec![12, 10]);
        assert_eq!(a.values, NumericValues::F32(vec![-9.5; 120]), "{file}");
        let a = s
            .read_numeric(
                "fill/chunked_unalloc_gzip_i4",
                Some(&[sl(3, 9, 2), sl(0, 10, 3)]),
            )
            .unwrap();
        assert_eq!(a.values, NumericValues::I32(vec![-77; 3 * 4]));
        let a = s.read_numeric("fill/contig_unalloc_i4", None).unwrap();
        assert_eq!(a.values, NumericValues::I32(vec![42; 120]), "{file}");
        // Only chunk (0,0) is allocated; everything else must equal the fill value 7.
        let a = s.read_numeric("fill/partial_chunked_i4", None).unwrap();
        let NumericValues::I32(v) = &a.values else {
            panic!("kind")
        };
        for i in 0..12 {
            for j in 0..10 {
                let want = if i < 5 && j < 4 {
                    (i * 10 + j) as i32
                } else {
                    7
                };
                assert_eq!(v[i * 10 + j], want, "{file} ({i},{j})");
            }
        }
        // A selection straddling allocated and unallocated chunks.
        let a = s
            .read_numeric("fill/partial_chunked_i4", Some(&[sl(3, 8, 1), sl(2, 7, 2)]))
            .unwrap();
        let want: Vec<i32> = (3..8)
            .flat_map(|i| {
                (2..7)
                    .step_by(2)
                    .map(move |j| if i < 5 && j < 4 { i * 10 + j } else { 7 })
            })
            .collect();
        assert_eq!(a.values, NumericValues::I32(want));
    }
}

#[test]
fn little_endian_mixed_widths_read_exactly() {
    for file in FILES {
        let s = ReadOnlyHdfSession::open(fixture(file)).unwrap();
        for kind in ["i1", "u1", "i2", "u2", "i4", "u4", "i8", "u8", "f4", "f8"] {
            let path = format!("endian/le_{kind}");
            let le = s.read_numeric(&path, None).unwrap();
            assert_eq!(
                le.values,
                expected(kind, &all(12), &all(10)),
                "{file} {path}"
            );
            let sub = s
                .read_numeric(&path, Some(&[sl(4, 9, 2), sl(3, 10, 3)]))
                .unwrap();
            assert_eq!(
                sub.values,
                expected(kind, &strided(4, 9, 2), &strided(3, 10, 3))
            );
        }
    }
}

/// The reader supports only native byte order. Big-endian storage must be an explicit,
/// typed Unsupported result (with a capability record naming the order), never wrong values.
#[cfg(target_endian = "little")]
#[test]
fn big_endian_storage_is_explicit_unsupported_not_misread() {
    for file in FILES {
        let s = ReadOnlyHdfSession::open(fixture(file)).unwrap();
        let mut names: Vec<String> = ["i2", "u2", "i4", "u4", "i8", "u8", "f4", "f8"]
            .iter()
            .map(|k| format!("endian/be_{k}"))
            .collect();
        names.extend(
            [
                "be_f8_gzip_chunked",
                "be_i4_fletcher",
                "be_extreme_u8",
                "be_extreme_i8",
            ]
            .iter()
            .map(|n| format!("endian/{n}")),
        );
        names.push("series/f4_be".to_string());
        for path in names {
            let e = s.read_numeric(&path, None).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Unsupported, "{file} {path}: {e}");
            let cap = s.numeric_capability(&path).unwrap();
            let sel = vec![sl(0, 1, 1); cap.shape.len()];
            let e = s.read_numeric(&path, Some(&sel)).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Unsupported, "{file} {path} sliced");
            assert_eq!(cap.byte_order, Some(ByteOrder::Big), "{path}");
            assert!(
                cap.kind.is_none() && !cap.capability.is_supported(),
                "{path}"
            );
        }
    }
}

#[test]
fn zero_length_and_single_element_dimensions() {
    for file in FILES {
        let s = ReadOnlyHdfSession::open(fixture(file)).unwrap();
        for (path, shape) in [
            ("shape/empty_1d_i4", vec![0u64]),
            ("shape/empty_2d_f4", vec![0, 5]),
            ("shape/empty_cols_f4", vec![5, 0]),
            ("shape/empty_chunked_f4", vec![0]),
        ] {
            let a = s.read_numeric(path, None).unwrap();
            assert_eq!(a.shape, shape, "{file} {path}");
            assert_eq!(len_of(&a.values), 0, "{file} {path}");
            assert_eq!(s.numeric_capability(path).unwrap().shape, shape);
        }
        let a = s.read_numeric("shape/one_1d_i4", None).unwrap();
        assert_eq!(
            (a.shape.clone(), a.values),
            (vec![1], NumericValues::I32(vec![123456]))
        );
        let a = s.read_numeric("shape/one_2d_f8", None).unwrap();
        assert_eq!(
            (a.shape.clone(), a.values),
            (vec![1, 1], NumericValues::F64(vec![2.5]))
        );
        for step in [1, 2, u64::MAX] {
            let a = s
                .read_numeric("shape/one_1d_i4", Some(&[sl(0, 1, step)]))
                .unwrap();
            assert_eq!(a.values, NumericValues::I32(vec![123456]));
        }
        // Empty selection on a non-empty dataset, and a selection on an empty dataset.
        let a = s
            .read_numeric("filt/gzip_i4", Some(&[sl(4, 4, 1), sl(0, 10, 1)]))
            .unwrap();
        assert_eq!(a.shape, vec![0, 10]);
        assert_eq!(len_of(&a.values), 0);
        let a = s
            .read_numeric("shape/empty_2d_f4", Some(&[sl(0, 0, 1), sl(0, 5, 2)]))
            .unwrap();
        assert_eq!(a.shape, vec![0, 3]);
    }
}

#[test]
fn invalid_slices_are_typed_invalid_requests() {
    let s = ReadOnlyHdfSession::open(fixture(FILES[0])).unwrap();
    let bad: Vec<(&str, Vec<Slice>)> = vec![
        ("zero step", vec![sl(0, 5, 0), sl(0, 10, 1)]),
        ("reversed", vec![sl(6, 5, 1), sl(0, 10, 1)]),
        ("end past extent", vec![sl(0, 13, 1), sl(0, 10, 1)]),
        ("start past extent", vec![sl(13, 13, 1), sl(0, 10, 1)]),
        ("far past extent", vec![sl(0, u64::MAX, 1), sl(0, 10, 1)]),
        ("second dim out of range", vec![sl(0, 12, 1), sl(0, 11, 1)]),
        ("too few slices", vec![sl(0, 12, 1)]),
        (
            "too many slices",
            vec![sl(0, 12, 1), sl(0, 10, 1), sl(0, 1, 1)],
        ),
        ("no slices", vec![]),
        ("max start", vec![sl(u64::MAX, u64::MAX, 1), sl(0, 10, 1)]),
    ];
    for (label, sel) in bad {
        let e = s.read_numeric("filt/gzip_i4", Some(&sel)).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest, "{label}: {e}");
    }
    // end == shape with start == end is an empty but valid request.
    assert!(s
        .read_numeric("filt/gzip_i4", Some(&[sl(12, 12, 1), sl(10, 10, 3)]))
        .is_ok());
    let e = s
        .read_numeric("shape/empty_1d_i4", Some(&[sl(0, 1, 1)]))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
}

#[test]
fn one_dimensional_slice_matrix_matches_reference() {
    // shape/line_i4 is 25 gzip values in chunks of 7 with v[k] = k.
    for file in FILES {
        for parallel in [false, true] {
            let s = ReadOnlyHdfSession::with_options(fixture(file), 2, parallel).unwrap();
            let mut checked = 0;
            for start in 0..=25u64 {
                for end in start..=25u64 {
                    for step in [1u64, 2, 3, 5, 7, 8, 24, 25, 26, 1 << 40] {
                        let got = s
                            .read_numeric("shape/line_i4", Some(&[sl(start, end, step)]))
                            .unwrap();
                        let want: Vec<i32> = (start..end)
                            .step_by(step.min(1 << 20) as usize)
                            .map(|k| k as i32)
                            .collect();
                        assert_eq!(got.shape, vec![want.len() as u64], "{start}..{end}:{step}");
                        assert_eq!(
                            got.values,
                            NumericValues::I32(want),
                            "{start}..{end}:{step}"
                        );
                        checked += 1;
                    }
                }
            }
            assert_eq!(checked, 351 * 10);
        }
    }
}

#[test]
fn two_dimensional_pseudo_random_slices_match_reference() {
    // Deterministic xorshift so the sequence is frozen.
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |m: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % m
    };
    let s = ReadOnlyHdfSession::open(fixture(FILES[1])).unwrap();
    for case in 0..400 {
        let r0 = next(13);
        let r1 = r0 + next(13 - r0);
        let c0 = next(11);
        let c1 = c0 + next(11 - c0);
        let (rs, cs) = (1 + next(6), 1 + next(6));
        for (path, kind) in [
            ("filt/shuffle_fletcher_gzip_i2", "i2"),
            ("endian/le_f8", "f8"),
            ("filt/shuffle_gzip_f8", "f8"),
        ] {
            let got = s
                .read_numeric(path, Some(&[sl(r0, r1, rs), sl(c0, c1, cs)]))
                .unwrap();
            let rows = strided(r0 as usize, r1 as usize, rs as usize);
            let cols = strided(c0 as usize, c1 as usize, cs as usize);
            assert_eq!(
                got.shape,
                vec![rows.len() as u64, cols.len() as u64],
                "case {case} {path}"
            );
            assert_eq!(
                got.values,
                expected(kind, &rows, &cols),
                "case {case} {path}"
            );
        }
    }
}

#[test]
fn thread_counts_and_serial_parallel_modes_are_equal() {
    let paths = [
        "filt/gzip_i4",
        "filt/shuffle_gzip_f8",
        "filt/fletcher32_f4",
        "filt/shuffle_fletcher_gzip_i2",
        "fill/partial_chunked_i4",
        "endian/le_f8",
        "series/f4_chunked_small",
        "series/f4_wide",
    ];
    for file in FILES {
        let base = ReadOnlyHdfSession::with_options(fixture(file), 1, false).unwrap();
        for threads in [1usize, 2, 4] {
            for parallel in [false, true] {
                let s = ReadOnlyHdfSession::with_options(fixture(file), threads, parallel).unwrap();
                for path in paths {
                    let a = base.read_numeric(path, None).unwrap();
                    let b = s.read_numeric(path, None).unwrap();
                    assert_eq!(
                        bytes_of(&a),
                        bytes_of(&b),
                        "{file} {path} t{threads} p{parallel}"
                    );
                    let sel = [sl(1, a.shape[0], 2), sl(0, a.shape[1], 3)];
                    assert_eq!(
                        bytes_of(&base.read_numeric(path, Some(&sel)).unwrap()),
                        bytes_of(&s.read_numeric(path, Some(&sel)).unwrap()),
                        "{file} {path} sliced t{threads} p{parallel}"
                    );
                }
            }
        }
    }
}

#[test]
fn thread_option_bounds_are_typed() {
    for threads in [0usize, 1025, usize::MAX] {
        let e = ReadOnlyHdfSession::with_options(fixture(FILES[0]), threads, true)
            .err()
            .unwrap();
        assert_eq!(e.kind, ErrorKind::InvalidRequest, "threads {threads}");
    }
    assert!(ReadOnlyHdfSession::with_options(fixture(FILES[0]), 1024, true).is_ok());
}

#[test]
fn outputs_outlive_session_and_sessions_share_across_threads() {
    let (a, b, path) = {
        let s = ReadOnlyHdfSession::open(fixture(FILES[0])).unwrap();
        let a = s
            .read_numeric("filt/shuffle_fletcher_gzip_i2", None)
            .unwrap();
        let b = s
            .read_numeric("endian/le_u8", Some(&[sl(10, 12, 1), sl(8, 10, 1)]))
            .unwrap();
        (a, b, s.path().to_path_buf())
    };
    assert_eq!(path, fixture(FILES[0]));
    assert_eq!(a.values, expected("i2", &all(12), &all(10)));
    assert_eq!(b.values, expected("u8", &[10, 11], &[8, 9]));
    let s = ReadOnlyHdfSession::open(&path).unwrap();
    assert_eq!(
        s.read_numeric("filt/shuffle_fletcher_gzip_i2", None)
            .unwrap(),
        a
    );
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4u64)
            .map(|k| {
                let s = &s;
                scope.spawn(move || {
                    for _ in 0..20 {
                        let r = s
                            .read_numeric("filt/gzip_i4", Some(&[sl(k, 12, 1), sl(0, 10, 1)]))
                            .unwrap();
                        assert_eq!(r.shape[0], 12 - k);
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().unwrap();
        }
    });
}

#[test]
fn missing_paths_are_not_found_not_empty() {
    let s = ReadOnlyHdfSession::open(fixture(FILES[0])).unwrap();
    for path in [
        "nope",
        "filt/absent",
        "absent/gzip_i4",
        "filt/gzip_i4/child",
    ] {
        assert_eq!(
            s.read_numeric(path, None).unwrap_err().kind,
            ErrorKind::NotFound,
            "{path}"
        );
        assert_eq!(
            s.numeric_capability(path).unwrap_err().kind,
            ErrorKind::NotFound,
            "{path}"
        );
        assert_eq!(
            s.read_numeric(path, Some(&[sl(0, 1, 1)])).unwrap_err().kind,
            ErrorKind::NotFound,
            "{path}"
        );
    }
    assert_eq!(
        s.group_members("filt/absent").unwrap_err().kind,
        ErrorKind::NotFound
    );
    let members = s.group_members("filt").unwrap();
    assert_eq!(members.datasets.len(), 6);
    assert!(members.groups.is_empty());
    let batch = s
        .read_numeric_batch(&["filt/gzip_i4", "filt/gzip_i4", "shape/one_1d_i4"])
        .unwrap();
    assert_eq!(
        batch.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["filt/gzip_i4", "filt/gzip_i4", "shape/one_1d_i4"]
    );
    assert_eq!(batch[0].1, batch[1].1);
    let e = s
        .read_numeric_batch(&["filt/gzip_i4", "filt/absent"])
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
    assert!(s.read_numeric_batch(&[]).unwrap().is_empty());
}

fn corrupt_copy(src: &str, tag: &str, offsets: &[(u64, u8)]) -> PathBuf {
    let mut bytes = std::fs::read(fixture(src)).unwrap();
    for &(at, mask) in offsets {
        bytes[at as usize] ^= mask;
    }
    let dst = scratch(tag).join(src);
    std::fs::write(&dst, bytes).unwrap();
    dst
}

#[test]
fn corrupted_chunk_payload_is_corrupt_error_not_panic() {
    let m = manifest();
    for file in FILES {
        let info = &m["chunk_info"][file];
        for (dataset, mask) in [
            ("filt/gzip_i4", 0xFFu8),
            ("filt/fletcher32_f4", 0x01),
            ("filt/shuffle_fletcher_gzip_i2", 0x40),
        ] {
            let off = info[dataset]["byte_offset"].as_u64().unwrap();
            let size = info[dataset]["size"].as_u64().unwrap();
            let at = off + size / 2;
            let bad = corrupt_copy(
                file,
                &format!("{}-{}", file, dataset.replace("/", "_")),
                &[(at, mask)],
            );
            let s = ReadOnlyHdfSession::open(&bad).unwrap();
            let e = s.read_numeric(dataset, None).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Corrupt, "{file} {dataset}: {e}");
            let e = s
                .read_numeric(dataset, Some(&[sl(0, 2, 1), sl(0, 2, 1)]))
                .unwrap_err();
            assert_eq!(e.kind, ErrorKind::Corrupt, "{file} {dataset} sliced: {e}");
            // A selection wholly inside an undamaged chunk still decodes.
            let ok = s
                .read_numeric(dataset, Some(&[sl(11, 12, 1), sl(9, 10, 1)]))
                .unwrap();
            assert_eq!(len_of(&ok.values), 1);
            std::fs::remove_dir_all(bad.parent().unwrap()).unwrap();
        }
    }
}

#[test]
fn truncated_and_garbage_files_fail_with_typed_errors() {
    let dir = scratch("trunc");
    let src = std::fs::read(fixture(FILES[0])).unwrap();
    for (label, bytes) in [
        ("empty", Vec::new()),
        ("signature_only", src[..8].to_vec()),
        ("half_superblock", src[..48].to_vec()),
        ("quarter", src[..src.len() / 4].to_vec()),
        ("half", src[..src.len() / 2].to_vec()),
        ("garbage", vec![0xA5; 4096]),
    ] {
        let p = dir.join(format!("{label}.h5"));
        std::fs::write(&p, bytes).unwrap();
        // Either the open or the reads must report a typed error; a panic fails the test.
        match ReadOnlyHdfSession::open(&p) {
            Err(e) => assert!(
                matches!(
                    e.kind,
                    ErrorKind::Corrupt | ErrorKind::Io | ErrorKind::Unsupported
                ),
                "{label}: {e}"
            ),
            Ok(s) => {
                for path in ["filt/gzip_i4", "series/f4_wide", "endian/le_u8"] {
                    if let Err(e) = s.read_numeric(path, None) {
                        assert!(
                            matches!(
                                e.kind,
                                ErrorKind::Corrupt | ErrorKind::NotFound | ErrorKind::Io
                            ),
                            "{label} {path}: {e}"
                        );
                    }
                }
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn string_storage_is_never_reinterpreted_as_numbers() {
    let s = ReadOnlyHdfSession::open(fixture("coverage_mesh_nogroups.h5")).unwrap();
    assert_eq!(
        s.read_strings("Geometry/2D Flow Areas/Names").unwrap(),
        vec!["Only Names".to_string()]
    );
    assert_eq!(
        s.read_strings("Geometry/2D Flow Areas/Missing")
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    let e = s.read_numeric("Geometry/2D Flow Areas/Names", None);
    assert!(
        matches!(&e, Err(err) if err.kind == ErrorKind::Unsupported),
        "{e:?}"
    );
    let cap = s
        .numeric_capability("Geometry/2D Flow Areas/Names")
        .unwrap();
    assert!(cap.kind.is_none() && !cap.capability.is_supported());
}
