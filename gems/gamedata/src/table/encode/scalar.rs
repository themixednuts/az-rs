use std::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64,
};

use bytes::Buf;

use crate::GameDataError;
use crate::table::vlq;

pub(super) fn read_u8(data: &mut &[u8], field: &str) -> Result<u8, GameDataError> {
    if data.is_empty() {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_u8())
}

pub(super) fn read_i8(data: &mut &[u8], field: &str) -> Result<i8, GameDataError> {
    if data.is_empty() {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_i8())
}

pub(super) fn read_i16(data: &mut &[u8], field: &str) -> Result<i16, GameDataError> {
    if data.remaining() < 2 {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_i16_le())
}

pub(super) fn read_u16(data: &mut &[u8], field: &str) -> Result<u16, GameDataError> {
    if data.remaining() < 2 {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_u16_le())
}

pub(super) fn read_u32(data: &mut &[u8], field: &str) -> Result<u32, GameDataError> {
    if data.remaining() < 4 {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_u32_le())
}

pub(super) fn read_u64(data: &mut &[u8], field: &str) -> Result<u64, GameDataError> {
    if data.remaining() < 8 {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_u64_le())
}

pub(super) fn read_vlq_i32(data: &mut &[u8], field: &str) -> Result<i32, GameDataError> {
    let value = vlq::read_i64(data, field)?;
    i32::try_from(value)
        .map_err(|_| GameDataError::Decode(format!("{field} value {value} does not fit i32")))
}

pub(super) fn read_nonzero_i8(data: &mut &[u8], field: &str) -> Result<NonZeroI8, GameDataError> {
    NonZeroI8::new(read_i8(data, field)?)
        .ok_or_else(|| GameDataError::Decode(format!("{field} must be non-zero")))
}

pub(super) fn read_nonzero_i16(data: &mut &[u8], field: &str) -> Result<NonZeroI16, GameDataError> {
    NonZeroI16::new(read_i16(data, field)?)
        .ok_or_else(|| GameDataError::Decode(format!("{field} must be non-zero")))
}

pub(super) fn read_nonzero_i32(data: &mut &[u8], field: &str) -> Result<NonZeroI32, GameDataError> {
    NonZeroI32::new(read_vlq_i32(data, field)?)
        .ok_or_else(|| GameDataError::Decode(format!("{field} must be non-zero")))
}

pub(super) fn read_nonzero_i64(data: &mut &[u8], field: &str) -> Result<NonZeroI64, GameDataError> {
    NonZeroI64::new(vlq::read_i64(data, field)?)
        .ok_or_else(|| GameDataError::Decode(format!("{field} must be non-zero")))
}

pub(super) fn read_nonzero_u8(data: &mut &[u8], field: &str) -> Result<NonZeroU8, GameDataError> {
    NonZeroU8::new(read_u8(data, field)?)
        .ok_or_else(|| GameDataError::Decode(format!("{field} must be non-zero")))
}

pub(super) fn read_nonzero_u16(data: &mut &[u8], field: &str) -> Result<NonZeroU16, GameDataError> {
    NonZeroU16::new(read_u16(data, field)?)
        .ok_or_else(|| GameDataError::Decode(format!("{field} must be non-zero")))
}

pub(super) fn read_nonzero_u32(data: &mut &[u8], field: &str) -> Result<NonZeroU32, GameDataError> {
    NonZeroU32::new(vlq::read_u32(data, field)?)
        .ok_or_else(|| GameDataError::Decode(format!("{field} must be non-zero")))
}

pub(super) fn read_nonzero_u64(data: &mut &[u8], field: &str) -> Result<NonZeroU64, GameDataError> {
    NonZeroU64::new(vlq::read_u64(data, field)?)
        .ok_or_else(|| GameDataError::Decode(format!("{field} must be non-zero")))
}

pub(super) fn read_f32(data: &mut &[u8], field: &str) -> Result<f32, GameDataError> {
    if data.remaining() < 4 {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_f32_le())
}

pub(super) fn read_f64(data: &mut &[u8], field: &str) -> Result<f64, GameDataError> {
    if data.remaining() < 8 {
        return Err(truncated(field, data.remaining()));
    }
    Ok(data.get_f64_le())
}

pub(super) fn truncated(field: &str, remaining: usize) -> GameDataError {
    GameDataError::Decode(format!(
        "GameData table payload truncated while reading {field} ({remaining} byte(s) left)"
    ))
}
