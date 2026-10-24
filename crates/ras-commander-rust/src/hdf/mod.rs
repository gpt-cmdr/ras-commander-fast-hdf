//! Persistent read-only path sessions. Outputs own their buffers and outlive the session.
//! The source must remain immutable while a session is open (memory-mapped reader).
//! Sessions share a process pool per explicit worker count; metadata/data caches are session-local.
use crate::{Capability, UnsupportedCode};
mod encoding;
use crate::{Error, ErrorKind, Result};
use encoding::map_error;
pub use encoding::{EncodingDescriptor, FloatLayout};
use hdf5_reader::{Hdf5File, SliceInfo, SliceInfoElem};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NumericKind {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
}
/// Recommended `max_bytes` for metadata and identity reads. `max_bytes` has one meaning in
/// every metadata call (capability reports, inventories, identity tables, unit evidence): the
/// largest raw stored size of any single dataset, table or attribute consulted by that call.
/// Result selections (`read_*`) instead bound the bytes they return. Pass the same value to a
/// capability report and to the matching reads to get matching verdicts.
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Budget the unbudgeted whole-read entry points (`read_numeric`,
/// `read_numeric_batch`, `read_topology`) apply: 2 GiB of decoded values.
///
/// The declared shape of a dataset comes from the file, so a 1 KB file can
/// declare terabytes. Reads larger than the budget are refused with
/// `Unsupported{ResourceLimit}` before anything is allocated. Callers that
/// need a different limit use the `*_bounded` entry points.
pub const DEFAULT_READ_MAX_BYTES: usize = 2 * 1024 * 1024 * 1024;
impl NumericKind {
    /// Stable spelling that names the exact stored width (`f32`, `i64`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
}
impl ByteOrder {
    /// Stable snake_case spelling (never the Debug form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Little => "little",
            Self::Big => "big",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum NumericValues {
    I8(Vec<i8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct NumericArray {
    pub shape: Vec<u64>,
    pub values: NumericValues,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Slice {
    pub start: u64,
    pub end: u64,
    pub step: u64,
}
/// Refusal reason shared by readers whose envelope cannot represent an H5S_NULL dataspace.
pub(crate) const NULL_DATASPACE: &str =
    "NULL dataspace holds no elements (h5py Empty); no array or list represents it";
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NumericCapability {
    pub max_shape: Option<Vec<u64>>,
    pub encoding: Option<EncodingDescriptor>,
    pub byte_order: Option<ByteOrder>,
    pub signed: Option<bool>,
    pub dataset: String,
    /// Stored dims (`[]` for both Scalar and Null; see `dataspace`).
    pub shape: Vec<u64>,
    /// Explicit class; a Null dataset is Unsupported(schema), never a scalar.
    pub dataspace: Dataspace,
    pub chunks: Option<Vec<u32>>,
    pub kind: Option<NumericKind>,
    pub capability: Capability,
    #[deprecated(note = "use capability; compatibility for pinned Python bridge")]
    pub unsupported_reason: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ByteOrder {
    Little,
    Big,
}
/// Decoded-chunk cache counters and configured retained-memory limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStatistics {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
    pub retained_bytes: usize,
    pub entries: usize,
    pub max_bytes: usize,
    pub max_slots: usize,
}
/// Raw attribute payload is retained, but is not a claim that compound/reference values
/// have been semantically decoded. Strings are decoded only when the reader supports them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attribute {
    pub name: String,
    pub shape: Vec<u64>,
    pub attribute_type: AttributeType,
    pub datatype_label: String,
    pub raw_bytes: Vec<u8>,
    pub strings: Option<Vec<String>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GroupMembers {
    pub groups: Vec<String>,
    pub datasets: Vec<String>,
}

#[non_exhaustive]
pub struct ReadOnlyHdfSession {
    pub(crate) file: Hdf5File,
    pub(crate) pool: std::sync::Arc<rayon::ThreadPool>,
    memo: memo::Memo,
    path: PathBuf,
    parallel: bool,
    chunk_cache_bytes: usize,
}
impl std::fmt::Debug for ReadOnlyHdfSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadOnlyHdfSession")
            .field("path", &self.path)
            .field("parallel", &self.parallel)
            .finish_non_exhaustive()
    }
}
impl Slice {
    pub fn new(start: u64, end: u64, step: u64) -> Self {
        Self { start, end, step }
    }
}
impl ReadOnlyHdfSession {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::with_options(path, 1, true)
    }
    pub fn with_options(path: impl AsRef<Path>, threads: usize, parallel: bool) -> Result<Self> {
        {
            let defaults = hdf5_reader::OpenOptions::default();
            Self::with_cache_options(
                path,
                threads,
                parallel,
                defaults.chunk_cache_bytes,
                defaults.chunk_cache_slots,
            )
        }
    }
    /// Configure retained decoded-chunk bytes/slots. Zero bytes disables retention.
    pub fn with_cache_options(
        path: impl AsRef<Path>,
        threads: usize,
        parallel: bool,
        chunk_cache_bytes: usize,
        chunk_cache_slots: usize,
    ) -> Result<Self> {
        if !(1..=1_048_576).contains(&chunk_cache_slots) {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "chunk_cache_slots must be in 1..=1048576",
            ));
        }
        let pool = crate::pool(threads)?;
        let options = hdf5_reader::OpenOptions {
            chunk_cache_bytes,
            chunk_cache_slots,
            ..Default::default()
        };
        let file = Hdf5File::open_with_options(path.as_ref(), options)
            .map_err(|e| map_error(e).with_path(path.as_ref().to_path_buf()))?;
        Ok(Self {
            file,
            memo: memo::Memo::default(),
            pool,
            path: path.as_ref().to_path_buf(),
            parallel,
            chunk_cache_bytes,
        })
    }
    #[cfg(feature = "mesh")]
    pub(crate) fn parallel_enabled(&self) -> bool {
        self.parallel
    }
    /// Worker count of this session-owned pool (also used by mesh assembly).
    pub fn thread_count(&self) -> usize {
        self.pool.current_num_threads()
    }
    /// Snapshot counters without changing cache policy or source data.
    pub fn chunk_cache_stats(&self) -> CacheStatistics {
        let s = self.file.chunk_cache_stats();
        CacheStatistics {
            hits: s.hits,
            misses: s.misses,
            inserts: s.inserts,
            evictions: s.evictions,
            retained_bytes: s.current_bytes,
            entries: s.entries,
            max_bytes: s.max_bytes,
            max_slots: s.max_slots,
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn group_members(&self, path: &str) -> Result<GroupMembers> {
        self.memo(format!("group_members:{:?}", (path,)), || {
            let group = self.file.group(path).map_err(map_error)?;
            let (groups, datasets) = group.members().map_err(map_error)?;
            Ok(GroupMembers {
                groups: groups.into_iter().map(|g| g.name().to_owned()).collect(),
                datasets: datasets.into_iter().map(|d| d.name().to_owned()).collect(),
            })
        })
    }
    pub fn group_attributes(&self, path: &str) -> Result<Vec<Attribute>> {
        self.file
            .group(path)
            .map_err(map_error)?
            .attributes()
            .map_err(map_error)?
            .into_iter()
            .map(attribute)
            .collect()
    }
    pub fn dataset_attributes(&self, path: &str) -> Result<Vec<Attribute>> {
        self.file
            .dataset(path)
            .map_err(map_error)?
            .attributes()
            .into_iter()
            .map(attribute)
            .collect()
    }
    /// Decoded strings, row-major, as h5py returns them. A NULL dataspace is
    /// Unsupported(schema) rather than an empty list (`raw_fixed_strings` reports it with an
    /// explicit `dataspace`). SPACEPAD elements drop trailing spaces and then trailing NULs
    /// (unwritten fill), as HDF5's NULLPAD conversion plus NumPy `S` trimming do.
    ///
    /// The stored datatype envelope is proven first with the same rule as typed
    /// metadata: variable-length text (whose global-heap references the generic reader
    /// would resolve without index/length validation), reserved padding or charset
    /// flags, and non-text classes are refused as `Unsupported(Encoding)` instead of
    /// being decoded as an empty, aliased or re-padded string.
    pub fn read_strings(&self, path: &str) -> Result<Vec<String>> {
        let ds = self.file.dataset(path).map_err(map_error)?;
        // Non-text datasets keep the generic reader's own type-mismatch error.
        if matches!(
            ds.dtype(),
            hdf5_reader::Datatype::String { .. }
                | hdf5_reader::Datatype::VarLen {
                    kind: hdf5_reader::VarLenKind::String,
                    ..
                }
        ) {
            let proven = metadata_proof::proof(&ds.raw_datatype().map_err(map_error)?)?;
            if let metadata_proof::Kind::Unsupported(reason) = &proven.kind {
                return Err(Error::unsupported(
                    UnsupportedCode::Encoding,
                    format!("{path}: unproven text encoding ({reason})"),
                ));
            }
            if !matches!(proven.kind, metadata_proof::Kind::Text { .. }) {
                return Err(Error::unsupported(
                    UnsupportedCode::Encoding,
                    format!("{path}: dataset is not proven fixed-length text"),
                ));
            }
        }
        if matches!(ds.dtype(), hdf5_reader::Datatype::String { .. })
            && Dataspace::of_dataset(&ds)?.is_null()
        {
            return Err(Error::unsupported(UnsupportedCode::Schema, NULL_DATASPACE));
        }
        let space_padded = matches!(
            ds.dtype(),
            hdf5_reader::Datatype::String {
                padding: hdf5_reader::StringPadding::SpacePad,
                ..
            }
        );
        let values = ds.read_strings().map_err(map_error)?;
        Ok(if space_padded {
            // The reader already dropped trailing spaces; drop the NUL fill before them.
            values
                .into_iter()
                .map(|v| v.trim_end_matches(' ').to_owned())
                .collect()
        } else {
            values
        })
    }
    #[allow(deprecated)] // Populate the pinned bridge compatibility field.
    pub fn numeric_capability(&self, path: &str) -> Result<NumericCapability> {
        self.memo(format!("numeric_capability:{:?}", (path,)), || {
            let ds = self.file.dataset(path).map_err(map_error)?;
            let (parsed_encoding, verdict) = match encoding::encoding(&self.file, &ds) {
                Ok(enc) => {
                    let verdict = encoding::native(ds.dtype(), &enc)
                        .map_err(|e| Error::unsupported(UnsupportedCode::Encoding, e));
                    (Some(enc), verdict)
                }
                Err(e) => (None, Err(e)),
            };
            let (kind, unsupported_reason) = match verdict {
                Ok(k) => (Some(k), None),
                Err(e) if e.kind == ErrorKind::Unsupported => (None, Some(e.message)),
                Err(e) => return Err(e),
            };
            let (byte_order, signed) = match ds.dtype() {
                hdf5_reader::Datatype::FixedPoint {
                    byte_order, signed, ..
                } => (Some(byte_order), Some(*signed)),
                hdf5_reader::Datatype::FloatingPoint { byte_order, .. }
                | hdf5_reader::Datatype::Bitfield { byte_order, .. } => (Some(byte_order), None),
                _ => (None, None),
            };
            let dataspace = Dataspace::of_dataset(&ds)?;
            let byte_order = byte_order.map(|b| match b {
                hdf5_reader::ByteOrder::LittleEndian => ByteOrder::Little,
                hdf5_reader::ByteOrder::BigEndian => ByteOrder::Big,
            });
            Ok(NumericCapability {
                max_shape: ds.max_dims().map(<[u64]>::to_vec),
                encoding: parsed_encoding,
                byte_order,
                signed,
                dataset: path.into(),
                shape: ds.shape().to_vec(),
                chunks: ds.chunks(),
                kind,
                capability: match &unsupported_reason {
                    Some(r) => Capability::unsupported(UnsupportedCode::Encoding, r.clone()),
                    None if dataspace.is_null() => {
                        Capability::unsupported(UnsupportedCode::Schema, NULL_DATASPACE)
                    }
                    None => Capability::Supported,
                },
                dataspace,
                unsupported_reason,
            })
        })
    }
    /// Read a numeric dataset or selection, refusing more than
    /// [`DEFAULT_READ_MAX_BYTES`] of values before allocating.
    pub fn read_numeric(&self, path: &str, selection: Option<&[Slice]>) -> Result<NumericArray> {
        self.read_numeric_bounded(path, selection, DEFAULT_READ_MAX_BYTES)
    }
    /// [`read_numeric`](Self::read_numeric) with an explicit budget: a
    /// selection whose values exceed `max_bytes` is refused with
    /// `Unsupported{ResourceLimit}` before anything is allocated.
    pub fn read_numeric_bounded(
        &self,
        path: &str,
        selection: Option<&[Slice]>,
        max_bytes: usize,
    ) -> Result<NumericArray> {
        let cached = self.numeric_dataset(path)?;
        let ds = &cached.dataset;
        // Only a proven numeric kind reaches the NULL refusal; other dtypes keep their errors.
        if self.numeric_capability(path)?.kind.is_some() && Dataspace::of_dataset(ds)?.is_null() {
            return Err(Error::unsupported(UnsupportedCode::Schema, NULL_DATASPACE));
        }
        let select = selection
            .map(|s| -> Result<SliceInfo> {
                if s.len() != ds.ndim() {
                    return Err(Error::new(
                        ErrorKind::InvalidRequest,
                        "selection must have one slice per dimension",
                    ));
                }
                let mut selections = Vec::with_capacity(s.len());
                for (i, s) in s.iter().enumerate() {
                    if s.step == 0 || s.start > s.end || s.end > ds.shape()[i] {
                        return Err(Error::new(
                            ErrorKind::InvalidRequest,
                            format!("invalid slice dimension {i}"),
                        ));
                    }
                    // A huge stride selects at most one element. Normalize it
                    // before the generic reader's internal strided arithmetic.
                    let step = s.step.min((s.end - s.start).max(1));
                    if s.end.checked_add(step).is_none() {
                        return Err(Error::new(
                            ErrorKind::InvalidRequest,
                            "slice arithmetic overflow",
                        ));
                    }
                    selections.push(SliceInfoElem::Slice {
                        start: s.start,
                        end: s.end,
                        step,
                    });
                }
                Ok(SliceInfo { selections })
            })
            .transpose()?;
        let kind = if let Some(kind) = cached.kind.get() {
            *kind
        } else {
            let enc = encoding::encoding(&self.file, ds)?;
            let kind = encoding::native(ds.dtype(), &enc)
                .map_err(|e| Error::unsupported(UnsupportedCode::Encoding, e))?;
            let _ = cached.kind.set(kind);
            kind
        };
        let elements = match selection {
            Some(s) => s.iter().try_fold(1u64, |n, s| {
                n.checked_mul((s.end - s.start).div_ceil(s.step))
            }),
            None => ds.shape().iter().try_fold(1u64, |n, &d| n.checked_mul(d)),
        };
        let read_bytes = elements.and_then(|n| {
            n.checked_mul(match kind {
                NumericKind::I8 | NumericKind::U8 => 1,
                NumericKind::I16 | NumericKind::U16 => 2,
                NumericKind::I32 | NumericKind::U32 | NumericKind::F32 => 4,
                _ => 8,
            })
        });
        if read_bytes.is_none_or(|n| n > max_bytes as u64) {
            return Err(Error::unsupported(
                UnsupportedCode::ResourceLimit,
                format!(
                    "{path}: selection of {} bytes exceeds the {max_bytes} byte read budget",
                    read_bytes.map_or_else(|| "more than u64::MAX".into(), |n| n.to_string())
                ),
            ));
        }
        let use_parallel =
            self.parallel && self.thread_count() > 1 && read_bytes.unwrap_or(u64::MAX) >= 64 * 1024;
        // Recognize full selections for cache bypass and the bounded hot-array path.
        let full = selection.is_none_or(|sel| {
            sel.iter()
                .zip(ds.shape())
                .all(|(s, &n)| s.start == 0 && s.end == n && s.step == 1)
        });
        let bypass = read_bytes.is_some_and(|n| n > self.chunk_cache_bytes as u64) && full;
        // Equivalent full slices can use the reader's existing bounded hot-array path.
        let select = if full { None } else { select };
        macro_rules! read_as {
            ($t:ty,$v:ident) => {{
                let array = if bypass {
                    if use_parallel {
                        self.pool
                            .install(|| ds.read_array_uncached_parallel::<$t>())
                    } else {
                        ds.read_array_uncached::<$t>()
                    }
                } else {
                    match (&select, use_parallel) {
                        (Some(s), true) => self.pool.install(|| ds.read_slice_parallel::<$t>(s)),
                        (Some(s), false) => ds.read_slice::<$t>(s),
                        (None, true) => self.pool.install(|| ds.read_array_parallel::<$t>()),
                        (None, false) => ds.read_array::<$t>(),
                    }
                }
                .map_err(map_error)?;
                let shape = array.shape().iter().map(|&v| v as u64).collect();
                let values = if array.is_standard_layout() {
                    array.into_raw_vec_and_offset().0
                } else {
                    array.iter().copied().collect()
                };
                Ok(NumericArray {
                    shape,
                    values: NumericValues::$v(values),
                })
            }};
        }
        match kind {
            NumericKind::I8 => read_as!(i8, I8),
            NumericKind::I16 => read_as!(i16, I16),
            NumericKind::I32 => read_as!(i32, I32),
            NumericKind::I64 => read_as!(i64, I64),
            NumericKind::U8 => read_as!(u8, U8),
            NumericKind::U16 => read_as!(u16, U16),
            NumericKind::U32 => read_as!(u32, U32),
            NumericKind::U64 => read_as!(u64, U64),
            NumericKind::F32 => read_as!(f32, F32),
            NumericKind::F64 => read_as!(f64, F64),
        }
    }
    pub(crate) fn gather_numeric(
        &self,
        path: &str,
        kind: NumericKind,
        rows: Slice,
        columns: &[u64],
    ) -> Result<Option<NumericArray>> {
        let cached = self.numeric_dataset(path)?;
        let ds = &cached.dataset;
        macro_rules! gather {
            ($t:ty,$v:ident) => {{
                let result = if self.parallel && self.thread_count() > 1 {
                    ds.read_rows_columns_chunked_in_pool::<$t>(
                        (rows.start, rows.end, rows.step),
                        columns,
                        &self.pool,
                    )
                } else {
                    ds.read_rows_columns_chunked::<$t>((rows.start, rows.end, rows.step), columns)
                };
                let Some(a) = result.map_err(map_error)? else {
                    return Ok(None);
                };
                let shape = a.shape().iter().map(|&n| n as u64).collect();
                Ok(Some(NumericArray {
                    shape,
                    values: NumericValues::$v(a.into_raw_vec_and_offset().0),
                }))
            }};
        }
        match kind {
            NumericKind::I8 => gather!(i8, I8),
            NumericKind::I16 => gather!(i16, I16),
            NumericKind::I32 => gather!(i32, I32),
            NumericKind::I64 => gather!(i64, I64),
            NumericKind::U8 => gather!(u8, U8),
            NumericKind::U16 => gather!(u16, U16),
            NumericKind::U32 => gather!(u32, U32),
            NumericKind::U64 => gather!(u64, U64),
            NumericKind::F32 => gather!(f32, F32),
            NumericKind::F64 => gather!(f64, F64),
        }
    }
    /// Ordered output preserves duplicate requests. No hidden reopen or Python fallback.
    /// The whole batch shares [`DEFAULT_READ_MAX_BYTES`].
    pub fn read_numeric_batch(&self, paths: &[&str]) -> Result<Vec<(String, NumericArray)>> {
        self.read_numeric_batch_bounded(paths, DEFAULT_READ_MAX_BYTES)
    }
    /// [`read_numeric_batch`](Self::read_numeric_batch) with an explicit budget
    /// shared by the whole batch. Every dataset's size is checked before any
    /// is read, so an over-budget batch allocates nothing.
    pub fn read_numeric_batch_bounded(
        &self,
        paths: &[&str],
        max_bytes: usize,
    ) -> Result<Vec<(String, NumericArray)>> {
        let mut total = 0u64;
        for p in paths {
            total = total
                .checked_add(self.numeric_byte_len(p)?)
                .filter(|&n| n <= max_bytes as u64)
                .ok_or_else(|| {
                    Error::unsupported(
                        UnsupportedCode::ResourceLimit,
                        format!("batch exceeds the {max_bytes} byte read budget at {p}"),
                    )
                })?;
        }
        paths
            .iter()
            .map(|p| {
                Ok((
                    p.to_string(),
                    self.read_numeric_bounded(p, None, max_bytes)?,
                ))
            })
            .collect()
    }
    /// Decoded byte length of a whole numeric dataset, from metadata only.
    /// Overflow saturates, which every budget refuses.
    pub(crate) fn numeric_byte_len(&self, path: &str) -> Result<u64> {
        let capability = self.numeric_capability(path)?;
        let Some(kind) = capability.kind else {
            // Unsupported encodings keep their own error from the read itself.
            return Ok(0);
        };
        let width: u64 = match kind {
            NumericKind::I8 | NumericKind::U8 => 1,
            NumericKind::I16 | NumericKind::U16 => 2,
            NumericKind::I32 | NumericKind::U32 | NumericKind::F32 => 4,
            _ => 8,
        };
        Ok(capability
            .shape
            .iter()
            .try_fold(width, |n, &d| n.checked_mul(d))
            .unwrap_or(u64::MAX))
    }
}
fn attribute(a: hdf5_reader::Attribute) -> Result<Attribute> {
    let strings = match &a.datatype {
        hdf5_reader::Datatype::String { .. } => Some(a.read_strings().map_err(map_error)?),
        hdf5_reader::Datatype::VarLen { .. } => a.decoded_strings.clone(),
        _ => None,
    };
    Ok(Attribute {
        name: a.name,
        shape: a.shape,
        attribute_type: AttributeType::unproven(
            "legacy attribute API does not retain the raw datatype; use typed_group_attributes"
                .into(),
        ),
        datatype_label: format!("{:?}", a.datatype),
        raw_bytes: a.raw_data,
        strings,
    })
}
pub mod metadata;

pub mod structures;
pub mod time;
pub mod units;
pub mod xs1d;

use metadata::MetadataValue;
#[path = "metadata_encoding.rs"]
mod metadata_proof;

pub mod types;
pub use types::{
    AttributeType, Dataspace, Identity, ResultMode, StringCharset, StringPadding, TypeClass,
};
pub mod layers;
pub mod lines;
mod location;
pub mod mesh_geometry;
pub mod mesh_identity;
pub mod reference_points;

pub mod mesh_results;

pub mod catalogue_results;

mod memo;

#[cfg(test)]
mod perf_contract_tests;

pub use hdf5_reader::{DEFAULT_CHUNK_CACHE_BYTES, DEFAULT_CHUNK_CACHE_SLOTS};

pub mod environment;
pub mod families;
mod one_d_layers;
pub mod one_d_summary;
pub mod stored;
pub mod structures_1d;
mod terrain_modifications;
pub mod xs_geometry;
pub mod xs_results;
