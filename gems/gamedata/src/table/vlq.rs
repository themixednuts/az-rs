//! Compact integer payload encoding for `GameData` table values.

#[cfg(any(feature = "authoring", test))]
use bytes::BufMut;

use crate::GameDataError;

#[cfg(any(feature = "authoring", test))]
pub(super) fn write_u32(bytes: &mut Vec<u8>, value: u32) {
    write_u64(bytes, u64::from(value));
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn write_u64(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        bytes.put_u8(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    // The loop only exits below `0x80`, so this mask is a no-op that also
    // proves the byte cannot truncate.
    bytes.put_u8((value & 0x7f) as u8);
}

#[cfg(any(feature = "authoring", test))]
pub(super) fn write_i64(bytes: &mut Vec<u8>, value: i64) {
    write_u64(bytes, zigzag_encode(value));
}

pub(super) fn read_u32(data: &mut &[u8], field: &str) -> Result<u32, GameDataError> {
    let value = read_u64(data, field)?;
    u32::try_from(value)
        .map_err(|_| GameDataError::Decode(format!("{field} value {value} exceeds u32")))
}

pub(super) fn read_u64(data: &mut &[u8], field: &str) -> Result<u64, GameDataError> {
    let mut value = 0u64;
    let mut shift = 0u32;
    for _ in 0..10 {
        let Some((&byte, rest)) = data.split_first() else {
            return Err(GameDataError::Decode(format!(
                "GameData table payload truncated while reading {field} (0 byte(s) left)"
            )));
        };
        *data = rest;

        let chunk = u64::from(byte & 0x7f);
        if shift == 63 && chunk > 1 {
            return Err(GameDataError::Decode(format!("{field} VLQ exceeds u64")));
        }
        value |= chunk << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
    Err(GameDataError::Decode(format!(
        "{field} VLQ exceeds 10 bytes"
    )))
}

pub(super) fn read_i64(data: &mut &[u8], field: &str) -> Result<i64, GameDataError> {
    read_u64(data, field).map(zigzag_decode)
}

#[inline]
#[cfg(any(feature = "authoring", test))]
const fn zigzag_encode(value: i64) -> u64 {
    // Zigzag reinterprets the two's-complement bits; the sign is carried by
    // the arithmetic-shift mask, not lost.
    (value.cast_unsigned() << 1) ^ (value >> 63).cast_unsigned()
}

#[inline]
const fn zigzag_decode(value: u64) -> i64 {
    // The logical shift clears the top bit before reinterpretation, so the
    // magnitude half can never wrap negative.
    (value >> 1).cast_signed() ^ -(value & 1).cast_signed()
}
