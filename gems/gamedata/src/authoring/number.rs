use std::num::{
    NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64,
};

use ron::value::Number;

use crate::GameDataError;

pub(super) fn number_to_i8(value: Number) -> Result<i8, GameDataError> {
    i8::try_from(number_to_i64(value)?).map_err(|_| number_range_error(value, "i8"))
}

pub(super) fn number_to_i16(value: Number) -> Result<i16, GameDataError> {
    i16::try_from(number_to_i64(value)?).map_err(|_| number_range_error(value, "i16"))
}

pub(super) fn number_to_i32(value: Number) -> Result<i32, GameDataError> {
    i32::try_from(number_to_i64(value)?).map_err(|_| number_range_error(value, "i32"))
}

pub(super) fn number_to_u8(value: Number) -> Result<u8, GameDataError> {
    u8::try_from(number_to_u64(value)?).map_err(|_| number_range_error(value, "u8"))
}

pub(super) fn number_to_u16(value: Number) -> Result<u16, GameDataError> {
    u16::try_from(number_to_u64(value)?).map_err(|_| number_range_error(value, "u16"))
}

pub(super) fn number_to_u32(value: Number) -> Result<u32, GameDataError> {
    u32::try_from(number_to_u64(value)?).map_err(|_| number_range_error(value, "u32"))
}

// The only uncovered variant is ron's private `__NonExhaustive` marker,
// which is `#[cfg(not(doc))]`: naming it would stop compiling under
// `cargo doc` and reaches into a private interface.
#[allow(clippy::match_wildcard_for_single_variants)]
pub(super) fn number_to_i64(value: Number) -> Result<i64, GameDataError> {
    match value {
        Number::I8(value) => Ok(i64::from(value)),
        Number::I16(value) => Ok(i64::from(value)),
        Number::I32(value) => Ok(i64::from(value)),
        Number::I64(value) => Ok(value),
        Number::U8(value) => Ok(i64::from(value)),
        Number::U16(value) => Ok(i64::from(value)),
        Number::U32(value) => Ok(i64::from(value)),
        Number::U64(value) => i64::try_from(value).map_err(|_| number_range_error(value, "i64")),
        Number::F32(_) | Number::F64(_) => Err(GameDataError::Decode(format!(
            "expected integer number, found {value:?}"
        ))),
        _ => unreachable!("unknown RON number variant"),
    }
}

// The only uncovered variant is ron's private `__NonExhaustive` marker,
// which is `#[cfg(not(doc))]`: naming it would stop compiling under
// `cargo doc` and reaches into a private interface.
#[allow(clippy::match_wildcard_for_single_variants)]
pub(super) fn number_to_u64(value: Number) -> Result<u64, GameDataError> {
    match value {
        Number::I8(value) => u64::try_from(value).map_err(|_| number_range_error(value, "u64")),
        Number::I16(value) => u64::try_from(value).map_err(|_| number_range_error(value, "u64")),
        Number::I32(value) => u64::try_from(value).map_err(|_| number_range_error(value, "u64")),
        Number::I64(value) => u64::try_from(value).map_err(|_| number_range_error(value, "u64")),
        Number::U8(value) => Ok(u64::from(value)),
        Number::U16(value) => Ok(u64::from(value)),
        Number::U32(value) => Ok(u64::from(value)),
        Number::U64(value) => Ok(value),
        Number::F32(_) | Number::F64(_) => Err(GameDataError::Decode(format!(
            "expected unsigned integer number, found {value:?}"
        ))),
        _ => unreachable!("unknown RON number variant"),
    }
}

pub(super) fn number_to_nonzero_i8(value: Number) -> Result<NonZeroI8, GameDataError> {
    NonZeroI8::new(number_to_i8(value)?).ok_or_else(|| number_range_error(value, "non-zero i8"))
}

pub(super) fn number_to_nonzero_i16(value: Number) -> Result<NonZeroI16, GameDataError> {
    NonZeroI16::new(number_to_i16(value)?).ok_or_else(|| number_range_error(value, "non-zero i16"))
}

pub(super) fn number_to_nonzero_i32(value: Number) -> Result<NonZeroI32, GameDataError> {
    NonZeroI32::new(number_to_i32(value)?).ok_or_else(|| number_range_error(value, "non-zero i32"))
}

pub(super) fn number_to_nonzero_i64(value: Number) -> Result<NonZeroI64, GameDataError> {
    NonZeroI64::new(number_to_i64(value)?).ok_or_else(|| number_range_error(value, "non-zero i64"))
}

pub(super) fn number_to_nonzero_u8(value: Number) -> Result<NonZeroU8, GameDataError> {
    NonZeroU8::new(number_to_u8(value)?).ok_or_else(|| number_range_error(value, "non-zero u8"))
}

pub(super) fn number_to_nonzero_u16(value: Number) -> Result<NonZeroU16, GameDataError> {
    NonZeroU16::new(number_to_u16(value)?).ok_or_else(|| number_range_error(value, "non-zero u16"))
}

pub(super) fn number_to_nonzero_u32(value: Number) -> Result<NonZeroU32, GameDataError> {
    NonZeroU32::new(number_to_u32(value)?).ok_or_else(|| number_range_error(value, "non-zero u32"))
}

pub(super) fn number_to_nonzero_u64(value: Number) -> Result<NonZeroU64, GameDataError> {
    NonZeroU64::new(number_to_u64(value)?).ok_or_else(|| number_range_error(value, "non-zero u64"))
}

fn number_range_error<T: std::fmt::Debug>(value: T, expected: &str) -> GameDataError {
    GameDataError::Decode(format!("number {value:?} is out of range for {expected}"))
}

/// Widens any authored RON number to the `f64` a cell of that type stores.
///
/// 64-bit integers past 2^53 round to the nearest representable `f64`; that
/// is the defined result of authoring one into a float column.
// The only uncovered variant is ron's private `__NonExhaustive` marker,
// which is `#[cfg(not(doc))]`: naming it would stop compiling under
// `cargo doc` and reaches into a private interface.
#[allow(clippy::match_wildcard_for_single_variants)]
#[allow(clippy::cast_precision_loss)]
pub(super) fn number_to_f64(value: Number) -> f64 {
    match value {
        Number::I8(value) => f64::from(value),
        Number::I16(value) => f64::from(value),
        Number::I32(value) => f64::from(value),
        Number::I64(value) => value as f64,
        Number::U8(value) => f64::from(value),
        Number::U16(value) => f64::from(value),
        Number::U32(value) => f64::from(value),
        Number::U64(value) => value as f64,
        Number::F32(value) => f64::from(value.get()),
        Number::F64(value) => value.get(),
        _ => unreachable!("unknown RON number variant"),
    }
}

/// Narrows any authored RON number to the `f32` a cell of that type stores.
///
/// Integers past 2^24 and `f64` values outside the `f32` range round or
/// saturate; that is the defined result of authoring one into a float
/// column, and matches what the encoder writes.
// The only uncovered variant is ron's private `__NonExhaustive` marker,
// which is `#[cfg(not(doc))]`: naming it would stop compiling under
// `cargo doc` and reaches into a private interface.
#[allow(clippy::match_wildcard_for_single_variants)]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub(super) fn number_to_f32(value: Number) -> f32 {
    match value {
        Number::I8(value) => f32::from(value),
        Number::I16(value) => f32::from(value),
        Number::I32(value) => value as f32,
        Number::I64(value) => value as f32,
        Number::U8(value) => f32::from(value),
        Number::U16(value) => f32::from(value),
        Number::U32(value) => value as f32,
        Number::U64(value) => value as f32,
        Number::F32(value) => value.get(),
        Number::F64(value) => value.get() as f32,
        _ => unreachable!("unknown RON number variant"),
    }
}
