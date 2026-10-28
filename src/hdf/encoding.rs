use super::reader::{Dataset, Error, File};
use super::NumericKind as Native;
use super::TypeClass;
use hdf5_reader::storage::Storage;
use hdf5_reader::{ByteOrder, Datatype};
fn classify(e: &Error) -> crate::ErrorKind {
    match e {
        Error::Context { source, .. } => classify(source),
        Error::DatasetNotFound(_) | Error::GroupNotFound(_) | Error::AttributeNotFound(_) => {
            crate::ErrorKind::NotFound
        }
        Error::Io(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                crate::ErrorKind::FileNotFound
            } else {
                crate::ErrorKind::Io
            }
        }
        Error::UnsupportedEncoding(_) | Error::ResourceLimit(_) => crate::ErrorKind::Unsupported,
        Error::UnsupportedSuperblockVersion(_)
        | Error::UnsupportedObjectHeaderVersion(_)
        | Error::UnsupportedBTreeVersion(_)
        | Error::UnsupportedSymbolTableNodeVersion(_)
        | Error::UnsupportedLocalHeapVersion(_)
        | Error::UnsupportedGlobalHeapVersion(_)
        | Error::UnsupportedFractalHeapVersion(_)
        | Error::UnsupportedDataspaceVersion(_)
        | Error::UnsupportedDatatypeClass(_)
        | Error::UnsupportedLayoutClass(_)
        | Error::UnsupportedLayoutVersion(_)
        | Error::UnsupportedFilterPipelineVersion(_)
        | Error::UnsupportedFillValueVersion(_)
        | Error::UnsupportedLinkVersion(_)
        | Error::UnsupportedLinkType(_)
        | Error::UnsupportedAttributeVersion(_)
        | Error::UnsupportedBTreeV2RecordType(_)
        | Error::UnsupportedChunkIndexType(_)
        | Error::UnsupportedOffsetSize(_)
        | Error::UnsupportedLengthSize(_)
        | Error::UnsupportedFilter(_) => crate::ErrorKind::Unsupported,
        Error::TypeMismatch { .. } | Error::SliceOutOfBounds { .. } => {
            crate::ErrorKind::InvalidRequest
        }
        Error::ChecksumMismatch { .. }
        | Error::UnexpectedEof { .. }
        | Error::InvalidMagic
        | Error::InvalidBTreeSignature
        | Error::InvalidSymbolTableNodeSignature
        | Error::InvalidLocalHeapSignature
        | Error::InvalidGlobalHeapSignature
        | Error::InvalidFractalHeapSignature
        | Error::InvalidObjectHeaderSignature
        | Error::InvalidBTreeV2Signature { .. }
        | Error::InvalidFixedArraySignature { .. }
        | Error::InvalidExtensibleArraySignature { .. } => crate::ErrorKind::Corrupt,
        Error::Other(message) if known_reader_limitation(message) => crate::ErrorKind::Unsupported,
        Error::Other(message)
            if message == "external raw data file could not be resolved"
                || message
                    .strip_prefix("external raw data file '")
                    .and_then(|s| s.strip_suffix("' could not be resolved"))
                    .is_some() =>
        {
            crate::ErrorKind::FileNotFound
        }
        _ => crate::ErrorKind::Corrupt,
    }
}
// Temporary allow-list for untyped Other messages in the pinned reader. Match
// only audited message templates and their numeric u8 version field, never Display
// substrings. Replace with typed generic codes when upstream exposes them.
const READER_VERSION_PREFIXES: &[&str] = &[
    "unsupported fixed array header version ",
    "unsupported fixed array data block version ",
    "unsupported extensible array header version ",
    "unsupported extensible array data block version ",
    "unsupported extensible array secondary block version ",
    "unsupported extensible array index block version ",
];
fn known_reader_limitation(message: &str) -> bool {
    message == "SOHM table lookup not yet supported — file uses shared object header messages"
        || READER_VERSION_PREFIXES.iter().any(|prefix| {
            message
                .strip_prefix(prefix)
                .is_some_and(|v| v.parse::<u8>().is_ok_and(|n| n.to_string() == v))
        })
}
/// Raw datatype message fields. The pinned hdf5-reader parses but discards
/// bit offset/precision and the floating-point layout, so they are read here
/// directly from the object header instead of inferring them from `Datatype`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodingDescriptor {
    pub class: TypeClass,
    pub version: u8,
    pub flags: u32,
    pub size: u32,
    pub bit_offset: Option<u16>,
    pub bit_precision: Option<u16>,
    pub float: Option<FloatLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FloatLayout {
    pub sign_location: u8,
    pub normalization: u8,
    pub exponent_location: u8,
    pub exponent_size: u8,
    pub mantissa_location: u8,
    pub mantissa_size: u8,
    pub exponent_bias: u32,
}

const MSG_DATATYPE: u16 = 0x0003;
const MSG_CONTINUATION: u16 = 0x0010;

fn le(bytes: &[u8], at: usize, width: usize) -> crate::Result<u64> {
    let field = bytes
        .get(
            at..at
                .checked_add(width)
                .ok_or_else(|| corrupt("header offset overflow"))?,
        )
        .ok_or_else(|| corrupt("truncated object header"))?;
    Ok(field
        .iter()
        .rev()
        .fold(0u64, |value, &byte| (value << 8) | u64::from(byte)))
}

fn read_range(storage: &dyn Storage, address: u64, len: u64) -> crate::Result<Vec<u8>> {
    let len = usize::try_from(len).map_err(|_| {
        unsupported("object header chunk too large")
            .with_code(crate::UnsupportedCode::ResourceLimit)
    })?;
    if len > 1 << 24 {
        return Err(unsupported("object header chunk exceeds 16 MiB")
            .with_code(crate::UnsupportedCode::ResourceLimit));
    }
    address
        .checked_add(len as u64)
        .ok_or_else(|| corrupt("address range overflow"))?;
    let buffer = storage.read_range(address, len).map_err(map_error)?;
    if buffer.as_ref().len() != len {
        return Err(corrupt("short storage range read"));
    }
    Ok(buffer.as_ref().to_vec())
}

/// Message located in an object header: (type, flags, payload).
type Message = (u16, u8, Vec<u8>);

fn header_messages(
    storage: &dyn Storage,
    address: u64,
    offset_size: usize,
    length_size: usize,
) -> crate::Result<Vec<Message>> {
    let prefix = read_range(storage, address, 4)?;
    let mut messages = Vec::new();
    let v2 = prefix == b"OHDR";
    let mut creation_order = false;
    if v2 {
        let fixed = read_range(storage, address, 6)?;
        if fixed[4] != 2 {
            return Err(unsupported(format!("object header version {}", fixed[4])));
        }
        let flags = fixed[5];
        creation_order = flags & 0x04 != 0;
        let width = 1usize << (flags & 0x03);
        let mut start = 6 + if flags & 0x20 != 0 { 16 } else { 0 };
        start += if flags & 0x10 != 0 { 4 } else { 0 };
        let size = le(
            &read_range(storage, address, (start + width) as u64)?,
            start,
            width,
        )?;
        start += width;
        let chunk = read_range(
            storage,
            address,
            (start as u64)
                .checked_add(size)
                .ok_or_else(|| corrupt("object header size overflow"))?,
        )?;
        v2_messages(&chunk[start..], creation_order, &mut messages)?;
    } else {
        let fixed = read_range(storage, address, 16)?;
        if fixed[0] != 1 {
            return Err(unsupported(format!("object header version {}", fixed[0])));
        }
        let size = le(&fixed, 8, 4)?;
        let chunk = read_range(
            storage,
            address
                .checked_add(16)
                .ok_or_else(|| corrupt("object header address overflow"))?,
            size,
        )?;
        v1_messages(&chunk, &mut messages)?;
    }
    // Follow continuation messages breadth-first; each block is read once.
    let mut visited = std::collections::HashSet::new();
    let mut scanned = 0;
    while scanned < messages.len() {
        let (kind, _, payload) = &messages[scanned];
        scanned += 1;
        if *kind != MSG_CONTINUATION {
            continue;
        }
        let at = le(payload, 0, offset_size)?;
        let len = le(payload, offset_size, length_size)?;
        if !visited.insert(at) || visited.len() > 1024 {
            return Err(corrupt("cyclic or excessive object header continuations"));
        }
        let chunk = read_range(storage, at, len)?;
        if v2 {
            if chunk.len() < 8 || &chunk[..4] != b"OCHK" {
                return Err(corrupt("invalid continuation signature"));
            }
            v2_messages(&chunk[4..chunk.len() - 4], creation_order, &mut messages)?;
        } else {
            v1_messages(&chunk, &mut messages)?;
        }
    }
    Ok(messages)
}

fn v1_messages(chunk: &[u8], out: &mut Vec<Message>) -> crate::Result<()> {
    let mut at = 0;
    while at + 8 <= chunk.len() {
        let kind = le(chunk, at, 2)? as u16;
        let size = le(chunk, at + 2, 2)? as usize;
        let flags = chunk[at + 4];
        let payload = chunk
            .get(at + 8..at + 8 + size)
            .ok_or_else(|| corrupt("message extends past header chunk"))?;
        out.push((kind, flags, payload.to_vec()));
        at += 8 + size;
    }
    Ok(())
}

fn v2_messages(chunk: &[u8], creation_order: bool, out: &mut Vec<Message>) -> crate::Result<()> {
    let envelope = if creation_order { 6 } else { 4 };
    let mut at = 0;
    while at + envelope <= chunk.len() {
        let kind = u16::from(chunk[at]);
        let size = le(chunk, at + 1, 2)? as usize;
        let flags = chunk[at + 3];
        let payload = chunk
            .get(at + envelope..at + envelope + size)
            .ok_or_else(|| corrupt("message extends past header chunk"))?;
        out.push((kind, flags, payload.to_vec()));
        at += envelope + size;
    }
    Ok(())
}

fn datatype_payload(file: &File, address: u64, depth: u8) -> crate::Result<Vec<u8>> {
    let sb = file.superblock();
    if sb.base_address != 0 {
        return Err(unsupported("non-zero superblock base address"));
    }
    let (offset_size, length_size) = (sb.offset_size as usize, sb.length_size as usize);
    let messages = header_messages(file.storage(), address, offset_size, length_size)?;
    let mut found = messages
        .into_iter()
        .filter(|(kind, _, _)| *kind == MSG_DATATYPE);
    let (_, flags, payload) = found
        .next()
        .ok_or_else(|| corrupt("object header has no datatype message"))?;
    if found.next().is_some() {
        return Err(corrupt("object header has several datatype messages"));
    }
    if flags & 0x02 == 0 {
        return Ok(payload);
    }
    // Shared (committed) datatype: follow its object-header address once.
    if depth > 0 {
        return Err(corrupt("nested shared datatype"));
    }
    let target = match (payload.first(), payload.get(1)) {
        (Some(1), _) => le(&payload, 8, offset_size)?,
        (Some(2), _) => le(&payload, 2, offset_size)?,
        (Some(3), Some(2)) => le(&payload, 2, offset_size)?,
        (Some(3), Some(1)) => return Err(unsupported("datatype stored in shared-message heap")),
        _ => return Err(corrupt("unknown shared datatype message")),
    };
    datatype_payload(file, target, depth + 1)
}

pub(crate) fn encoding(file: &File, ds: &Dataset) -> crate::Result<EncodingDescriptor> {
    let payload = datatype_payload(file, ds.address(), 0)?;
    let head = le(&payload, 0, 4)? as u32;
    let class = (head & 0x0f) as u8;
    let mut enc = EncodingDescriptor {
        class: TypeClass::from_code(class),
        version: ((head >> 4) & 0x0f) as u8,
        flags: head >> 8,
        size: le(&payload, 4, 4)? as u32,
        bit_offset: None,
        bit_precision: None,
        float: None,
    };
    if class == 0 || class == 1 {
        enc.bit_offset = Some(le(&payload, 8, 2)? as u16);
        enc.bit_precision = Some(le(&payload, 10, 2)? as u16);
    }
    if class == 1 {
        enc.float = Some(FloatLayout {
            sign_location: ((enc.flags >> 8) & 255) as u8,
            normalization: ((enc.flags >> 4) & 3) as u8,
            exponent_location: le(&payload, 12, 1)? as u8,
            exponent_size: le(&payload, 13, 1)? as u8,
            mantissa_location: le(&payload, 14, 1)? as u8,
            mantissa_size: le(&payload, 15, 1)? as u8,
            exponent_bias: le(&payload, 16, 4)? as u32,
        });
    }
    Ok(enc)
}

/// Accept only encodings whose stored bytes equal the native element bytes.
pub(super) fn native(dtype: &Datatype, enc: &EncodingDescriptor) -> Result<Native, String> {
    let full =
        |bits: u32| enc.bit_offset == Some(0) && enc.bit_precision.map(u32::from) == Some(bits);
    match dtype {
        Datatype::FixedPoint {
            size,
            signed,
            byte_order,
        } => {
            if enc.class != TypeClass::Integer || enc.size != u32::from(*size) {
                return Err("datatype message disagrees with parsed integer type".into());
            }
            if *size > 1 && !native_order(byte_order) {
                return Err("non-native byte order".into());
            }
            if !full(u32::from(*size) * 8) {
                return Err(format!(
                    "integer bit offset {:?} / precision {:?} differs from {} storage bits",
                    enc.bit_offset,
                    enc.bit_precision,
                    u32::from(*size) * 8
                ));
            }
            // Bits 1-2 are low/high padding; with full precision there is none.
            Ok(match (*size, *signed) {
                (1, true) => Native::I8,
                (2, true) => Native::I16,
                (4, true) => Native::I32,
                (8, true) => Native::I64,
                (1, false) => Native::U8,
                (2, false) => Native::U16,
                (4, false) => Native::U32,
                (8, false) => Native::U64,
                _ => return Err("unsupported integer width".into()),
            })
        }
        Datatype::FloatingPoint { size, byte_order } => {
            if enc.class != TypeClass::Float || enc.size != u32::from(*size) {
                return Err("datatype message disagrees with parsed float type".into());
            }
            if enc.flags & 0x40 != 0 {
                return Err("VAX floating-point byte order".into());
            }
            if !native_order(byte_order) {
                return Err("non-native byte order".into());
            }
            let (bits, ieee, native) = match *size {
                4 => (32, (31, 23, 8, 0, 23, 127), Native::F32),
                8 => (64, (63, 52, 11, 0, 52, 1023), Native::F64),
                _ => return Err("unsupported float width".into()),
            };
            let layout = enc.float.ok_or("missing float layout")?;
            let sign_location = (enc.flags >> 8) & 0xff;
            let normalization = (enc.flags >> 4) & 0x03;
            let padding = (enc.flags >> 1) & 0x07;
            let actual = (
                sign_location,
                u32::from(layout.exponent_location),
                u32::from(layout.exponent_size),
                u32::from(layout.mantissa_location),
                u32::from(layout.mantissa_size),
                layout.exponent_bias,
            );
            if !full(bits) || actual != ieee || normalization != 2 || padding != 0 {
                return Err(format!(
                    "non-IEEE float encoding: sign/exponent/mantissa/bias {actual:?}, \
                     normalization {normalization}, padding {padding:#b}, \
                     offset {:?}, precision {:?}",
                    enc.bit_offset, enc.bit_precision
                ));
            }
            Ok(native)
        }
        other => Err(format!("unsupported numeric dtype: {other:?}")),
    }
}

fn native_order(order: &ByteOrder) -> bool {
    matches!(
        (cfg!(target_endian = "little"), order),
        (true, ByteOrder::LittleEndian) | (false, ByteOrder::BigEndian)
    )
}
fn is_resource_limit(e: &Error) -> bool {
    match e {
        Error::Context { source, .. } => is_resource_limit(source),
        Error::ResourceLimit(_) => true,
        _ => false,
    }
}
/// The reader's error text, except that `UnsupportedEncoding` keeps the
/// wording of the reader the engine embedded up to 0.2.3 ("unsupported {0}"
/// instead of upstream's "unsupported encoding: {0}"), so engine messages
/// stay as they were.
fn message(e: &Error) -> String {
    match e {
        Error::Context { path, source } => format!("{path}: {}", message(source)),
        Error::UnsupportedEncoding(detail) => format!("unsupported {detail}"),
        other => other.to_string(),
    }
}
pub(crate) fn map_error(e: Error) -> crate::Error {
    let kind = classify(&e);
    let mut result = crate::Error::new(kind, message(&e));
    if is_resource_limit(&e) {
        result = result.with_code(crate::UnsupportedCode::ResourceLimit);
    } else if kind == crate::ErrorKind::Unsupported {
        // Reader limitations (versions, layouts, filters, datatype classes) are encoding gaps.
        result = result.with_code(crate::UnsupportedCode::Encoding);
    }
    match &e {
        Error::Context { path, .. } => result.with_subject(path.clone()),
        _ => result,
    }
}
#[cfg(test)]
mod capability_error_tests {
    use super::*;
    #[test]
    fn known_reader_messages_have_exact_classifications() {
        for prefix in READER_VERSION_PREFIXES {
            let message = format!("{prefix}2");
            assert_eq!(
                classify(&Error::Other(message.clone())),
                crate::ErrorKind::Unsupported
            );
            assert_eq!(
                classify(&Error::Other(format!("{message} garbage"))),
                crate::ErrorKind::Corrupt
            );
            assert_eq!(
                classify(&Error::InvalidData(message)),
                crate::ErrorKind::Corrupt
            );
        }
        assert_eq!(
            classify(&Error::Other(
                "SOHM table lookup not yet supported — file uses shared object header messages"
                    .into()
            )),
            crate::ErrorKind::Unsupported
        );
        for message in [
            "external raw data file could not be resolved",
            "external raw data file 'x.bin' could not be resolved",
        ] {
            assert_eq!(
                classify(&Error::Other(message.into())),
                crate::ErrorKind::FileNotFound
            );
        }
        assert_eq!(
            classify(&Error::TypeMismatch {
                expected: "f32".into(),
                actual: "u8".into()
            }),
            crate::ErrorKind::InvalidRequest
        );
        assert_eq!(
            classify(&Error::SliceOutOfBounds {
                dim: 0,
                index: 9,
                size: 1
            }),
            crate::ErrorKind::InvalidRequest
        );
        for error in [
            Error::ChecksumMismatch {
                expected: 1,
                actual: 2,
            },
            Error::UnexpectedEof {
                offset: 0,
                needed: 4,
                available: 1,
            },
            Error::InvalidBTreeSignature,
            Error::InvalidObjectHeaderSignature,
            Error::InvalidFixedArraySignature { context: "test" },
        ] {
            assert_eq!(
                classify(&error.with_context("/data")),
                crate::ErrorKind::Corrupt
            );
        }
    }
    #[test]
    fn typed_classification_never_trusts_display() {
        assert_eq!(
            classify(&Error::UnsupportedEncoding("supported limitation".into())),
            crate::ErrorKind::Unsupported
        );
        for text in [
            "unsupported invented kind",
            "unknown shared message v2 type: 2",
            "unsupported shared message v3 type: 255",
        ] {
            assert_eq!(
                classify(&Error::InvalidData(text.into())),
                crate::ErrorKind::Corrupt
            );
        }
    }
}

fn corrupt(message: impl Into<String>) -> crate::Error {
    crate::Error::new(crate::ErrorKind::Corrupt, message)
}
fn unsupported(message: impl Into<String>) -> crate::Error {
    crate::Error::unsupported(crate::UnsupportedCode::Encoding, message)
}
#[cfg(test)]
mod verification_tests {
    use super::*;
    #[test]
    fn truncated_verification_is_corruption() {
        assert_eq!(le(&[0], 0, 8).unwrap_err().kind, crate::ErrorKind::Corrupt);
        let mut out = Vec::new();
        assert_eq!(
            v1_messages(&[3, 0, 255, 255, 0, 0, 0, 0], &mut out)
                .unwrap_err()
                .kind,
            crate::ErrorKind::Corrupt
        );
    }
}

#[cfg(test)]
mod address_safety_tests {
    use super::*;
    struct Malformed;
    impl Storage for Malformed {
        fn len(&self) -> u64 {
            u64::MAX
        }
        fn read_range(
            &self,
            _: u64,
            len: usize,
        ) -> hdf5_reader::error::Result<hdf5_reader::storage::StorageBuffer> {
            let mut b = vec![0; len];
            b[0] = 1;
            Ok(hdf5_reader::storage::StorageBuffer::from_vec(b))
        }
    }
    struct Short;
    impl Storage for Short {
        fn len(&self) -> u64 {
            100
        }
        fn read_range(
            &self,
            _: u64,
            _: usize,
        ) -> hdf5_reader::error::Result<hdf5_reader::storage::StorageBuffer> {
            Ok(hdf5_reader::storage::StorageBuffer::from_vec(vec![]))
        }
    }
    #[test]
    fn short_storage_reads_are_corrupt() {
        assert_eq!(
            header_messages(&Short, 0, 8, 8).unwrap_err().kind,
            crate::ErrorKind::Corrupt
        );
    }
    #[test]
    fn file_derived_header_address_overflow_is_corrupt() {
        assert_eq!(
            header_messages(&Malformed, u64::MAX - 8, 8, 8)
                .unwrap_err()
                .kind,
            crate::ErrorKind::Corrupt
        );
        assert_eq!(
            read_range(&Malformed, u64::MAX, 2).unwrap_err().kind,
            crate::ErrorKind::Corrupt
        );
    }
}

impl EncodingDescriptor {
    #[deprecated(note = "match typed class; retained for pinned bridge migration")]
    pub fn class_id(&self) -> u8 {
        self.class.code()
    }
}
