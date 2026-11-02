//! Full numeric reads into caller-owned storage. Existing owned reads are unchanged.
use super::{Dataspace, NumericArray, NumericKind, NumericValues, ReadOnlyHdfSession};
use crate::{Error, ErrorKind, Result, UnsupportedCode};

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidRequest, message)
}

impl ReadOnlyHdfSession {
    /// Fill an existing numeric array with a full dataset, preserving its allocation.
    /// Shape and numeric kind must match exactly; no conversion or resizing is done.
    /// Preflight errors leave the destination untouched. After a decoding or I/O
    /// failure its entire contents must be treated as invalid, since writes may be partial.
    pub fn read_numeric_into(&self, path: &str, destination: &mut NumericArray) -> Result<()> {
        self.read_numeric_into_bounded(path, destination, super::DEFAULT_READ_MAX_BYTES)
    }

    /// [`Self::read_numeric_into`] with an output-byte limit. The caller's buffer
    /// is still subject to the session's conservative in-flight decode reservation.
    pub fn read_numeric_into_bounded(
        &self,
        path: &str,
        destination: &mut NumericArray,
        max_bytes: usize,
    ) -> Result<()> {
        macro_rules! fill {
            ($($variant:ident),*) => {
                match &mut destination.values {
                    $(NumericValues::$variant(values) => self.read_numeric_into_slice(
                        path, &destination.shape, NumericKind::$variant, values, max_bytes,
                    ),)*
                }
            };
        }
        fill!(I8, I16, I32, I64, U8, U16, U32, U64, F32, F64)
    }

    pub(crate) fn read_numeric_into_slice<T: hdf5_reader::H5Type>(
        &self,
        path: &str,
        shape: &[u64],
        destination_kind: NumericKind,
        destination: &mut [T],
        max_bytes: usize,
    ) -> Result<()> {
        let cached = self.numeric_dataset(path)?;
        let ds = &cached.dataset;
        let capability = self.numeric_capability(path)?;
        if capability.dataspace == Dataspace::Null {
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                super::NULL_DATASPACE,
            ));
        }
        let kind = if let Some(kind) = cached.kind.get() {
            *kind
        } else {
            let encoding = super::encoding::encoding(&self.file, ds)?;
            let kind = super::encoding::native(ds.dtype(), &encoding)
                .map_err(|e| Error::unsupported(UnsupportedCode::Encoding, e))?;
            let _ = cached.kind.set(kind);
            kind
        };
        if kind != destination_kind {
            return Err(invalid(
                "destination numeric dtype must match the dataset exactly",
            ));
        }
        if shape != ds.shape() {
            return Err(invalid(
                "destination shape must match the full dataset exactly",
            ));
        }
        let elements = ds.num_elements().map_err(super::map_error)?;
        if u64::try_from(destination.len()).ok() != Some(elements) {
            return Err(invalid(
                "destination buffer length disagrees with its shape",
            ));
        }
        let bytes = destination.len().checked_mul(std::mem::size_of::<T>());
        if bytes.is_none_or(|bytes| bytes > max_bytes) {
            return Err(Error::unsupported(
                UnsupportedCode::ResourceLimit,
                format!("{path}: destination exceeds the {max_bytes} byte read budget"),
            ));
        }
        ds.read_into(destination).map_err(super::map_error)
    }
}
