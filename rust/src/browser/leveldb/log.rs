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

use super::Record;
use super::varint::{read_length_prefixed, read_u32_le, read_u64_le};

const BLOCK_SIZE: usize = 32 * 1024;
const HEADER_SIZE: usize = 7;
const BATCH_HEADER_SIZE: usize = 12;

const TYPE_ZERO: u8 = 0;
const TYPE_FULL: u8 = 1;
const TYPE_FIRST: u8 = 2;
const TYPE_MIDDLE: u8 = 3;
const TYPE_LAST: u8 = 4;

const OP_DELETE: u8 = 0;
const OP_PUT: u8 = 1;

/// Feed every put/delete found in `data` to `emit`.
pub(super) fn read_log(data: &[u8], emit: &mut impl FnMut(Record)) {
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
            return;
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
            return;
        }
        let Some(payload) = data.get(payload_start..payload_start + length) else {
            return;
        };
        offset = payload_start + length;

        match kind {
            TYPE_FULL => {
                assembled.clear();
                in_fragmented = false;
                read_batch(payload, emit);
            }
            TYPE_FIRST => {
                assembled.clear();
                assembled.extend_from_slice(payload);
                in_fragmented = true;
            }
            TYPE_MIDDLE if in_fragmented => assembled.extend_from_slice(payload),
            TYPE_LAST if in_fragmented => {
                assembled.extend_from_slice(payload);
                in_fragmented = false;
                read_batch(&assembled, emit);
                assembled.clear();
            }
            _ => return,
        }
    }
}

fn read_batch(batch: &[u8], emit: &mut impl FnMut(Record)) {
    let (Some(sequence), Some(count)) = (read_u64_le(batch, 0), read_u32_le(batch, 8)) else {
        return;
    };
    let mut rest = batch.get(BATCH_HEADER_SIZE..).unwrap_or_default();
    for index in 0..u64::from(count) {
        let Some((&op, after_op)) = rest.split_first() else {
            return;
        };
        let Some((key, after_key)) = read_length_prefixed(after_op) else {
            return;
        };
        let value = match op {
            OP_PUT => {
                let Some((value, after_value)) = read_length_prefixed(after_key) else {
                    return;
                };
                rest = after_value;
                Some(value.to_vec())
            }
            OP_DELETE => {
                rest = after_key;
                None
            }
            _ => return,
        };
        emit(Record {
            key: key.to_vec(),
            sequence: sequence.wrapping_add(index),
            value,
        });
    }
}
