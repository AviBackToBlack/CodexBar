//! Raw Snappy block decompression (the format LevelDB uses for compressed table blocks).
//!
//! Format reference: <https://github.com/google/snappy/blob/main/format_description.txt>.
//! The stream is a varint uncompressed length followed by literal and copy elements.
//! Output size is bounded by the caller so a hostile length preamble cannot exhaust memory.

use super::varint::read_varint32;

/// Why a Snappy block could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SnappyError {
    #[error("snappy length preamble is missing or malformed")]
    BadPreamble,
    #[error("snappy output of {0} bytes exceeds the {1} byte limit")]
    TooLarge(usize, usize),
    #[error("snappy stream is truncated")]
    Truncated,
    #[error("snappy copy references data before the start of the output")]
    BadOffset,
    #[error("snappy stream length does not match its preamble")]
    LengthMismatch,
}

/// Decompress one raw Snappy block, refusing to produce more than `max_len` bytes.
pub fn decompress(input: &[u8], max_len: usize) -> Result<Vec<u8>, SnappyError> {
    let (expected, mut pos) = read_varint32(input).ok_or(SnappyError::BadPreamble)?;
    let expected = expected as usize;
    if expected > max_len {
        return Err(SnappyError::TooLarge(expected, max_len));
    }

    let mut out = Vec::with_capacity(expected);
    while pos < input.len() {
        let tag = input[pos];
        pos += 1;
        match tag & 0b11 {
            0b00 => {
                let mut len = usize::from(tag >> 2);
                if len >= 60 {
                    let extra = len - 59;
                    let bytes = input.get(pos..pos + extra).ok_or(SnappyError::Truncated)?;
                    pos += extra;
                    len = bytes
                        .iter()
                        .rev()
                        .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
                }
                len = len.checked_add(1).ok_or(SnappyError::Truncated)?;
                let literal = pos
                    .checked_add(len)
                    .and_then(|end| input.get(pos..end))
                    .ok_or(SnappyError::Truncated)?;
                if out.len() + literal.len() > expected {
                    return Err(SnappyError::LengthMismatch);
                }
                out.extend_from_slice(literal);
                pos += len;
            }
            kind => {
                let (len, offset) = match kind {
                    0b01 => {
                        let low = *input.get(pos).ok_or(SnappyError::Truncated)?;
                        pos += 1;
                        (
                            4 + usize::from((tag >> 2) & 0b111),
                            (usize::from(tag >> 5) << 8) | usize::from(low),
                        )
                    }
                    0b10 => {
                        let bytes = input.get(pos..pos + 2).ok_or(SnappyError::Truncated)?;
                        pos += 2;
                        (
                            1 + usize::from(tag >> 2),
                            usize::from(u16::from_le_bytes([bytes[0], bytes[1]])),
                        )
                    }
                    _ => {
                        let bytes = input.get(pos..pos + 4).ok_or(SnappyError::Truncated)?;
                        pos += 4;
                        (
                            1 + usize::from(tag >> 2),
                            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
                        )
                    }
                };
                copy_within_output(&mut out, offset, len, expected)?;
            }
        }
    }

    if out.len() == expected {
        Ok(out)
    } else {
        Err(SnappyError::LengthMismatch)
    }
}

/// Append `len` bytes copied from `offset` bytes back; the ranges may overlap (run-length style).
fn copy_within_output(
    out: &mut Vec<u8>,
    offset: usize,
    len: usize,
    expected: usize,
) -> Result<(), SnappyError> {
    if offset == 0 || offset > out.len() {
        return Err(SnappyError::BadOffset);
    }
    if out.len() + len > expected {
        return Err(SnappyError::LengthMismatch);
    }
    let start = out.len() - offset;
    for i in 0..len {
        out.push(out[start + i]);
    }
    Ok(())
}
