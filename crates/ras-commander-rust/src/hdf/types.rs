//! Losslessly proven metadata types; labels are display-only.
use super::{
    metadata_proof::{Kind, Proof},
    ByteOrder,
};
use crate::{Capability, UnsupportedCode};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TypeClass {
    Integer,
    Float,
    FixedString,
    Compound,
    Other(OtherTypeClass),
}
/// Unknown class code, constructed only through TypeClass::from_code.
/// Its private field prevents an Other value from aliasing a named class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OtherTypeClass(u8);
impl TypeClass {
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Integer,
            1 => Self::Float,
            3 => Self::FixedString,
            6 => Self::Compound,
            c => Self::Other(OtherTypeClass(c)),
        }
    }
    pub fn code(self) -> u8 {
        match self {
            Self::Integer => 0,
            Self::Float => 1,
            Self::FixedString => 3,
            Self::Compound => 6,
            Self::Other(c) => c.0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StringCharset {
    Ascii,
    Utf8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StringPadding {
    NullTerminate,
    NullPad,
    SpacePad,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttributeType {
    Int {
        signed: bool,
        bytes: usize,
        order: ByteOrder,
    },
    Float {
        bytes: usize,
        order: ByteOrder,
    },
    FixedString {
        charset: StringCharset,
        padding: StringPadding,
        width: usize,
    },
    /// HDF5 bitfield with full precision and zero offset; values are exact bytes
    /// (`MetadataValue::Bitfield`), which h5py exposes as `uint<8*size>` arrays.
    Bitfield {
        size: usize,
        order: ByteOrder,
    },
    Compound,
    /// Present but not provable; the status is derived by [`Self::capability`].
    Unsupported {
        code: UnsupportedCode,
        reason: String,
    },
}
impl AttributeType {
    /// The single status of this type: Supported for every proven shape.
    pub fn capability(&self) -> Capability {
        match self {
            Self::Unsupported { code, reason } => Capability::unsupported(*code, reason.clone()),
            _ => Capability::Supported,
        }
    }
    pub(super) fn from_proof(p: &Proof) -> Self {
        let order = |big| {
            if big {
                ByteOrder::Big
            } else {
                ByteOrder::Little
            }
        };
        match &p.kind {
            Kind::Integer { signed, big } => Self::Int {
                signed: *signed,
                bytes: p.width,
                order: order(*big),
            },
            Kind::Float { big } => Self::Float {
                bytes: p.width,
                order: order(*big),
            },
            Kind::Bitfield { big } => Self::Bitfield {
                size: p.width,
                order: order(*big),
            },
            Kind::Text { padding, utf8 } => Self::FixedString {
                charset: if *utf8 {
                    StringCharset::Utf8
                } else {
                    StringCharset::Ascii
                },
                padding: match padding {
                    0 => StringPadding::NullTerminate,
                    1 => StringPadding::NullPad,
                    _ => StringPadding::SpacePad,
                },
                width: p.width,
            },
            Kind::Compound(_) => Self::Compound,
            Kind::Unsupported(reason) => Self::unproven(reason.clone()),
        }
    }
    pub(super) fn unproven(reason: String) -> Self {
        Self::Unsupported {
            code: UnsupportedCode::Encoding,
            reason,
        }
    }
}

/// HDF5 dataspace class, explicit so a NULL dataspace (h5py `Empty(dtype)`) is never
/// confused with a scalar (shape `[]`) or a zero-length simple array (`array([], dtype)`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Dataspace {
    /// One element, shape `[]`.
    Scalar,
    /// A simple array with these dimensions (any may be zero).
    Simple(Vec<u64>),
    /// H5S_NULL: no elements and no shape.
    Null,
}
impl Dataspace {
    /// Stable spelling: `scalar`, `simple` or `null`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::Simple(_) => "simple",
            Self::Null => "null",
        }
    }
    /// Stored element count (Scalar 1, Null 0); `None` on overflow.
    pub fn element_count(&self) -> Option<u64> {
        match self {
            Self::Scalar => Some(1),
            Self::Null => Some(0),
            Self::Simple(d) => d.iter().try_fold(1u64, |n, x| n.checked_mul(*x)),
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    pub(crate) fn from_message(m: &hdf5_reader::messages::dataspace::DataspaceMessage) -> Self {
        use hdf5_reader::messages::dataspace::DataspaceType as T;
        match m.dataspace_type {
            T::Null => Self::Null,
            T::Scalar => Self::Scalar,
            T::Simple => Self::Simple(m.dims.clone()),
        }
    }
    /// A dataset's class without a vendor accessor: rank zero is Scalar (one element) or
    /// Null (no elements); any other rank is Simple.
    pub(crate) fn of_dataset(ds: &hdf5_reader::Dataset) -> crate::Result<Self> {
        if ds.ndim() > 0 {
            return Ok(Self::Simple(ds.shape().to_vec()));
        }
        Ok(
            match ds.num_elements().map_err(super::encoding::map_error)? {
                0 => Self::Null,
                _ => Self::Scalar,
            },
        )
    }
}

/// Steady or unsteady result storage. One shared type for output blocks and 1D selections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ResultMode {
    Steady,
    Unsteady,
}
impl ResultMode {
    /// Stable snake_case spelling (never the Debug form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steady => "steady",
            Self::Unsteady => "unsteady",
        }
    }
}

/// Where a described object lives: the file, the attribute table it is a row of, and the row.
/// One shape for cross sections, structure geometry identities and mesh descriptors; the
/// object's own names (river/reach/station, connection, mesh name) stay domain-specific fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Identity {
    pub source: std::path::PathBuf,
    /// HDF path of the attribute table the row belongs to.
    pub table: String,
    /// Zero-based row of that table.
    pub row: u64,
}
impl Identity {
    pub fn new(source: impl Into<std::path::PathBuf>, table: impl Into<String>, row: u64) -> Self {
        Self {
            source: source.into(),
            table: table.into(),
            row,
        }
    }
}
