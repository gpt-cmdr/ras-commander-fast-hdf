//! Bounded metadata decoding with raw per-attribute and member encoding proof.
use super::metadata_proof as proof;
use super::{encoding::map_error, ReadOnlyHdfSession};
use super::{AttributeType, Dataspace, StringCharset, StringPadding};
use crate::{Capability, UnsupportedCode};
use crate::{Error, ErrorKind, Result};
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MetadataValue {
    Signed(Vec<i64>),
    Unsigned(Vec<u64>),
    Float(Vec<f64>),
    Strings(Vec<String>),
    /// Exact stored HDF5 bitfield bytes, row-major; each element is
    /// `AttributeType::Bitfield { size }` bytes in its declared order. No numeric coercion.
    Bitfield(Vec<u8>),
    /// Present but not provable as a typed value; the code says why.
    Unsupported {
        code: UnsupportedCode,
        reason: String,
    },
    /// Present but damaged.
    Corrupt {
        reason: String,
    },
}
impl MetadataValue {
    pub fn unsupported(code: UnsupportedCode, reason: impl Into<String>) -> Self {
        Self::Unsupported {
            code,
            reason: reason.into(),
        }
    }
    pub fn corrupt(reason: impl Into<String>) -> Self {
        Self::Corrupt {
            reason: reason.into(),
        }
    }
    /// The single status of this value, derived from the variant (never stored beside it).
    pub fn capability(&self) -> Capability {
        match self {
            Self::Unsupported { code, reason } => Capability::unsupported(*code, reason.clone()),
            Self::Corrupt { reason } => Capability::corrupt(reason.clone()),
            _ => Capability::Supported,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TypedAttribute {
    pub name: String,
    /// Scalar `[]`, simple dims, and `[0]` for a NULL dataspace (kept for compatibility).
    /// Use `dataspace` to tell NULL (h5py `Empty`) from a zero-length simple array.
    pub shape: Vec<u64>,
    pub dataspace: Dataspace,
    pub attribute_type: AttributeType,
    pub datatype_label: String,
    /// Exact attribute payload, including bytes that cannot be decoded as text.
    pub raw_bytes: Vec<u8>,
    pub value: MetadataValue,
}
/// A fixed-length string dataset exactly as stored: no trimming, decoding or padding removal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RawFixedStrings {
    /// Stored dims (`[]` for both Scalar and Null; see `dataspace`).
    pub shape: Vec<u64>,
    /// Null (h5py `Empty`) has no elements and empty `raw_bytes`.
    pub dataspace: Dataspace,
    /// Declared bytes per element.
    pub width: usize,
    pub padding: StringPadding,
    pub charset: StringCharset,
    /// Row-major stored bytes, `width` per element, padding bytes retained (h5py `S<width>`).
    pub raw_bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CompoundColumn {
    pub name: String,
    pub byte_offset: u32,
    pub attribute_type: AttributeType,
    pub datatype_label: String,
    pub value: MetadataValue,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CompoundTable {
    /// Stored dims (`[]` for both Scalar and Null; see `dataspace`). Null has no rows.
    pub shape: Vec<u64>,
    pub dataspace: Dataspace,
    pub columns: Vec<CompoundColumn>,
}
/// The root `File Type` values HEC-RAS writes in its geometry (`.g##.hdf`) and plan
/// (`.p##.hdf`) HDF files, exactly as h5py decodes them (padding removed).
///
/// They identify the file as HEC-RAS's, not which kind it is: plans written before 6.4 can say
/// `HEC-RAS Geometry`, and 6.4+ geometry files say `HEC-RAS Results`. HEC terrain and land
/// cover files (`HEC Terrain`, `HEC Land Cover`) are not geometry or plan files.
pub const HEC_RAS_FILE_TYPES: &[&str] = &["HEC-RAS Geometry", "HEC-RAS Results"];

/// The stored root `File Type` of a file ([`ReadOnlyHdfSession::hec_ras_file_type`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoredFileType {
    /// No root `File Type` attribute.
    Absent,
    /// One of [`HEC_RAS_FILE_TYPES`], exactly.
    HecRas(String),
    /// A readable single string that is not a HEC-RAS geometry or plan file type (for example
    /// `HEC Terrain`), exactly as stored.
    Other(String),
}

impl StoredFileType {
    /// Whether the file says it is a HEC-RAS geometry or plan HDF file.
    pub fn is_hec_ras(&self) -> bool {
        matches!(self, Self::HecRas(_))
    }
}

impl ReadOnlyHdfSession {
    /// The stored root `File Type` attribute, classified against [`HEC_RAS_FILE_TYPES`].
    ///
    /// Callers use it to decide whether an HDF5 file is a HEC-RAS geometry or plan file before
    /// reading HEC-RAS layouts from it. Only the attribute is read. The value must be a single
    /// string (a scalar or a one-element array); any other stored shape or type, or a payload
    /// the engine cannot decode, is `Unsupported` or `Corrupt`, never a guess.
    pub fn hec_ras_file_type(&self) -> Result<StoredFileType> {
        let attributes = self.typed_group_attributes("/")?;
        let Some(attribute) = attributes.iter().find(|a| a.name == "File Type") else {
            return Ok(StoredFileType::Absent);
        };
        match &attribute.value {
            MetadataValue::Strings(v) if v.len() == 1 => {
                let text = v[0].clone();
                Ok(if HEC_RAS_FILE_TYPES.contains(&text.as_str()) {
                    StoredFileType::HecRas(text)
                } else {
                    StoredFileType::Other(text)
                })
            }
            MetadataValue::Corrupt { reason } => Err(Error::new(
                ErrorKind::Corrupt,
                format!("root File Type attribute: {reason}"),
            )),
            MetadataValue::Unsupported { code, reason } => Err(Error::unsupported(
                *code,
                format!("root File Type attribute: {reason}"),
            )),
            _ => Err(Error::unsupported(
                UnsupportedCode::Encoding,
                "root File Type attribute is not a single string",
            )),
        }
    }
    /// [`Self::hec_ras_file_type`] of an HDF5 file held in memory, without opening a session
    /// over it and without copying all of it. `name` names the file in error messages.
    ///
    /// The reader needs storage it owns, so the bytes it reads are copied: the first 64 KiB,
    /// then each 4 KiB page it asked for outside them, until it reads the attribute without
    /// asking for any other. A result is kept only when every byte read was copied, so it is
    /// the result of [`Self::from_bytes`] then `hec_ras_file_type`, error kinds and messages
    /// included. HEC-RAS files need the first 64 KiB and, when HEC-RAS moved the root
    /// attributes later in the file, a few pages more; never more than the whole file.
    pub fn hec_ras_file_type_of_bytes(
        data: &[u8],
        name: impl Into<std::path::PathBuf>,
    ) -> Result<StoredFileType> {
        Self::sniff_file_type(data, name.into()).0
    }
    /// [`Self::hec_ras_file_type_of_bytes`] and the bytes it copied for its last run.
    fn sniff_file_type(data: &[u8], name: std::path::PathBuf) -> (Result<StoredFileType>, u64) {
        super::sniff::read_sparse(data, |storage| {
            super::reader::File::from_storage(storage)
                .map_err(|e| map_error(e).with_path(name.clone()))
                .and_then(|file| Self::over_file(file, name.clone()))
                .and_then(|session| session.hec_ras_file_type())
        })
    }
    pub fn typed_group_attributes(&self, path: &str) -> Result<Vec<TypedAttribute>> {
        self.file
            .group(path)
            .map_err(map_error)?
            .raw_attributes()
            .map_err(map_error)?
            .iter()
            .map(typed_attribute)
            .collect()
    }
    /// Every attribute of one dataset, typed with the same encoding proof as family
    /// descriptors (exact `raw_bytes` retained; per-attribute unproven or damaged payloads stay
    /// in each attribute's `value` capability). For datasets outside every read family.
    ///
    /// - An absent dataset (or parent group) is `Unavailable`.
    /// - `max_bytes` bounds the summed stored attribute payloads (a variable-length value counts
    ///   its heap reference). It is checked incrementally and refused as
    ///   `Unsupported(resource_limit)` before any attribute is copied or typed. It does not bound
    ///   the object header parse: the reader materializes every attribute message (including
    ///   dense storage) when it opens the dataset, as for every dataset read.
    /// - A soft link to a dataset is followed. A dangling soft link, or an external link (never
    ///   followed), reads as absent (`Unavailable`).
    /// - An empty path, a `.`/`..` component or a path naming a group is `InvalidRequest`
    ///   (use `typed_group_attributes` for groups). Malformed attribute messages propagate.
    pub fn typed_dataset_attributes(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> Result<crate::Reported<Vec<TypedAttribute>>> {
        let Some(ds) = self.dataset_at(path)? else {
            return Ok(crate::Reported::unavailable("dataset absent"));
        };
        let raw = ds.raw_attributes().map_err(map_error)?;
        // Admit attributes one at a time and stop at the first that crosses the budget, before
        // any is copied or typed. (The reader has already parsed the object header's attribute
        // messages when the dataset was opened; see docs/reference/engine-api.md.)
        let mut used = 0usize;
        for a in raw.iter() {
            used = match used.checked_add(a.message().raw_data.len()) {
                Some(n) if n <= max_bytes => n,
                _ => {
                    return Ok(crate::Reported::unsupported(
                        UnsupportedCode::ResourceLimit,
                        "dataset attribute byte budget exceeded",
                    ))
                }
            };
        }
        raw.iter()
            .map(typed_attribute)
            .collect::<Result<Vec<_>>>()
            .map(crate::Reported::supported)
    }
    /// Exact stored bytes of a fixed-length string dataset with its declared width, padding
    /// and charset, so callers can reproduce h5py `S<width>` values (e.g. NULLPAD bytes)
    /// that the decoded `read_strings` / stored `Strings` envelopes trim. These are the
    /// *stored* bytes: h5py reads into NULLPAD memory, so HDF5 converts SPACEPAD elements
    /// (trailing spaces become NULs) and NULLTERM elements (bytes after the first NUL become
    /// NULs); NULLPAD elements are returned unchanged. Apply that rule with `padding` to
    /// reproduce h5py's `S<width>` bytes.
    ///
    /// - An absent dataset is `Unavailable`; a dataset that is not proven fixed-length text
    ///   (numeric, variable-length, reserved padding/charset) is `Unsupported(encoding)`.
    /// - `max_bytes` bounds the stored payload (`Unsupported(resource_limit)` when exceeded,
    ///   checked before reading). A payload that disagrees with shape x width is `Corrupt`.
    /// - Path validation matches `typed_dataset_attributes` (`InvalidRequest`).
    pub fn raw_fixed_strings(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> Result<crate::Reported<RawFixedStrings>> {
        let Some(ds) = self.dataset_at(path)? else {
            return Ok(crate::Reported::unavailable("dataset absent"));
        };
        let reported = |e: Error| -> Result<crate::Reported<RawFixedStrings>> {
            Ok(match crate::capability::error_to_capability(e)? {
                Capability::Corrupt { reason } => crate::Reported::corrupt(reason),
                c => crate::Reported::unsupported(
                    c.code().unwrap_or(UnsupportedCode::Encoding),
                    c.reason().unwrap_or("unproven fixed string"),
                ),
            })
        };
        let p = match ds
            .raw_datatype()
            .map_err(map_error)
            .and_then(|b| proof::proof(&b))
        {
            Ok(p) => p,
            Err(e) => return reported(e),
        };
        let AttributeType::FixedString {
            charset,
            padding,
            width,
        } = AttributeType::from_proof(&p)
        else {
            return Ok(crate::Reported::unsupported(
                UnsupportedCode::Encoding,
                "dataset is not proven fixed-length text",
            ));
        };
        let bytes = match ds.raw_byte_len().map_err(map_error) {
            Ok(n) => n,
            Err(e) => return reported(e),
        };
        if bytes > max_bytes {
            return Ok(crate::Reported::unsupported(
                UnsupportedCode::ResourceLimit,
                "fixed string byte budget exceeded",
            ));
        }
        let raw_bytes = match ds.read_raw_bytes().map_err(map_error) {
            Ok(raw) => raw,
            Err(e) => return reported(e),
        };
        let dataspace = match Dataspace::of_dataset(&ds) {
            Ok(d) => d,
            Err(e) => return reported(e),
        };
        let expected = dataspace
            .element_count()
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| n.checked_mul(width));
        if expected != Some(raw_bytes.len()) {
            return Ok(crate::Reported::corrupt(
                "fixed string shape/width/payload mismatch",
            ));
        }
        Ok(crate::Reported::supported(RawFixedStrings {
            shape: ds.shape().to_vec(),
            dataspace,
            width,
            padding,
            charset,
            raw_bytes,
        }))
    }
    /// One exact dataset for the path-addressed readers: `None` when absent; a group path or
    /// an empty/`.`/`..` path is `InvalidRequest`.
    fn dataset_at(&self, path: &str) -> Result<Option<crate::hdf::reader::Dataset>> {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() || parts.iter().any(|p| *p == "." || *p == "..") {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "expected an exact HDF dataset path",
            ));
        }
        match crate::capability::absent_ok(self.file.dataset(path).map_err(map_error))? {
            Some(ds) => Ok(Some(ds)),
            None if crate::capability::absent_ok(self.file.group(path).map_err(map_error))?
                .is_some() =>
            {
                Err(Error::new(
                    ErrorKind::InvalidRequest,
                    "path names a group, not a dataset",
                ))
            }
            None => Ok(None),
        }
    }
    /// Simple scalar compound members, bounded by raw bytes. Unsupported columns remain explicit.
    pub fn simple_compound(&self, path: &str, max_bytes: usize) -> Result<CompoundTable> {
        self.simple_compound_with_raw(path, max_bytes)
            .map(|(table, _)| table)
    }
    pub(crate) fn simple_compound_with_raw(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> Result<(CompoundTable, Vec<u8>)> {
        let ds = self.file.dataset(path).map_err(map_error)?;
        let p = proof::proof(&ds.raw_datatype().map_err(map_error)?)?;
        let fields = match &p.kind {
            proof::Kind::Compound(v) => v,
            _ => {
                return Err(Error::unsupported(
                    UnsupportedCode::Schema,
                    "dataset is not proven compound",
                ))
            }
        };
        let bytes = ds.raw_byte_len().map_err(map_error)?;
        if bytes > max_bytes {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "compound exceeds caller byte limit",
            )
            .with_code(UnsupportedCode::ResourceLimit));
        }
        let dataspace = Dataspace::of_dataset(&ds)?;
        let count = dataspace
            .element_count()
            .ok_or_else(|| Error::new(ErrorKind::Corrupt, "compound count overflow"))?;
        let raw = ds.read_raw_bytes().map_err(map_error)?;
        if usize::try_from(count)
            .ok()
            .and_then(|n| n.checked_mul(p.width))
            != Some(raw.len())
        {
            return Err(Error::new(
                ErrorKind::Corrupt,
                "compound shape/width/payload mismatch",
            ));
        }
        let mut columns = Vec::new();
        for (name, offset, member) in fields {
            let value = if let proof::Kind::Unsupported(reason) = &member.kind {
                MetadataValue::unsupported(UnsupportedCode::Encoding, reason.clone())
            } else {
                let data: Vec<u8> = raw
                    .chunks_exact(p.width)
                    .flat_map(|row| row[*offset..*offset + member.width].iter().copied())
                    .collect();
                proof::decode(member, &data, count)?
            };
            columns.push(CompoundColumn {
                name: name.clone(),
                byte_offset: u32::try_from(*offset).map_err(|_| {
                    Error::new(ErrorKind::Corrupt, "compound member offset exceeds u32")
                })?,
                attribute_type: AttributeType::from_proof(member),
                datatype_label: format!("{:?}", member.kind),
                value,
            });
        }
        Ok((
            CompoundTable {
                shape: ds.shape().to_vec(),
                dataspace,
                columns,
            },
            raw,
        ))
    }
}
pub(super) fn typed_attribute(raw: &hdf5_reader::RawAttribute) -> Result<TypedAttribute> {
    let a = raw.message();
    let count = a
        .dataspace
        .num_elements()
        .map_err(|e| map_error(e.into()))?;
    let failure = |e: Error| {
        if e.kind == ErrorKind::Unsupported {
            MetadataValue::unsupported(e.code.unwrap_or(UnsupportedCode::Encoding), e.message)
        } else {
            MetadataValue::corrupt(e.message)
        }
    };
    let (attribute_type, datatype_label, value) = match proof::proof(raw.raw_datatype()) {
        Ok(p) => {
            let label = format!("{:?}", p.kind);
            let value = match proof::decode(&p, &a.raw_data, count) {
                Ok(v) => v,
                Err(e) => failure(e),
            };
            (AttributeType::from_proof(&p), label, value)
        }
        Err(e) => (
            AttributeType::unproven(e.message.clone()),
            "unproven datatype envelope".into(),
            failure(e),
        ),
    };
    let dataspace = Dataspace::from_message(&a.dataspace);
    let shape = match a.dataspace.dataspace_type {
        hdf5_reader::messages::dataspace::DataspaceType::Scalar => vec![],
        hdf5_reader::messages::dataspace::DataspaceType::Null => vec![0],
        _ => a.dataspace.dims.clone(),
    };
    Ok(TypedAttribute {
        name: a.name.clone(),
        shape,
        dataspace,
        attribute_type,
        datatype_label,
        raw_bytes: a.raw_data.clone(),
        value,
    })
}

#[cfg(test)]
mod sniff_tests {
    use super::*;
    use std::path::Path;

    fn fixture(relative: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(relative),
        )
        .unwrap()
    }

    #[test]
    fn only_the_ranges_the_reader_reads_are_copied() {
        use super::super::sniff::FIRST_BYTES;
        // Root attributes in the root object header: the first 64 KiB hold them.
        let small = fixture("terrio_io/pipes.h5");
        let (r, copied) = ReadOnlyHdfSession::sniff_file_type(&small, "pipes.h5".into());
        assert!(r.unwrap().is_hec_ras());
        assert_eq!(copied, (small.len() as u64).min(FIRST_BYTES));
        // A user block: the superblock is found past the first copy, and the reader then
        // refuses the file (it does not apply a base address), exactly as it does whole.
        let data = fixture("file_type/userblock.h5");
        let (r, copied) = ReadOnlyHdfSession::sniff_file_type(&data, "userblock.h5".into());
        assert!(copied > FIRST_BYTES, "{copied}");
        assert_eq!(r.unwrap_err().kind, crate::ErrorKind::Unsupported);
        // Without the user block, the same bytes are read.
        let rest = &data[128 * 1024..];
        let (r, copied) = ReadOnlyHdfSession::sniff_file_type(rest, "userblock.h5".into());
        assert_eq!(r.unwrap(), StoredFileType::HecRas("HEC-RAS Results".into()));
        assert_eq!(copied, rest.len() as u64);
        // Dense root attributes after 80 KiB of data: a few pages past the first copy.
        for (name, expected) in [
            ("late_dense_attributes.h5", Some("HEC-RAS Geometry")),
            ("late_dense_absent.h5", None),
        ] {
            let data = fixture(&format!("file_type/{name}"));
            let (r, copied) = ReadOnlyHdfSession::sniff_file_type(&data, name.into());
            assert!(
                copied > FIRST_BYTES && copied < data.len() as u64,
                "{name}: {copied}"
            );
            match (r.unwrap(), expected) {
                (StoredFileType::HecRas(t), Some(e)) => assert_eq!(t, e),
                (StoredFileType::Absent, None) => {}
                (other, _) => panic!("{name}: {other:?}"),
            }
        }
    }

    /// Opt-in: the bytes each file of `RAS_SNIFF_CORPUS` (`;`-separated) needs, and the same
    /// result as a session over the whole file.
    #[test]
    #[ignore = "requires RAS_SNIFF_CORPUS"]
    fn sniff_copy_on_corpus_files() {
        for path in std::env::var("RAS_SNIFF_CORPUS").unwrap().split(';') {
            let data = std::fs::read(path).unwrap();
            let (r, copied) = ReadOnlyHdfSession::sniff_file_type(&data, path.into());
            let full = ReadOnlyHdfSession::from_bytes(data.clone(), path, 1, false)
                .and_then(|s| s.hec_ras_file_type());
            assert_eq!(format!("{r:?}"), format!("{full:?}"), "{path}");
            println!("{path}\t{}\t{copied}\t{r:?}", data.len());
        }
    }
}
