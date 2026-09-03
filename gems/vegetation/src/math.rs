use bevy::prelude::*;

/// The tolerance vegetation uses to decide two poses are the same pose.
const CLOSE_TOLERANCE: f32 = 0.0001;

pub fn normalized_or(value: Vec3, fallback: Vec3) -> Vec3 {
    let normalized = value.normalize_or_zero();
    if normalized == Vec3::ZERO {
        fallback
    } else {
        normalized
    }
}

pub fn vec3_close(lhs: Vec3, rhs: Vec3) -> bool {
    lhs.distance_squared(rhs) <= CLOSE_TOLERANCE
}

pub fn quat_close(lhs: Quat, rhs: Quat) -> bool {
    (lhs.dot(rhs).abs() - 1.0).abs() <= CLOSE_TOLERANCE
}

pub const fn f32_close(lhs: f32, rhs: f32) -> bool {
    (lhs - rhs).abs() <= CLOSE_TOLERANCE
}

/// Widens a vegetation-domain integer to `f32`.
///
/// Sector coordinates, densities and sizes are small editor-authored integers —
/// `sector_density` defaults to 20 and `view_rectangle_size` to 13 — so they sit
/// far inside the 24 bits `f32` represents exactly. Past 2^24 the low bits drop,
/// which moves a placement point by less than one grid step; the C++ source
/// widens the same fields the same way.
#[allow(clippy::cast_precision_loss)]
pub const fn to_f32(value: i32) -> f32 {
    value as f32
}

/// Widens a placement-grid count to `f32`. See [`to_f32`] for the range.
#[allow(clippy::cast_precision_loss)]
pub const fn count_to_f32(value: usize) -> f32 {
    value as f32
}

/// Narrows a world-space coordinate onto the integer sector lattice.
///
/// std has no checked `f32 -> i32`, and the saturating `as` is what the C++
/// `(int)` cast does: a world coordinate past `i32` is outside every sector
/// rectangle either way, so it clamps to one that is also outside.
#[allow(clippy::cast_possible_truncation)]
pub const fn to_i32(value: f32) -> i32 {
    value as i32
}
