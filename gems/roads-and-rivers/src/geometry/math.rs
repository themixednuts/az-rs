//! Spline geometry math helpers.

/// Exact widening of a grid or sector count to `f32`.
///
/// Sector counts and their indices are bounded by a spline's length divided by
/// its segment length - thousands at most, far below 2^24, the largest integer
/// `f32` represents exactly. Widening one of them loses nothing.
pub(super) trait ExactF32 {
    /// Widen `self` to `f32`.
    fn exact_f32(self) -> f32;
}

impl ExactF32 for usize {
    // Exact below 2^24; no fallible integer-to-float conversion exists to use
    // instead, so the trait doc carries the bound.
    #[allow(clippy::cast_precision_loss)]
    #[inline]
    fn exact_f32(self) -> f32 {
        self as f32
    }
}

/// Round a length ratio up to a sector count.
///
/// Saturates exactly like the primitive conversion it replaces: `NaN` and
/// negative ratios give `0`, ratios past `usize::MAX` give `usize::MAX`.
/// Callers treat `0` as "nothing to build".
// Saturating truncation is the point of the function; `f32` has no fallible
// conversion to `usize`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[inline]
pub(super) const fn ceil_count(ratio: f32) -> usize {
    ratio.ceil() as usize
}

/// Narrow a vertex index or count to the `u32` a mesh index buffer holds.
///
/// Saturates at [`u32::MAX`] instead of wrapping. A mesh with that many
/// vertices cannot be built on any supported target, so the clamp is
/// unreachable; it is here so the impossible case degenerates visibly rather
/// than aliasing index 0.
#[inline]
pub(super) fn mesh_index(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(super) fn ease_in_out(mut t: f32) -> f32 {
    t *= 2.0;
    if t < 1.0 {
        return 0.5 * t * t;
    }
    t -= 1.0;
    -0.5 * t.mul_add(t - 2.0, -1.0)
}

pub(super) fn inverse_lerp(start: f32, end: f32, value: f32) -> f32 {
    let denom = end - start;
    if denom.abs() <= f32::EPSILON {
        return 0.0;
    }
    ((value - start) / denom).clamp(0.0, 1.0)
}
