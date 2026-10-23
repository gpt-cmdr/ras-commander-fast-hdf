//! Bounded temporal reductions over HEC-RAS `(time, element)` result series.
//!
//! The reduction mirrors `HdfResultsMesh._get_mesh_temporal_maximum`:
//!
//! - Rows are timesteps and columns are elements (cells or faces), matching
//!   the HDF `Rows=Times` orientation. Values are reduced in `f32`; `f64`
//!   storage is rounded to `f32` first, exactly like NumPy `astype(float32)`.
//! - With a per-element offset (`Cells Minimum Elevation`), each sample is
//!   `value - offset` in `f32`, then clipped below at zero. The offset is not
//!   a dry sentinel; no value is ever treated as "dry" by magnitude.
//! - NaN and +/- infinity (before or after the offset) are missing samples.
//!   Valid negative stored values are kept; only derived values are clipped.
//! - The maximum ignores missing samples; an element without a finite sample
//!   has a NaN maximum, a peak index of -1 and a zero finite count.
//! - Ties keep the first (lowest) row index.
//! - An optional finite threshold counts samples with `value >= threshold`
//!   (inclusive) and records the first such row, or -1.
//!
//! Indices are row positions, not times; the Python adapter maps them to
//! timestamps without nanosecond coercion.
//!
//! Memory contract of the native reader (`reduce_temporal`):
//!
//! - Returned block: each read returns at most `block_rows` complete rows,
//!   `block_rows * elements * stored_item_bytes <= max_block_bytes`, except
//!   the one-row floor (one row is always read even if wider than the budget).
//!   `float64` blocks are narrowed into a second `float32` buffer, so the
//!   transient block peak is 1.5x the budget for `float64` storage.
//! - Storage chunks: HDF5 inflates a whole storage chunk to read any part of
//!   it. A chunk intersecting a block is decompressed in full, so a chunk
//!   larger than the budget (for example one `(all_times, all_elements)`
//!   chunk) is unavoidably resident while it is decompressed; that is storage
//!   decompression, not the returned series. The per-file chunk cache (64 MiB
//!   by default, reported as `chunk_cache_bytes`) retains decompressed chunks
//!   up to its size; larger chunks are not cached and are decompressed again
//!   for each block that touches them (more CPU, not more memory). Several
//!   chunks of one block may be decompressed concurrently.
//! - Accumulators: 20 bytes per element (36 with a threshold).

use crate::{Error, ErrorKind};
use rayon::prelude::*;

/// Element slab processed by one task; keeps per-task work cache-friendly.
const SLAB: usize = 4096;

/// Rows per read block for a `(time, element)` series.
///
/// The returned block (`rows * row_bytes`) never exceeds `max_block_bytes`
/// except for the one-row floor: a block always holds at least one complete
/// row, even when a single row is wider than the budget. When whole storage
/// chunks along time fit the budget, blocks are whole-chunk multiples so no
/// chunk straddles two blocks; when one storage chunk alone exceeds the
/// budget, blocks are budget-sized row slabs within that chunk.
pub fn block_rows(chunk_rows: u64, row_bytes: u64, max_block_bytes: u64) -> u64 {
    let budget_rows = (max_block_bytes / row_bytes.max(1)).max(1);
    let chunk_rows = chunk_rows.max(1);
    if chunk_rows <= budget_rows {
        budget_rows / chunk_rows * chunk_rows
    } else {
        budget_rows
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Reduction {
    pub maximum: Vec<f32>,
    pub peak_index: Vec<i64>,
    pub finite_count: Vec<u64>,
    pub exceed_count: Option<Vec<u64>>,
    pub first_exceed_index: Option<Vec<i64>>,
    pub rows: u64,
}

/// Mutable views of one slab: maximum, peak, finite count and exceedance.
type Slab<'a> = (
    &'a mut [f32],
    &'a mut [i64],
    &'a mut [u64],
    Option<(&'a mut [u64], &'a mut [i64])>,
);

/// Streaming accumulator; blocks of complete rows may arrive in any size.
#[derive(Debug)]
#[non_exhaustive]
pub struct Accumulator {
    elements: usize,
    offset: Option<Vec<f32>>,
    threshold: Option<f32>,
    rows: u64,
    maximum: Vec<f32>,
    peak_index: Vec<i64>,
    finite_count: Vec<u64>,
    exceed_count: Vec<u64>,
    first_exceed_index: Vec<i64>,
}

impl Accumulator {
    pub fn new(
        elements: usize,
        offset: Option<Vec<f32>>,
        threshold: Option<f32>,
    ) -> crate::Result<Self> {
        if let Some(offset) = &offset {
            if offset.len() != elements {
                return Err(Error::new(
                    ErrorKind::InvalidRequest,
                    format!(
                        "offset length {} does not match element count {elements}",
                        offset.len()
                    ),
                ));
            }
        }
        if threshold.is_some_and(|t| !t.is_finite()) {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "threshold must be finite",
            ));
        }
        let with_threshold = threshold.is_some();
        Ok(Self {
            elements,
            offset,
            threshold,
            rows: 0,
            maximum: vec![f32::NAN; elements],
            peak_index: vec![-1; elements],
            finite_count: vec![0; elements],
            exceed_count: if with_threshold {
                vec![0; elements]
            } else {
                Vec::new()
            },
            first_exceed_index: if with_threshold {
                vec![-1; elements]
            } else {
                Vec::new()
            },
        })
    }

    /// Fold a row-major block of `values.len() / elements` complete rows.
    #[allow(clippy::manual_is_multiple_of)] // Preserve compatibility with the extracted implementation MSRV.
    pub fn update(&mut self, values: &[f32], parallel: bool) -> crate::Result<()> {
        let n = self.elements;
        if n == 0 {
            if !values.is_empty() {
                return Err(Error::new(
                    ErrorKind::InvalidRequest,
                    "values supplied for zero elements",
                ));
            }
            return Ok(());
        }
        if values.len() % n != 0 {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                format!(
                    "block of {} values is not a whole number of {n}-element rows",
                    values.len()
                ),
            ));
        }
        let rows = values.len() / n;
        let row0 = i64::try_from(self.rows)
            .map_err(|_| Error::new(ErrorKind::InvalidRequest, "row index overflow"))?;
        if i64::try_from(rows)
            .ok()
            .and_then(|r| row0.checked_add(r))
            .is_none()
        {
            return Err(Error::new(ErrorKind::InvalidRequest, "row index overflow"));
        }
        let threshold = self.threshold;
        let offset = self.offset.as_deref();
        let fold = |start: usize,
                    maximum: &mut [f32],
                    peak: &mut [i64],
                    count: &mut [u64],
                    mut exceed: Option<(&mut [u64], &mut [i64])>| {
            let width = maximum.len();
            let off = offset.map(|o| &o[start..start + width]);
            for r in 0..rows {
                let row = &values[r * n + start..r * n + start + width];
                let index = row0 + r as i64;
                for j in 0..width {
                    let mut v = row[j];
                    if let Some(off) = off {
                        v -= off[j];
                    }
                    if !v.is_finite() {
                        continue;
                    }
                    if off.is_some() && v < 0.0 {
                        v = 0.0;
                    }
                    count[j] += 1;
                    let current = maximum[j];
                    if current.is_nan() || v > current {
                        maximum[j] = v;
                        peak[j] = index;
                    }
                    if let (Some(t), Some((counts, first))) = (threshold, exceed.as_mut()) {
                        if v >= t {
                            counts[j] += 1;
                            if first[j] < 0 {
                                first[j] = index;
                            }
                        }
                    }
                }
            }
        };
        let slabs = self
            .maximum
            .chunks_mut(SLAB)
            .zip(self.peak_index.chunks_mut(SLAB))
            .zip(self.finite_count.chunks_mut(SLAB));
        let mut work: Vec<Slab<'_>> = if threshold.is_some() {
            slabs
                .zip(
                    self.exceed_count
                        .chunks_mut(SLAB)
                        .zip(self.first_exceed_index.chunks_mut(SLAB)),
                )
                .map(|(((m, p), c), e)| (m, p, c, Some(e)))
                .collect()
        } else {
            slabs.map(|((m, p), c)| (m, p, c, None)).collect()
        };
        let task = |(slab, (m, p, c, e)): (usize, &mut Slab<'_>)| {
            let e = e.as_mut().map(|(a, b)| (&mut **a, &mut **b));
            fold(slab * SLAB, m, p, c, e);
        };
        if parallel && work.len() > 1 {
            work.par_iter_mut().enumerate().for_each(task);
        } else {
            work.iter_mut().enumerate().for_each(task);
        }
        self.rows = self
            .rows
            .checked_add(
                u64::try_from(rows)
                    .map_err(|_| Error::new(ErrorKind::InvalidRequest, "row count overflow"))?,
            )
            .ok_or_else(|| Error::new(ErrorKind::InvalidRequest, "row count overflow"))?;
        Ok(())
    }

    pub fn finish(self) -> Reduction {
        let with_threshold = self.threshold.is_some();
        Reduction {
            maximum: self.maximum,
            peak_index: self.peak_index,
            finite_count: self.finite_count,
            exceed_count: with_threshold.then_some(self.exceed_count),
            first_exceed_index: with_threshold.then_some(self.first_exceed_index),
            rows: self.rows,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reduce(
        rows: &[&[f32]],
        offset: Option<Vec<f32>>,
        threshold: Option<f32>,
        parallel: bool,
    ) -> Reduction {
        let n = rows.first().map_or(0, |r| r.len());
        let mut acc = Accumulator::new(n, offset, threshold).unwrap();
        for row in rows {
            acc.update(row, parallel).unwrap();
        }
        acc.finish()
    }

    #[test]
    fn missing_ties_and_all_missing() {
        let nan = f32::NAN;
        let inf = f32::INFINITY;
        let r = reduce(
            &[
                &[1.0, nan, -5.0, inf],
                &[3.0, nan, -2.0, 2.0],
                &[3.0, nan, -7.0, nan],
            ],
            None,
            None,
            false,
        );
        assert_eq!(r.maximum[0], 3.0);
        assert_eq!(r.peak_index, vec![1, -1, 1, 1]);
        assert!(r.maximum[1].is_nan());
        // Valid negative stored values are kept; infinity is missing.
        assert_eq!(r.maximum[2], -2.0);
        assert_eq!(r.maximum[3], 2.0);
        assert_eq!(r.finite_count, vec![3, 0, 3, 1]);
        assert_eq!(r.rows, 3);
        assert!(r.exceed_count.is_none());
    }

    #[test]
    fn offset_clips_and_threshold_is_inclusive() {
        let r = reduce(
            &[&[10.0, 5.0, 1.0], &[12.0, 4.0, 2.0], &[11.0, 6.0, 3.0]],
            Some(vec![10.0, 7.0, f32::NAN]),
            Some(1.0),
            false,
        );
        assert_eq!(r.maximum[..2], [2.0, 0.0]);
        assert!(r.maximum[2].is_nan());
        assert_eq!(r.peak_index, vec![1, 0, -1]);
        assert_eq!(r.exceed_count.unwrap(), vec![2, 0, 0]);
        assert_eq!(r.first_exceed_index.unwrap(), vec![1, -1, -1]);
    }

    #[test]
    fn blocks_and_parallel_slabs_match_serial() {
        let n = SLAB * 2 + 17;
        let rows: Vec<Vec<f32>> = (0..7)
            .map(|t| {
                (0..n)
                    .map(|j| match (t * 31 + j) % 11 {
                        0 => f32::NAN,
                        k => (k as f32 - 5.0) * (j % 3) as f32,
                    })
                    .collect()
            })
            .collect();
        let refs: Vec<&[f32]> = rows.iter().map(Vec::as_slice).collect();
        let serial = reduce(&refs, None, Some(0.0), false);
        let mut acc = Accumulator::new(n, None, Some(0.0)).unwrap();
        acc.update(&rows[..3].concat(), true).unwrap();
        acc.update(&rows[3..].concat(), true).unwrap();
        let blocked = acc.finish();
        assert_eq!(serial.peak_index, blocked.peak_index);
        assert_eq!(serial.finite_count, blocked.finite_count);
        assert_eq!(serial.exceed_count, blocked.exceed_count);
        assert_eq!(serial.first_exceed_index, blocked.first_exceed_index);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&serial.maximum), bits(&blocked.maximum));
    }

    #[test]
    fn block_rows_respect_budget_and_one_row_floor() {
        // Whole chunks fit: whole-chunk multiples.
        assert_eq!(block_rows(12, 1000, 64_000), 60);
        assert_eq!(block_rows(1, 1000, 64_000), 64);
        // One storage chunk (all 865 rows) exceeds the budget: budget-sized slabs.
        assert_eq!(block_rows(865, 1000, 64_000), 64);
        assert_eq!(block_rows(865, 1000, 5_500), 5);
        // Budget smaller than one row: one-row floor.
        assert_eq!(block_rows(865, 1000, 1), 1);
        assert_eq!(block_rows(1, 1000, 999), 1);
        assert_eq!(block_rows(0, 0, 0), 1);
        for (chunk, row, budget) in [(865u64, 359_516u64, 16u64 << 20), (7, 8, 100), (3, 4, 11)] {
            let rows = block_rows(chunk, row, budget);
            assert!(rows == 1 || rows * row <= budget, "{chunk} {row} {budget}");
        }
    }

    #[test]
    fn rejects_bad_shapes_and_threshold() {
        assert!(Accumulator::new(3, Some(vec![0.0; 2]), None).is_err());
        assert!(Accumulator::new(3, None, Some(f32::NAN)).is_err());
        let mut acc = Accumulator::new(3, None, None).unwrap();
        assert!(acc.update(&[1.0; 4], false).is_err());
        let empty = Accumulator::new(0, None, None).unwrap().finish();
        assert!(empty.maximum.is_empty() && empty.rows == 0);
    }
}

/// Requested row-block budget, not a total process RSS promise. Float64 storage
/// is narrowed to float32 only for this result-specific reduction API.
/// PartialEq follows IEEE float equality: a request containing NaN is not equal to itself.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReductionRequest {
    pub offset: Option<Vec<f32>>,
    pub threshold: Option<f32>,
    pub max_block_bytes: u64,
    pub parallel: bool,
}
impl ReductionRequest {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_offset(mut self, value: Option<Vec<f32>>) -> Self {
        self.offset = value;
        self
    }
    pub fn with_threshold(mut self, value: Option<f32>) -> Self {
        self.threshold = value;
        self
    }
    pub fn with_max_block_bytes(mut self, value: u64) -> Self {
        self.max_block_bytes = value;
        self
    }
    pub fn with_parallel(mut self, value: bool) -> Self {
        self.parallel = value;
        self
    }
}
impl Default for ReductionRequest {
    fn default() -> Self {
        Self {
            offset: None,
            threshold: None,
            max_block_bytes: 16 << 20,
            parallel: true,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TemporalReduction {
    pub values: Reduction,
    pub block_rows: u64,
    pub blocks: u64,
    /// Source storage chunk dimensions; None when no chunk layout is present (including compact/contiguous/virtual).
    pub chunk_shape: Option<Vec<u64>>,
    pub stored_item_bytes: u64,
    pub storage_chunk_bytes: Option<u64>,
    pub chunk_cache_bytes: usize,
}
impl crate::hdf::ReadOnlyHdfSession {
    /// Reduce only 2D `(time, element)` float storage. No assumed dry sentinel.
    /// Peak indices are rows; timestamp parsing/timezone policy is a separate API.
    pub fn reduce_temporal(
        &self,
        path: &str,
        request: ReductionRequest,
    ) -> crate::Result<TemporalReduction> {
        use crate::hdf::{NumericKind, NumericValues, Slice};
        use crate::{Error, ErrorKind};
        if request.max_block_bytes == 0 {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "max_block_bytes must be positive",
            ));
        }
        let cap = self.numeric_capability(path)?;
        if cap.shape.len() != 2 {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "temporal reduction requires 2D time,element storage",
            ));
        }
        let item = match cap.kind {
            Some(NumericKind::F32) => 4,
            Some(NumericKind::F64) => 8,
            _ => {
                return Err(Error::unsupported(
                    crate::UnsupportedCode::Encoding,
                    "temporal reductions support float32/float64",
                ))
            }
        };
        let (times, elements) = (cap.shape[0], cap.shape[1]);
        let n = usize::try_from(elements)
            .map_err(|_| Error::new(ErrorKind::InvalidRequest, "element count overflow"))?;
        let row_bytes = elements
            .checked_mul(item)
            .ok_or_else(|| Error::new(ErrorKind::InvalidRequest, "row byte count overflow"))?;
        let step = block_rows(
            cap.chunks
                .as_ref()
                .and_then(|c| c.first())
                .copied()
                .map(u64::from)
                .unwrap_or(1),
            row_bytes,
            request.max_block_bytes,
        );
        let storage_chunk_bytes = cap
            .chunks
            .as_ref()
            .map(|c| c.iter().fold(item, |s, &d| s.saturating_mul(u64::from(d))));
        let mut acc = Accumulator::new(n, request.offset, request.threshold)?;
        let mut start = 0;
        let mut blocks = 0;
        while start < times && n > 0 {
            let end = times.min(start.saturating_add(step));
            let selection = [
                Slice {
                    start,
                    end,
                    step: 1,
                },
                Slice {
                    start: 0,
                    end: elements,
                    step: 1,
                },
            ];
            let block = self.read_numeric(path, Some(&selection))?;
            let values = match block.values {
                NumericValues::F32(v) => v,
                NumericValues::F64(v) => v.into_iter().map(|v| v as f32).collect(),
                _ => {
                    return Err(Error::new(
                        ErrorKind::Corrupt,
                        "series datatype changed within session",
                    ))
                }
            };
            let expected = usize::try_from(end - start)
                .ok()
                .and_then(|r| r.checked_mul(n));
            if expected != Some(values.len()) {
                return Err(Error::new(ErrorKind::Corrupt, "unexpected block size"));
            }
            self.pool
                .install(|| acc.update(&values, request.parallel))?;
            start = end;
            blocks += 1;
        }
        let mut values = acc.finish();
        values.rows = times;
        Ok(TemporalReduction {
            values,
            block_rows: step,
            blocks,
            chunk_shape: cap
                .chunks
                .as_ref()
                .map(|c| c.iter().copied().map(u64::from).collect()),
            stored_item_bytes: item,
            storage_chunk_bytes,
            chunk_cache_bytes: self.file.chunk_cache_stats().max_bytes,
        })
    }
}

impl Reduction {
    pub fn new(
        maximum: Vec<f32>,
        peak_index: Vec<i64>,
        finite_count: Vec<u64>,
        exceed_count: Option<Vec<u64>>,
        first_exceed_index: Option<Vec<i64>>,
        rows: u64,
    ) -> Self {
        Self {
            maximum,
            peak_index,
            finite_count,
            exceed_count,
            first_exceed_index,
            rows,
        }
    }
}
