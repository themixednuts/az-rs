//! Numeric conversions across the Lua ↔ native `LyShine` boundary.
//!
//! Lua has exactly two numeric types — an `i64` integer and an `f64`
//! number — while the engine side counts entities in `u64` handles and
//! measures UI in `f32` scalars. Every narrowing that crossing needs is
//! spelled out once here, with the range argument that makes it sound,
//! rather than written as a bare `as` in each bus handler.

use bevy_mod_scripting::lua::mlua::Value;

use crate::canvas::UiEntityId;

/// Reinterpret a Lua integer as the [`UiEntityId`] it addresses.
///
/// `UiEntityId` wraps a `u64`; Lua carries it as the `i64` with the same
/// bit pattern, so ids above `i64::MAX` reach us negative. The round
/// trip through [`lua_id`] is exact for every id.
#[inline]
pub(super) const fn entity_id(id: i64) -> UiEntityId {
    UiEntityId::new(id.cast_unsigned())
}

/// Reinterpret a [`UiEntityId`] as the Lua integer scripts pass around.
///
/// Inverse of [`entity_id`]: the `u64` handle keeps its bit pattern, so
/// handles above `i64::MAX` surface to Lua as negative integers — which
/// is what the Lumberyard behavior context did with `AZ::EntityId` too.
#[inline]
pub(super) const fn lua_id(id: UiEntityId) -> i64 {
    id.as_u64().cast_signed()
}

/// Read argument `index` as the `f32` scalar Bevy UI stores.
///
/// Accepts either Lua numeric type and returns `None` for anything else
/// (including a missing argument), leaving the caller to pick its own
/// default. Both conversions are lossy by construction — see
/// [`scalar_from_number`] and [`scalar_from_integer`].
pub(super) fn scalar(args: &[Value], index: usize) -> Option<f32> {
    match args.get(index) {
        Some(Value::Number(value)) => Some(scalar_from_number(*value)),
        Some(Value::Integer(value)) => Some(scalar_from_integer(*value)),
        _ => None,
    }
}

/// Read argument `index` as an unsigned engine-side count.
pub(super) fn count(args: &[Value], index: usize) -> Option<u64> {
    match args.get(index) {
        Some(Value::Integer(value)) => Some(value.cast_unsigned()),
        Some(Value::Number(value)) => Some(count_from_number(*value)),
        _ => None,
    }
}

/// Narrow a Lua `number` to the `f32` every UI scalar is stored in.
///
/// Positions, scales, font sizes and alpha are all `f32` on the Bevy
/// side, so this narrowing *is* the boundary: it keeps ~7 significant
/// digits and sends magnitudes past `f32::MAX` to infinity rather than
/// wrapping. That matches what Lumberyard's `float` behavior-context
/// bindings did with the same Lua values.
#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "narrowing f64 to f32 is the intended conversion here and std offers no non-`as` narrowing cast"
)]
const fn scalar_from_number(value: f64) -> f32 {
    value as f32
}

/// Widen a Lua integer to the `f32` every UI scalar is stored in.
///
/// UI scalars are `f32`, so integers beyond 2^24 lose their low bits.
/// Scripts only ever pass pixel counts, indices and percentages through
/// here, all far inside that range; the Lumberyard binding had the same
/// `static_cast<float>` behaviour for out-of-range values.
#[inline]
#[allow(
    clippy::cast_precision_loss,
    reason = "UI scalars are f32, so an i64 argument is widened by design; std offers no lossless i64 -> f32"
)]
const fn scalar_from_integer(value: i64) -> f32 {
    value as f32
}

/// Clamp and truncate a Lua number to an unsigned engine-side count.
#[inline]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Rust's saturating float-to-integer cast is the required boundary conversion"
)]
const fn count_from_number(value: f64) -> u64 {
    value as u64
}
