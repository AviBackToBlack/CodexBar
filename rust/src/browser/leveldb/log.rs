//! LevelDB write-ahead log (`*.log`) reader.
//!
//! A log is a sequence of 32 KiB blocks holding physical records (7 byte header: masked CRC32C,
//! length, type) that are reassembled into logical records. Each logical record is a write batch:
//! `sequence: u64le`, `count: u32le`, then `count` operations (`1 key value` puts and `0 key`
//! deletes, with varint-length-prefixed byte strings). Operation `i` has sequence `sequence + i`.
//!
//! The live log of a running browser routinely ends in a half-written record, so a truncated or
//! malformed tail ends the scan quietly and everything before it is kept. Record checksums are
//! not verified: this is a best-effort read of another program's cache, not a database recovery.

use super::varint::{read_length_prefixed, read_u32_le, read_u64_le};
use super::{MAX_RECORDS_PER_DIRECTORY, Record};

const BLOCK_SIZE: usize = 32 * 1024;
const HEADER_SIZE: usize = 7;
const BATCH_HEADER_SIZE: usize = 12;
const MAX_LOG_RECORD_BYTES: usize = 16 * 1024 * 1024;

const TYPE_ZERO: u8 = 0;
const TYPE_FULL: u8 = 1;
const TYPE_FIRST: u8 = 2;
const TYPE_MIDDLE: u8 = 3;
const TYPE_LAST: u8 = 4;

const OP_DELETE: u8 = 0;
const OP_PUT: u8 = 1;
const MAX_SEQUENCE: u64 = (1 << 56) - 1;

/// Feed every put/delete found in `data` to `emit`.
#[cfg(test)]
pub(super) fn read_log(data: &[u8], emit: &mut impl FnMut(Record)) {
    let _ = read_log_until(data, &mut |record| {
        emit(record);
        true
    });
}

/// Read a log until it is malformed or `emit` asks the scan to stop.
pub(super) fn read_log_until(data: &[u8], emit: &mut impl FnMut(Record) -> bool) -> bool {
    let mut assembled: Vec<u8> = Vec::new();
    let mut in_fragmented = false;

    let mut offset = 0usize;
    while offset < data.len() {
        let block_remaining = BLOCK_SIZE - (offset % BLOCK_SIZE);
        if block_remaining < HEADER_SIZE {
            offset += block_remaining;
            continue;
        }
        let Some(header) = data.get(offset..offset + HEADER_SIZE) else {
            return false;
        };
        let length = usize::from(u16::from_le_bytes([header[4], header[5]]));
        let kind = header[6];
        if kind == TYPE_ZERO && length == 0 {
            // Zero padding: the rest of this block is unused.
            offset += block_remaining;
            continue;
        }
        let payload_start = offset + HEADER_SIZE;
        if HEADER_SIZE + length > block_remaining {
            return false;
        }
        let Some(payload) = data.get(payload_start..payload_start + length) else {
            return false;
        };
        offset = payload_start + length;

        match kind {
            TYPE_FULL => {
                assembled.clear();
                in_fragmented = false;
                if !read_batch(payload, emit) {
                    return false;
                }
            }
            TYPE_FIRST => {
                assembled.clear();
                if !append_fragment(&mut assembled, payload) {
                    return false;
                }
                in_fragmented = true;
            }
            TYPE_MIDDLE if in_fragmented => {
                if !append_fragment(&mut assembled, payload) {
                    return false;
                }
            }
            TYPE_LAST if in_fragmented => {
                if !append_fragment(&mut assembled, payload) {
                    return false;
                }
                in_fragmented = false;
                if !read_batch(&assembled, emit) {
                    return false;
                }
                assembled.clear();
            }
            _ => return false,
        }
    }
    true
}

fn append_fragment(assembled: &mut Vec<u8>, payload: &[u8]) -> bool {
    let Some(new_len) = assembled.len().checked_add(payload.len()) else {
        return false;
    };
    if new_len > MAX_LOG_RECORD_BYTES || assembled.try_reserve(payload.len()).is_err() {
        return false;
    }
    assembled.extend_from_slice(payload);
    true
}

fn read_batch(batch: &[u8], emit: &mut impl FnMut(Record) -> bool) -> bool {
    if !visit_batch(batch, &mut |_, _, _| true) {
        return true;
    }
    visit_batch(batch, &mut |sequence, key, value| {
        emit(Record {
            key: key.to_vec(),
            sequence,
            value: value.map(|value| value.to_vec()),
        })
    })
}

/// Validate the whole batch before applying any of its operations. A torn or malformed write
/// batch must not leave a valid-looking prefix in the returned database state.
fn visit_batch<'a>(
    batch: &'a [u8],
    emit: &mut impl FnMut(u64, &'a [u8], Option<&'a [u8]>) -> bool,
) -> bool {
    let (Some(sequence), Some(count)) = (read_u64_le(batch, 0), read_u32_le(batch, 8)) else {
        return false;
    };
    if sequence > MAX_SEQUENCE || u64::from(count) > MAX_RECORDS_PER_DIRECTORY as u64 {
        return false;
    }
    let mut rest = match batch.get(BATCH_HEADER_SIZE..) {
        Some(rest) => rest,
        None => return false,
    };
    for index in 0..u64::from(count) {
        let Some((&op, after_op)) = rest.split_first() else {
            return false;
        };
        let Some((key, after_key)) = read_length_prefixed(after_op) else {
            return false;
        };
        let value = match op {
            OP_PUT => {
                let Some((value, after_value)) = read_length_prefixed(after_key) else {
                    return false;
                };
                rest = after_value;
                Some(value)
            }
            OP_DELETE => {
                rest = after_key;
                None
            }
            _ => return false,
        };
        let Some(record_sequence) = sequence.checked_add(index).filter(|s| *s <= MAX_SEQUENCE)
        else {
            return false;
        };
        if !emit(record_sequence, key, value) {
            return false;
        }
    }
    rest.is_empty()
}
