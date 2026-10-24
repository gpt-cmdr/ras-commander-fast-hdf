// ras-commander-fast-hdf: modified copy of roteiro-gis/netcdf-rust v0.9.3, MIT OR Apache-2.0,
// Copyright (c) 2026 Ian Norden. Changes are listed in vendor/hdf5-reader/PROVENANCE.md.
//! HDF5 External Data Files message (type 0x0007).
//!
//! Indicates that raw data for a dataset is stored in one or more external
//! files. Rarely used in practice, especially in NetCDF files.

use crate::hdf5_reader::error::Result;
use crate::hdf5_reader::io::Cursor;
pub use crate::hdf5_core::{ExternalFileSlot, ExternalFilesMessage};

/// Parse an external data files message.
pub fn parse(
    cursor: &mut Cursor<'_>,
    offset_size: u8,
    length_size: u8,
    msg_size: usize,
) -> Result<ExternalFilesMessage> {
    let start = cursor.position();

    let _version = cursor.read_u8()?;
    let _reserved = cursor.read_bytes(3)?;

    // Allocated slots (for pre-allocation; may differ from used slots)
    let _allocated_slots = cursor.read_u16_le()?;
    // Used slots — actual number of entries
    let used_slots = cursor.read_u16_le()? as usize;

    let heap_address = cursor.read_offset(offset_size)?;

    let mut slots = Vec::with_capacity(used_slots);
    for _ in 0..used_slots {
        let name_offset = cursor.read_length(length_size)?;
        let offset = cursor.read_length(length_size)?;
        let size = cursor.read_length(length_size)?;
        slots.push(ExternalFileSlot {
            name_offset,
            offset,
            size,
        });
    }

    let consumed = (cursor.position() - start) as usize;
    if consumed < msg_size {
        cursor.skip(msg_size - consumed)?;
    }

    Ok(ExternalFilesMessage {
        heap_address,
        slots,
    })
}
