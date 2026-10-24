// ras-commander-fast-hdf: modified copy of roteiro-gis/netcdf-rust v0.9.3, MIT OR Apache-2.0,
// Copyright (c) 2026 Ian Norden. Changes are listed in vendor/hdf5-reader/PROVENANCE.md.
//! HDF5 Extensible Array (EA) chunk index.
//!
//! This is the default chunk index for datasets with one unlimited dimension
//! and `libver='latest'`. It uses a three-level hierarchy:
//!
//! - `EAHD` — Extensible Array Header
//! - `EAIB` — Extensible Array Index Block
//! - `EADB` — Extensible Array Data Block
//! - `EASB` — Extensible Array Secondary Block

use crate::hdf5_reader::checksum::jenkins_lookup3;
use crate::hdf5_reader::chunk_index::ChunkEntry;
use crate::hdf5_reader::error::{Error, Result};
use crate::hdf5_reader::io::Cursor;
use crate::hdf5_reader::storage::Storage;

const EAHD_SIGNATURE: [u8; 4] = *b"EAHD";
const EAIB_SIGNATURE: [u8; 4] = *b"EAIB";
const EADB_SIGNATURE: [u8; 4] = *b"EADB";
const EASB_SIGNATURE: [u8; 4] = *b"EASB";

/// Parsed Extensible Array Header.
#[derive(Debug)]
struct EaHeader {
    client_id: u8,
    element_size: u8,
    _max_nelmts_bits: u8,
    idx_blk_elmts: u8,
    data_blk_min_elmts: u8,
    sec_blk_min_data_ptrs: u8,
    max_dblk_page_nelmts_bits: u8,
    _nelmts: u64,
    index_block_address: u64,
}

/// Parse the Extensible Array Header.
///
/// On-disk layout (from H5EA_HEADER_SIZE):
/// sig(4) + ver(1) + client_id(1) +
/// element_size(1) + max_nelmts_bits(1) + idx_blk_elmts(1) +
/// data_blk_min_elmts(1) + sec_blk_min_data_ptrs(1) + max_dblk_page_nelmts_bits(1) +
/// 6 statistics fields (each length_size) +
/// index_block_address(offset_size) + checksum(4)
fn parse_header(data: &[u8], address: u64, offset_size: u8, length_size: u8) -> Result<EaHeader> {
    let mut cursor = Cursor::new(data);
    cursor.set_position(address);

    let sig = cursor.read_bytes(4)?;
    if sig != EAHD_SIGNATURE {
        return Err(Error::InvalidExtensibleArraySignature {
            context: "header signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported extensible array header version {}",
            version
        )));
    }

    let client_id = cursor.read_u8()?;
    let element_size = cursor.read_u8()?;
    let max_nelmts_bits = cursor.read_u8()?;
    let idx_blk_elmts = cursor.read_u8()?;
    let data_blk_min_elmts = cursor.read_u8()?;
    let sec_blk_min_data_ptrs = cursor.read_u8()?;
    let max_dblk_page_nelmts_bits = cursor.read_u8()?;

    // Statistics (6 fields, each length_size bytes)
    let _nsuper_blks = cursor.read_length(length_size)?;
    let _super_blk_size = cursor.read_length(length_size)?;
    let _ndata_blks = cursor.read_length(length_size)?;
    let _data_blk_size = cursor.read_length(length_size)?;
    let _max_idx_set = cursor.read_length(length_size)?;
    let nelmts = cursor.read_length(length_size)?;

    let index_block_address = cursor.read_offset(offset_size)?;

    // Checksum
    let header_end = cursor.position();
    let header_bytes = &data[address as usize..header_end as usize];
    let stored_checksum = cursor.read_u32_le()?;
    let computed = jenkins_lookup3(header_bytes);
    if stored_checksum != computed {
        return Err(Error::ChecksumMismatch {
            expected: stored_checksum,
            actual: computed,
        });
    }

    Ok(EaHeader {
        client_id,
        element_size,
        _max_nelmts_bits: max_nelmts_bits,
        idx_blk_elmts,
        data_blk_min_elmts,
        sec_blk_min_data_ptrs,
        max_dblk_page_nelmts_bits,
        _nelmts: nelmts,
        index_block_address,
    })
}

fn parse_header_storage(
    storage: &dyn Storage,
    address: u64,
    offset_size: u8,
    length_size: u8,
) -> Result<EaHeader> {
    let header_len = 4
        + 1
        + 1
        + 1
        + 1
        + 1
        + 1
        + 1
        + 1
        + 6 * usize::from(length_size)
        + usize::from(offset_size)
        + 4;
    let bytes = storage.read_range(address, header_len)?;
    parse_header(bytes.as_ref(), 0, offset_size, length_size)
}

/// Compute the super block layout.
///
/// Returns a vec of (elements_per_data_block, num_data_blocks) for each super block.
/// Stops generating entries once cumulative capacity exceeds `nelmts`.
fn compute_super_block_layout(header: &EaHeader) -> Vec<(u64, u64)> {
    let mut layout = Vec::new();
    let dblk_min = header.data_blk_min_elmts as u64;
    let sblk_min = header.sec_blk_min_data_ptrs as u64;
    let nelmts = header._nelmts;
    let mut cumulative = header.idx_blk_elmts as u64;

    for sb_idx in 0u32..64 {
        if cumulative >= nelmts {
            break;
        }
        let elmts_per_dblk = dblk_min.saturating_mul(1u64 << (sb_idx / 2));
        let num_dblks = sblk_min.saturating_mul(1u64 << sb_idx.div_ceil(2));
        layout.push((elmts_per_dblk, num_dblks));
        cumulative = cumulative.saturating_add(elmts_per_dblk.saturating_mul(num_dblks));
    }

    layout
}

/// Append `count` undefined placeholder entries without exceeding `max_entries`.
///
/// Placeholders only keep linear indices aligned; entries past the dataset's
/// chunk count are never mapped, so a crafted element count cannot make the
/// reader materialize billions of them.
fn push_undefined(entries: &mut Vec<EaRawEntry>, count: u64, max_entries: usize) {
    let room = max_entries.saturating_sub(entries.len()) as u64;
    entries.extend((0..count.min(room)).map(|_| EaRawEntry {
        address: u64::MAX,
        chunk_size: 0,
        filter_mask: 0,
    }));
}

/// Number of chunks covering the dataset extent.
fn total_chunks(dataset_shape: &[u64], chunk_dims: &[u32]) -> Result<usize> {
    let mut total = 1usize;
    for count in chunks_per_dim(dataset_shape, chunk_dims)? {
        total = checked_mul_usize(
            total,
            checked_usize_from_u64(count, "extensible array chunk count")?,
            "extensible array chunk count",
        )?;
    }
    Ok(total)
}

fn chunks_per_dim(dataset_shape: &[u64], chunk_dims: &[u32]) -> Result<Vec<u64>> {
    if chunk_dims.len() < dataset_shape.len() {
        return Err(Error::InvalidData(
            "extensible array chunk rank is smaller than the dataset rank".into(),
        ));
    }
    dataset_shape
        .iter()
        .zip(chunk_dims)
        .map(|(&extent, &chunk)| {
            if chunk == 0 {
                return Err(Error::InvalidData("chunk dimension has zero extent".into()));
            }
            Ok(extent.div_ceil(u64::from(chunk)))
        })
        .collect()
}

fn checked_add_usize(left: usize, right: usize, context: &str) -> Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| Error::InvalidData(format!("{context} exceeds platform usize capacity")))
}

fn checked_mul_usize(left: usize, right: usize, context: &str) -> Result<usize> {
    left.checked_mul(right)
        .ok_or_else(|| Error::InvalidData(format!("{context} exceeds platform usize capacity")))
}

fn checked_add_u64(left: u64, right: u64, context: &str) -> Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| Error::InvalidData(format!("{context} exceeds u64 capacity")))
}

fn checked_usize_from_u64(value: u64, context: &str) -> Result<usize> {
    usize::try_from(value).map_err(|_| {
        Error::InvalidData(format!(
            "{context} value {value} exceeds platform usize capacity"
        ))
    })
}

fn checked_u64_from_usize(value: usize, context: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::InvalidData(format!("{context} exceeds u64 capacity")))
}

fn ea_block_header_len(offset_size: u8, sizeof_nelmts: usize) -> Result<usize> {
    let len = checked_add_usize(4, 1, "extensible array block header length")?;
    let len = checked_add_usize(len, 1, "extensible array block header length")?;
    let len = checked_add_usize(
        len,
        usize::from(offset_size),
        "extensible array block header length",
    )?;
    checked_add_usize(len, sizeof_nelmts, "extensible array block header length")
}

fn ea_page_nelmts(max_page_bits: u8) -> Result<usize> {
    if max_page_bits == 0 {
        return Ok(0);
    }
    1usize.checked_shl(u32::from(max_page_bits)).ok_or_else(|| {
        Error::InvalidData("extensible array page element count exceeds usize capacity".into())
    })
}

fn index_block_len(header: &EaHeader, sb_layout: &[(u64, u64)], offset_size: u8) -> Result<usize> {
    let offset_size = usize::from(offset_size);
    let inline_bytes = checked_mul_usize(
        usize::from(header.idx_blk_elmts),
        usize::from(header.element_size),
        "extensible array index block inline entry bytes",
    )?;
    let direct_ptrs = checked_mul_usize(
        2,
        usize::from(header.sec_blk_min_data_ptrs),
        "extensible array index block direct data block pointer count",
    )?;
    let direct_bytes = checked_mul_usize(
        direct_ptrs,
        offset_size,
        "extensible array index block direct data block address bytes",
    )?;
    let secondary_ptrs = sb_layout.len().saturating_sub(direct_ptrs);
    let secondary_bytes = checked_mul_usize(
        secondary_ptrs,
        offset_size,
        "extensible array index block secondary block address bytes",
    )?;

    let mut len = 4usize;
    for part in [
        1,
        1,
        offset_size,
        inline_bytes,
        direct_bytes,
        secondary_bytes,
        4,
    ] {
        len = checked_add_usize(len, part, "extensible array index block length")?;
    }
    Ok(len)
}

fn secondary_page_bitmap_bytes(
    max_page_bits: u8,
    elmts_per_dblk: u64,
    num_dblks: u64,
) -> Result<usize> {
    let page_nelmts = ea_page_nelmts(max_page_bits)?;
    if page_nelmts == 0 {
        return Ok(0);
    }

    let elmts_per_dblk =
        checked_usize_from_u64(elmts_per_dblk, "extensible array data block element count")?;
    if elmts_per_dblk <= page_nelmts {
        return Ok(0);
    }

    let num_dblks = checked_usize_from_u64(
        num_dblks,
        "extensible array secondary block data block count",
    )?;
    let pages_per_dblk = elmts_per_dblk.div_ceil(page_nelmts);
    let total_pages = checked_mul_usize(
        num_dblks,
        pages_per_dblk,
        "extensible array secondary block page bitmap bit count",
    )?;
    Ok(total_pages.div_ceil(8))
}

fn secondary_block_len(
    num_dblk_addrs: usize,
    offset_size: u8,
    sizeof_nelmts: usize,
    page_bitmap_bytes: usize,
) -> Result<usize> {
    let offset_bytes = usize::from(offset_size);
    let addr_bytes = checked_mul_usize(
        num_dblk_addrs,
        offset_bytes,
        "extensible array secondary block data block address bytes",
    )?;

    let mut len = ea_block_header_len(offset_size, sizeof_nelmts)?;
    for part in [page_bitmap_bytes, addr_bytes, 4] {
        len = checked_add_usize(len, part, "extensible array secondary block length")?;
    }
    Ok(len)
}

fn data_block_len_from_bitmap(
    num_entries: usize,
    page_nelmts: usize,
    entry_size: u8,
    offset_size: u8,
    sizeof_nelmts: usize,
    page_bitmap: &[u8],
) -> Result<usize> {
    let num_pages = num_entries.div_ceil(page_nelmts);
    let bitmap_bytes = num_pages.div_ceil(8);
    let mut len = checked_add_usize(
        ea_block_header_len(offset_size, sizeof_nelmts)?,
        bitmap_bytes,
        "extensible array data block length",
    )?;

    for page_idx in 0..num_pages {
        let byte_idx = page_idx / 8;
        let bit_idx = page_idx % 8;
        let page_initialized =
            byte_idx < page_bitmap.len() && (page_bitmap[byte_idx] & (1 << bit_idx)) != 0;
        if !page_initialized {
            continue;
        }

        let entries_in_page = if page_idx == num_pages - 1 {
            let remainder = num_entries % page_nelmts;
            if remainder == 0 {
                page_nelmts
            } else {
                remainder
            }
        } else {
            page_nelmts
        };
        let entry_bytes = checked_mul_usize(
            entries_in_page,
            usize::from(entry_size),
            "extensible array data block page entry bytes",
        )?;
        let page_bytes = checked_add_usize(
            entry_bytes,
            4,
            "extensible array data block page byte length",
        )?;
        len = checked_add_usize(len, page_bytes, "extensible array data block length")?;
    }

    Ok(len)
}

fn unpaged_data_block_len(
    num_entries: usize,
    offset_size: u8,
    entry_size: u8,
    sizeof_nelmts: usize,
) -> Result<usize> {
    let entry_bytes = checked_mul_usize(
        num_entries,
        usize::from(entry_size),
        "extensible array data block entry bytes",
    )?;
    let len = checked_add_usize(
        ea_block_header_len(offset_size, sizeof_nelmts)?,
        entry_bytes,
        "extensible array data block length",
    )?;
    checked_add_usize(len, 4, "extensible array data block length")
}

/// A single raw entry.
struct EaRawEntry {
    address: u64,
    chunk_size: u64,
    filter_mask: u32,
}

/// Read `count` entries from the cursor.
fn read_entries(
    cursor: &mut Cursor<'_>,
    count: usize,
    is_filtered: bool,
    offset_size: u8,
    entry_size: u8,
) -> Result<Vec<EaRawEntry>> {
    let needed = checked_mul_usize(
        count,
        usize::from(entry_size).max(1),
        "extensible array entry run",
    )?;
    if needed as u64 > cursor.remaining() {
        return Err(Error::UnexpectedEof {
            offset: cursor.position(),
            needed: needed as u64,
            available: cursor.remaining(),
        });
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let address = cursor.read_offset(offset_size)?;
        let (chunk_size, filter_mask) = if is_filtered {
            let chunk_size_len = entry_size
                .checked_sub(offset_size)
                .and_then(|remaining| remaining.checked_sub(4))
                .ok_or_else(|| Error::InvalidData("invalid extensible array entry size".into()))?;
            // 1–8 bytes wide (sized to the chunk), not a "length" of the superblock size.
            let cs = cursor.read_uvar(chunk_size_len as usize)?;
            let fm = cursor.read_u32_le()?;
            (cs, fm)
        } else {
            (0, 0)
        };
        entries.push(EaRawEntry {
            address,
            chunk_size,
            filter_mask,
        });
    }
    Ok(entries)
}

/// Parse a data block and return its entries.
///
/// `sizeof_nelmts` is `ceil(max_nelmts_bits / 8)` — used for the block_off field.
#[allow(clippy::too_many_arguments)]
fn parse_data_block(
    data: &[u8],
    address: u64,
    num_entries: usize,
    max_entries: usize,
    is_filtered: bool,
    max_page_bits: u8,
    offset_size: u8,
    entry_size: u8,
    sizeof_nelmts: usize,
) -> Result<Vec<EaRawEntry>> {
    let mut cursor = Cursor::new(data);
    cursor.set_position(address);

    let sig = cursor.read_bytes(4)?;
    if sig != EADB_SIGNATURE {
        return Err(Error::InvalidExtensibleArraySignature {
            context: "data block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported extensible array data block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;

    // Block offset: sizeof_nelmts bytes indicating this block's element index offset.
    cursor.skip(sizeof_nelmts)?;

    // Paging is used only when nelmts exceeds 2^page_bits.
    let page_nelmts = ea_page_nelmts(max_page_bits)?;

    if page_nelmts > 0 && num_entries > page_nelmts {
        // Paged data block
        let num_pages = num_entries.div_ceil(page_nelmts);
        let bitmap_bytes = num_pages.div_ceil(8);
        let page_bitmap = cursor.read_bytes(bitmap_bytes)?.to_vec();

        let mut all_entries = Vec::with_capacity(num_entries.min(max_entries));
        for page_idx in 0..num_pages {
            if all_entries.len() >= max_entries {
                break;
            }
            let byte_idx = page_idx / 8;
            let bit_idx = page_idx % 8;
            let page_initialized =
                byte_idx < page_bitmap.len() && (page_bitmap[byte_idx] & (1 << bit_idx)) != 0;

            let entries_in_page = if page_idx == num_pages - 1 {
                let remainder = num_entries % page_nelmts;
                if remainder == 0 {
                    page_nelmts
                } else {
                    remainder
                }
            } else {
                page_nelmts
            };

            if page_initialized {
                let page_entries = read_entries(
                    &mut cursor,
                    entries_in_page,
                    is_filtered,
                    offset_size,
                    entry_size,
                )?;
                let _page_checksum = cursor.read_u32_le()?;
                all_entries.extend(page_entries);
            } else {
                push_undefined(&mut all_entries, entries_in_page as u64, max_entries);
            }
        }
        all_entries.truncate(max_entries);
        Ok(all_entries)
    } else {
        // Non-paged data block
        let entries = read_entries(
            &mut cursor,
            num_entries,
            is_filtered,
            offset_size,
            entry_size,
        )?;
        let _checksum = cursor.read_u32_le()?;
        Ok(entries)
    }
}

/// Parse a secondary block and return its data block addresses.
fn parse_secondary_block(
    data: &[u8],
    address: u64,
    num_dblk_addrs: usize,
    offset_size: u8,
    sizeof_nelmts: usize,
    page_bitmap_bytes: usize,
) -> Result<Vec<u64>> {
    let mut cursor = Cursor::new(data);
    cursor.set_position(address);

    let sig = cursor.read_bytes(4)?;
    if sig != EASB_SIGNATURE {
        return Err(Error::InvalidExtensibleArraySignature {
            context: "secondary block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported extensible array secondary block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;
    cursor.skip(sizeof_nelmts)?;

    if page_bitmap_bytes > 0 {
        cursor.skip(page_bitmap_bytes)?;
    }

    let mut addrs = Vec::with_capacity(num_dblk_addrs);
    for _ in 0..num_dblk_addrs {
        addrs.push(cursor.read_offset(offset_size)?);
    }

    // Skip checksum
    let _checksum = cursor.read_u32_le()?;

    Ok(addrs)
}

fn parse_secondary_block_storage(
    storage: &dyn Storage,
    address: u64,
    num_dblk_addrs: usize,
    offset_size: u8,
    sizeof_nelmts: usize,
    page_bitmap_bytes: usize,
) -> Result<Vec<u64>> {
    let read_len = secondary_block_len(
        num_dblk_addrs,
        offset_size,
        sizeof_nelmts,
        page_bitmap_bytes,
    )?;
    let bytes = storage.read_range(address, read_len)?;
    parse_secondary_block(
        bytes.as_ref(),
        0,
        num_dblk_addrs,
        offset_size,
        sizeof_nelmts,
        page_bitmap_bytes,
    )
}

fn read_entry_at(
    data: &[u8],
    position: u64,
    is_filtered: bool,
    offset_size: u8,
    entry_size: u8,
) -> Result<EaRawEntry> {
    let mut cursor = Cursor::new(data);
    cursor.set_position(position);
    let mut entries = read_entries(&mut cursor, 1, is_filtered, offset_size, entry_size)?;
    entries
        .pop()
        .ok_or_else(|| Error::InvalidData("missing extensible array entry".into()))
}

fn read_entry_at_storage(
    storage: &dyn Storage,
    position: u64,
    is_filtered: bool,
    offset_size: u8,
    entry_size: u8,
) -> Result<EaRawEntry> {
    let bytes = storage.read_range(position, usize::from(entry_size))?;
    let mut cursor = Cursor::new(bytes.as_ref());
    let mut entries = read_entries(&mut cursor, 1, is_filtered, offset_size, entry_size)?;
    entries
        .pop()
        .ok_or_else(|| Error::InvalidData("missing extensible array entry".into()))
}

fn linear_target_offsets(
    dataset_shape: &[u64],
    chunk_dims: &[u32],
    chunk_bounds: Option<(&[u64], &[u64])>,
) -> Result<Vec<(usize, Vec<u64>)>> {
    let ndim = dataset_shape.len();
    let chunks_per_dim = chunks_per_dim(dataset_shape, chunk_dims)?;

    if ndim == 0 {
        return Ok(vec![(0, Vec::new())]);
    }
    // An empty extent has no chunks to look up.
    if chunks_per_dim.contains(&0) {
        return Ok(Vec::new());
    }

    let (first_chunk, last_chunk): (Vec<u64>, Vec<u64>) = match chunk_bounds {
        Some((first, last)) => (first.to_vec(), last.to_vec()),
        None => (
            vec![0u64; ndim],
            chunks_per_dim
                .iter()
                .map(|count| count.saturating_sub(1))
                .collect(),
        ),
    };
    if first_chunk.len() != ndim || last_chunk.len() != ndim {
        return Err(Error::InvalidData(
            "extensible array chunk bounds do not match the dataset rank".into(),
        ));
    }

    let mut targets = Vec::new();
    let mut chunk_indices = first_chunk.clone();
    loop {
        let mut linear_idx = 0u64;
        for (dim, chunk_index) in chunk_indices.iter().enumerate() {
            linear_idx = linear_idx
                .checked_mul(chunks_per_dim[dim])
                .and_then(|n| n.checked_add(*chunk_index))
                .ok_or_else(|| {
                    Error::InvalidData("extensible array chunk index overflows".into())
                })?;
        }
        let offsets = chunk_indices
            .iter()
            .enumerate()
            .map(|(dim, chunk_index)| {
                chunk_index
                    .checked_mul(u64::from(chunk_dims[dim]))
                    .ok_or_else(|| {
                        Error::InvalidData("extensible array chunk offset overflows".into())
                    })
            })
            .collect::<Result<Vec<u64>>>()?;
        targets.push((
            checked_usize_from_u64(linear_idx, "extensible array chunk index")?,
            offsets,
        ));

        let mut advanced = false;
        for dim in (0..ndim).rev() {
            if chunk_indices[dim] < last_chunk[dim] {
                chunk_indices[dim] += 1;
                if dim + 1 < ndim {
                    chunk_indices[(dim + 1)..ndim].copy_from_slice(&first_chunk[(dim + 1)..ndim]);
                }
                advanced = true;
                break;
            }
        }

        if !advanced {
            break;
        }
    }

    Ok(targets)
}

#[allow(clippy::too_many_arguments)]
fn read_data_block_entry(
    data: &[u8],
    address: u64,
    num_entries: usize,
    local_idx: usize,
    is_filtered: bool,
    max_page_bits: u8,
    offset_size: u8,
    entry_size: u8,
    sizeof_nelmts: usize,
) -> Result<EaRawEntry> {
    let mut cursor = Cursor::new(data);
    cursor.set_position(address);

    let sig = cursor.read_bytes(4)?;
    if sig != EADB_SIGNATURE {
        return Err(Error::InvalidExtensibleArraySignature {
            context: "data block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported extensible array data block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;
    cursor.skip(sizeof_nelmts)?;

    let page_nelmts = ea_page_nelmts(max_page_bits)?;

    if page_nelmts > 0 && num_entries > page_nelmts {
        let num_pages = num_entries.div_ceil(page_nelmts);
        let bitmap_bytes = num_pages.div_ceil(8);
        let page_bitmap = cursor.read_bytes(bitmap_bytes)?.to_vec();
        let data_start = cursor.position();

        let target_page = local_idx / page_nelmts;
        let within_page = local_idx % page_nelmts;
        let byte_idx = target_page / 8;
        let bit_idx = target_page % 8;
        let page_initialized =
            byte_idx < page_bitmap.len() && (page_bitmap[byte_idx] & (1 << bit_idx)) != 0;
        if !page_initialized {
            return Ok(EaRawEntry {
                address: u64::MAX,
                chunk_size: 0,
                filter_mask: 0,
            });
        }

        let mut page_start = data_start;
        for page_idx in 0..target_page {
            let entries_in_page = if page_idx == num_pages - 1 {
                let remainder = num_entries % page_nelmts;
                if remainder == 0 {
                    page_nelmts
                } else {
                    remainder
                }
            } else {
                page_nelmts
            };
            let page_byte_idx = page_idx / 8;
            let page_bit_idx = page_idx % 8;
            let initialized = page_byte_idx < page_bitmap.len()
                && (page_bitmap[page_byte_idx] & (1 << page_bit_idx)) != 0;
            if initialized {
                let page_len = checked_add_usize(
                    checked_mul_usize(
                        entries_in_page,
                        usize::from(entry_size),
                        "extensible array page size",
                    )?,
                    4,
                    "extensible array page size",
                )?;
                page_start = checked_add_u64(
                    page_start,
                    checked_u64_from_usize(page_len, "extensible array page size")?,
                    "extensible array page offset",
                )?;
            }
        }

        let within = checked_mul_usize(
            within_page,
            usize::from(entry_size),
            "extensible array entry offset",
        )?;
        let position = checked_add_u64(
            page_start,
            checked_u64_from_usize(within, "extensible array entry offset")?,
            "extensible array entry offset",
        )?;
        return read_entry_at(data, position, is_filtered, offset_size, entry_size);
    }

    let within = checked_mul_usize(
        local_idx,
        usize::from(entry_size),
        "extensible array entry offset",
    )?;
    let position = checked_add_u64(
        cursor.position(),
        checked_u64_from_usize(within, "extensible array entry offset")?,
        "extensible array entry offset",
    )?;
    read_entry_at(data, position, is_filtered, offset_size, entry_size)
}

#[allow(clippy::too_many_arguments)]
fn read_data_block_entry_storage(
    storage: &dyn Storage,
    address: u64,
    num_entries: usize,
    local_idx: usize,
    is_filtered: bool,
    max_page_bits: u8,
    offset_size: u8,
    entry_size: u8,
    sizeof_nelmts: usize,
) -> Result<EaRawEntry> {
    let header_len = 4 + 1 + 1 + usize::from(offset_size) + sizeof_nelmts;
    let header = storage.read_range(address, header_len)?;
    let mut cursor = Cursor::new(header.as_ref());

    let sig = cursor.read_bytes(4)?;
    if sig != EADB_SIGNATURE {
        return Err(Error::InvalidExtensibleArraySignature {
            context: "data block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported extensible array data block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;
    cursor.skip(sizeof_nelmts)?;

    let base = checked_add_u64(
        address,
        checked_u64_from_usize(header_len, "EA data block header length")?,
        "EA data block entry base",
    )?;
    let page_nelmts = ea_page_nelmts(max_page_bits)?;

    if page_nelmts > 0 && num_entries > page_nelmts {
        let num_pages = num_entries.div_ceil(page_nelmts);
        let bitmap_bytes = num_pages.div_ceil(8);
        let page_bitmap = storage.read_range(base, bitmap_bytes)?;
        let data_start = checked_add_u64(
            base,
            checked_u64_from_usize(bitmap_bytes, "EA bitmap size")?,
            "EA data block page data start",
        )?;

        let target_page = local_idx / page_nelmts;
        let within_page = local_idx % page_nelmts;
        let byte_idx = target_page / 8;
        let bit_idx = target_page % 8;
        let page_initialized =
            byte_idx < page_bitmap.len() && (page_bitmap[byte_idx] & (1 << bit_idx)) != 0;
        if !page_initialized {
            return Ok(EaRawEntry {
                address: u64::MAX,
                chunk_size: 0,
                filter_mask: 0,
            });
        }

        let mut page_start = data_start;
        for page_idx in 0..target_page {
            let entries_in_page = if page_idx == num_pages - 1 {
                let remainder = num_entries % page_nelmts;
                if remainder == 0 {
                    page_nelmts
                } else {
                    remainder
                }
            } else {
                page_nelmts
            };
            let page_byte_idx = page_idx / 8;
            let page_bit_idx = page_idx % 8;
            let initialized = page_byte_idx < page_bitmap.len()
                && (page_bitmap[page_byte_idx] & (1 << page_bit_idx)) != 0;
            if initialized {
                let entry_bytes = checked_mul_usize(
                    entries_in_page,
                    usize::from(entry_size),
                    "EA page entry bytes",
                )?;
                let page_size = checked_add_usize(entry_bytes, 4, "EA page size")?;
                page_start = checked_add_u64(
                    page_start,
                    checked_u64_from_usize(page_size, "EA page size")?,
                    "EA page start",
                )?;
            }
        }

        let within_page_offset =
            checked_mul_usize(within_page, usize::from(entry_size), "EA page entry offset")?;
        let position = checked_add_u64(
            page_start,
            checked_u64_from_usize(within_page_offset, "EA page entry offset")?,
            "EA page entry position",
        )?;
        return read_entry_at_storage(storage, position, is_filtered, offset_size, entry_size);
    }

    let local_offset = checked_mul_usize(local_idx, usize::from(entry_size), "EA entry offset")?;
    let position = checked_add_u64(
        base,
        checked_u64_from_usize(local_offset, "EA entry offset")?,
        "EA entry position",
    )?;
    read_entry_at_storage(storage, position, is_filtered, offset_size, entry_size)
}

#[allow(clippy::too_many_arguments)]
fn read_data_block_storage(
    storage: &dyn Storage,
    address: u64,
    num_entries: usize,
    max_entries: usize,
    is_filtered: bool,
    max_page_bits: u8,
    offset_size: u8,
    entry_size: u8,
    sizeof_nelmts: usize,
) -> Result<Vec<EaRawEntry>> {
    let page_nelmts = ea_page_nelmts(max_page_bits)?;
    let read_len = if page_nelmts > 0 && num_entries > page_nelmts {
        let header_len = ea_block_header_len(offset_size, sizeof_nelmts)?;
        let num_pages = num_entries.div_ceil(page_nelmts);
        let bitmap_bytes = num_pages.div_ceil(8);
        let bitmap_end = checked_add_usize(
            header_len,
            bitmap_bytes,
            "extensible array data block bitmap range",
        )?;
        let prefix = storage.read_range(address, bitmap_end)?;
        let page_bitmap = &prefix.as_ref()[header_len..bitmap_end];
        data_block_len_from_bitmap(
            num_entries,
            page_nelmts,
            entry_size,
            offset_size,
            sizeof_nelmts,
            page_bitmap,
        )?
    } else {
        unpaged_data_block_len(num_entries, offset_size, entry_size, sizeof_nelmts)?
    };

    let block = storage.read_range(address, read_len)?;
    parse_data_block(
        block.as_ref(),
        0,
        num_entries,
        max_entries,
        is_filtered,
        max_page_bits,
        offset_size,
        entry_size,
        sizeof_nelmts,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_extensible_array_chunk_entries_bounded(
    data: &[u8],
    header: &EaHeader,
    offset_size: u8,
    dataset_shape: &[u64],
    chunk_dims: &[u32],
    chunk_bounds: (&[u64], &[u64]),
    sb_layout: &[(u64, u64)],
    sizeof_nelmts: usize,
) -> Result<Vec<ChunkEntry>> {
    let is_filtered = header.client_id == 1;
    let targets = linear_target_offsets(dataset_shape, chunk_dims, Some(chunk_bounds))?;

    let mut cursor = Cursor::new(data);
    cursor.set_position(header.index_block_address);

    let sig = cursor.read_bytes(4)?;
    if sig != EAIB_SIGNATURE {
        return Err(Error::InvalidExtensibleArraySignature {
            context: "index block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported extensible array index block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;

    let num_inline = header.idx_blk_elmts as usize;
    let inline_start = cursor.position();
    cursor.skip(checked_mul_usize(
        num_inline,
        usize::from(header.element_size),
        "extensible array inline entry bytes",
    )?)?;

    let ndblk_addrs = 2 * header.sec_blk_min_data_ptrs as usize;
    let mut direct_dblk_addrs = Vec::with_capacity(ndblk_addrs);
    for _ in 0..ndblk_addrs {
        direct_dblk_addrs.push(cursor.read_offset(offset_size)?);
    }

    let nsblks = sb_layout.len();
    let nsblk_addrs = nsblks.saturating_sub(ndblk_addrs);
    let mut sec_block_addrs = Vec::with_capacity(nsblk_addrs);
    for _ in 0..nsblk_addrs {
        sec_block_addrs.push(cursor.read_offset(offset_size)?);
    }

    let mut secondary_block_cache: Vec<Option<Vec<u64>>> = vec![None; sec_block_addrs.len()];
    let mut entries = Vec::new();

    for (linear_idx, offsets) in targets {
        let raw = if linear_idx < num_inline {
            let inline_offset = checked_mul_usize(
                linear_idx,
                usize::from(header.element_size),
                "extensible array inline entry offset",
            )?;
            read_entry_at(
                data,
                checked_add_u64(
                    inline_start,
                    checked_u64_from_usize(inline_offset, "extensible array inline entry offset")?,
                    "extensible array inline entry offset",
                )?,
                is_filtered,
                offset_size,
                header.element_size,
            )?
        } else {
            let mut relative_idx = (linear_idx - num_inline) as u64;
            let mut sb_idx = None;
            for (candidate_idx, (elmts_per_dblk, num_dblks)) in sb_layout.iter().enumerate() {
                let capacity = elmts_per_dblk.saturating_mul(*num_dblks);
                if relative_idx < capacity {
                    sb_idx = Some(candidate_idx);
                    break;
                }
                relative_idx -= capacity;
            }

            let Some(sb_idx) = sb_idx else {
                continue;
            };
            let (elmts_per_dblk, _) = sb_layout[sb_idx];
            let dblk_idx = (relative_idx / elmts_per_dblk) as usize;
            let local_idx = (relative_idx % elmts_per_dblk) as usize;

            let dblk_addr = if sb_idx < 2 {
                let base = sb_layout[..sb_idx]
                    .iter()
                    .map(|(_, num_dblks)| *num_dblks as usize)
                    .sum::<usize>();
                *direct_dblk_addrs.get(base + dblk_idx).unwrap_or(&u64::MAX)
            } else {
                let sec_cache_idx = sb_idx - 2;
                if secondary_block_cache[sec_cache_idx].is_none() {
                    let sec_addr = sec_block_addrs
                        .get(sec_cache_idx)
                        .copied()
                        .unwrap_or(u64::MAX);
                    if Cursor::is_undefined_offset(sec_addr, offset_size) {
                        secondary_block_cache[sec_cache_idx] = Some(Vec::new());
                    } else {
                        let (_, num_dblks) = sb_layout[sb_idx];
                        let page_bitmap_bytes = secondary_page_bitmap_bytes(
                            header.max_dblk_page_nelmts_bits,
                            elmts_per_dblk,
                            num_dblks,
                        )?;
                        secondary_block_cache[sec_cache_idx] = Some(parse_secondary_block(
                            data,
                            sec_addr,
                            checked_usize_from_u64(
                                num_dblks,
                                "extensible array secondary block data block count",
                            )?,
                            offset_size,
                            sizeof_nelmts,
                            page_bitmap_bytes,
                        )?);
                    }
                }

                secondary_block_cache[sec_cache_idx]
                    .as_ref()
                    .and_then(|addrs| addrs.get(dblk_idx))
                    .copied()
                    .unwrap_or(u64::MAX)
            };

            if Cursor::is_undefined_offset(dblk_addr, offset_size) {
                continue;
            }

            read_data_block_entry(
                data,
                dblk_addr,
                elmts_per_dblk as usize,
                local_idx,
                is_filtered,
                header.max_dblk_page_nelmts_bits,
                offset_size,
                header.element_size,
                sizeof_nelmts,
            )?
        };

        if Cursor::is_undefined_offset(raw.address, offset_size) {
            continue;
        }

        entries.push(ChunkEntry {
            address: raw.address,
            size: raw.chunk_size,
            filter_mask: raw.filter_mask,
            offsets,
        });
    }

    Ok(entries)
}

/// Collect chunk entries from an Extensible Array index.
///
/// Walks the EAHD → EAIB → (EADB / EASB → EADB) hierarchy and converts
/// linear entry indices to multi-dimensional chunk offsets.
pub fn collect_extensible_array_chunk_entries(
    data: &[u8],
    header_address: u64,
    offset_size: u8,
    length_size: u8,
    dataset_shape: &[u64],
    chunk_dims: &[u32],
    chunk_bounds: Option<(&[u64], &[u64])>,
) -> Result<Vec<ChunkEntry>> {
    let header = parse_header(data, header_address, offset_size, length_size)?;

    // Non-inline EA block geometry is not qualified: the old mapping silently
    // omitted allocated chunks (including default indices 36+). Refuse both
    // full and bounded reads rather than interpreting missing entries as fill.
    if header._nelmts > u64::from(header.idx_blk_elmts) {
        return Err(Error::UnsupportedEncoding(
            "non-inline extensible-array chunk index (unqualified block mapping)".into(),
        ));
    }

    if Cursor::is_undefined_offset(header.index_block_address, offset_size) {
        return Ok(Vec::new());
    }

    let is_filtered = header.client_id == 1;
    let sb_layout = compute_super_block_layout(&header);
    let sizeof_nelmts = (header._max_nelmts_bits as usize).div_ceil(8);

    if let Some(bounds) = chunk_bounds {
        return collect_extensible_array_chunk_entries_bounded(
            data,
            &header,
            offset_size,
            dataset_shape,
            chunk_dims,
            bounds,
            &sb_layout,
            sizeof_nelmts,
        );
    }

    // Parse the index block.
    let mut cursor = Cursor::new(data);
    cursor.set_position(header.index_block_address);

    let sig = cursor.read_bytes(4)?;
    if sig != EAIB_SIGNATURE {
        return Err(Error::InvalidExtensibleArraySignature {
            context: "index block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported extensible array index block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;

    // 1. Inline elements (idx_blk_elmts entries stored directly).
    let num_inline = header.idx_blk_elmts as usize;
    let inline_entries = read_entries(
        &mut cursor,
        num_inline,
        is_filtered,
        offset_size,
        header.element_size,
    )?;

    // 2. Data block addresses stored directly in the index block.
    // The number is 2 * sec_blk_min_data_ptrs (from HDF5: EA_IBLOCK_NDBLK_ADDRS).
    let ndblk_addrs = 2 * header.sec_blk_min_data_ptrs as usize;
    let mut direct_dblk_addrs = Vec::with_capacity(ndblk_addrs);
    for _ in 0..ndblk_addrs {
        direct_dblk_addrs.push(cursor.read_offset(offset_size)?);
    }

    // 3. Secondary block addresses for super blocks 2+.
    // nsblk_addrs = max(0, nsblks - ndblk_addrs) where nsblks is the total
    // number of super blocks needed to cover nelmts.
    // compute_super_block_layout already stops once capacity >= nelmts,
    // so sb_layout.len() is the total number of super blocks needed.
    let nsblks = sb_layout.len();

    let nsblk_addrs = nsblks.saturating_sub(ndblk_addrs);
    let mut sec_block_addrs = Vec::with_capacity(nsblk_addrs);
    for _ in 0..nsblk_addrs {
        sec_block_addrs.push(cursor.read_offset(offset_size)?);
    }

    // Skip checksum at end of index block
    let _checksum = cursor.read_u32_le()?;

    // Now collect all entries.
    let max_entries = total_chunks(dataset_shape, chunk_dims)?;
    let mut all_entries: Vec<EaRawEntry> = Vec::new();

    // Inline entries
    all_entries.extend(inline_entries);

    // Data blocks from direct addresses (super blocks 0-1)
    let mut dblk_addr_idx = 0;
    for sb_idx_iter in 0..2usize.min(nsblks) {
        if sb_idx_iter >= sb_layout.len() {
            break;
        }
        let (elmts_per_dblk, num_dblks) = sb_layout[sb_idx_iter];
        for _ in 0..num_dblks {
            if dblk_addr_idx >= direct_dblk_addrs.len() {
                break;
            }
            let dblk_addr = direct_dblk_addrs[dblk_addr_idx];
            dblk_addr_idx += 1;

            if Cursor::is_undefined_offset(dblk_addr, offset_size) {
                push_undefined(&mut all_entries, elmts_per_dblk, max_entries);
            } else {
                let dblk_entries = parse_data_block(
                    data,
                    dblk_addr,
                    checked_usize_from_u64(
                        elmts_per_dblk,
                        "extensible array data block element count",
                    )?,
                    max_entries.saturating_sub(all_entries.len()),
                    is_filtered,
                    header.max_dblk_page_nelmts_bits,
                    offset_size,
                    header.element_size,
                    sizeof_nelmts,
                )?;
                all_entries.extend(dblk_entries);
            }
        }
    }

    // Data blocks from super blocks 2+ (via secondary blocks)
    for (sec_idx, &sec_addr) in sec_block_addrs.iter().enumerate() {
        let sb_idx_iter = sec_idx + 2;
        if sb_idx_iter >= sb_layout.len() {
            break;
        }
        let (elmts_per_dblk, num_dblks) = sb_layout[sb_idx_iter];

        if Cursor::is_undefined_offset(sec_addr, offset_size) {
            push_undefined(
                &mut all_entries,
                elmts_per_dblk.saturating_mul(num_dblks),
                max_entries,
            );
            continue;
        }

        // Per HDF5 spec III.H "Extensible Array Secondary Block", the secondary
        // block contains a page initialization bitmap when data blocks are paged.
        // Bitmap size = ceil(num_dblks * pages_per_dblk / 8).
        let page_bitmap_bytes = secondary_page_bitmap_bytes(
            header.max_dblk_page_nelmts_bits,
            elmts_per_dblk,
            num_dblks,
        )?;
        let dblk_addrs = parse_secondary_block(
            data,
            sec_addr,
            checked_usize_from_u64(
                num_dblks,
                "extensible array secondary block data block count",
            )?,
            offset_size,
            sizeof_nelmts,
            page_bitmap_bytes,
        )?;

        for &dblk_addr in &dblk_addrs {
            if Cursor::is_undefined_offset(dblk_addr, offset_size) {
                push_undefined(&mut all_entries, elmts_per_dblk, max_entries);
            } else {
                let dblk_entries = parse_data_block(
                    data,
                    dblk_addr,
                    checked_usize_from_u64(
                        elmts_per_dblk,
                        "extensible array data block element count",
                    )?,
                    max_entries.saturating_sub(all_entries.len()),
                    is_filtered,
                    header.max_dblk_page_nelmts_bits,
                    offset_size,
                    header.element_size,
                    sizeof_nelmts,
                )?;
                all_entries.extend(dblk_entries);
            }
        }
    }

    // Convert linear indices to chunk offsets.
    all_entries.truncate(max_entries);
    let ndim = dataset_shape.len();
    let chunks_per_dim = chunks_per_dim(dataset_shape, chunk_dims)?;
    if chunks_per_dim.contains(&0) {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for (linear_idx, raw) in all_entries.iter().enumerate() {
        if Cursor::is_undefined_offset(raw.address, offset_size) {
            continue;
        }

        let mut remaining = linear_idx as u64;
        let mut offsets = vec![0u64; ndim];
        for d in (0..ndim).rev() {
            offsets[d] = (remaining % chunks_per_dim[d])
                .checked_mul(u64::from(chunk_dims[d]))
                .ok_or_else(|| {
                    Error::InvalidData("extensible array chunk offset overflows".into())
                })?;
            remaining /= chunks_per_dim[d];
        }

        if let Some((first_chunk, last_chunk)) = chunk_bounds {
            let overlaps = offsets.iter().enumerate().all(|(dim, offset)| {
                let chunk_index = *offset / u64::from(chunk_dims[dim]);
                chunk_index >= first_chunk[dim] && chunk_index <= last_chunk[dim]
            });
            if !overlaps {
                continue;
            }
        }

        entries.push(ChunkEntry {
            address: raw.address,
            size: raw.chunk_size,
            filter_mask: raw.filter_mask,
            offsets,
        });
    }

    Ok(entries)
}

/// Collect chunk entries from an Extensible Array index using random-access storage.
pub fn collect_extensible_array_chunk_entries_storage(
    storage: &dyn Storage,
    header_address: u64,
    offset_size: u8,
    length_size: u8,
    dataset_shape: &[u64],
    chunk_dims: &[u32],
    chunk_bounds: Option<(&[u64], &[u64])>,
) -> Result<Vec<ChunkEntry>> {
    let header = parse_header_storage(storage, header_address, offset_size, length_size)?;

    // Non-inline EA block geometry is not qualified: the old mapping silently
    // omitted allocated chunks (including default indices 36+). Refuse both
    // full and bounded reads rather than interpreting missing entries as fill.
    if header._nelmts > u64::from(header.idx_blk_elmts) {
        return Err(Error::UnsupportedEncoding(
            "non-inline extensible-array chunk index (unqualified block mapping)".into(),
        ));
    }

    if Cursor::is_undefined_offset(header.index_block_address, offset_size) {
        return Ok(Vec::new());
    }

    let is_filtered = header.client_id == 1;
    let sb_layout = compute_super_block_layout(&header);
    let sizeof_nelmts = (header._max_nelmts_bits as usize).div_ceil(8);

    if let Some(bounds) = chunk_bounds {
        let targets = linear_target_offsets(dataset_shape, chunk_dims, Some(bounds))?;
        let index_block = storage.read_range(
            header.index_block_address,
            index_block_len(&header, &sb_layout, offset_size)?,
        )?;
        let mut cursor = Cursor::new(index_block.as_ref());
        let sig = cursor.read_bytes(4)?;
        if sig != EAIB_SIGNATURE {
            return Err(Error::InvalidExtensibleArraySignature {
                context: "index block signature mismatch",
            });
        }
        let version = cursor.read_u8()?;
        if version != 0 {
            return Err(Error::Other(format!(
                "unsupported extensible array index block version {}",
                version
            )));
        }
        let _client_id = cursor.read_u8()?;
        let _header_address = cursor.read_offset(offset_size)?;
        let num_inline = header.idx_blk_elmts as usize;
        let inline_start = cursor.position();
        cursor.skip(checked_mul_usize(
            num_inline,
            usize::from(header.element_size),
            "extensible array inline entry bytes",
        )?)?;

        let ndblk_addrs = 2 * header.sec_blk_min_data_ptrs as usize;
        let mut direct_dblk_addrs = Vec::with_capacity(ndblk_addrs);
        for _ in 0..ndblk_addrs {
            direct_dblk_addrs.push(cursor.read_offset(offset_size)?);
        }

        let nsblks = sb_layout.len();
        let nsblk_addrs = nsblks.saturating_sub(ndblk_addrs);
        let mut sec_block_addrs = Vec::with_capacity(nsblk_addrs);
        for _ in 0..nsblk_addrs {
            sec_block_addrs.push(cursor.read_offset(offset_size)?);
        }

        let mut secondary_block_cache: Vec<Option<Vec<u64>>> = vec![None; sec_block_addrs.len()];
        let mut entries = Vec::new();

        for (linear_idx, offsets) in targets {
            let raw = if linear_idx < num_inline {
                let inline_offset = checked_add_u64(
                    inline_start,
                    checked_u64_from_usize(
                        checked_mul_usize(
                            linear_idx,
                            usize::from(header.element_size),
                            "extensible array inline entry offset",
                        )?,
                        "extensible array inline entry offset",
                    )?,
                    "extensible array inline entry offset",
                )?;
                let position = checked_add_u64(
                    header.index_block_address,
                    inline_offset,
                    "extensible array inline entry offset",
                )?;
                read_entry_at_storage(
                    storage,
                    position,
                    is_filtered,
                    offset_size,
                    header.element_size,
                )?
            } else {
                let mut relative_idx = (linear_idx - num_inline) as u64;
                let mut sb_idx = None;
                for (candidate_idx, (elmts_per_dblk, num_dblks)) in sb_layout.iter().enumerate() {
                    let capacity = elmts_per_dblk.saturating_mul(*num_dblks);
                    if relative_idx < capacity {
                        sb_idx = Some(candidate_idx);
                        break;
                    }
                    relative_idx -= capacity;
                }

                let Some(sb_idx) = sb_idx else {
                    continue;
                };
                let (elmts_per_dblk, _) = sb_layout[sb_idx];
                let dblk_idx = (relative_idx / elmts_per_dblk) as usize;
                let local_idx = (relative_idx % elmts_per_dblk) as usize;

                let dblk_addr = if sb_idx < 2 {
                    let base = sb_layout[..sb_idx]
                        .iter()
                        .map(|(_, num_dblks)| *num_dblks as usize)
                        .sum::<usize>();
                    *direct_dblk_addrs.get(base + dblk_idx).unwrap_or(&u64::MAX)
                } else {
                    let sec_cache_idx = sb_idx - 2;
                    if secondary_block_cache[sec_cache_idx].is_none() {
                        let sec_addr = sec_block_addrs
                            .get(sec_cache_idx)
                            .copied()
                            .unwrap_or(u64::MAX);
                        if Cursor::is_undefined_offset(sec_addr, offset_size) {
                            secondary_block_cache[sec_cache_idx] = Some(Vec::new());
                        } else {
                            let (_, num_dblks) = sb_layout[sb_idx];
                            let page_bitmap_bytes = secondary_page_bitmap_bytes(
                                header.max_dblk_page_nelmts_bits,
                                elmts_per_dblk,
                                num_dblks,
                            )?;
                            secondary_block_cache[sec_cache_idx] =
                                Some(parse_secondary_block_storage(
                                    storage,
                                    sec_addr,
                                    checked_usize_from_u64(
                                        num_dblks,
                                        "extensible array secondary block data block count",
                                    )?,
                                    offset_size,
                                    sizeof_nelmts,
                                    page_bitmap_bytes,
                                )?);
                        }
                    }

                    secondary_block_cache[sec_cache_idx]
                        .as_ref()
                        .and_then(|addrs| addrs.get(dblk_idx))
                        .copied()
                        .unwrap_or(u64::MAX)
                };

                if Cursor::is_undefined_offset(dblk_addr, offset_size) {
                    continue;
                }

                read_data_block_entry_storage(
                    storage,
                    dblk_addr,
                    checked_usize_from_u64(
                        elmts_per_dblk,
                        "extensible array data block element count",
                    )?,
                    local_idx,
                    is_filtered,
                    header.max_dblk_page_nelmts_bits,
                    offset_size,
                    header.element_size,
                    sizeof_nelmts,
                )?
            };

            if Cursor::is_undefined_offset(raw.address, offset_size) {
                continue;
            }

            entries.push(ChunkEntry {
                address: raw.address,
                size: raw.chunk_size,
                filter_mask: raw.filter_mask,
                offsets,
            });
        }

        return Ok(entries);
    }

    let index_block_len = index_block_len(&header, &sb_layout, offset_size)?;
    let data = storage.read_range(header.index_block_address, index_block_len)?;
    let mut cursor = Cursor::new(data.as_ref());
    cursor.set_position(0);

    let sig = cursor.read_bytes(4)?;
    if sig != EAIB_SIGNATURE {
        return Err(Error::InvalidExtensibleArraySignature {
            context: "index block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported extensible array index block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;

    let num_inline = header.idx_blk_elmts as usize;
    let inline_entries = read_entries(
        &mut cursor,
        num_inline,
        is_filtered,
        offset_size,
        header.element_size,
    )?;

    let ndblk_addrs = 2 * header.sec_blk_min_data_ptrs as usize;
    let mut direct_dblk_addrs = Vec::with_capacity(ndblk_addrs);
    for _ in 0..ndblk_addrs {
        direct_dblk_addrs.push(cursor.read_offset(offset_size)?);
    }

    let nsblks = sb_layout.len();
    let nsblk_addrs = nsblks.saturating_sub(ndblk_addrs);
    let mut sec_block_addrs = Vec::with_capacity(nsblk_addrs);
    for _ in 0..nsblk_addrs {
        sec_block_addrs.push(cursor.read_offset(offset_size)?);
    }
    let _checksum = cursor.read_u32_le()?;

    let max_entries = total_chunks(dataset_shape, chunk_dims)?;
    let mut all_entries: Vec<EaRawEntry> = Vec::new();
    all_entries.extend(inline_entries);

    let mut dblk_addr_idx = 0;
    for sb_idx_iter in 0..2usize.min(nsblks) {
        if sb_idx_iter >= sb_layout.len() {
            break;
        }
        let (elmts_per_dblk, num_dblks) = sb_layout[sb_idx_iter];
        for _ in 0..num_dblks {
            if dblk_addr_idx >= direct_dblk_addrs.len() {
                break;
            }
            let dblk_addr = direct_dblk_addrs[dblk_addr_idx];
            dblk_addr_idx += 1;

            if Cursor::is_undefined_offset(dblk_addr, offset_size) {
                push_undefined(&mut all_entries, elmts_per_dblk, max_entries);
            } else {
                let dblk_entries = read_data_block_storage(
                    storage,
                    dblk_addr,
                    checked_usize_from_u64(
                        elmts_per_dblk,
                        "extensible array data block element count",
                    )?,
                    max_entries.saturating_sub(all_entries.len()),
                    is_filtered,
                    header.max_dblk_page_nelmts_bits,
                    offset_size,
                    header.element_size,
                    sizeof_nelmts,
                )?;
                all_entries.extend(dblk_entries);
            }
        }
    }

    for (sb_idx_iter, &(elmts_per_dblk, num_dblks)) in sb_layout.iter().enumerate().skip(2) {
        let sec_idx = sb_idx_iter - 2;
        let sec_addr = *sec_block_addrs.get(sec_idx).unwrap_or(&u64::MAX);
        if Cursor::is_undefined_offset(sec_addr, offset_size) {
            push_undefined(
                &mut all_entries,
                elmts_per_dblk.saturating_mul(num_dblks),
                max_entries,
            );
            continue;
        }

        let page_bitmap_bytes = secondary_page_bitmap_bytes(
            header.max_dblk_page_nelmts_bits,
            elmts_per_dblk,
            num_dblks,
        )?;
        let dblk_addrs = parse_secondary_block_storage(
            storage,
            sec_addr,
            checked_usize_from_u64(
                num_dblks,
                "extensible array secondary block data block count",
            )?,
            offset_size,
            sizeof_nelmts,
            page_bitmap_bytes,
        )?;

        for dblk_addr in dblk_addrs {
            if Cursor::is_undefined_offset(dblk_addr, offset_size) {
                push_undefined(&mut all_entries, elmts_per_dblk, max_entries);
            } else {
                let dblk_entries = read_data_block_storage(
                    storage,
                    dblk_addr,
                    checked_usize_from_u64(
                        elmts_per_dblk,
                        "extensible array data block element count",
                    )?,
                    max_entries.saturating_sub(all_entries.len()),
                    is_filtered,
                    header.max_dblk_page_nelmts_bits,
                    offset_size,
                    header.element_size,
                    sizeof_nelmts,
                )?;
                all_entries.extend(dblk_entries);
            }
        }
    }

    all_entries.truncate(max_entries);
    let ndim = dataset_shape.len();
    let chunks_per_dim = chunks_per_dim(dataset_shape, chunk_dims)?;
    if chunks_per_dim.contains(&0) {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for (linear_idx, raw) in all_entries.iter().enumerate() {
        if Cursor::is_undefined_offset(raw.address, offset_size) {
            continue;
        }

        let mut remaining = linear_idx as u64;
        let mut offsets = vec![0u64; ndim];
        for d in (0..ndim).rev() {
            offsets[d] = (remaining % chunks_per_dim[d])
                .checked_mul(u64::from(chunk_dims[d]))
                .ok_or_else(|| {
                    Error::InvalidData("extensible array chunk offset overflows".into())
                })?;
            remaining /= chunks_per_dim[d];
        }

        if let Some((first_chunk, last_chunk)) = chunk_bounds {
            let overlaps = offsets.iter().enumerate().all(|(dim, offset)| {
                let chunk_index = *offset / u64::from(chunk_dims[dim]);
                chunk_index >= first_chunk[dim] && chunk_index <= last_chunk[dim]
            });
            if !overlaps {
                continue;
            }
        }

        entries.push(ChunkEntry {
            address: raw.address,
            size: raw.chunk_size,
            filter_mask: raw.filter_mask,
            offsets,
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    #[test]
    fn perf2_ea_non_inline_refuses_full_slab_and_gather() {
        let file =
            crate::hdf5_reader::Hdf5File::from_bytes(include_bytes!("../tests/fixtures/perf2/ea_latest.h5"))
                .unwrap();
        let d = file.dataset("data").unwrap();
        for error in [
            d.read_array::<f32>().unwrap_err(),
            d.read_rows_columns_chunked::<f32>((35, 50, 1), &[0, 100])
                .unwrap_err(),
            d.read_array_uncached::<f32>().unwrap_err(),
        ] {
            assert!(error
                .to_string()
                .contains("unsupported non-inline extensible-array"));
        }
    }

    use super::*;
    use crate::hdf5_reader::storage::{Storage, StorageBuffer};
    use std::sync::Mutex;

    const TEST_OFFSET_SIZE: u8 = 8;
    const TEST_LENGTH_SIZE: u8 = 8;
    const TEST_HEADER_ADDR: u64 = 0;
    const TEST_INDEX_ADDR: u64 = 128;
    const TEST_SECONDARY_ADDR: u64 = 512;
    const TEST_DATA_BLOCK_ADDR: u64 = 768;

    struct RecordingStorage {
        data: Vec<u8>,
        ranges: Mutex<Vec<(u64, usize)>>,
    }

    impl RecordingStorage {
        fn new(data: Vec<u8>) -> Self {
            Self {
                data,
                ranges: Mutex::new(Vec::new()),
            }
        }

        fn ranges(&self) -> Vec<(u64, usize)> {
            self.ranges.lock().unwrap().clone()
        }

        fn clear_ranges(&self) {
            self.ranges.lock().unwrap().clear();
        }
    }

    impl Storage for RecordingStorage {
        fn len(&self) -> u64 {
            self.data.len() as u64
        }

        fn read_range(&self, offset: u64, len: usize) -> Result<StorageBuffer> {
            let start = usize::try_from(offset).map_err(|_| Error::OffsetOutOfBounds(offset))?;
            let end = start.checked_add(len).ok_or(Error::UnexpectedEof {
                offset,
                needed: len as u64,
                available: self.data.len().saturating_sub(start) as u64,
            })?;
            if end > self.data.len() {
                return Err(Error::UnexpectedEof {
                    offset,
                    needed: len as u64,
                    available: self.data.len().saturating_sub(start) as u64,
                });
            }
            self.ranges.lock().unwrap().push((offset, len));
            Ok(StorageBuffer::from_vec(self.data[start..end].to_vec()))
        }
    }

    fn put(data: &mut [u8], offset: u64, bytes: &[u8]) {
        let start = offset as usize;
        data[start..start + bytes.len()].copy_from_slice(bytes);
    }

    fn push_u64(buf: &mut Vec<u8>, value: u64) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    fn push_offset(buf: &mut Vec<u8>, value: u64) {
        push_u64(buf, value);
    }

    fn ea_header_bytes(nelmts: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&EAHD_SIGNATURE);
        buf.push(0); // version
        buf.push(0); // unfiltered raw chunk client
        buf.push(8); // element size: address only
        buf.push(8); // max_nelmts_bits, so sizeof_nelmts is 1
        buf.push(0); // inline elements
        buf.push(1); // minimum data block elements
        buf.push(1); // minimum secondary block data pointers
        buf.push(0); // unpaged data blocks
        for value in [0, 0, 0, 0, 0, nelmts] {
            push_u64(&mut buf, value);
        }
        push_offset(&mut buf, TEST_INDEX_ADDR);
        let checksum = jenkins_lookup3(&buf);
        buf.extend_from_slice(&checksum.to_le_bytes());
        buf
    }

    fn index_block_bytes() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&EAIB_SIGNATURE);
        buf.push(0); // version
        buf.push(0); // client id
        push_offset(&mut buf, TEST_HEADER_ADDR);
        push_offset(&mut buf, u64::MAX); // first direct data block
        push_offset(&mut buf, u64::MAX); // second direct data block
        push_offset(&mut buf, TEST_SECONDARY_ADDR);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf
    }

    fn secondary_block_bytes() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&EASB_SIGNATURE);
        buf.push(0); // version
        buf.push(0); // client id
        push_offset(&mut buf, TEST_HEADER_ADDR);
        buf.push(0); // block offset, sizeof_nelmts = 1
        push_offset(&mut buf, TEST_DATA_BLOCK_ADDR);
        push_offset(&mut buf, u64::MAX);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf
    }

    fn data_block_bytes() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&EADB_SIGNATURE);
        buf.push(0); // version
        buf.push(0); // client id
        push_offset(&mut buf, TEST_HEADER_ADDR);
        buf.push(0); // block offset, sizeof_nelmts = 1
        push_offset(&mut buf, 0x2000);
        push_offset(&mut buf, 0x2010);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf
    }

    fn storage_fixture() -> RecordingStorage {
        let mut data = vec![0u8; 4096];
        put(&mut data, TEST_HEADER_ADDR, &ea_header_bytes(7));
        put(&mut data, TEST_INDEX_ADDR, &index_block_bytes());
        put(&mut data, TEST_SECONDARY_ADDR, &secondary_block_bytes());
        put(&mut data, TEST_DATA_BLOCK_ADDR, &data_block_bytes());
        RecordingStorage::new(data)
    }

    #[test]
    fn eahd_bad_signature() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"XXXX");
        let err = parse_header(&data, 0, 8, 8).unwrap_err();
        assert!(matches!(err, Error::InvalidExtensibleArraySignature { .. }));
    }

    #[test]
    fn super_block_layout_matches_spec_rows() {
        let header = EaHeader {
            client_id: 0,
            element_size: 8,
            _max_nelmts_bits: 32,
            idx_blk_elmts: 2,
            data_blk_min_elmts: 2,
            sec_blk_min_data_ptrs: 2,
            max_dblk_page_nelmts_bits: 0,
            _nelmts: 100,
            index_block_address: 0,
        };
        let layout = compute_super_block_layout(&header);
        // sb 0: elmts_per_dblk = 2 * 2^0 = 2, num_dblks = 2 * 2^0 = 2  (cap = 4 elements)
        assert_eq!(layout[0], (2, 2));
        // sb 1: elmts_per_dblk = 2 * 2^0 = 2, num_dblks = 2 * 2^1 = 4  (cap = 8 elements)
        assert_eq!(layout[1], (2, 4));
        // sb 2: elmts_per_dblk = 2 * 2^1 = 4, num_dblks = 2 * 2^1 = 4  (cap = 16 elements)
        assert_eq!(layout[2], (4, 4));
        // sb 3: elmts_per_dblk = 2 * 2^1 = 4, num_dblks = 2 * 2^2 = 8  (cap = 32 elements)
        assert_eq!(layout[3], (4, 8));
    }

    #[test]
    fn non_inline_storage_scans_refuse_before_reading_index_blocks() {
        let storage = storage_fixture();
        for bounds in [None, Some((&[3u64][..], &[3u64][..]))] {
            storage.clear_ranges();
            let error = collect_extensible_array_chunk_entries_storage(
                &storage,
                TEST_HEADER_ADDR,
                TEST_OFFSET_SIZE,
                TEST_LENGTH_SIZE,
                &[7],
                &[1],
                bounds,
            )
            .unwrap_err();
            assert!(error
                .to_string()
                .contains("unsupported non-inline extensible-array"));
            assert_eq!(storage.ranges(), vec![(TEST_HEADER_ADDR, 72)]);
        }
    }
}
