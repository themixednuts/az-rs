//! Grid conversion math shared by the heightmap, chunk, region and water code.
//!
//! Terrain work moves constantly between integer grid coordinates and the
//! floats that sampling and mesh generation run on. Both directions are lossy
//! in general and exact for the values this crate actually holds, so the
//! conversions live here once, documented, instead of as bare `as` casts
//! spread across the crate.

/// Exact widening of a small integer to `f32`.
///
/// `f32` represents every integer below 2^24 (16,777,216) exactly, and every
/// power of two far past that. Terrain grid quantities - heightmap and surface
/// resolutions, vertex and index counts, region grid coordinates, terrain
/// sizes in metres, quadtree cell coordinates - all sit orders of magnitude
/// below that bound, so widening one of them loses nothing. Past the bound the
/// conversion rounds to nearest, an error already far below the precision of a
/// world coordinate at that scale.
pub trait ExactF32 {
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

impl ExactF32 for u32 {
    // Exact below 2^24, and exact at every power of two (quadtree side lengths).
    #[allow(clippy::cast_precision_loss)]
    #[inline]
    fn exact_f32(self) -> f32 {
        self as f32
    }
}

impl ExactF32 for i32 {
    // Exact for magnitudes below 2^24; region coordinates and metre counts.
    #[allow(clippy::cast_precision_loss)]
    #[inline]
    fn exact_f32(self) -> f32 {
        self as f32
    }
}

/// Floor a float grid coordinate to an index.
///
/// Saturates exactly like the primitive conversion it replaces: `NaN` and
/// negative coordinates give `0`, coordinates past `usize::MAX` give
/// `usize::MAX`. Nothing is clamped to a resolution here, so callers that
/// treat an out-of-range coordinate as a miss keep doing so - use
/// [`clamped_index`] when the coordinate is already known to be in range.
// Saturating truncation is the point of the function; `f32` has no fallible
// conversion to `usize`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[inline]
pub const fn floor_index(value: f32) -> usize {
    value.floor() as usize
}

/// Round a float grid coordinate to the nearest index.
///
/// Saturates the same way [`floor_index`] does.
// See `floor_index`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[inline]
pub const fn round_index(value: f32) -> usize {
    value.round() as usize
}

/// Floor a float grid coordinate to an index inside `0..=max`.
///
/// For a coordinate the caller has already bounds-checked, the clamp is a
/// no-op except in one case: a ratio that is mathematically just under 1.0 can
/// round up to exactly 1.0 in `f32`, which puts the scaled coordinate one cell
/// past the end. The clamp absorbs that, which is why this is a clamp and not
/// a `debug_assert!`.
#[inline]
pub fn clamped_index(value: f32, max: usize) -> usize {
    floor_index(value).min(max)
}

/// Narrow a vertex index or count to the `u32` a mesh index buffer holds.
///
/// Saturates at [`u32::MAX`] instead of wrapping. A mesh with that many
/// vertices cannot be built on any supported target - it would need tens of
/// gigabytes of vertex data - so the clamp is unreachable; it is here so that
/// the impossible case degenerates visibly rather than aliasing index 0.
#[inline]
pub fn mesh_index(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Rescale an index from a `0..=max_from` grid onto a `0..=max_to` grid.
pub fn remap_index(value: usize, max_from: usize, max_to: usize) -> usize {
    if max_from == 0 {
        return 0;
    }
    let scaled = (value.exact_f32() * max_to.exact_f32()) / max_from.exact_f32();
    round_index(scaled).min(max_to)
}
