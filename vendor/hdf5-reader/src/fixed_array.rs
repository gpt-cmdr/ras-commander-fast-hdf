//! HDF5 Fixed Array (FA) chunk index.
//!
//! This is the default chunk index for fixed-size chunked datasets created with
//! `libver='latest'`. It stores chunk entries in a flat array (optionally paged)
//! with a single-level header → data block structure.
//!
//! Structures:
//! - `FAHD` — Fixed Array Header
//! - `FADB` — Fixed Array Data Block

use crate::checksum::jenkins_lookup3;
use crate::chunk_index::ChunkEntry;
use crate::error::{Error, Result};
use crate::io::Cursor;
use crate::storage::Storage;

const FAHD_SIGNATURE: [u8; 4] = *b"FAHD";
const FADB_SIGNATURE: [u8; 4] = *b"FADB";

/// Parsed Fixed Array Header.
#[derive(Debug)]
struct FaHeader {
    client_id: u8,
    entry_size: u8,
    page_bits: u8,
    num_entries: u64,
    data_block_address: u64,
}

/// Parse the Fixed Array Header at the given address.
///
/// On-disk layout (from H5FA_HEADER_SIZE):
/// sig(4) + ver(1) + client_id(1) + entry_size(1) + page_bits(1)
/// + nelmts(length_size) + dblk_addr(offset_size) + checksum(4)
fn parse_header(data: &[u8], address: u64, offset_size: u8, length_size: u8) -> Result<FaHeader> {
    let mut cursor = Cursor::new(data);
    cursor.set_position(address);

    let sig = cursor.read_bytes(4)?;
    if sig != FAHD_SIGNATURE {
        return Err(Error::InvalidFixedArraySignature {
            context: "header signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported fixed array header version {}",
            version
        )));
    }

    let client_id = cursor.read_u8()?;
    let entry_size = cursor.read_u8()?;
    let page_bits = cursor.read_u8()?;
    let num_entries = cursor.read_length(length_size)?;
    let data_block_address = cursor.read_offset(offset_size)?;

    // Checksum covers everything from signature through data_block_address.
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

    Ok(FaHeader {
        client_id,
        entry_size,
        page_bits,
        num_entries,
        data_block_address,
    })
}

fn parse_header_storage(
    storage: &dyn Storage,
    address: u64,
    offset_size: u8,
    length_size: u8,
) -> Result<FaHeader> {
    let header_len = 4 + 1 + 1 + 1 + 1 + usize::from(length_size) + usize::from(offset_size) + 4;
    let bytes = storage.read_range(address, header_len)?;
    parse_header(bytes.as_ref(), 0, offset_size, length_size)
}

/// A single raw fixed-array entry (before conversion to ChunkEntry).
#[derive(Debug)]
struct FaRawEntry {
    address: u64,
    chunk_size: u64,
    filter_mask: u32,
}

fn size_error(what: &str) -> Error {
    Error::InvalidData(format!("fixed array {what} overflows"))
}

fn add(a: usize, b: usize, what: &str) -> Result<usize> {
    a.checked_add(b).ok_or_else(|| size_error(what))
}

fn mul(a: usize, b: usize, what: &str) -> Result<usize> {
    a.checked_mul(b).ok_or_else(|| size_error(what))
}

fn add_u64(a: u64, b: usize, what: &str) -> Result<u64> {
    u64::try_from(b)
        .ok()
        .and_then(|b| a.checked_add(b))
        .ok_or_else(|| size_error(what))
}

/// Sizes of a Fixed Array data block derived from its header with checked
/// arithmetic. Every count and offset taken from the file goes through here,
/// so a crafted entry count or page size cannot wrap.
struct FaGeometry {
    num_entries: usize,
    entry_bytes: usize,
    /// Entries per page when the data block is paged.
    entries_per_page: Option<usize>,
}

impl FaGeometry {
    fn new(header: &FaHeader, offset_size: u8) -> Result<Self> {
        let num_entries = usize::try_from(header.num_entries).map_err(|_| {
            Error::InvalidData("fixed array entry count exceeds platform usize capacity".into())
        })?;
        let entry_bytes = usize::from(header.entry_size);
        if entry_bytes < usize::from(offset_size) {
            return Err(Error::InvalidData(
                "fixed array entry size is smaller than an address".into(),
            ));
        }
        // Paging is used only when nelmts exceeds 2^page_bits. A page size
        // beyond usize can never be exceeded, so such an array is not paged.
        let entries_per_page = match header.page_bits {
            0 => None,
            bits => 1usize
                .checked_shl(u32::from(bits))
                .filter(|&per_page| num_entries > per_page),
        };
        Ok(Self {
            num_entries,
            entry_bytes,
            entries_per_page,
        })
    }

    fn num_pages(&self) -> usize {
        self.entries_per_page
            .map_or(0, |per_page| self.num_entries.div_ceil(per_page))
    }

    fn bitmap_bytes(&self) -> usize {
        self.num_pages().div_ceil(8)
    }

    fn entries_in_page(&self, page_idx: usize) -> usize {
        let per_page = self.entries_per_page.unwrap_or(self.num_entries);
        if page_idx + 1 == self.num_pages() {
            match self.num_entries % per_page {
                0 => per_page,
                remainder => remainder,
            }
        } else {
            per_page
        }
    }

    /// Bytes of `entries` entries plus the trailing checksum.
    fn run_bytes(&self, entries: usize) -> Result<usize> {
        add(mul(entries, self.entry_bytes, "page size")?, 4, "page size")
    }

    /// Total data block length, including every page, in closed form.
    fn block_len(&self, header_len: usize) -> Result<usize> {
        match self.entries_per_page {
            None => add(
                header_len,
                self.run_bytes(self.num_entries)?,
                "data block size",
            ),
            Some(per_page) => {
                let pages = self.num_pages();
                let full = mul(pages - 1, self.run_bytes(per_page)?, "data block size")?;
                let last = self.run_bytes(self.entries_in_page(pages - 1))?;
                let len = add(header_len, self.bitmap_bytes(), "data block size")?;
                add(add(len, full, "data block size")?, last, "data block size")
            }
        }
    }

    /// Byte offset of entry `index` within a run of entries.
    fn entry_offset(&self, index: usize) -> Result<usize> {
        mul(index, self.entry_bytes, "entry offset")
    }

    fn check_index(&self, linear_idx: usize) -> Result<()> {
        if linear_idx >= self.num_entries {
            return Err(Error::InvalidData(format!(
                "chunk {linear_idx} is outside the fixed array's {} entries",
                self.num_entries
            )));
        }
        Ok(())
    }
}

/// Read entries from a Fixed Array Data Block.
fn parse_data_block(
    data: &[u8],
    address: u64,
    header: &FaHeader,
    offset_size: u8,
) -> Result<Vec<FaRawEntry>> {
    let mut cursor = Cursor::new(data);
    cursor.set_position(address);

    let sig = cursor.read_bytes(4)?;
    if sig != FADB_SIGNATURE {
        return Err(Error::InvalidFixedArraySignature {
            context: "data block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported fixed array data block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;

    let geometry = FaGeometry::new(header, offset_size)?;
    let is_filtered = header.client_id == 1;

    // The whole block must be present before anything is sized from it, so
    // the entry vector is bounded by the bytes actually in the file.
    let header_len = 4 + 1 + 1 + usize::from(offset_size);
    let block_len = geometry.block_len(header_len)?;
    let available = (data.len() as u64).saturating_sub(address);
    if block_len as u64 > available {
        return Err(Error::UnexpectedEof {
            offset: address,
            needed: block_len as u64,
            available,
        });
    }

    if geometry.entries_per_page.is_none() {
        // Non-paged: all entries inline followed by a single checksum.
        let entries = read_entries(
            &mut cursor,
            geometry.num_entries,
            is_filtered,
            offset_size,
            header.entry_size,
        )?;
        // Skip the trailing checksum (already verified structurally).
        let _checksum = cursor.read_u32_le()?;
        return Ok(entries);
    }

    // Paged: entries are split into pages of `2^page_bits` entries each.
    // The page init bitmap tells which pages have been initialized; the
    // others have undefined addresses that are filtered out later.
    let num_pages = geometry.num_pages();
    let page_bitmap = cursor.read_bytes(geometry.bitmap_bytes())?.to_vec();

    let mut all_entries = Vec::with_capacity(geometry.num_entries);

    for page_idx in 0..num_pages {
        let byte_idx = page_idx / 8;
        let bit_idx = page_idx % 8;
        let page_initialized =
            byte_idx < page_bitmap.len() && (page_bitmap[byte_idx] & (1 << bit_idx)) != 0;
        let entries_in_this_page = geometry.entries_in_page(page_idx);

        if page_initialized {
            let page_entries = read_entries(
                &mut cursor,
                entries_in_this_page,
                is_filtered,
                offset_size,
                header.entry_size,
            )?;
            // Each page has its own checksum.
            let _page_checksum = cursor.read_u32_le()?;
            all_entries.extend(page_entries);
        } else {
            // Uninitialized page — fill with undefined entries.
            all_entries.extend((0..entries_in_this_page).map(|_| FaRawEntry {
                address: u64::MAX,
                chunk_size: 0,
                filter_mask: 0,
            }));
        }
    }

    Ok(all_entries)
}

/// Read `count` entries from the cursor.
fn read_entries(
    cursor: &mut Cursor<'_>,
    count: usize,
    is_filtered: bool,
    offset_size: u8,
    entry_size: u8,
) -> Result<Vec<FaRawEntry>> {
    // Never size the vector beyond what the remaining bytes can hold.
    let needed = mul(count, usize::from(entry_size).max(1), "entry run")?;
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
                .ok_or_else(|| Error::InvalidData("invalid fixed array entry size".into()))?;
            // 1–8 bytes wide (sized to the chunk), not a "length" of the superblock size.
            let cs = cursor.read_uvar(chunk_size_len as usize)?;
            let fm = cursor.read_u32_le()?;
            (cs, fm)
        } else {
            (0, 0)
        };
        entries.push(FaRawEntry {
            address,
            chunk_size,
            filter_mask,
        });
    }
    Ok(entries)
}

fn read_entry_at(
    data: &[u8],
    position: u64,
    is_filtered: bool,
    offset_size: u8,
    entry_size: u8,
) -> Result<FaRawEntry> {
    let mut cursor = Cursor::new(data);
    cursor.set_position(position);
    let mut entries = read_entries(&mut cursor, 1, is_filtered, offset_size, entry_size)?;
    entries
        .pop()
        .ok_or_else(|| Error::InvalidData("missing fixed array entry".into()))
}

fn read_entry_at_storage(
    storage: &dyn Storage,
    position: u64,
    is_filtered: bool,
    offset_size: u8,
    entry_size: u8,
) -> Result<FaRawEntry> {
    let bytes = storage.read_range(position, usize::from(entry_size))?;
    let mut cursor = Cursor::new(bytes.as_ref());
    let mut entries = read_entries(&mut cursor, 1, is_filtered, offset_size, entry_size)?;
    entries
        .pop()
        .ok_or_else(|| Error::InvalidData("missing fixed array entry".into()))
}

fn chunks_per_dim(dataset_shape: &[u64], chunk_dims: &[u32]) -> Result<Vec<u64>> {
    if chunk_dims.len() < dataset_shape.len() {
        return Err(Error::InvalidData(
            "fixed array chunk rank is smaller than the dataset rank".into(),
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
            "fixed array chunk bounds do not match the dataset rank".into(),
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
                .ok_or_else(|| size_error("chunk index"))?;
        }
        let offsets = chunk_indices
            .iter()
            .enumerate()
            .map(|(dim, chunk_index)| {
                chunk_index
                    .checked_mul(u64::from(chunk_dims[dim]))
                    .ok_or_else(|| size_error("chunk offset"))
            })
            .collect::<Result<Vec<u64>>>()?;
        let linear_idx = usize::try_from(linear_idx).map_err(|_| size_error("chunk index"))?;
        targets.push((linear_idx, offsets));

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

/// Start of each initialized page; pages follow the bitmap at `pages_start`.
fn page_offsets(
    geometry: &FaGeometry,
    page_bitmap: &[u8],
    pages_start: u64,
) -> Result<Vec<Option<u64>>> {
    let mut page_offsets = vec![None; geometry.num_pages()];
    let mut next_page_start = pages_start;
    for (page_idx, page_offset) in page_offsets.iter_mut().enumerate() {
        let byte_idx = page_idx / 8;
        let bit_idx = page_idx % 8;
        let page_initialized =
            byte_idx < page_bitmap.len() && (page_bitmap[byte_idx] & (1 << bit_idx)) != 0;

        if page_initialized {
            *page_offset = Some(next_page_start);
            let page_len = geometry.run_bytes(geometry.entries_in_page(page_idx))?;
            next_page_start = add_u64(next_page_start, page_len, "page offset")?;
        }
    }
    Ok(page_offsets)
}

/// Position of entry `linear_idx`, or `None` for an uninitialized page.
fn entry_position(
    geometry: &FaGeometry,
    entries_start: u64,
    page_offsets: &[Option<u64>],
    linear_idx: usize,
) -> Result<Option<u64>> {
    geometry.check_index(linear_idx)?;
    match geometry.entries_per_page {
        None => Ok(Some(add_u64(
            entries_start,
            geometry.entry_offset(linear_idx)?,
            "entry offset",
        )?)),
        Some(per_page) => {
            let Some(page_start) = page_offsets.get(linear_idx / per_page).copied().flatten()
            else {
                return Ok(None);
            };
            let within = geometry.entry_offset(linear_idx % per_page)?;
            Ok(Some(add_u64(page_start, within, "entry offset")?))
        }
    }
}

fn collect_fixed_array_chunk_entries_bounded(
    data: &[u8],
    header: &FaHeader,
    offset_size: u8,
    dataset_shape: &[u64],
    chunk_dims: &[u32],
    chunk_bounds: (&[u64], &[u64]),
) -> Result<Vec<ChunkEntry>> {
    let targets = linear_target_offsets(dataset_shape, chunk_dims, Some(chunk_bounds))?;
    let mut cursor = Cursor::new(data);
    cursor.set_position(header.data_block_address);

    let sig = cursor.read_bytes(4)?;
    if sig != FADB_SIGNATURE {
        return Err(Error::InvalidFixedArraySignature {
            context: "data block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported fixed array data block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;

    let geometry = FaGeometry::new(header, offset_size)?;
    let is_filtered = header.client_id == 1;

    let page_offsets = if geometry.entries_per_page.is_some() {
        let page_bitmap = cursor.read_bytes(geometry.bitmap_bytes())?.to_vec();
        page_offsets(&geometry, &page_bitmap, cursor.position())?
    } else {
        Vec::new()
    };
    let entries_start = cursor.position();

    let mut entries = Vec::new();
    for (linear_idx, offsets) in targets {
        let Some(position) = entry_position(&geometry, entries_start, &page_offsets, linear_idx)?
        else {
            continue;
        };
        let raw = read_entry_at(data, position, is_filtered, offset_size, header.entry_size)?;
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

fn collect_fixed_array_chunk_entries_bounded_storage(
    storage: &dyn Storage,
    header: &FaHeader,
    offset_size: u8,
    dataset_shape: &[u64],
    chunk_dims: &[u32],
    chunk_bounds: (&[u64], &[u64]),
) -> Result<Vec<ChunkEntry>> {
    let targets = linear_target_offsets(dataset_shape, chunk_dims, Some(chunk_bounds))?;
    let block_header_len = 4 + 1 + 1 + usize::from(offset_size);
    let header_bytes = storage.read_range(header.data_block_address, block_header_len)?;
    let mut cursor = Cursor::new(header_bytes.as_ref());

    let sig = cursor.read_bytes(4)?;
    if sig != FADB_SIGNATURE {
        return Err(Error::InvalidFixedArraySignature {
            context: "data block signature mismatch",
        });
    }

    let version = cursor.read_u8()?;
    if version != 0 {
        return Err(Error::Other(format!(
            "unsupported fixed array data block version {}",
            version
        )));
    }

    let _client_id = cursor.read_u8()?;
    let _header_address = cursor.read_offset(offset_size)?;

    let geometry = FaGeometry::new(header, offset_size)?;
    let is_filtered = header.client_id == 1;
    let entries_start = add_u64(
        header.data_block_address,
        block_header_len,
        "data block address",
    )?;

    let (entries_start, page_offsets) = if geometry.entries_per_page.is_some() {
        let bitmap_bytes = geometry.bitmap_bytes();
        let page_bitmap = storage.read_range(entries_start, bitmap_bytes)?;
        let pages_start = add_u64(entries_start, bitmap_bytes, "page bitmap")?;
        let offsets = page_offsets(&geometry, page_bitmap.as_ref(), pages_start)?;
        (pages_start, offsets)
    } else {
        (entries_start, Vec::new())
    };

    let mut entries = Vec::new();
    for (linear_idx, offsets) in targets {
        let Some(position) = entry_position(&geometry, entries_start, &page_offsets, linear_idx)?
        else {
            continue;
        };
        let raw = read_entry_at_storage(
            storage,
            position,
            is_filtered,
            offset_size,
            header.entry_size,
        )?;
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

/// Convert defined raw entries to chunk entries, in linear index order.
/// `is_filtered == Some(false)` reports unfiltered chunks with size 0.
fn raw_entries_to_chunks(
    raw_entries: &[FaRawEntry],
    offset_size: u8,
    dataset_shape: &[u64],
    chunk_dims: &[u32],
    is_filtered: Option<bool>,
) -> Result<Vec<ChunkEntry>> {
    let ndim = dataset_shape.len();
    let chunks_per_dim = chunks_per_dim(dataset_shape, chunk_dims)?;
    if chunks_per_dim.contains(&0) {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for (linear_idx, raw) in raw_entries.iter().enumerate() {
        // Skip undefined addresses (unallocated chunks).
        if Cursor::is_undefined_offset(raw.address, offset_size) {
            continue;
        }

        // Convert linear index to multi-dimensional chunk offsets.
        let mut remaining = linear_idx as u64;
        let mut offsets = vec![0u64; ndim];
        for d in (0..ndim).rev() {
            offsets[d] = (remaining % chunks_per_dim[d])
                .checked_mul(u64::from(chunk_dims[d]))
                .ok_or_else(|| size_error("chunk offset"))?;
            remaining /= chunks_per_dim[d];
        }

        entries.push(ChunkEntry {
            address: raw.address,
            size: match is_filtered {
                Some(false) => 0,
                _ => raw.chunk_size,
            },
            filter_mask: raw.filter_mask,
            offsets,
        });
    }

    Ok(entries)
}

/// Collect chunk entries from a Fixed Array index.
///
/// Reads the FAHD header and FADB data block, then converts linear entry
/// indices to multi-dimensional chunk offsets.
pub fn collect_fixed_array_chunk_entries(
    data: &[u8],
    header_address: u64,
    offset_size: u8,
    length_size: u8,
    dataset_shape: &[u64],
    chunk_dims: &[u32],
    chunk_bounds: Option<(&[u64], &[u64])>,
) -> Result<Vec<ChunkEntry>> {
    let header = parse_header(data, header_address, offset_size, length_size)?;

    if Cursor::is_undefined_offset(header.data_block_address, offset_size) {
        return Ok(Vec::new());
    }

    if let Some(bounds) = chunk_bounds {
        return collect_fixed_array_chunk_entries_bounded(
            data,
            &header,
            offset_size,
            dataset_shape,
            chunk_dims,
            bounds,
        );
    }

    let raw_entries = parse_data_block(data, header.data_block_address, &header, offset_size)?;
    raw_entries_to_chunks(&raw_entries, offset_size, dataset_shape, chunk_dims, None)
}

/// Collect chunk entries from a Fixed Array index using random-access storage.
pub fn collect_fixed_array_chunk_entries_storage(
    storage: &dyn Storage,
    header_address: u64,
    offset_size: u8,
    length_size: u8,
    dataset_shape: &[u64],
    chunk_dims: &[u32],
    chunk_bounds: Option<(&[u64], &[u64])>,
) -> Result<Vec<ChunkEntry>> {
    let header = parse_header_storage(storage, header_address, offset_size, length_size)?;

    if Cursor::is_undefined_offset(header.data_block_address, offset_size) {
        return Ok(Vec::new());
    }

    if let Some(bounds) = chunk_bounds {
        return collect_fixed_array_chunk_entries_bounded_storage(
            storage,
            &header,
            offset_size,
            dataset_shape,
            chunk_dims,
            bounds,
        );
    }

    let geometry = FaGeometry::new(&header, offset_size)?;
    let is_filtered = header.client_id == 1;
    let header_len = 4 + 1 + 1 + usize::from(offset_size);
    // Checked closed form: a wrapped length would read a short block and then
    // size the entry vector from the unwrapped count.
    let block_len = geometry.block_len(header_len)?;
    let block = storage.read_range(header.data_block_address, block_len)?;
    let raw_entries = parse_data_block(block.as_ref(), 0, &header, offset_size)?;
    raw_entries_to_chunks(
        &raw_entries,
        offset_size,
        dataset_shape,
        chunk_dims,
        Some(is_filtered),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(entry_size: u8, page_bits: u8, num_entries: u64) -> FaHeader {
        FaHeader {
            client_id: 0,
            entry_size,
            page_bits,
            num_entries,
            data_block_address: 0,
        }
    }

    #[test]
    fn block_len_overflow_is_an_error_not_a_wrap() {
        // 2^61 entries of 8 bytes wraps a 64-bit length to 0.
        let geometry = FaGeometry::new(&header(8, 0, 1 << 61), 8).unwrap();
        assert!(geometry.block_len(14).is_err());
        // Paged: 2^62 entries in 2^10-entry pages is a closed form, no loop.
        let geometry = FaGeometry::new(&header(8, 10, 1 << 62), 8).unwrap();
        assert!(geometry.block_len(14).is_err());
    }

    #[test]
    fn paged_block_len_matches_the_page_walk() {
        for (num_entries, page_bits) in [(1025u64, 10u8), (2048, 10), (5000, 9), (7, 2)] {
            let geometry = FaGeometry::new(&header(8, page_bits, num_entries), 8).unwrap();
            let mut walked = 14 + geometry.bitmap_bytes();
            for page in 0..geometry.num_pages() {
                walked += geometry.entries_in_page(page) * 8 + 4;
            }
            assert_eq!(geometry.block_len(14).unwrap(), walked);
        }
    }

    #[test]
    fn page_bits_beyond_usize_mean_unpaged() {
        let geometry = FaGeometry::new(&header(8, 200, 4), 8).unwrap();
        assert!(geometry.entries_per_page.is_none());
        assert_eq!(geometry.block_len(14).unwrap(), 14 + 4 * 8 + 4);
    }

    #[test]
    fn entry_count_beyond_the_block_is_refused_before_allocation() {
        let mut cursor = Cursor::new(&[0u8; 16]);
        assert!(read_entries(&mut cursor, usize::MAX / 2, false, 8, 8).is_err());
    }

    #[test]
    fn filtered_entries_variable_width_sizes() {
        for width in 1u8..=8 {
            let mut bytes = Vec::new();
            let sizes = [u64::MAX >> (8 * (8 - width)), 1];
            for (index, size) in sizes.iter().enumerate() {
                bytes.extend_from_slice(&(0x1234u64 + index as u64).to_le_bytes());
                bytes.extend_from_slice(&size.to_le_bytes()[..usize::from(width)]);
                bytes.extend_from_slice(&(0x1020_3040u32 + index as u32).to_le_bytes());
            }
            let mut cursor = Cursor::new(&bytes);
            let entries = read_entries(&mut cursor, 2, true, 8, 12 + width).unwrap();
            for (index, entry) in entries.iter().enumerate() {
                assert_eq!(entry.address, 0x1234 + index as u64);
                assert_eq!(entry.chunk_size, sizes[index], "width {width}");
                assert_eq!(entry.filter_mask, 0x1020_3040 + index as u32);
            }
            assert_eq!(cursor.position(), bytes.len() as u64);
        }
    }

    #[test]
    fn filtered_entries_reject_invalid_size_widths() {
        for width in [0u8, 9] {
            let bytes = [0u8; 32];
            let mut cursor = Cursor::new(&bytes);
            let error = read_entries(&mut cursor, 1, true, 8, 12 + width).unwrap_err();
            assert!(matches!(error, Error::InvalidData(_)));
        }
    }

    #[test]
    fn filtered_entries_reject_truncated_size_and_mask() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x1234u64.to_le_bytes());
        bytes.extend_from_slice(&[0x56, 0x34, 0x12]);
        bytes.extend_from_slice(&0x1020_3040u32.to_le_bytes());
        for length in [8, 9, 10, 11, 12, 13, 14] {
            let mut cursor = Cursor::new(&bytes[..length]);
            assert!(read_entries(&mut cursor, 1, true, 8, 15).is_err());
        }
    }

    #[test]
    fn fahd_bad_signature() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"XXXX");
        let err = parse_header(&data, 0, 8, 8).unwrap_err();
        assert!(matches!(err, Error::InvalidFixedArraySignature { .. }));
    }

    #[test]
    fn fadb_bad_signature() {
        let header = FaHeader {
            client_id: 0,
            entry_size: 8,
            page_bits: 0,
            num_entries: 1,
            data_block_address: 0,
        };
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"XXXX");
        let err = parse_data_block(&data, 0, &header, 8).unwrap_err();
        assert!(matches!(err, Error::InvalidFixedArraySignature { .. }));
    }
}
