//! Bounded protobuf wire reader shared by the Grok billing, product-usage and
//! reset-coupon parsers (upstream `GrokProtobufField`, v0.69.0).
//!
//! Every read is bounds-checked: varints stop after ten bytes and reject
//! overflow, and length-delimited values cannot extend past the enclosing
//! message. Unknown wire types and out-of-range field numbers are malformed.

/// Largest field number protobuf allows (2^29 - 1).
const MAX_FIELD_NUMBER: u64 = 536_870_911;

/// The payload of one decoded field. Fixed 64-bit values are consumed but
/// never interpreted; length-delimited bytes stay opaque until a caller knows
/// they hold a message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum FieldValue<'a> {
    Varint(u64),
    Fixed64,
    Message(&'a [u8]),
    Fixed32(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ProtobufField<'a> {
    pub(super) number: u64,
    pub(super) value: FieldValue<'a>,
}

impl<'a> ProtobufField<'a> {
    /// Decode every field of `data`, or `None` when any of it is malformed.
    pub(super) fn fields(data: &'a [u8]) -> Option<Vec<Self>> {
        let mut fields = Vec::new();
        let mut index = 0;
        while index < data.len() {
            fields.push(Self::read(data, &mut index)?);
        }
        Some(fields)
    }

    /// Decode one field at `index`, advancing it past the field. `index` is
    /// left untouched when the field is malformed.
    pub(super) fn read(data: &'a [u8], index: &mut usize) -> Option<Self> {
        let (key, mut next) = read_varint(data, *index)?;
        let number = key >> 3;
        if number == 0 || number > MAX_FIELD_NUMBER {
            return None;
        }
        let value = match key & 0x07 {
            0 => {
                let (value, end) = read_varint(data, next)?;
                next = end;
                FieldValue::Varint(value)
            }
            1 => {
                next = advance(data, next, 8)?;
                FieldValue::Fixed64
            }
            2 => {
                let (len, start) = read_varint(data, next)?;
                next = advance(data, start, usize::try_from(len).ok()?)?;
                FieldValue::Message(&data[start..next])
            }
            5 => {
                let end = advance(data, next, 4)?;
                let bytes: [u8; 4] = data[next..end].try_into().ok()?;
                next = end;
                FieldValue::Fixed32(f32::from_le_bytes(bytes))
            }
            _ => return None,
        };
        *index = next;
        Some(Self { number, value })
    }

    pub(super) fn varint(&self) -> Option<u64> {
        match self.value {
            FieldValue::Varint(value) => Some(value),
            _ => None,
        }
    }

    pub(super) fn fixed32(&self) -> Option<f32> {
        match self.value {
            FieldValue::Fixed32(value) => Some(value),
            _ => None,
        }
    }

    pub(super) fn message(&self) -> Option<&'a [u8]> {
        match self.value {
            FieldValue::Message(bytes) => Some(bytes),
            _ => None,
        }
    }
}

/// Whether `data` starts like a raw (unframed) protobuf message. Valid keys
/// cannot begin with field number 0, so a gRPC-web data flag never matches.
pub(super) fn looks_like_protobuf_payload(data: &[u8]) -> bool {
    let Some(&first) = data.first() else {
        return false;
    };
    first >> 3 > 0 && matches!(first & 0x07, 0 | 1 | 2 | 5)
}

fn advance(data: &[u8], index: usize, len: usize) -> Option<usize> {
    index.checked_add(len).filter(|end| *end <= data.len())
}

fn read_varint(data: &[u8], mut index: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0;
    while index < data.len() && shift < 64 {
        let byte = data[index];
        index += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, index));
        }
        shift += 7;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_reader_bounds_continuation_and_length_reads() {
        let mut endless = vec![0x08];
        endless.extend([0x80; 4096]);
        let mut index = 0;
        assert_eq!(ProtobufField::read(&endless, &mut index), None);
        assert_eq!(index, 0);

        let malformed: [&[u8]; 6] = [
            &[0x00],
            &[0x02, 0],
            &[0x09, 0],
            &[0x15, 0, 0],
            &[0x12, 0x7F, 0],
            &[0x08],
        ];
        for bytes in malformed {
            assert_eq!(ProtobufField::fields(bytes), None, "{bytes:?}");
        }
    }

    #[test]
    fn wire_reader_accepts_maximum_integers_and_leaves_opaque_bytes_uninterpreted() {
        let mut maximum = vec![0x08];
        maximum.extend([0xFF; 9]);
        maximum.push(0x01);
        maximum.extend([0x12, 0x01, 0xFF]);

        let fields = ProtobufField::fields(&maximum).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].varint(), Some(u64::MAX));
        assert_eq!(fields[1].message(), Some(&[0xFF][..]));
    }

    #[test]
    fn wire_reader_decodes_fixed_width_values_and_rejects_overflow() {
        let mut bytes = vec![0x0d];
        bytes.extend(6.0f32.to_le_bytes());
        bytes.push(0x11);
        bytes.extend([0; 8]);
        let fields = ProtobufField::fields(&bytes).unwrap();
        assert_eq!(fields[0].fixed32(), Some(6.0));
        assert_eq!(fields[1].value, FieldValue::Fixed64);
        assert_eq!(fields[1].varint(), None);

        let mut overflow = vec![0x08];
        overflow.extend([0xFF; 9]);
        overflow.push(0x02);
        assert_eq!(ProtobufField::fields(&overflow), None);
        assert_eq!(ProtobufField::fields(&[0x08, 0x80]), None);
    }
}
