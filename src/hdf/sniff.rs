//! Reading a little of an in-memory file without copying all of it.
//!
//! The reader needs storage it owns (`'static`), so it cannot borrow the caller's bytes. A
//! [`SparseCopy`] holds copies of some byte ranges of the file and stands in for the whole of
//! it. A read outside them is recorded and fails. [`read_sparse`] runs a read against the
//! held ranges, adds the ranges that were missed, and runs it again until nothing is missed.
//! Only that last run's result is kept: every byte it read was a byte of the file, so it is the
//! result the read gives over the whole file, errors included.
// Lists of held byte ranges, often of one range, are not ranges to expand.
#![allow(clippy::single_range_in_vec_init)]
use std::ops::Range;
use std::sync::{Arc, Mutex};

/// Bytes held before the first run: the superblock and, in HEC-RAS files, the root group.
pub(super) const FIRST_BYTES: u64 = 64 * 1024;
/// Missed ranges are widened to whole pages of this size.
const PAGE: u64 = 4096;
/// Runs before the whole file is copied instead.
const MAX_RUNS: usize = 32;

/// Copies of some ranges of a file of `len` bytes, sorted, disjoint and not adjacent.
pub(super) struct SparseCopy {
    pieces: Vec<(u64, Arc<[u8]>)>,
    len: u64,
    missed: Mutex<Vec<Range<u64>>>,
}

impl SparseCopy {
    fn new(data: &[u8], held: &[Range<u64>]) -> Self {
        Self {
            pieces: held
                .iter()
                .map(|r| (r.start, data[r.start as usize..r.end as usize].into()))
                .collect(),
            len: data.len() as u64,
            missed: Mutex::default(),
        }
    }
    fn missed(&self) -> Vec<Range<u64>> {
        std::mem::take(&mut *self.missed.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl hdf5_reader::Storage for SparseCopy {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_range(
        &self,
        offset: u64,
        len: usize,
    ) -> hdf5_reader::error::Result<hdf5_reader::StorageBuffer> {
        let Some(end) = offset
            .checked_add(len as u64)
            .filter(|&end| end <= self.len)
        else {
            // Past the end of the file: what a storage of the whole file reports.
            return Err(hdf5_reader::error::Error::UnexpectedEof {
                offset,
                needed: len as u64,
                available: self.len.saturating_sub(offset),
            });
        };
        let i = self.pieces.partition_point(|(start, _)| *start <= offset);
        if let Some((start, bytes)) = i.checked_sub(1).map(|i| &self.pieces[i]) {
            if end <= start + bytes.len() as u64 {
                let at = (offset - start) as usize;
                return Ok(hdf5_reader::StorageBuffer::from_vec(
                    bytes[at..at + len].to_vec(),
                ));
            }
        }
        self.missed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(offset..end);
        Err(hdf5_reader::error::Error::InvalidData(
            "read outside the copied ranges".into(),
        ))
    }
}

/// `held` with `more` added, widened to pages and clipped to `len`, merged.
fn merge(held: &[Range<u64>], more: &[Range<u64>], len: u64) -> Vec<Range<u64>> {
    let mut all: Vec<Range<u64>> = held.to_vec();
    all.extend(more.iter().map(|r| {
        let start = r.start / PAGE * PAGE;
        let end = r.end.div_ceil(PAGE).saturating_mul(PAGE).min(len);
        start..end
    }));
    all.sort_by_key(|r| r.start);
    let mut out: Vec<Range<u64>> = Vec::with_capacity(all.len());
    for r in all {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// `read` over copies of only the ranges of `data` it needs, and the number of bytes copied
/// for its last run. See the module documentation.
pub(super) fn read_sparse<T>(data: &[u8], read: impl Fn(Arc<SparseCopy>) -> T) -> (T, u64) {
    let len = data.len() as u64;
    let mut held = vec![0..len.min(FIRST_BYTES)];
    for run in 1.. {
        let storage = Arc::new(SparseCopy::new(data, &held));
        let result = read(Arc::clone(&storage));
        let missed = storage.missed();
        let whole = held.len() == 1 && held[0] == (0..len);
        if missed.is_empty() || whole {
            return (result, held.iter().map(|r| r.end - r.start).sum());
        }
        held = if run >= MAX_RUNS {
            vec![0..len]
        } else {
            merge(&held, &missed, len)
        };
    }
    unreachable!("the loop returns once the whole file is held")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hdf5_reader::Storage;

    #[test]
    fn pieces_serve_only_reads_they_hold() {
        let data: Vec<u8> = (0..=255).cycle().take(20_000).collect();
        let s = SparseCopy::new(&data, &[0..100, 8192..12288]);
        assert_eq!(&*s.read_range(10, 5).unwrap(), &data[10..15]);
        assert_eq!(&*s.read_range(8192, 4096).unwrap(), &data[8192..12288]);
        assert_eq!(s.read_range(100, 0).unwrap().len(), 0);
        assert!(s.read_range(95, 10).is_err());
        assert!(s.read_range(12000, 300).is_err());
        assert!(matches!(
            s.read_range(19_990, 20),
            Err(hdf5_reader::error::Error::UnexpectedEof { .. })
        ));
        assert!(s.read_range(u64::MAX, 2).is_err());
        assert_eq!(s.missed(), [95..105, 12000..12300]);
    }

    #[test]
    fn missed_ranges_are_paged_clipped_and_merged() {
        assert_eq!(
            merge(
                &[0..65536],
                &[70000..70100, 65536..65540, 99_000..99_999],
                99_999
            ),
            [0..73728, 98304..99_999]
        );
    }

    #[test]
    fn reads_rerun_until_nothing_is_missed() {
        let data = vec![7u8; 300_000];
        // A read that needs one byte far out, then one more that only the first shows.
        let (sum, copied) = read_sparse(&data, |s| {
            let a = s.read_range(200_000, 1).map(|b| b[0]);
            let b = a
                .as_ref()
                .ok()
                .map(|_| s.read_range(250_000, 1).map(|b| b[0]));
            (a.ok(), b.and_then(|b| b.ok()))
        });
        assert_eq!(sum, (Some(7), Some(7)));
        assert_eq!(copied, FIRST_BYTES + 2 * PAGE);
        // Bytes shorter than the first copy are held whole.
        assert_eq!(read_sparse(&data[..10], |s| s.len()), (10, 10));
    }
}
