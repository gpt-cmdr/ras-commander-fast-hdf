// ras-commander-fast-hdf: modified copy of roteiro-gis/netcdf-rust v0.9.3, MIT OR Apache-2.0,
// Copyright (c) 2026 Ian Norden. Changes are listed in vendor/hdf5-reader/PROVENANCE.md.
//! HDF5 Header Continuation message (type 0x0010).
//!
//! Indicates that additional header messages are stored at another location
//! in the file. The parser follows these to read the complete object header.

use crate::hdf5_reader::error::Result;
use crate::hdf5_reader::io::Cursor;

/// Parsed continuation message.
#[derive(Debug, Clone)]
pub struct ContinuationMessage {
    /// Absolute file offset of the continuation block.
    pub offset: u64,
    /// Length in bytes of the continuation block.
    pub length: u64,
}

/// Parse a header continuation message.
pub fn parse(
    cursor: &mut Cursor<'_>,
    offset_size: u8,
    length_size: u8,
    _msg_size: usize,
) -> Result<ContinuationMessage> {
    let offset = cursor.read_offset(offset_size)?;
    let length = cursor.read_length(length_size)?;

    Ok(ContinuationMessage { offset, length })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_continuation() {
        let mut data = Vec::new();
        data.extend_from_slice(&0x4000u64.to_le_bytes());
        data.extend_from_slice(&256u64.to_le_bytes());

        let mut cursor = Cursor::new(&data);
        let msg = parse(&mut cursor, 8, 8, data.len()).unwrap();
        assert_eq!(msg.offset, 0x4000);
        assert_eq!(msg.length, 256);
    }
}
