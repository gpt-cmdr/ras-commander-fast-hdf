//! Behavioural coverage for temporal reductions (results.rs).
//!
//! Property-style tables compare the streaming Accumulator and ReadOnlyHdfSession::reduce_temporal
//! against a deliberately simple per-element reference reduction written in this file.
#![cfg(all(feature = "hdf", feature = "results"))]
use ras_commander_fast_hdf::hdf::{NumericValues, ReadOnlyHdfSession};
use ras_commander_fast_hdf::results::{block_rows, Accumulator, Reduction, ReductionRequest};
use ras_commander_fast_hdf::ErrorKind;
use std::path::PathBuf;

const FILES: [&str; 2] = ["coverage_numeric_v1.h5", "coverage_numeric_v2.h5"];

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/coverage")
        .join(name)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self, m: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % m
    }
}

/// Simple reference: one element at a time, rows in order, no slabs, no blocks.
fn reference(
    values: &[f32],
    n: usize,
    offset: Option<&[f32]>,
    threshold: Option<f32>,
) -> Reduction {
    let rows = values.len().checked_div(n).unwrap_or(0);
    let mut maximum = vec![f32::NAN; n];
    let mut peak = vec![-1i64; n];
    let mut count = vec![0u64; n];
    let mut exceed = vec![0u64; n];
    let mut first = vec![-1i64; n];
    for j in 0..n {
        for r in 0..rows {
            let mut v = values[r * n + j];
            if let Some(o) = offset {
                v -= o[j];
            }
            if v.is_nan() || v.is_infinite() {
                continue;
            }
            if offset.is_some() && v < 0.0 {
                v = 0.0;
            }
            count[j] += 1;
            if maximum[j].is_nan() || v > maximum[j] {
                maximum[j] = v;
                peak[j] = r as i64;
            }
            if let Some(t) = threshold {
                if v >= t {
                    exceed[j] += 1;
                    if first[j] < 0 {
                        first[j] = r as i64;
                    }
                }
            }
        }
    }
    Reduction::new(
        maximum,
        peak,
        count,
        threshold.map(|_| exceed),
        threshold.map(|_| first),
        rows as u64,
    )
}

/// Equality that distinguishes NaN positions and signed zero by bit pattern.
fn assert_same(a: &Reduction, b: &Reduction, ctx: &str) {
    let bits = |v: &[f32]| {
        v.iter()
            .map(|x| if x.is_nan() { u32::MAX } else { x.to_bits() })
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&a.maximum), bits(&b.maximum), "{ctx}: maximum");
    assert_eq!(a.peak_index, b.peak_index, "{ctx}: peak");
    assert_eq!(a.finite_count, b.finite_count, "{ctx}: count");
    assert_eq!(a.exceed_count, b.exceed_count, "{ctx}: exceed");
    assert_eq!(a.first_exceed_index, b.first_exceed_index, "{ctx}: first");
    assert_eq!(a.rows, b.rows, "{ctx}: rows");
}

const SPECIAL: [f32; 14] = [
    f32::NAN,
    f32::INFINITY,
    f32::NEG_INFINITY,
    0.0,
    -0.0,
    1.0,
    1.0,
    -1.0,
    2.5,
    -7.25,
    3.0e38,
    -3.0e38,
    1.0e-40,
    16_777_216.0,
];

fn table(rng: &mut Rng, rows: usize, n: usize) -> Vec<f32> {
    (0..rows * n)
        .map(|_| match rng.next(4) {
            0 => SPECIAL[rng.next(SPECIAL.len() as u64) as usize],
            1 => (rng.next(9) as f32) - 4.0,
            _ => (rng.next(2001) as f32 - 1000.0) / 8.0,
        })
        .collect()
}

fn run_blocks(
    values: &[f32],
    n: usize,
    offset: Option<Vec<f32>>,
    threshold: Option<f32>,
    cuts: &[usize],
    parallel: bool,
) -> Reduction {
    let mut acc = Accumulator::new(n, offset, threshold).unwrap();
    let rows = values.len().checked_div(n).unwrap_or(0);
    let mut start = 0;
    for &c in cuts.iter().chain(std::iter::once(&rows)) {
        let c = c.min(rows);
        if c > start {
            acc.update(&values[start * n..c * n], parallel).unwrap();
            start = c;
        }
    }
    acc.finish()
}

#[test]
fn accumulator_matches_reference_on_random_tables() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let widths = [1usize, 2, 3, 5, 17, 4095, 4096, 4097, 8200];
    let thresholds = [
        None,
        Some(0.0f32),
        Some(1.0),
        Some(-1.0),
        Some(2.5),
        Some(-0.0),
        Some(3.0e38),
    ];
    let mut cases = 0;
    for &n in &widths {
        for rows in [0usize, 1, 2, 3, 7, 12] {
            if n > 100 && rows > 7 {
                continue;
            }
            let values = table(&mut rng, rows, n);
            let offset_vec: Vec<f32> = (0..n)
                .map(|_| match rng.next(6) {
                    0 => f32::NAN,
                    1 => f32::INFINITY,
                    2 => f32::NEG_INFINITY,
                    3 => -3.0e38,
                    _ => (rng.next(41) as f32) - 20.0,
                })
                .collect();
            for with_offset in [false, true] {
                for threshold in thresholds {
                    let off = with_offset.then_some(offset_vec.as_slice());
                    let want = reference(&values, n, off, threshold);
                    let mut cuts: Vec<usize> = (0..rng.next(4))
                        .map(|_| rng.next(rows as u64 + 1) as usize)
                        .collect();
                    cuts.sort_unstable();
                    for parallel in [false, true] {
                        let got = run_blocks(
                            &values,
                            n,
                            with_offset.then(|| offset_vec.clone()),
                            threshold,
                            &cuts,
                            parallel,
                        );
                        let ctx = format!("n{n} rows{rows} off{with_offset} thr{threshold:?} cuts{cuts:?} par{parallel}");
                        assert_same(&got, &want, &ctx);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert!(cases > 1000, "{cases}");
}

#[test]
fn every_row_split_gives_identical_results_and_global_row_indices() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let (rows, n) = (11usize, 6usize);
    let values = table(&mut rng, rows, n);
    let want = reference(&values, n, None, Some(0.5));
    // All two-cut partitions, including empty leading and trailing blocks.
    for a in 0..=rows {
        for b in a..=rows {
            for parallel in [false, true] {
                let got = run_blocks(&values, n, None, Some(0.5), &[a, b], parallel);
                assert_same(&got, &want, &format!("cuts {a},{b}"));
            }
        }
    }
    let cuts: Vec<usize> = (1..rows).collect();
    assert_same(
        &run_blocks(&values, n, None, Some(0.5), &cuts, true),
        &want,
        "row at a time",
    );
}

#[test]
fn missing_value_semantics_are_pinned() {
    let nan = f32::NAN;
    let inf = f32::INFINITY;
    // Columns: all-NaN, +inf only, -inf only, mixed, negative-only, single finite, zero/neg-zero tie.
    let rows: [[f32; 7]; 4] = [
        [nan, inf, -inf, nan, -3.0, nan, 0.0],
        [nan, inf, -inf, 2.0, -2.0, nan, -0.0],
        [nan, inf, -inf, inf, -2.0, 4.0, 0.0],
        [nan, inf, -inf, -inf, -9.0, nan, -0.0],
    ];
    let flat: Vec<f32> = rows.iter().flatten().copied().collect();
    let mut acc = Accumulator::new(7, None, None).unwrap();
    acc.update(&flat, false).unwrap();
    let r = acc.finish();
    assert!(r.maximum[0].is_nan() && r.maximum[1].is_nan() && r.maximum[2].is_nan());
    assert_eq!(r.peak_index, vec![-1, -1, -1, 1, 1, 2, 0]);
    assert_eq!(r.finite_count, vec![0, 0, 0, 1, 4, 1, 4]);
    assert_eq!(r.maximum[3], 2.0);
    // Valid negative stored values are kept without an offset; a tie keeps the first row.
    assert_eq!(r.maximum[4], -2.0);
    assert_eq!(r.maximum[5], 4.0);
    // 0.0 then -0.0 ties: the first sample (positive zero at row 0) wins.
    assert_eq!(r.maximum[6].to_bits(), 0.0f32.to_bits());
    assert_eq!(r.peak_index[6], 0);
    assert_eq!(r.rows, 4);
    assert!(r.exceed_count.is_none() && r.first_exceed_index.is_none());
}

#[test]
fn offset_clip_threshold_and_dry_semantics() {
    // Rows are time, columns are elements; elevation offsets are per element.
    let flat = [
        -5.0f32, 1.0, 2.0, 7.0, //
        -3.0, 0.5, 2.0, 7.0, //
        -9.0, 0.25, 2.0, 6.0,
    ];
    let offset = vec![-4.0, 0.0, 2.0, f32::INFINITY];
    for threshold in [None, Some(0.0f32), Some(0.5), Some(-1.0)] {
        let mut acc = Accumulator::new(4, Some(offset.clone()), threshold).unwrap();
        acc.update(&flat, true).unwrap();
        let r = acc.finish();
        // col0: -1 -> 0 (clipped), 1, -5 -> 0 : max 1 at row 1.
        // col1: 1, .5, .25 : max 1 at row 0.   col2: dry (0) on every row, tie -> row 0.
        // col3: value - inf = -inf -> missing on every row (offset is not a dry sentinel).
        assert_eq!(r.maximum[..3], [1.0, 1.0, 0.0]);
        assert!(r.maximum[3].is_nan());
        assert_eq!(r.peak_index, vec![1, 0, 0, -1]);
        assert_eq!(r.finite_count, vec![3, 3, 3, 0]);
        match threshold {
            None => assert!(r.exceed_count.is_none()),
            Some(t) if t <= 0.0 => {
                // Inclusive threshold: clipped dry samples (exactly 0) count as exceedances.
                assert_eq!(r.exceed_count.unwrap(), vec![3, 3, 3, 0]);
                assert_eq!(r.first_exceed_index.unwrap(), vec![0, 0, 0, -1]);
            }
            Some(_) => {
                assert_eq!(r.exceed_count.unwrap(), vec![1, 2, 0, 0]);
                assert_eq!(r.first_exceed_index.unwrap(), vec![1, 0, -1, -1]);
            }
        }
    }
}

#[test]
fn threshold_is_inclusive_at_every_representable_boundary() {
    let t = 1.5f32;
    let below = f32::from_bits(t.to_bits() - 1);
    let above = f32::from_bits(t.to_bits() + 1);
    let flat = [below, t, above, t];
    let mut acc = Accumulator::new(1, None, Some(t)).unwrap();
    acc.update(&flat, false).unwrap();
    let r = acc.finish();
    assert_eq!(r.exceed_count.unwrap(), vec![3]);
    assert_eq!(r.first_exceed_index.unwrap(), vec![1]);
    assert_eq!(r.peak_index, vec![2]);
    assert_eq!(r.maximum, vec![above]);
}

#[test]
fn accumulator_rejects_bad_requests_with_invalid_request_kind() {
    let kind = |e: ras_commander_fast_hdf::Error| e.kind;
    assert_eq!(
        kind(Accumulator::new(3, Some(vec![0.0; 4]), None).err().unwrap()),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(Accumulator::new(0, Some(vec![0.0]), None).err().unwrap()),
        ErrorKind::InvalidRequest
    );
    for t in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            kind(Accumulator::new(3, None, Some(t)).err().unwrap()),
            ErrorKind::InvalidRequest
        );
    }
    let mut acc = Accumulator::new(3, None, None).unwrap();
    assert_eq!(
        kind(acc.update(&[1.0; 5], false).unwrap_err()),
        ErrorKind::InvalidRequest
    );
    // A rejected block does not poison the accumulator or advance the row counter.
    acc.update(&[1.0, 2.0, 3.0], false).unwrap();
    assert_eq!(acc.finish().rows, 1);
    let mut zero = Accumulator::new(0, Some(vec![]), Some(1.0)).unwrap();
    assert_eq!(
        kind(zero.update(&[1.0], false).unwrap_err()),
        ErrorKind::InvalidRequest
    );
    zero.update(&[], true).unwrap();
    let r = zero.finish();
    assert!(r.maximum.is_empty() && r.exceed_count.as_ref().unwrap().is_empty() && r.rows == 0);
    // No rows at all: all-missing summary; exceed buffers exist when a threshold was requested.
    let r = Accumulator::new(3, None, Some(0.0)).unwrap().finish();
    assert!(r.maximum.iter().all(|v| v.is_nan()));
    assert_eq!(r.peak_index, vec![-1; 3]);
    assert_eq!(r.finite_count, vec![0; 3]);
    assert_eq!(r.exceed_count.unwrap(), vec![0; 3]);
    assert_eq!(r.first_exceed_index.unwrap(), vec![-1; 3]);
}

#[test]
fn block_rows_invariants_hold_over_extreme_parameter_grid() {
    let chunks = [0u64, 1, 2, 3, 7, 12, 865, u64::MAX];
    let row_bytes = [0u64, 1, 4, 148, 1000, 359_516, 1 << 40, u64::MAX];
    let budgets = [
        0u64,
        1,
        7,
        100,
        999,
        1000,
        64_000,
        16 << 20,
        1 << 40,
        u64::MAX,
    ];
    for &c in &chunks {
        for &r in &row_bytes {
            for &b in &budgets {
                let rows = block_rows(c, r, b);
                assert!(rows >= 1, "{c} {r} {b}");
                let budget_rows = (b / r.max(1)).max(1);
                let c1 = c.max(1);
                if c1 <= budget_rows {
                    assert_eq!(rows % c1, 0, "{c} {r} {b}: whole chunks");
                    assert!(
                        rows <= budget_rows
                            && u128::from(rows) + u128::from(c1) > u128::from(budget_rows),
                        "{c} {r} {b}: maximal"
                    );
                } else {
                    assert_eq!(
                        rows, budget_rows,
                        "{c} {r} {b}: slab within oversized chunk"
                    );
                }
                if rows > 1 {
                    assert!(
                        u128::from(rows) * u128::from(r) <= u128::from(b),
                        "{c} {r} {b}: budget"
                    );
                }
            }
        }
    }
}

/// Read a float series fully and narrow to f32 exactly as the reduction contract describes.
fn load(s: &ReadOnlyHdfSession, path: &str) -> (Vec<f32>, usize, usize) {
    let a = s.read_numeric(path, None).unwrap();
    let values = match a.values {
        NumericValues::F32(v) => v,
        NumericValues::F64(v) => v.into_iter().map(|x| x as f32).collect(),
        other => panic!("not float: {other:?}"),
    };
    (values, a.shape[0] as usize, a.shape[1] as usize)
}

const SERIES: [(&str, u64); 8] = [
    ("series/f4_chunked_gzip", 4),
    ("series/f4_chunked_small", 4),
    ("series/f8_one_chunk", 8),
    ("series/f4_contiguous", 4),
    ("series/f4_wide", 4),
    ("series/f4_one", 4),
    ("series/f4_one_row", 4),
    ("series/f4_one_col", 4),
];

#[test]
fn reduce_temporal_matches_reference_across_budgets_threads_and_options() {
    for file in FILES {
        for threads in [1usize, 2, 4] {
            for parallel in [false, true] {
                let s = ReadOnlyHdfSession::with_options(fixture(file), threads, parallel).unwrap();
                for (path, item) in SERIES {
                    let (values, times, n) = load(&s, path);
                    let offset: Vec<f32> = (0..n).map(|j| (j % 5) as f32 - 1.0).collect();
                    let row_bytes = n as u64 * item;
                    for budget in [
                        1,
                        row_bytes - 1,
                        row_bytes,
                        row_bytes + 1,
                        row_bytes * 3,
                        row_bytes * 7,
                        4096,
                        u64::MAX,
                    ] {
                        for (use_offset, threshold) in [
                            (false, None),
                            (true, None),
                            (false, Some(1.0f32)),
                            (true, Some(0.0f32)),
                        ] {
                            let off = use_offset.then_some(offset.as_slice());
                            let want = reference(&values, n, off, threshold);
                            let got = s
                                .reduce_temporal(
                                    path,
                                    ReductionRequest::new()
                                        .with_offset(off.map(<[f32]>::to_vec))
                                        .with_threshold(threshold)
                                        .with_max_block_bytes(budget.max(1))
                                        .with_parallel(parallel),
                                )
                                .unwrap();
                            let ctx = format!("{file} {path} t{threads} p{parallel} budget{budget} off{use_offset} thr{threshold:?}");
                            assert_same(&got.values, &want, &ctx);
                            assert_eq!(got.stored_item_bytes, item, "{ctx}");
                            assert!(got.blocks >= 1 && got.block_rows >= 1, "{ctx}");
                            assert_eq!(
                                got.blocks,
                                (times as u64).div_ceil(got.block_rows),
                                "{ctx}: block count"
                            );
                            if got.block_rows > 1 {
                                assert!(
                                    got.block_rows * row_bytes <= budget.max(1),
                                    "{ctx}: budget respected"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn block_budget_boundaries_select_expected_row_counts() {
    // series/f4_chunked_gzip is 30 x 37 float32 in 7-row chunks: row = 148 bytes.
    let s = ReadOnlyHdfSession::open(fixture(FILES[0])).unwrap();
    let path = "series/f4_chunked_gzip";
    let run = |budget: u64| {
        let r = s
            .reduce_temporal(path, ReductionRequest::new().with_max_block_bytes(budget))
            .unwrap();
        (r.block_rows, r.blocks, r.storage_chunk_bytes)
    };
    assert_eq!(run(148 * 14).0, 14); // two whole chunks exactly fit
    assert_eq!(run(148 * 14 - 1).0, 7); // one byte short -> down to one whole chunk
    assert_eq!(run(148 * 7).0, 7); // exactly one chunk
    assert_eq!(run(148 * 7 - 1).0, 6); // chunk exceeds budget -> budget-sized slab
    assert_eq!(run(148 * 6).0, 6);
    assert_eq!(run(148).0, 1); // exactly one row
    assert_eq!(run(147).0, 1); // below one row: one-row floor
    assert_eq!(run(1).0, 1);
    assert_eq!(run(148 * 14), (14, 3, Some(7 * 37 * 4)));
    assert_eq!(run(148 * 6).1, 5);
    assert_eq!(run(148).1, 30);
    // Budget larger than the data, and the maximum possible budget: a single block.
    assert_eq!((run(148 * 30).0, run(148 * 30).1), (28, 2)); // whole-chunk multiple below the row count
    assert_eq!(run(148 * 35).1, 1);
    assert_eq!(run(u64::MAX).1, 1);
    // Contiguous storage has no chunk shape; block rows are purely budget driven.
    let r = s
        .reduce_temporal(
            "series/f4_contiguous",
            ReductionRequest::new().with_max_block_bytes(20 * 4),
        )
        .unwrap();
    assert_eq!(
        (r.block_rows, r.blocks, r.storage_chunk_bytes),
        (4, 3, None)
    );
    assert!(r.chunk_cache_bytes > 0);
}

#[test]
fn float64_overflow_underflow_and_dry_offsets_follow_narrowing_contract() {
    let s = ReadOnlyHdfSession::open(fixture(FILES[1])).unwrap();
    let r = s
        .reduce_temporal("series/f8_overflow", ReductionRequest::default())
        .unwrap()
        .values;
    // f64 -> f32 narrowing happens before the missing test: 1e300 and 3.5e38 become +inf (missing),
    // -1e300 becomes -inf (missing), and 1e-60 underflows to a finite 0.0.
    assert_eq!(r.maximum, vec![1.0, 2.0, 3.0e38f32]);
    assert_eq!(r.peak_index, vec![1, 1, 0]);
    assert_eq!(r.finite_count, vec![2, 1, 2]);
    let r = s
        .reduce_temporal(
            "series/f4_negative_dry",
            ReductionRequest::new()
                .with_offset(Some(vec![-4.0, 0.0, 2.0]))
                .with_threshold(Some(0.0)),
        )
        .unwrap()
        .values;
    assert_eq!(r.maximum, vec![1.0, 1.0, 0.0]);
    assert_eq!(r.peak_index, vec![1, 0, 0]);
    assert_eq!(r.exceed_count.unwrap(), vec![3, 3, 3]);
    let r = s
        .reduce_temporal("series/f4_negative_dry", ReductionRequest::default())
        .unwrap()
        .values;
    assert_eq!(r.maximum, vec![-3.0, 1.0, 2.0]);
    assert_eq!(r.peak_index, vec![1, 0, 0]);
}

#[test]
fn degenerate_series_shapes_and_fill_only_storage() {
    for file in FILES {
        let s = ReadOnlyHdfSession::open(fixture(file)).unwrap();
        let none = s
            .reduce_temporal("series/f4_zero_time", ReductionRequest::default())
            .unwrap();
        assert_eq!((none.blocks, none.values.rows), (0, 0));
        assert_eq!(none.values.peak_index, vec![-1; 6]);
        assert!(none.values.maximum.iter().all(|v| v.is_nan()));
        assert_eq!(none.values.finite_count, vec![0; 6]);
        let thr = s
            .reduce_temporal(
                "series/f4_zero_time",
                ReductionRequest::new().with_threshold(Some(1.0)),
            )
            .unwrap();
        assert_eq!(thr.values.exceed_count.unwrap(), vec![0; 6]);
        let wide = s
            .reduce_temporal("series/f4_zero_elem", ReductionRequest::default())
            .unwrap();
        assert!(wide.values.maximum.is_empty());
        assert_eq!(wide.blocks, 0);
        let off = s
            .reduce_temporal(
                "series/f4_zero_elem",
                ReductionRequest::new().with_offset(Some(vec![])),
            )
            .unwrap();
        assert!(off.values.peak_index.is_empty());
        // Unallocated chunked storage reads as its fill value, once per row.
        let fill = s
            .reduce_temporal("series/f4_unalloc_fill", ReductionRequest::default())
            .unwrap()
            .values;
        assert_eq!(fill.maximum, vec![1.5; 6]);
        assert_eq!(fill.peak_index, vec![0; 6]);
        assert_eq!(fill.finite_count, vec![8; 6]);
        let nan_fill = s
            .reduce_temporal(
                "series/f4_unalloc_nan_fill",
                ReductionRequest::new().with_threshold(Some(0.0)),
            )
            .unwrap()
            .values;
        assert!(nan_fill.maximum.iter().all(|v| v.is_nan()));
        assert_eq!(nan_fill.finite_count, vec![0; 6]);
        assert_eq!(nan_fill.first_exceed_index.unwrap(), vec![-1; 6]);
        assert_eq!(nan_fill.rows, 8);
        let one = s
            .reduce_temporal("series/f4_one", ReductionRequest::default())
            .unwrap()
            .values;
        assert_eq!(
            (one.maximum, one.peak_index, one.finite_count, one.rows),
            (vec![3.5], vec![0], vec![1], 1)
        );
    }
}

#[test]
fn reduce_temporal_rejects_unsupported_storage_and_bad_requests() {
    let s = ReadOnlyHdfSession::open(fixture(FILES[0])).unwrap();
    let kind = |path: &str, req: ReductionRequest| s.reduce_temporal(path, req).unwrap_err().kind;
    let d = ReductionRequest::default;
    assert_eq!(kind("series/i4_2d", d()), ErrorKind::Unsupported);
    assert_eq!(kind("series/f4_1d", d()), ErrorKind::InvalidRequest);
    assert_eq!(kind("series/f4_3d", d()), ErrorKind::InvalidRequest);
    assert_eq!(kind("series/absent", d()), ErrorKind::NotFound);
    assert_eq!(kind("series", d()), ErrorKind::NotFound);
    assert_eq!(
        kind(
            "series/f4_chunked_gzip",
            ReductionRequest::new().with_max_block_bytes(0)
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            "series/f4_chunked_gzip",
            ReductionRequest::new().with_offset(Some(vec![0.0; 36]))
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            "series/f4_chunked_gzip",
            ReductionRequest::new().with_offset(Some(vec![0.0; 38]))
        ),
        ErrorKind::InvalidRequest
    );
    for t in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            kind(
                "series/f4_chunked_gzip",
                ReductionRequest::new().with_threshold(Some(t))
            ),
            ErrorKind::InvalidRequest
        );
    }
    // Big-endian float storage is rejected explicitly rather than reduced from wrong bytes.
    #[cfg(target_endian = "little")]
    assert_eq!(kind("series/f4_be", d()), ErrorKind::Unsupported);
    // A zero-element series still validates the request before returning an empty result.
    assert_eq!(
        kind(
            "series/f4_zero_elem",
            ReductionRequest::new().with_offset(Some(vec![1.0]))
        ),
        ErrorKind::InvalidRequest
    );
}

#[test]
fn corrupted_series_chunk_surfaces_corrupt_not_a_partial_reduction() {
    let m: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/coverage/coverage_manifest.json")).unwrap();
    for file in FILES {
        let info = &m["chunk_info"][file]["series/f4_chunked_gzip"];
        let at = info["byte_offset"].as_u64().unwrap() + info["size"].as_u64().unwrap() / 2;
        let mut bytes = std::fs::read(fixture(file)).unwrap();
        bytes[at as usize] ^= 0xFF;
        let dir =
            std::env::temp_dir().join(format!("ras-coverage-series-{}-{file}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join(file);
        std::fs::write(&bad, bytes).unwrap();
        let s = ReadOnlyHdfSession::open(&bad).unwrap();
        for budget in [148u64, 148 * 7, u64::MAX] {
            let e = s
                .reduce_temporal(
                    "series/f4_chunked_gzip",
                    ReductionRequest::new().with_max_block_bytes(budget),
                )
                .unwrap_err();
            assert_eq!(e.kind, ErrorKind::Corrupt, "{file} budget {budget}: {e}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
