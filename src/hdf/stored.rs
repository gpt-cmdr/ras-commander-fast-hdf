//! Bounded owned envelopes used by domain inventories; no datatype coercion.
use super::{
    encoding::map_error,
    metadata::{typed_attribute, TypedAttribute},
    Identity, NumericArray, NumericKind, NumericValues, ReadOnlyHdfSession, Slice,
};
use crate::{Capability, Error, ErrorKind, Result, UnsupportedCode};

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StoredDataset {
    pub id: Identity,
    pub path: String,
    /// Stored dims (`[]` for both Scalar and Null; see `dataspace`).
    pub shape: Vec<u64>,
    pub dataspace: super::Dataspace,
    pub kind: Option<NumericKind>,
    pub storage: String,
    pub attributes: Vec<TypedAttribute>,
    pub capability: Capability,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum StoredValues {
    Numeric(NumericArray),
    Strings {
        shape: Vec<u64>,
        values: Vec<String>,
    },
    Compound(super::mesh_geometry::ExactCompound),
    /// Exact byte-wide HDF bitfield payload, not a numeric coercion. Whole-array only.
    Bitfield {
        shape: Vec<u64>,
        raw_bytes: Vec<u8>,
    },
}
pub(super) fn budget(n: usize, max: usize) -> Result<()> {
    if n > max {
        Err(Error::new(
            ErrorKind::InvalidRequest,
            "stored read byte budget exceeded",
        )
        .with_code(UnsupportedCode::ResourceLimit))
    } else {
        Ok(())
    }
}
pub(super) fn attributes(
    s: &ReadOnlyHdfSession,
    path: &str,
    group: bool,
    max: usize,
) -> Result<Vec<TypedAttribute>> {
    let raw = if group {
        s.file
            .group(path)
            .map_err(map_error)?
            .raw_attributes()
            .map_err(map_error)?
    } else {
        s.file
            .dataset(path)
            .map_err(map_error)?
            .raw_attributes()
            .map_err(map_error)?
    };
    raw.iter()
        .map(|a| {
            budget(a.message().raw_data.len(), max)?;
            typed_attribute(a)
        })
        .collect()
}
/// Header-only failure record: never retries payload/attribute qualification after a refusal.
pub(super) fn refused_descriptor(
    s: &ReadOnlyHdfSession,
    path: &str,
    capability: Capability,
) -> Result<StoredDataset> {
    let ds = s.file.dataset(path).map_err(map_error)?;
    let storage = match ds.dtype() {
        hdf5_reader::Datatype::FixedPoint { .. } | hdf5_reader::Datatype::FloatingPoint { .. } => {
            "numeric"
        }
        hdf5_reader::Datatype::String { .. } => "strings",
        hdf5_reader::Datatype::Compound { .. } => "compound",
        hdf5_reader::Datatype::Bitfield { size: 1, .. } => "bitfield_bytes",
        _ => "unqualified",
    };
    Ok(StoredDataset {
        id: Identity::new(s.path(), path, 0),
        path: path.into(),
        shape: ds.shape().to_vec(),
        dataspace: super::Dataspace::of_dataset(&ds)?,
        kind: None,
        storage: storage.into(),
        attributes: Vec::new(),
        capability,
    })
}
/// Inventory-only isolation. Requested reads retain their strict error contract.
pub(super) fn inventory_descriptor(
    s: &ReadOnlyHdfSession,
    path: &str,
    max: usize,
) -> Result<StoredDataset> {
    match descriptor(s, path, max) {
        Ok(d) => Ok(d),
        Err(e) => refused_descriptor(s, path, crate::capability::error_to_capability(e)?),
    }
}
pub(super) fn descriptor(s: &ReadOnlyHdfSession, path: &str, max: usize) -> Result<StoredDataset> {
    let ds = s.file.dataset(path).map_err(map_error)?;
    let attributes = attributes(s, path, false, max)?;
    let numeric = s.numeric_capability(path)?;
    let (storage, kind, capability) = if numeric.capability.is_supported() {
        ("numeric", numeric.kind, Capability::Supported)
    } else {
        match ds.dtype() {
            hdf5_reader::Datatype::Bitfield { size: 1, .. } => {
                ("bitfield_bytes", None, Capability::Supported)
            }
            hdf5_reader::Datatype::String { .. } => {
                budget(ds.raw_byte_len().map_err(map_error)?, max)?;
                let cap = match s.read_strings(path) {
                    Ok(_) => Capability::Supported,
                    Err(e) => crate::capability::error_to_capability(e)?,
                };
                ("strings", None, cap)
            }
            hdf5_reader::Datatype::Compound { .. } => {
                let cap = match s.simple_compound(path, max) {
                    Ok(t) => t
                        .columns
                        .iter()
                        .map(|c| c.value.capability())
                        .find(|c| !c.is_supported())
                        .unwrap_or(Capability::Supported),
                    Err(e) => crate::capability::error_to_capability(e)?,
                };
                ("compound", None, cap)
            }
            _ => ("unqualified", None, numeric.capability),
        }
    };
    Ok(StoredDataset {
        id: Identity::new(s.path(), path, 0),
        path: path.into(),
        shape: ds.shape().to_vec(),
        dataspace: super::Dataspace::of_dataset(&ds)?,
        kind,
        storage: storage.into(),
        attributes,
        capability,
    })
}
pub(super) fn read(
    s: &ReadOnlyHdfSession,
    d: &StoredDataset,
    slices: Option<&[Slice]>,
    max: usize,
) -> Result<StoredValues> {
    if !d.capability.is_supported() {
        let reason = d.capability.reason().unwrap_or("unqualified stored layout");
        return Err(match d.capability {
            Capability::Corrupt { .. } => Error::new(ErrorKind::Corrupt, reason),
            _ => Error::unsupported(
                d.capability.code().unwrap_or(UnsupportedCode::Schema),
                reason,
            ),
        });
    }
    match d.storage.as_str() {
        "numeric" => {
            let selection: Vec<_> = slices.map_or_else(
                || d.shape.iter().map(|&n| Slice::new(0, n, 1)).collect(),
                |v| v.to_vec(),
            );
            if selection.len() != d.shape.len() {
                return Err(Error::new(
                    ErrorKind::InvalidRequest,
                    "selection rank differs",
                ));
            }
            let mut count = 1usize;
            let mut shape = Vec::new();
            for (slice, &n) in selection.iter().zip(&d.shape) {
                if slice.step == 0 || slice.start > slice.end || slice.end > n {
                    return Err(Error::new(
                        ErrorKind::InvalidRequest,
                        "invalid stored selection",
                    ));
                }
                let len = (slice.end - slice.start).div_ceil(slice.step);
                shape.push(len);
                count = usize::try_from(len)
                    .ok()
                    .and_then(|len| count.checked_mul(len))
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidRequest, "selection size overflow")
                            .with_code(UnsupportedCode::ResourceLimit)
                    })?;
            }
            let kind = d.kind.ok_or_else(|| {
                Error::unsupported(UnsupportedCode::Encoding, "unproven numeric kind")
            })?;
            let width = match kind {
                NumericKind::I8 | NumericKind::U8 => 1,
                NumericKind::I16 | NumericKind::U16 => 2,
                NumericKind::I32 | NumericKind::U32 | NumericKind::F32 => 4,
                _ => 8,
            };
            budget(
                count.checked_mul(width).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidRequest, "selection bytes overflow")
                        .with_code(UnsupportedCode::ResourceLimit)
                })?,
                max,
            )?;
            let a = if count == 0 {
                NumericArray {
                    shape,
                    values: match kind {
                        NumericKind::I8 => NumericValues::I8(Vec::new()),
                        NumericKind::I16 => NumericValues::I16(Vec::new()),
                        NumericKind::I32 => NumericValues::I32(Vec::new()),
                        NumericKind::I64 => NumericValues::I64(Vec::new()),
                        NumericKind::U8 => NumericValues::U8(Vec::new()),
                        NumericKind::U16 => NumericValues::U16(Vec::new()),
                        NumericKind::U32 => NumericValues::U32(Vec::new()),
                        NumericKind::U64 => NumericValues::U64(Vec::new()),
                        NumericKind::F32 => NumericValues::F32(Vec::new()),
                        NumericKind::F64 => NumericValues::F64(Vec::new()),
                    },
                }
            } else {
                s.read_numeric(&d.path, Some(&selection))?
            };
            Ok(StoredValues::Numeric(a))
        }
        _ if slices.is_some() => Err(Error::new(
            ErrorKind::InvalidRequest,
            "slices require numeric storage",
        )),
        "bitfield_bytes" => {
            let ds = s.file.dataset(&d.path).map_err(map_error)?;
            budget(ds.raw_byte_len().map_err(map_error)?, max)?;
            Ok(StoredValues::Bitfield {
                shape: d.shape.clone(),
                raw_bytes: ds.read_raw_bytes().map_err(map_error)?,
            })
        }
        "strings" => {
            let ds = s.file.dataset(&d.path).map_err(map_error)?;
            budget(ds.raw_byte_len().map_err(map_error)?, max)?;
            Ok(StoredValues::Strings {
                shape: d.shape.clone(),
                // NumPy fixed-S scalars omit trailing NULs, including unwritten
                // fill values in a source declared SPACEPAD (a real 5.0.7 Computation Block).
                values: s
                    .read_strings(&d.path)?
                    .into_iter()
                    .map(|v| v.trim_end_matches('\0').to_owned())
                    .collect(),
            })
        }
        "compound" => Ok(StoredValues::Compound(
            super::mesh_geometry::exact_compound(s, &d.path, max)?,
        )),
        _ => Err(Error::unsupported(
            UnsupportedCode::Encoding,
            "unqualified stored datatype",
        )),
    }
}
