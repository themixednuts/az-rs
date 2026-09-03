use az_physics::{Axis3, ColliderShape, PhysicsError, PhysicsPose};
use glam::{Quat, Vec3};
use rapier3d::prelude::{ColliderBuilder, Rotation, Vector};

/// Converts an unsigned integer to `f32` without a lossy cast.
///
/// Both 16-bit halves convert exactly and the fused multiply-add rounds the
/// recombined value once, so the result is bit-identical to `value as f32`
/// for every input.
#[inline]
pub fn f32_from_u32(value: u32) -> f32 {
    let bytes = value.to_be_bytes();
    let high = u16::from_be_bytes([bytes[0], bytes[1]]);
    let low = u16::from_be_bytes([bytes[2], bytes[3]]);
    f32::from(high).mul_add(65_536.0, f32::from(low))
}

/// Converts a container length or index to `f32`.
///
/// Vertex, index, wheel, and segment counts come from authored assets and stay
/// far below `u32::MAX`; a longer container saturates instead of wrapping.
#[inline]
pub fn f32_from_usize(value: usize) -> f32 {
    f32_from_u32(u32::try_from(value).unwrap_or(u32::MAX))
}

/// Converts a signed count, gear index, or difference to `f32`.
#[inline]
pub fn f32_from_i32(value: i32) -> f32 {
    let magnitude = f32_from_u32(value.unsigned_abs());
    if value < 0 { -magnitude } else { magnitude }
}

/// Converts a container length or index to the `u32` an index buffer uses.
///
/// A mesh holding more than `u32::MAX` vertices cannot be addressed by a `u32`
/// index at all, so the conversion saturates instead of wrapping.
#[inline]
pub fn u32_from_usize(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Converts a container length to the `i32` the vehicle gear tables use.
///
/// Gear tables are authored and hold a handful of entries, so a longer table
/// saturates instead of wrapping to a negative gear.
#[inline]
pub fn i32_from_usize(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Number of equal substeps that cover `time_step` with none longer than
/// `maximum`.
///
/// Rust's float-to-integer `as` saturates at the type bounds and maps NaN to
/// zero, and the `max(1)` turns every degenerate quotient into a single
/// substep, so the truncation and sign loss the lints describe are exactly the
/// intended clamp. Rust has no non-`as` float-to-integer conversion.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the saturating float-to-integer cast is the intended clamp and has no non-`as` equivalent"
)]
#[inline]
pub fn substeps(time_step: f32, maximum: f32) -> u32 {
    ((time_step / maximum).ceil() as u32).max(1)
}

/// Rounds `value` to the nearest `i64` quantization key.
///
/// Rust's float-to-integer `as` saturates at the type bounds and maps NaN to
/// zero, which is the wanted behaviour for a spatial hash key, and Rust has no
/// non-`as` float-to-integer conversion.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the saturating float-to-integer cast is intended for a hash key and has no non-`as` equivalent"
)]
#[inline]
pub const fn i64_from_f32(value: f32) -> i64 {
    value.round() as i64
}

pub const fn vector(value: Vec3) -> Vector {
    Vector::new(value.x, value.y, value.z)
}

pub const fn vec3(value: Vector) -> Vec3 {
    Vec3::new(value.x, value.y, value.z)
}

fn rotation(value: Quat) -> Rotation {
    Rotation::from_xyzw(value.x, value.y, value.z, value.w)
}

fn quat(value: Rotation) -> Quat {
    Quat::from_xyzw(value.x, value.y, value.z, value.w)
}

pub fn pose(value: PhysicsPose) -> rapier3d::math::Pose {
    rapier3d::math::Pose::from_parts(vector(value.translation), rotation(value.rotation))
}

pub fn physics_pose(value: &rapier3d::math::Pose) -> PhysicsPose {
    PhysicsPose {
        translation: vec3(value.translation),
        rotation: quat(value.rotation),
    }
}

pub fn collider(shape: &ColliderShape) -> Result<(ColliderBuilder, PhysicsPose), PhysicsError> {
    let pair = match shape {
        ColliderShape::Sphere { radius } => (ColliderBuilder::ball(*radius), PhysicsPose::IDENTITY),
        ColliderShape::Cuboid { half_extents } => (
            ColliderBuilder::cuboid(half_extents.x, half_extents.y, half_extents.z),
            PhysicsPose::IDENTITY,
        ),
        ColliderShape::Capsule {
            axis,
            half_height,
            radius,
        } => (capsule(*axis, *half_height, *radius), PhysicsPose::IDENTITY),
        ColliderShape::Cylinder {
            axis,
            half_height,
            radius,
        } => (
            ColliderBuilder::cylinder(*half_height, *radius),
            rotation_pose(axis_rotation(*axis)),
        ),
        ColliderShape::RoundedCuboid {
            half_extents,
            border_radius,
        } => (
            ColliderBuilder::round_cuboid(
                half_extents.x,
                half_extents.y,
                half_extents.z,
                *border_radius,
            ),
            PhysicsPose::IDENTITY,
        ),
        ColliderShape::RoundedCylinder {
            axis,
            half_height,
            radius,
            border_radius,
        } => (
            ColliderBuilder::round_cylinder(*half_height, *radius, *border_radius),
            rotation_pose(axis_rotation(*axis)),
        ),
        ColliderShape::CapsuleSegment {
            endpoint_a,
            endpoint_b,
            radius,
        } => (
            ColliderBuilder::capsule_from_endpoints(
                vector(*endpoint_a),
                vector(*endpoint_b),
                *radius,
            ),
            PhysicsPose::IDENTITY,
        ),
        ColliderShape::RoundedCylinderSegment {
            endpoint_a,
            endpoint_b,
            radius,
            border_radius,
        } => rounded_cylinder_segment(*endpoint_a, *endpoint_b, *radius, *border_radius),
        ColliderShape::ConvexHull {
            points,
            border_radius,
        } => (convex_hull(points, *border_radius)?, PhysicsPose::IDENTITY),
        ColliderShape::Triangle {
            vertices,
            border_radius,
        } => (triangle(*vertices, *border_radius), PhysicsPose::IDENTITY),
        ColliderShape::TriangleMesh { vertices, indices } => (
            trimesh(vertices.iter().copied(), indices.iter().copied())?,
            PhysicsPose::IDENTITY,
        ),
        ColliderShape::Plane {
            normal,
            offset,
            aabb_min,
            aabb_max,
        } => (
            clipped_plane(*normal, *offset, *aabb_min, *aabb_max)?,
            PhysicsPose::IDENTITY,
        ),
        ColliderShape::HeightField {
            width,
            length,
            heights,
            aabb_min,
            aabb_max,
            up_axis,
        } => (
            height_field_mesh(*width, *length, heights, *aabb_min, *aabb_max, *up_axis)?,
            PhysicsPose::IDENTITY,
        ),
    };
    Ok(pair)
}

fn capsule(axis: Axis3, half_height: f32, radius: f32) -> ColliderBuilder {
    match axis {
        Axis3::X => ColliderBuilder::capsule_x(half_height, radius),
        Axis3::Y => ColliderBuilder::capsule_y(half_height, radius),
        Axis3::Z => ColliderBuilder::capsule_z(half_height, radius),
    }
}

/// Rapier has no rounded-cylinder segment, so the segment is expressed as a
/// centred rounded cylinder plus the rotation that puts its `Y` axis on the
/// segment.
fn rounded_cylinder_segment(
    endpoint_a: Vec3,
    endpoint_b: Vec3,
    radius: f32,
    border_radius: f32,
) -> (ColliderBuilder, PhysicsPose) {
    let delta = endpoint_b - endpoint_a;
    (
        ColliderBuilder::round_cylinder(delta.length() * 0.5, radius, border_radius),
        PhysicsPose {
            translation: (endpoint_a + endpoint_b) * 0.5,
            rotation: Quat::from_rotation_arc(Vec3::Y, delta.normalize()),
        },
    )
}

/// # Errors
///
/// Returns [`PhysicsError::InvalidColliderTopology`] when the authored points
/// are degenerate enough that Rapier cannot hull them into a volume.
fn convex_hull(points: &[Vec3], border_radius: f32) -> Result<ColliderBuilder, PhysicsError> {
    let points: Vec<_> = points.iter().copied().map(vector).collect();
    if border_radius > 0.0 {
        ColliderBuilder::round_convex_hull(&points, border_radius)
    } else {
        ColliderBuilder::convex_hull(&points)
    }
    .ok_or(PhysicsError::InvalidColliderTopology(
        "convex hull points do not span a volume",
    ))
}

fn triangle(vertices: [Vec3; 3], border_radius: f32) -> ColliderBuilder {
    let [a, b, c] = vertices.map(vector);
    if border_radius > 0.0 {
        ColliderBuilder::round_triangle(a, b, c, border_radius)
    } else {
        ColliderBuilder::triangle(a, b, c)
    }
}

const fn rotation_pose(rotation: Quat) -> PhysicsPose {
    PhysicsPose {
        translation: Vec3::ZERO,
        rotation,
    }
}

fn axis_rotation(axis: Axis3) -> Quat {
    match axis {
        Axis3::X => Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2),
        Axis3::Y => Quat::IDENTITY,
        Axis3::Z => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
    }
}

fn trimesh(
    vertices: impl IntoIterator<Item = Vec3>,
    indices: impl IntoIterator<Item = [u32; 3]>,
) -> Result<ColliderBuilder, PhysicsError> {
    ColliderBuilder::trimesh(
        vertices.into_iter().map(vector).collect(),
        indices.into_iter().collect(),
    )
    .map_err(|_| PhysicsError::InvalidColliderTopology("triangle mesh construction failed"))
}

fn clipped_plane(
    normal: Vec3,
    offset: f32,
    aabb_min: Vec3,
    aabb_max: Vec3,
) -> Result<ColliderBuilder, PhysicsError> {
    /// Corner index pairs for the twelve edges of the authored bounds.
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (0, 2),
        (1, 3),
        (2, 3),
        (4, 5),
        (4, 6),
        (5, 7),
        (6, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];

    let normal = normal.normalize();
    let corners = [
        Vec3::new(aabb_min.x, aabb_min.y, aabb_min.z),
        Vec3::new(aabb_max.x, aabb_min.y, aabb_min.z),
        Vec3::new(aabb_min.x, aabb_max.y, aabb_min.z),
        Vec3::new(aabb_max.x, aabb_max.y, aabb_min.z),
        Vec3::new(aabb_min.x, aabb_min.y, aabb_max.z),
        Vec3::new(aabb_max.x, aabb_min.y, aabb_max.z),
        Vec3::new(aabb_min.x, aabb_max.y, aabb_max.z),
        Vec3::new(aabb_max.x, aabb_max.y, aabb_max.z),
    ];
    let mut points = Vec::with_capacity(6);
    for (start, end) in EDGES {
        let a = corners[start];
        let b = corners[end];
        let da = normal.dot(a) + offset;
        let db = normal.dot(b) + offset;
        if da.abs() <= 1.0e-5 {
            push_unique(&mut points, a);
        }
        if db.abs() <= 1.0e-5 {
            push_unique(&mut points, b);
        }
        if da.signum() != db.signum() {
            push_unique(&mut points, a + (b - a) * (da / (da - db)));
        }
    }
    if points.len() < 3 {
        return Err(PhysicsError::InvalidColliderTopology(
            "plane does not intersect its authored bounds",
        ));
    }
    let center = points.iter().copied().sum::<Vec3>() / f32_from_usize(points.len());
    let reference = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let tangent = normal.cross(reference).normalize();
    let bitangent = normal.cross(tangent);
    points.sort_by(|left, right| {
        let left = *left - center;
        let right = *right - center;
        left.dot(bitangent)
            .atan2(left.dot(tangent))
            .total_cmp(&right.dot(bitangent).atan2(right.dot(tangent)))
    });
    let mut indices: Vec<_> = (1..points.len() - 1)
        .map(|index| {
            let index = u32_from_usize(index);
            [0, index, index + 1]
        })
        .collect();
    if (points[1] - points[0])
        .cross(points[2] - points[0])
        .dot(normal)
        < 0.0
    {
        for triangle in &mut indices {
            triangle.swap(1, 2);
        }
    }
    trimesh(points, indices)
}

fn push_unique(points: &mut Vec<Vec3>, point: Vec3) {
    if points
        .iter()
        .all(|existing| existing.distance_squared(point) > 1.0e-10)
    {
        points.push(point);
    }
}

fn height_field_mesh(
    width: u32,
    length: u32,
    heights: &[f32],
    aabb_min: Vec3,
    aabb_max: Vec3,
    up_axis: Axis3,
) -> Result<ColliderBuilder, PhysicsError> {
    let width = width as usize;
    let length = length as usize;
    let mut vertices = Vec::with_capacity(width * length);
    for row in 0..length {
        let row_fraction = f32_from_usize(row) / f32_from_usize(length - 1);
        for column in 0..width {
            let column_fraction = f32_from_usize(column) / f32_from_usize(width - 1);
            let height = heights[row * width + column];
            vertices.push(match up_axis {
                Axis3::X => Vec3::new(
                    height,
                    (aabb_max.y - aabb_min.y).mul_add(column_fraction, aabb_min.y),
                    (aabb_max.z - aabb_min.z).mul_add(row_fraction, aabb_min.z),
                ),
                Axis3::Y => Vec3::new(
                    (aabb_max.x - aabb_min.x).mul_add(column_fraction, aabb_min.x),
                    height,
                    (aabb_max.z - aabb_min.z).mul_add(row_fraction, aabb_min.z),
                ),
                Axis3::Z => Vec3::new(
                    (aabb_max.x - aabb_min.x).mul_add(column_fraction, aabb_min.x),
                    (aabb_max.y - aabb_min.y).mul_add(row_fraction, aabb_min.y),
                    height,
                ),
            });
        }
    }
    let mut indices = Vec::with_capacity((width - 1) * (length - 1) * 2);
    for row in 0..length - 1 {
        for column in 0..width - 1 {
            let a = u32_from_usize(row * width + column);
            let b = a + 1;
            let c = a + u32_from_usize(width);
            let d = c + 1;
            indices.push([a, c, d]);
            indices.push([a, d, b]);
        }
    }
    let up = match up_axis {
        Axis3::X => Vec3::X,
        Axis3::Y => Vec3::Y,
        Axis3::Z => Vec3::Z,
    };
    if let Some(first) = indices.first()
        && (vertices[first[1] as usize] - vertices[first[0] as usize])
            .cross(vertices[first[2] as usize] - vertices[first[0] as usize])
            .dot(up)
            < 0.0
    {
        for triangle in &mut indices {
            triangle.swap(1, 2);
        }
    }
    trimesh(vertices, indices)
}
