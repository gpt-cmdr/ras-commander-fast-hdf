//! The engine's handle on the HDF5 reader, and the read policy the engine
//! applies on top of it.
//!
//! The reader is the `hdf5-reader` crate (roteiro-gis/netcdf-rust), a
//! general-purpose HDF5 decoder. Up to engine 0.2.3 a patched copy of it was
//! compiled into this crate, and two of its patches were CLB qualification
//! policies rather than fixes. One remains, here, unchanged in effect:
//!
//! - **Unbounded filters** (formerly reader patch 0008). A chunk is decoded
//!   with a decoded-size limit. In a pipeline of several filters, an
//!   intermediate stage is bounded by the encoded-size bound of the filters
//!   applied before it when writing. DEFLATE, shuffle, Fletcher-32, N-Bit and
//!   scale-offset have such a bound; LZ4 (32004), SZIP and unknown filters do
//!   not. hdf5-reader 0.10 then decodes the stages that come before such a
//!   filter in decoding order with only its 4 GiB per-filter ceiling; the
//!   engine refuses the read instead (`UnsupportedFilter`, so `Unsupported`
//!   with code `encoding`). The last filter of a pipeline is the first to be
//!   decoded; its output is bounded by the chunk size, so it may be of any
//!   kind.
//!
//! The other, the refusal of extensible-array chunk indexes with more elements
//! than their index block (formerly reader patch 0014), is lifted: hdf5-reader
//! 0.9.3 returned fill values for chunks past the index block, and 0.10.0 maps
//! them as libhdf5 does (roteiro-gis/netcdf-rust#102, #105). Every value of
//! the committed extensible-array fixtures (`tests/fixtures/reader_policy`,
//! up to super blocks and paged data blocks) is checked against h5py.
//!
//! [`File`], [`Group`] and [`Dataset`] wrap the reader's types and do not
//! dereference to them, so every dataset the engine reads goes through
//! [`Dataset`]'s read methods, which apply the policy before the reader
//! touches the chunk index or decodes a chunk. Metadata (shape, datatype,
//! attributes) is not affected, as before. The verdict is computed from the
//! dataset's (cached) object header, once per object address and file.
//!
//! The reader applied the policies while reading. Here they apply to the whole
//! dataset before reading, so they are slightly stricter, never looser:
//!
//! - a pipeline is judged by its filters, not by the per-chunk filter masks,
//!   so a dataset whose every chunk skips the unbounded filter, or a selection
//!   that touches only unallocated chunks, is refused too;
//! - when a refused dataset is also malformed in a way the reader would
//!   detect before reading its chunk index (an invalid chunk shape, say), the
//!   refusal is reported first.
//!
//! A read that selects no elements, and a dataset that has no chunk index
//! yet, read no chunks and are not refused, as before. No HEC-RAS file uses
//! LZ4, an unknown filter or an extensible-array index (HEC-RAS writes v1
//! B-tree indexes).
//!
//! Filtered fractal heaps (dense link or attribute storage written with an
//! I/O filter pipeline) were also decoded with the old patch 0008 in force;
//! they are not covered here. HEC-RAS does not write them.
//!
//! Every read also reserves its estimated memory from the file's in-flight
//! decode budget ([`super::decode_budget`]) after the policy admits it and
//! before the reader allocates, and holds it until the read returns.
use super::decode_budget::{DecodeBudget, Reservation};
use h5::messages::layout::DataLayout;
use h5::messages::HdfMessage;
use hdf5_reader as h5;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub(crate) use h5::error::Error;
pub(crate) type Result<T> = std::result::Result<T, Error>;

/// Filters with an encoded-size bound for an intermediate pipeline stage:
/// DEFLATE, shuffle, Fletcher-32, N-Bit and scale-offset.
fn bounded_filter(id: u16) -> bool {
    use h5::filters::{
        FILTER_DEFLATE, FILTER_FLETCHER32, FILTER_NBIT, FILTER_SCALEOFFSET, FILTER_SHUFFLE,
    };
    matches!(
        id,
        FILTER_DEFLATE | FILTER_FLETCHER32 | FILTER_SCALEOFFSET | FILTER_SHUFFLE | FILTER_NBIT
    )
}

/// Why a dataset's chunks are not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// A filter without encoded-size bound is an intermediate stage.
    UnboundedFilter(u16),
}

impl Refusal {
    /// The error the vendored reader raised (patch 0008).
    fn error(self) -> Error {
        match self {
            Self::UnboundedFilter(id) => Error::UnsupportedFilter(format!(
                "filter {id} has no qualified encoded-size bound in a bounded pipeline"
            )),
        }
    }
}

/// Per-file policy verdicts and raw attribute lists, by object address, and
/// the decode budget the file's reads draw on.
struct Shared {
    verdicts: Mutex<HashMap<u64, Option<Refusal>>>,
    raw_attributes: Mutex<HashMap<u64, Arc<[h5::RawAttribute]>>>,
    budget: Mutex<Arc<DecodeBudget>>,
}

/// An open HDF5 file.
#[derive(Clone)]
pub(crate) struct File {
    inner: h5::Hdf5File,
    shared: Arc<Shared>,
}

impl File {
    fn wrap(inner: h5::Hdf5File, budget: Arc<DecodeBudget>) -> Self {
        Self {
            inner,
            shared: Arc::new(Shared {
                verdicts: Mutex::default(),
                raw_attributes: Mutex::default(),
                budget: Mutex::new(budget),
            }),
        }
    }
    pub(crate) fn open_with_options(
        path: impl AsRef<std::path::Path>,
        options: h5::OpenOptions,
        budget: Arc<DecodeBudget>,
    ) -> Result<Self> {
        h5::Hdf5File::open_with_options(path, options).map(|f| Self::wrap(f, budget))
    }
    /// A file over any storage backend (the presence measurement counts the bytes it reads).
    #[cfg(all(test, feature = "mesh"))]
    pub(crate) fn from_storage_with_options(
        storage: h5::storage::DynStorage,
        options: h5::OpenOptions,
    ) -> Result<Self> {
        let budget = DecodeBudget::global()?;
        h5::Hdf5File::from_storage_with_options(storage, options).map(|f| Self::wrap(f, budget))
    }
    pub(crate) fn from_vec_with_options(
        data: Vec<u8>,
        options: h5::OpenOptions,
        budget: Arc<DecodeBudget>,
    ) -> Result<Self> {
        h5::Hdf5File::from_vec_with_options(data, options).map(|f| Self::wrap(f, budget))
    }
    /// The decode budget this file's reads draw on.
    pub(crate) fn decode_budget(&self) -> Arc<DecodeBudget> {
        self.shared
            .budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    /// Draw later reads (including on datasets already opened) from `budget`.
    pub(crate) fn set_decode_budget(&self, budget: Arc<DecodeBudget>) {
        *self.shared.budget.lock().unwrap_or_else(|e| e.into_inner()) = budget;
    }
    pub(crate) fn dataset(&self, path: &str) -> Result<Dataset> {
        Ok(Dataset {
            inner: self.inner.dataset(path)?,
            file: self.clone(),
        })
    }
    pub(crate) fn group(&self, path: &str) -> Result<Group> {
        Ok(Group {
            inner: self.inner.group(path)?,
            file: self.clone(),
        })
    }
    pub(crate) fn chunk_cache_stats(&self) -> h5::ChunkCacheStats {
        self.inner.chunk_cache_stats()
    }
    pub(crate) fn superblock(&self) -> &h5::superblock::Superblock {
        self.inner.superblock()
    }
    pub(crate) fn storage(&self) -> &dyn h5::Storage {
        self.inner.storage()
    }
    /// A file over a storage backend of the engine's own.
    pub(crate) fn from_storage(storage: h5::storage::DynStorage) -> Result<Self> {
        let budget = DecodeBudget::global()?;
        h5::Hdf5File::from_storage_with_options(storage, h5::OpenOptions::default())
            .map(|f| Self::wrap(f, budget))
    }

    /// The raw attributes of the object at `address`, parsed once per file
    /// (the reader parses the object header again on every call).
    fn raw_attributes(
        &self,
        address: u64,
        parse: impl FnOnce() -> Result<Vec<h5::RawAttribute>>,
    ) -> Result<Arc<[h5::RawAttribute]>> {
        let cache = || {
            self.shared
                .raw_attributes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
        };
        if let Some(found) = cache().get(&address) {
            return Ok(found.clone());
        }
        let parsed: Arc<[h5::RawAttribute]> = parse()?.into();
        Ok(cache().entry(address).or_insert(parsed).clone())
    }

    /// Both policies for the dataset whose object header is at `address`.
    /// Only verdicts are cached; a malformed index header is an error each
    /// time, as in the reader.
    fn verdict(&self, address: u64) -> Result<Option<Refusal>> {
        let cache = || {
            self.shared
                .verdicts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
        };
        if let Some(&verdict) = cache().get(&address) {
            return Ok(verdict);
        }
        let verdict = self.compute_verdict(address)?;
        cache().insert(address, verdict);
        Ok(verdict)
    }

    fn compute_verdict(&self, address: u64) -> Result<Option<Refusal>> {
        let header = self.inner.get_or_parse_header(address)?;
        // The reader keeps the last layout and filter pipeline message.
        let mut layout = None;
        let mut pipeline = None;
        for message in &header.messages {
            match message {
                HdfMessage::DataLayout(m) => layout = Some(&m.layout),
                HdfMessage::FilterPipeline(m) => pipeline = Some(m),
                _ => {}
            }
        }
        let Some(DataLayout::Chunked { address: index, .. }) = layout else {
            return Ok(None);
        };
        // No chunk index: nothing is allocated and every read is fill.
        if h5::io::Cursor::is_undefined_offset(*index, self.superblock().offset_size) {
            return Ok(None);
        }
        // Decoding runs from the last filter to the first; every filter but
        // the last is an intermediate stage.
        if let Some(pipeline) = pipeline {
            if let Some((_, earlier)) = pipeline.filters.split_last() {
                if let Some(f) = earlier.iter().find(|f| !bounded_filter(f.id)) {
                    return Ok(Some(Refusal::UnboundedFilter(f.id)));
                }
            }
        }
        Ok(None)
    }
}

/// A group of an open file.
pub(crate) struct Group {
    inner: h5::group::Group,
    file: File,
}

impl Group {
    /// Names of the child groups and datasets, opening the datasets (and
    /// failing as the reader does when one cannot be opened).
    pub(crate) fn member_lists(&self) -> Result<(Vec<String>, Vec<String>)> {
        let (groups, datasets) = self.inner.members()?;
        Ok((
            groups.iter().map(|g| g.name().to_owned()).collect(),
            datasets.iter().map(|d| d.name().to_owned()).collect(),
        ))
    }
    /// Names of the child groups and datasets, without opening the datasets.
    #[cfg(test)]
    pub(crate) fn member_names(&self) -> Result<(Vec<String>, Vec<String>)> {
        self.inner.member_names()
    }
    pub(crate) fn attributes(&self) -> Result<Vec<h5::Attribute>> {
        self.inner.attributes()
    }
    pub(crate) fn raw_attributes(&self) -> Result<Arc<[h5::RawAttribute]>> {
        self.file
            .raw_attributes(self.inner.address(), || self.inner.raw_attributes())
    }
}

/// A dataset of an open file. The read methods apply the engine's policies.
#[derive(Clone)]
pub(crate) struct Dataset {
    inner: h5::Dataset,
    file: File,
}

/// Number of elements of a selection (`None` on overflow).
fn selected(selection: &h5::SliceInfo, shape: &[u64]) -> Option<u64> {
    selection
        .selections
        .iter()
        .zip(shape)
        .try_fold(1u64, |n, (s, &dim)| {
            let count = match *s {
                h5::SliceInfoElem::Index(_) => 1,
                h5::SliceInfoElem::Slice { start, end, step } => {
                    let end = end.min(dim);
                    if step == 0 || start >= end {
                        0
                    } else {
                        (end - start).div_ceil(step)
                    }
                }
            };
            n.checked_mul(count)
        })
}

impl Dataset {
    pub(crate) fn address(&self) -> u64 {
        self.inner.address()
    }
    pub(crate) fn shape(&self) -> &[u64] {
        self.inner.shape()
    }
    pub(crate) fn ndim(&self) -> usize {
        self.inner.ndim()
    }
    pub(crate) fn dtype(&self) -> &h5::Datatype {
        self.inner.dtype()
    }
    pub(crate) fn max_dims(&self) -> Option<&[u64]> {
        self.inner.max_dims()
    }
    pub(crate) fn chunks(&self) -> Option<Vec<u32>> {
        self.inner.chunks()
    }
    pub(crate) fn num_elements(&self) -> Result<u64> {
        self.inner.num_elements()
    }
    pub(crate) fn raw_byte_len(&self) -> Result<usize> {
        self.inner.raw_byte_len()
    }
    pub(crate) fn raw_datatype(&self) -> Result<Vec<u8>> {
        self.inner.raw_datatype()
    }
    pub(crate) fn attributes(&self) -> Vec<h5::Attribute> {
        self.inner.attributes()
    }
    pub(crate) fn raw_attributes(&self) -> Result<Arc<[h5::RawAttribute]>> {
        self.file
            .raw_attributes(self.inner.address(), || self.inner.raw_attributes())
    }

    /// Apply the policies before a read of `elements` elements. The reader
    /// reads no chunk for an empty selection, so none is refused.
    fn admit(&self, elements: Option<u64>) -> Result<()> {
        if elements == Some(0) {
            return Ok(());
        }
        match self.file.verdict(self.inner.address())? {
            Some(refusal) => Err(refusal.error()),
            None => Ok(()),
        }
    }
    /// [`Self::admit`] for a whole-dataset read; `context` adds the dataset
    /// name to the error, as the reader's own full reads do.
    fn admit_all(&self, context: bool) -> Result<()> {
        let result = match self.inner.num_elements() {
            Ok(n) => self.admit(Some(n)),
            // The reader reports this itself.
            Err(_) => Ok(()),
        };
        if context {
            result.map_err(|e| e.with_context(self.inner.name()))
        } else {
            result
        }
    }
    fn chunked(&self) -> bool {
        self.inner.chunks().is_some()
    }

    /// Memory a read of `elements` elements (as `T`) holds while it runs: the
    /// returned values (at the larger of the stored and the returned element
    /// size) plus, for a chunked dataset, a stored and a decoded chunk buffer
    /// for each of up to `workers` decoding workers (no more workers than
    /// chunks). A chunk buffer counts at most the reader's chunk limit, since
    /// the reader refuses larger chunks before allocating them.
    fn decode_estimate<T>(&self, elements: u64, workers: usize) -> u64 {
        let stored = h5::dtype_element_size(self.inner.dtype()).unwrap_or(0) as u64;
        let element = stored.max(std::mem::size_of::<T>() as u64);
        let values = elements.saturating_mul(element);
        let Some(dims) = self.inner.chunks() else {
            return values;
        };
        let chunk_bytes = dims
            .iter()
            .fold(stored, |n, &d| n.saturating_mul(u64::from(d)))
            .min(h5::DEFAULT_MAX_CHUNK_BYTES as u64);
        let chunks = self
            .inner
            .shape()
            .iter()
            .zip(&dims)
            .fold(1u64, |n, (&extent, &d)| {
                n.saturating_mul(if d == 0 {
                    0
                } else {
                    extent.div_ceil(u64::from(d))
                })
            });
        let workers = (workers.max(1) as u64).min(chunks);
        values.saturating_add(workers.saturating_mul(chunk_bytes.saturating_mul(2)))
    }

    /// Reserve [`Self::decode_estimate`] from the file's decode budget. An
    /// empty or uncountable selection reserves nothing (the reader reads no
    /// chunk, or reports the overflow itself). On a rayon worker thread the
    /// read does not wait for budget (see [`super::decode_budget`]).
    fn reserve<T>(&self, elements: Option<u64>, workers: usize) -> Result<Reservation> {
        let bytes = match elements {
            Some(n) if n > 0 => self.decode_estimate::<T>(n, workers),
            _ => 0,
        };
        self.file
            .decode_budget()
            .reserve(bytes, rayon::current_thread_index().is_none())
            .map_err(Error::ResourceLimit)
    }
    /// [`Self::reserve`] for a whole-dataset read; `context` adds the dataset
    /// name to the error, as [`Self::admit_all`] does.
    fn reserve_all<T>(&self, workers: usize, context: bool) -> Result<Reservation> {
        let result = self.reserve::<T>(self.inner.num_elements().ok(), workers);
        if context {
            result.map_err(|e| e.with_context(self.inner.name()))
        } else {
            result
        }
    }

    pub(crate) fn read_array<T: h5::H5Type>(&self) -> Result<ndarray::ArrayD<T>> {
        self.admit_all(true)?;
        let _budget = self.reserve_all::<T>(1, true)?;
        self.inner.read_array()
    }
    /// Non-chunked datasets fall back to [`Self::read_array`] in the reader,
    /// which names the dataset in errors; chunked parallel reads do not.
    pub(crate) fn read_array_parallel<T: h5::H5Type>(&self) -> Result<ndarray::ArrayD<T>> {
        self.admit_all(!self.chunked())?;
        let _budget = self.reserve_all::<T>(rayon::current_num_threads(), !self.chunked())?;
        self.inner.read_array_parallel()
    }
    pub(crate) fn read_array_uncached<T: h5::H5Type>(&self) -> Result<ndarray::ArrayD<T>> {
        self.admit_all(true)?;
        let _budget = self.reserve_all::<T>(1, true)?;
        self.inner.read_array_uncached()
    }
    pub(crate) fn read_array_uncached_parallel<T: h5::H5Type>(&self) -> Result<ndarray::ArrayD<T>> {
        self.admit_all(!self.chunked())?;
        let _budget = self.reserve_all::<T>(rayon::current_num_threads(), !self.chunked())?;
        self.inner.read_array_uncached_parallel()
    }
    pub(crate) fn read_slice<T: h5::H5Type>(
        &self,
        selection: &h5::SliceInfo,
    ) -> Result<ndarray::ArrayD<T>> {
        let elements = selected(selection, self.inner.shape());
        self.admit(elements)?;
        let _budget = self.reserve::<T>(elements, 1)?;
        self.inner.read_slice(selection)
    }
    pub(crate) fn read_slice_parallel<T: h5::H5Type>(
        &self,
        selection: &h5::SliceInfo,
    ) -> Result<ndarray::ArrayD<T>> {
        let elements = selected(selection, self.inner.shape());
        self.admit(elements)?;
        let _budget = self.reserve::<T>(elements, rayon::current_num_threads())?;
        self.inner.read_slice_parallel(selection)
    }
    /// The policies apply only to selections the reader would read; it
    /// rejects others (and non-2-D or non-chunked datasets) first.
    /// Returns the number of elements gathered (`None` when the reader will
    /// reject the selection itself).
    fn admit_gather(&self, rows: (u64, u64, u64), columns: &[u64]) -> Result<Option<u64>> {
        let shape = self.inner.shape();
        if shape.len() != 2
            || rows.2 == 0
            || rows.0 > rows.1
            || rows.1 > shape[0]
            || columns.iter().any(|&c| c >= shape[1])
        {
            return Ok(None);
        }
        let rows = (rows.1 - rows.0).div_ceil(rows.2);
        let elements = rows.checked_mul(columns.len() as u64);
        self.admit(elements)?;
        Ok(elements)
    }
    pub(crate) fn read_rows_columns_chunked<T: h5::H5Type>(
        &self,
        rows: (u64, u64, u64),
        columns: &[u64],
    ) -> Result<Option<ndarray::ArrayD<T>>> {
        let elements = self.admit_gather(rows, columns)?;
        let _budget = self.reserve::<T>(elements, 1)?;
        self.inner.read_rows_columns_chunked(rows, columns)
    }
    pub(crate) fn read_rows_columns_chunked_in_pool<T: h5::H5Type>(
        &self,
        rows: (u64, u64, u64),
        columns: &[u64],
        pool: &rayon::ThreadPool,
    ) -> Result<Option<ndarray::ArrayD<T>>> {
        let elements = self.admit_gather(rows, columns)?;
        let _budget = self.reserve::<T>(elements, pool.current_num_threads())?;
        self.inner
            .read_rows_columns_chunked_in_pool(rows, columns, pool)
    }
    /// Raw stored element bytes; the reader names the dataset in errors.
    pub(crate) fn read_raw_bytes(&self) -> Result<Vec<u8>> {
        self.admit_all(true)?;
        let _budget = self.reserve_all::<u8>(1, true)?;
        self.inner.read_raw_bytes()
    }
    /// Decoded strings. Datasets of other types keep the reader's type error.
    pub(crate) fn read_strings(&self) -> Result<Vec<String>> {
        if matches!(
            self.inner.dtype(),
            h5::Datatype::String { .. }
                | h5::Datatype::VarLen {
                    kind: h5::VarLenKind::String,
                    ..
                }
        ) {
            self.admit_all(true)?;
        }
        let _budget = self.reserve_all::<u8>(1, true)?;
        self.inner.read_strings()
    }
}
