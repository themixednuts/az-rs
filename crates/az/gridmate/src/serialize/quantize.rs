//! Numeric narrowing shared by the quantizing marshalers.
//!
//! The compression codecs all follow the same shape: scale a float into a
//! destination integer's full range, then narrow it. Rust's `as` is saturating
//! for float-to-integer conversions (NaN maps to zero, out-of-range values
//! clamp to the bounds), which reproduces the vendor `static_cast` behaviour
//! these wire formats were matched against. There is no checked float-to-int
//! conversion in `std` to use instead.
//!
//! Routing every such conversion through this module keeps the truncation and
//! precision lints to a handful of reviewed sites rather than one per codec,
//! and gives the invariant a name at each call site.

/// Narrow a scaled `f32` to `u8`, saturating at the bounds.
///
/// Callers scale into `0.0..=255.0` first — usually via `clamp` — so the
/// saturation is not expected to engage on well-formed input.
#[inline]
// No checked float-to-int conversion exists in std; `as` saturates, which is
// exactly the vendor `static_cast` behaviour the wire format was matched to.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub const fn quantized_u8(value: f32) -> u8 {
    value as u8
}

/// Narrow a scaled `f32` to `u16`, saturating at the bounds.
///
/// Callers scale into `0.0..=65535.0` first — usually via `clamp` — so the
/// saturation is not expected to engage on well-formed input.
#[inline]
// No checked float-to-int conversion exists in std; `as` saturates, which is
// exactly the vendor `static_cast` behaviour the wire format was matched to.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub const fn quantized_u16(value: f32) -> u16 {
    value as u16
}

/// Narrow a scaled `f32` to `u32`, saturating at the bounds.
///
/// Callers scale into `0.0..=u32::MAX` first — usually via `clamp` — so the
/// saturation is not expected to engage on well-formed input.
#[inline]
// No checked float-to-int conversion exists in std; `as` saturates, which is
// exactly the vendor `static_cast` behaviour the wire format was matched to.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub const fn quantized_u32(value: f32) -> u32 {
    value as u32
}

/// Narrow an unquantized `f32` back to `i32`, truncating toward zero.
///
/// The value is the reconstructed offset within a codec's `MIN..=MAX` range,
/// so it is bounded by that range's width by construction.
#[inline]
// No checked float-to-int conversion exists in std; `as` truncates toward zero
// and saturates, matching the vendor `static_cast` round-trip.
#[allow(clippy::cast_possible_truncation)]
pub const fn unquantized_i32(value: f32) -> i32 {
    value as i32
}

/// Widen an `i32` to `f32` for quantization scaling.
///
/// Quantization ranges are protocol constants far below `2^24`, so the
/// conversion is exact for every range these codecs are instantiated with.
#[inline]
// Precision loss only above 2^24; quantization bounds are protocol constants
// orders of magnitude smaller, and the result is a scale factor either way.
#[allow(clippy::cast_precision_loss)]
pub const fn range_f32_i32(value: i32) -> f32 {
    value as f32
}

/// Widen a `u32` to `f32` for quantization scaling.
///
/// Used for full-range denominators such as `u32::MAX`, where the rounded
/// value is the intended scale factor.
#[inline]
// Precision loss above 2^24 is intended here: the value is a scale
// denominator, and the vendor computes it in `float` the same way.
#[allow(clippy::cast_precision_loss)]
pub const fn range_f32_u32(value: u32) -> f32 {
    value as f32
}
