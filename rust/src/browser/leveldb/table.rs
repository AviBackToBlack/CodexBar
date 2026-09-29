//! LevelDB sorted-table (`*.ldb` / `*.sst`) reader.
//!
//! Layout: data blocks, a metaindex block, an index block, and a 48 byte footer that ends with a
//! magic number. The footer holds block handles (varint offset, varint size) for the metaindex and
//! index blocks; index entries map a separator key to the handle of a data block. Every block is
//! followed by a 5 byte trailer (compression type, CRC32C). Entries inside a block share key
//! prefixes with the previous entry, and a restart array closes the block.
//!
//! Only the index and data blocks are read; the filter/metaindex blocks are irrelevant to a full
//! scan. Data-block keys are internal keys: the user key followed by `sequence << 8 | kind`.
//! Checksums are not verified (see `log.rs`); structural checks keep malformed input from
//! panicking or over-allocating.

use super::snappy;
use super::varint::{read_u32_le, read_u64_le, read_varint32, read_varint64};
use super::{MAX_BLOCK_BYTES, Record};

const FOOTER_SIZE: usize = 48;
const TABLE_MAGIC: u64 = 0xdb47_7524_8b80_fb57;
const BLOCK_TRAILER_SIZE: usize = 5;

const COMPRESSION_NONE: u8 = 0;
const COMPRESSION_SNAPPY: u8 = 1;

const KIND_DELETE: u64 = 0;
const KIND_VALUE: u64 = 1;

#[derive(Debug, thiserror::Error)]
pub(super) enum TableError {
    #[error("file is too short or has a bad table footer")]
    BadFooter,
    #[error("block handle points outside the file")]
    BadHandle,
    #[error("unsupported block compression type {0}")]
    UnsupportedCompression(u8),
    #[error("block is malformed")]
    BadBlock,
    #[error("snappy block failed to decode: {0}")]
    Snappy(#[from] snappy::SnappyError),
}

#[derive(Clone, Copy)]
struct BlockHandle {
    offset: usize,
    size: usize,
}

impl BlockHandle {
    /// Parse a handle from the front of `input`; returns it with the bytes consumed.
    fn parse(input: &[u8]) -> Option<(Self, usize)> {
        let (offset, first) = read_varint64(input)?;
        let (size, second) = read_varint64(input.get(first..)?)?;
        Some((
            Self {
                offset: usize::try_from(offset).ok()?,
                size: usize::try_from(size).ok()?,
            },
            first + second,
        ))
    }
}

/// Feed every put/delete found in the table `data` to `emit`.
///
/// A data block that cannot be decoded is skipped (and counted in the returned failure count) so
/// one bad block does not hide the rest of the table; a bad footer or index block is an error.
pub(super) fn read_table(data: &[u8], emit: &mut impl FnMut(Record)) -> Result<usize, TableError> {
    let footer_start = data
        .len()
        .checked_sub(FOOTER_SIZE)
        .ok_or(TableError::BadFooter)?;
    let footer = &data[footer_start..];
    if read_u64_le(footer, FOOTER_SIZE - 8) != Some(TABLE_MAGIC) {
        return Err(TableError::BadFooter);
    }
    let (_metaindex, used) = BlockHandle::parse(footer).ok_or(TableError::BadFooter)?;
    let (index, _) = BlockHandle::parse(&footer[used..]).ok_or(TableError::BadFooter)?;

    let index_block = read_block(data, index)?;
    let mut skipped_blocks = 0usize;
    for (_separator, handle_bytes) in BlockEntries::new(&index_block)? {
        let entry = BlockHandle::parse(handle_bytes)
            .map(|(handle, _)| handle)
            .ok_or(TableError::BadBlock);
        let block = entry.and_then(|handle| read_block(data, handle));
        let Ok(block) = block else {
            skipped_blocks += 1;
            continue;
        };
        if emit_block_records(&block, emit).is_err() {
            skipped_blocks += 1;
        }
    }
    Ok(skipped_blocks)
}

fn emit_block_records(block: &[u8], emit: &mut impl FnMut(Record)) -> Result<(), TableError> {
    for (internal_key, value) in BlockEntries::new(block)? {
        let Some(split) = internal_key.len().checked_sub(8) else {
            return Err(TableError::BadBlock);
        };
        let (user_key, trailer) = internal_key.split_at(split);
        let packed = read_u64_le(trailer, 0).ok_or(TableError::BadBlock)?;
        let value = match packed & 0xff {
            KIND_VALUE => Some(value.to_vec()),
            KIND_DELETE => None,
            _ => continue,
        };
        emit(Record {
            key: user_key.to_vec(),
            sequence: packed >> 8,
            value,
        });
    }
    Ok(())
}

/// Return the decompressed contents of the block at `handle` (without its trailer).
fn read_block(data: &[u8], handle: BlockHandle) -> Result<Vec<u8>, TableError> {
    let end = handle
        .offset
        .checked_add(handle.size)
        .and_then(|end| end.checked_add(BLOCK_TRAILER_SIZE))
        .ok_or(TableError::BadHandle)?;
    if data.len() < end {
        return Err(TableError::BadHandle);
    }
    let contents_end = end - BLOCK_TRAILER_SIZE;
    let raw = data
        .get(handle.offset..contents_end)
        .ok_or(TableError::BadHandle)?;
    let compression = data[contents_end];
    match compression {
        COMPRESSION_NONE => Ok(raw.to_vec()),
        COMPRESSION_SNAPPY => Ok(snappy::decompress(raw, MAX_BLOCK_BYTES)?),
        other => Err(TableError::UnsupportedCompression(other)),
    }
}

/// Iterator over the `(key, value)` entries of a decoded block, undoing prefix compression.
struct BlockEntries<'a> {
    entries: &'a [u8],
    key: Vec<u8>,
    failed: bool,
}

impl<'a> BlockEntries<'a> {
    fn new(block: &'a [u8]) -> Result<Self, TableError> {
        let restart_count =
            read_u32_le(block, block.len().saturating_sub(4)).ok_or(TableError::BadBlock)? as usize;
        let restarts_len = restart_count
            .checked_mul(4)
            .and_then(|len| len.checked_add(4))
            .ok_or(TableError::BadBlock)?;
        let entries_end = block
            .len()
            .checked_sub(restarts_len)
            .ok_or(TableError::BadBlock)?;
        Ok(Self {
            entries: &block[..entries_end],
            key: Vec::new(),
            failed: false,
        })
    }
}

impl<'a> Iterator for BlockEntries<'a> {
    type Item = (Vec<u8>, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.entries.is_empty() {
            return None;
        }
        let parsed = parse_entry(self.entries, &self.key);
        match parsed {
            Some((key, value, rest)) => {
                self.key.clone_from(&key);
                self.entries = rest;
                Some((key, value))
            }
            None => {
                self.failed = true;
                None
            }
        }
    }
}

/// Decode one entry: `shared`, `non_shared`, `value_len` varints, key delta, value.
fn parse_entry<'a>(
    entries: &'a [u8],
    previous_key: &[u8],
) -> Option<(Vec<u8>, &'a [u8], &'a [u8])> {
    let (shared, a) = read_varint32(entries)?;
    let (non_shared, b) = read_varint32(entries.get(a..)?)?;
    let (value_len, c) = read_varint32(entries.get(a + b..)?)?;
    let body = entries.get(a + b + c..)?;
    let (shared, non_shared, value_len) =
        (shared as usize, non_shared as usize, value_len as usize);
    let payload_len = non_shared.checked_add(value_len)?;
    if shared > previous_key.len() || body.len() < payload_len {
        return None;
    }
    let mut key = Vec::with_capacity(shared + non_shared);
    key.extend_from_slice(&previous_key[..shared]);
    key.extend_from_slice(&body[..non_shared]);
    Some((key, &body[non_shared..payload_len], &body[payload_len..]))
}
