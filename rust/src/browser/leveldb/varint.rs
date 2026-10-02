//! LevelDB varint and fixed-width helpers.

/// Decode a little-endian base-128 varint of at most 32 bits; returns the value and bytes used.
pub(super) fn read_varint32(input: &[u8]) -> Option<(u32, usize)> {
    let (value, used) = read_varint64(input)?;
    Some((u32::try_from(value).ok()?, used))
}

/// Decode a little-endian base-128 varint of at most 64 bits; returns the value and bytes used.
pub(super) fn read_varint64(input: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for (index, byte) in input.iter().take(10).enumerate() {
        let bits = u64::from(byte & 0x7f);
        if index == 9 && bits > 1 {
            return None;
        }
        value |= bits << (7 * index);
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
    }
    None
}

/// Split a varint-length-prefixed slice off the front of `input`, returning `(slice, rest)`.
pub(super) fn read_length_prefixed(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let (len, used) = read_varint32(input)?;
    let rest = &input[used..];
    let len = len as usize;
    (rest.len() >= len).then(|| rest.split_at(len))
}

pub(super) fn read_u32_le(input: &[u8], at: usize) -> Option<u32> {
    let bytes = input.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

pub(super) fn read_u64_le(input: &[u8], at: usize) -> Option<u64> {
    let bytes = input.get(at..at.checked_add(8)?)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}
