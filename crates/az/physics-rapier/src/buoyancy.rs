//! `CryPhysics` submerged-volume and medium-resistance geometry.
//!
//! The routines mirror `IGeometry::CalculateBuoyancy`,
//! `IGeometry::CalculateMediumResistance`, and the shared
//! `CalcMediumResistance` surface integral. Inputs stay in Azoth's typed shape
//! model; no Rapier handles leak into the calculation.

use az_physics::{Axis3, ColliderShape, FluidPlane, PhysicsPose};
use glam::{Quat, Vec2, Vec3};
use rapier3d::parry::{math::Vector, transformation::try_convex_hull};

use crate::convert::{f32_from_usize, u32_from_usize};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SubmergedGeometry {
    pub volume: f32,
    pub full_volume: f32,
    pub center: Vec3,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MediumResistance {
    pub linear: Vec3,
    pub angular: Vec3,
}

#[derive(Debug, Clone, Copy)]
pub struct MediumMotion {
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub center_of_mass: Vec3,
}

/// Cry's `CalcMediumResistance` polynomial moments over one boundary segment:
/// `xy` is the integral of `x * y`, `xxx` the integral of `x³`, and so on.
#[derive(Debug, Clone, Copy)]
struct SegmentMoments {
    xy: f32,
    xx: f32,
    xxy: f32,
    xyy: f32,
    xxx: f32,
}

pub fn submerged_geometry(
    shape: &ColliderShape,
    pose: PhysicsPose,
    plane: FluidPlane,
) -> SubmergedGeometry {
    match shape {
        ColliderShape::Sphere { radius } => sphere_submerged(*radius, pose.translation, plane),
        ColliderShape::Cuboid { half_extents } => {
            let (vertices, indices) = cuboid_mesh(*half_extents);
            mesh_submerged(&vertices, &indices, pose, plane)
        }
        ColliderShape::Capsule {
            axis,
            half_height,
            radius,
        } => cylinder_submerged(
            *radius,
            (*radius).mul_add(2.0f32 / 3.0, *half_height),
            axis_vector(*axis),
            pose,
            plane,
        ),
        ColliderShape::Cylinder {
            axis,
            half_height,
            radius,
        } => cylinder_submerged(*radius, *half_height, axis_vector(*axis), pose, plane),
        ColliderShape::CapsuleSegment {
            endpoint_a,
            endpoint_b,
            radius,
        } => {
            let segment = *endpoint_b - *endpoint_a;
            let center = (*endpoint_a + *endpoint_b) * 0.5;
            let local_pose = PhysicsPose {
                translation: center,
                rotation: Quat::from_rotation_arc(Vec3::Z, segment.normalize()),
            };
            cylinder_submerged(
                *radius,
                (*radius).mul_add(2.0f32 / 3.0, segment.length() * 0.5),
                Vec3::Z,
                pose * local_pose,
                plane,
            )
        }
        ColliderShape::ConvexHull { points, .. } => {
            let native = points
                .iter()
                .map(|point| Vector::new(point.x, point.y, point.z))
                .collect::<Vec<_>>();
            let Ok((vertices, indices)) = try_convex_hull(&native) else {
                return SubmergedGeometry::default();
            };
            let vertices = vertices
                .into_iter()
                .map(|point| Vec3::new(point.x, point.y, point.z))
                .collect::<Vec<_>>();
            mesh_submerged(&vertices, &indices, pose, plane)
        }
        ColliderShape::TriangleMesh { vertices, indices } => {
            mesh_submerged(vertices, indices, pose, plane)
        }
        // Open or height-field surfaces have no closed displaced volume.
        //
        // Rounded RockNRoll shapes do not occur in normal LmbrCentral's Cry
        // primitive component either. Their buoyancy is evaluated through their
        // closed solver mesh once materialized, not by changing the authored
        // radius here.
        ColliderShape::Triangle { .. }
        | ColliderShape::Plane { .. }
        | ColliderShape::HeightField { .. }
        | ColliderShape::RoundedCuboid { .. }
        | ColliderShape::RoundedCylinder { .. }
        | ColliderShape::RoundedCylinderSegment { .. } => SubmergedGeometry::default(),
    }
}

pub fn medium_resistance(
    shape: &ColliderShape,
    pose: PhysicsPose,
    plane: FluidPlane,
    motion: MediumMotion,
) -> MediumResistance {
    match shape {
        ColliderShape::Sphere { radius } => {
            sphere_medium_resistance(*radius, pose.translation, plane, motion)
        }
        ColliderShape::Cuboid { half_extents } => {
            let (vertices, indices) = cuboid_mesh(*half_extents);
            mesh_medium_resistance(&vertices, &indices, pose, plane, motion)
        }
        ColliderShape::ConvexHull { points, .. } => {
            let native = points
                .iter()
                .map(|point| Vector::new(point.x, point.y, point.z))
                .collect::<Vec<_>>();
            let Ok((vertices, indices)) = try_convex_hull(&native) else {
                return MediumResistance::default();
            };
            let vertices = vertices
                .into_iter()
                .map(|point| Vec3::new(point.x, point.y, point.z))
                .collect::<Vec<_>>();
            mesh_medium_resistance(&vertices, &indices, pose, plane, motion)
        }
        ColliderShape::TriangleMesh { vertices, indices } => {
            mesh_medium_resistance(vertices, indices, pose, plane, motion)
        }
        ColliderShape::Triangle { vertices, .. } => {
            let world = vertices.map(|point| pose.transform_point(point));
            let normal = (world[1] - world[0])
                .cross(world[2] - world[0])
                .normalize_or_zero();
            polygon_medium_resistance(&world, normal, plane, motion)
        }
        ColliderShape::Cylinder {
            axis,
            half_height,
            radius,
        } => revolved_medium_resistance(
            *radius,
            *half_height,
            axis_vector(*axis),
            false,
            pose,
            plane,
            motion,
        ),
        ColliderShape::Capsule {
            axis,
            half_height,
            radius,
        } => revolved_medium_resistance(
            *radius,
            *half_height,
            axis_vector(*axis),
            true,
            pose,
            plane,
            motion,
        ),
        ColliderShape::CapsuleSegment {
            endpoint_a,
            endpoint_b,
            radius,
        } => {
            let segment = *endpoint_b - *endpoint_a;
            let local_pose = PhysicsPose {
                translation: (*endpoint_a + *endpoint_b) * 0.5,
                rotation: Quat::from_rotation_arc(Vec3::Z, segment.normalize()),
            };
            revolved_medium_resistance(
                *radius,
                segment.length() * 0.5,
                Vec3::Z,
                true,
                pose * local_pose,
                plane,
                motion,
            )
        }
        ColliderShape::RoundedCuboid { .. }
        | ColliderShape::RoundedCylinder { .. }
        | ColliderShape::RoundedCylinderSegment { .. }
        | ColliderShape::Plane { .. }
        | ColliderShape::HeightField { .. } => MediumResistance::default(),
    }
}

fn sphere_submerged(radius: f32, center: Vec3, plane: FluidPlane) -> SubmergedGeometry {
    let x = (plane.origin - center).dot(plane.normal);
    let full_volume = (4.0f32 / 3.0) * core::f32::consts::PI * radius.powi(3);
    if x < -radius {
        return SubmergedGeometry {
            full_volume,
            center,
            ..Default::default()
        };
    }
    if x > radius {
        return SubmergedGeometry {
            volume: full_volume,
            full_volume,
            center,
        };
    }
    let volume = core::f32::consts::PI
        * x.mul_add(
            (x * x).mul_add(-(1.0f32 / 3.0), radius * radius),
            (2.0f32 / 3.0) * radius.powi(3),
        );
    let first_moment = core::f32::consts::PI
        * 0.25f32.mul_add(
            -radius.powi(4),
            0.25f32.mul_add(-x.powi(4), 0.5 * radius * radius * x * x),
        );
    SubmergedGeometry {
        volume,
        full_volume,
        center: center + plane.normal * (first_moment / volume),
    }
}

/// Exact cylinder displaced volume from `CCylinderGeom::CalculateBuoyancy`.
/// Capsules pass an equal-volume cylinder half-height, matching the native
/// `GEOM_CAPSULE` branch.
fn cylinder_submerged(
    radius: f32,
    half_height: f32,
    local_axis: Vec3,
    pose: PhysicsPose,
    plane: FluidPlane,
) -> SubmergedGeometry {
    let height = half_height * 2.0;
    let radius_squared = radius * radius;
    let axis = pose.rotation * local_axis;
    let axis_z = if axis.dot(plane.normal) < 0.0 {
        -axis
    } else {
        axis
    };
    let bottom = pose.translation - axis_z * (height * 0.5);
    let full_volume = core::f32::consts::PI * radius_squared * height;
    let axis_y = plane.normal.cross(axis_z);

    if axis_y.length_squared() < 0.0001 {
        // The cut plane is square to the axis, so the wetted part is a stub
        // cylinder rather than an oblique wedge.
        let wetted_height = (plane.origin - bottom).dot(plane.normal).clamp(0.0, height);
        return SubmergedGeometry {
            volume: core::f32::consts::PI * radius_squared * wetted_height,
            full_volume,
            center: bottom + axis_z * (wetted_height * 0.5),
        };
    }
    let cut = CylinderCut {
        radius,
        radius_squared,
        height,
        bottom,
        axis_x: axis_y.normalize().cross(axis_z),
        axis_z,
        plane,
    };
    if cut.radial_tilt().abs() <= f32::EPSILON {
        return mesh_revolved_submerged(radius, half_height, local_axis, pose, plane, 192);
    }

    match cut.displaced() {
        ObliqueVolume::Dry => SubmergedGeometry {
            full_volume,
            center: pose.translation,
            ..SubmergedGeometry::default()
        },
        ObliqueVolume::Full => SubmergedGeometry {
            volume: full_volume,
            full_volume,
            center: pose.translation,
        },
        ObliqueVolume::Partial { volume, moment } => SubmergedGeometry {
            volume: volume.max(0.0),
            full_volume,
            center: if volume > 0.0 {
                bottom + moment / volume
            } else {
                pose.translation
            },
        },
    }
}

/// A cylinder crossed obliquely by the fluid plane, in the cylinder's own
/// frame: `axis_z` runs from the wetted end toward the dry end, and `axis_x` is
/// the radial direction the cut slopes along.
#[derive(Debug, Clone, Copy)]
struct CylinderCut {
    radius: f32,
    radius_squared: f32,
    height: f32,
    bottom: Vec3,
    axis_x: Vec3,
    axis_z: Vec3,
    plane: FluidPlane,
}

/// Outcome of intersecting a cylinder with the fluid plane.
enum ObliqueVolume {
    /// The plane clears the cylinder on the dry side.
    Dry,
    /// The plane clears the cylinder on the wetted side.
    Full,
    /// Displaced volume and its first moment about the cylinder's wetted end.
    Partial { volume: f32, moment: Vec3 },
}

impl CylinderCut {
    /// How steeply the cut slopes across the cylinder's cross-section.
    fn radial_tilt(self) -> f32 {
        self.axis_x.dot(self.plane.normal)
    }

    /// How steeply the cut slopes along the cylinder's axis.
    fn axial_tilt(self) -> f32 {
        self.axis_z.dot(self.plane.normal)
    }

    /// Plane offset of `point` in units of the radial tilt.
    fn offset_of(self, point: Vec3) -> f32 {
        (self.plane.origin - point).dot(self.plane.normal) / self.radial_tilt()
    }

    /// Accumulates the up-to-three native pieces: the stub cylinder below the
    /// entry offset, the circular segment beside the exit offset, and the
    /// oblique wedge between them.
    fn displaced(self) -> ObliqueVolume {
        let radius = self.radius;
        let radius_squared = self.radius_squared;
        let height = self.height;
        let axial_tilt = self.axial_tilt();
        let mut volumes = [0.0; 4];
        let mut moments = [Vec3::ZERO; 4];
        let mut pieces = 0usize;

        let mut entry = self.offset_of(self.bottom);
        let entry_height;
        if entry >= radius {
            return ObliqueVolume::Dry;
        } else if entry < -radius {
            if axial_tilt * axial_tilt < 0.0001 {
                return ObliqueVolume::Full;
            }
            entry = -radius * 0.9999;
            entry_height = (self.plane.origin - (self.bottom - self.axis_x * radius))
                .dot(self.plane.normal)
                / axial_tilt;
            volumes[pieces] = core::f32::consts::PI * radius_squared * entry_height;
            moments[pieces] = self.axis_z * (entry_height * 0.5 * volumes[pieces]);
            pieces += 1;
        } else {
            entry_height = 0.0;
        }

        let mut exit = self.offset_of(self.bottom + self.axis_z * height);
        let exit_height;
        if exit < -radius {
            return ObliqueVolume::Full;
        } else if exit > radius {
            exit = radius * 0.9999;
            exit_height = (self.plane.origin - (self.bottom + self.axis_x * radius))
                .dot(self.plane.normal)
                / axial_tilt;
        } else {
            exit_height = height;
            let exit_chord = exit.mul_add(-exit, radius_squared).sqrt();
            volumes[pieces] = radius_squared.mul_add(
                core::f32::consts::FRAC_PI_2 - (exit / radius).asin(),
                -(exit * exit_chord),
            ) * (height - entry_height);
            moments[pieces] = self.axis_x
                * ((2.0f32 / 3.0) * exit_chord.powi(3) * (height - entry_height))
                + self.axis_z * ((height + entry_height) * 0.5 * volumes[pieces]);
            pieces += 1;
        }

        if exit > radius.mul_add(0.01, entry) {
            let (volume, moment) = self.wedge(entry, exit, entry_height, exit_height);
            volumes[pieces] = volume;
            moments[pieces] = moment;
            pieces += 1;
        }

        ObliqueVolume::Partial {
            volume: volumes[..pieces].iter().sum::<f32>(),
            moment: moments[..pieces].iter().copied().sum::<Vec3>(),
        }
    }

    /// Volume and first moment of the oblique wedge between the entry and exit
    /// offsets, whose sloping face is the cut plane.
    fn wedge(self, entry: f32, exit: f32, entry_height: f32, exit_height: f32) -> (f32, Vec3) {
        let radius = self.radius;
        let radius_squared = self.radius_squared;
        let inverse_span = 1.0 / (exit - entry);
        let slope = (exit_height - entry_height) * inverse_span;
        let entry_chord = entry.mul_add(-entry, radius_squared).sqrt();
        let entry_angle = (entry / radius).asin();
        let exit_chord = exit.mul_add(-exit, radius_squared).sqrt();
        let exit_angle = (exit / radius).asin();
        let volume = slope
            * radius_squared.mul_add(
                -(entry.mul_add(-entry_angle, exit * exit_angle) + exit_chord - entry_chord),
                (1.0f32 / 3.0).mul_add(
                    exit_chord.powi(3) - entry_chord.powi(3),
                    radius_squared.mul_add(exit_angle, exit * exit_chord) * (exit - entry),
                ),
            );
        let moment = self.axis_z
            * (slope * slope * 0.5).mul_add(
                radius_squared.mul_add(
                    -(entry * entry).mul_add(
                        -entry_angle,
                        (exit * exit).mul_add(
                            exit_angle,
                            (0.25 * radius_squared).mul_add(
                                -(exit_angle - entry_angle),
                                0.75 * entry.mul_add(-entry_chord, exit * exit_chord),
                            ),
                        ),
                    ),
                    0.5f32.mul_add(
                        entry.mul_add(-entry_chord.powi(3), exit * exit_chord.powi(3)),
                        radius_squared.mul_add(exit_angle, exit * exit_chord)
                            * entry.mul_add(-entry, exit * exit),
                    ),
                ),
                volume * entry.mul_add(-slope, entry_height),
            )
            + self.axis_x
                * ((2.0f32 / 3.0)
                    * slope
                    * entry_chord.powi(3).mul_add(
                        -(exit - entry),
                        ((3.0f32 / 8.0) * radius_squared).mul_add(
                            radius_squared.mul_add(
                                exit_angle - entry_angle,
                                entry.mul_add(-entry_chord, exit * exit_chord),
                            ),
                            0.25 * entry.mul_add(-entry_chord.powi(3), exit * exit_chord.powi(3)),
                        ),
                    ));
        (volume, moment)
    }
}

fn mesh_submerged(
    vertices: &[Vec3],
    indices: &[[u32; 3]],
    pose: PhysicsPose,
    plane: FluidPlane,
) -> SubmergedGeometry {
    let basis_x = plane.normal.any_orthonormal_vector();
    let basis_y = plane.normal.cross(basis_x);
    let to_plane = |point: Vec3| {
        let relative = pose.transform_point(point) - plane.origin;
        Vec3::new(
            relative.dot(basis_x),
            relative.dot(basis_y),
            relative.dot(plane.normal),
        )
    };
    let points = vertices.iter().copied().map(to_plane).collect::<Vec<_>>();
    let mut surface_triangles = Vec::<[Vec3; 3]>::new();
    let mut cap_points = Vec::<Vec3>::new();
    for &[a, b, c] in indices {
        let polygon = clip_polygon_below_plane(
            &[points[a as usize], points[b as usize], points[c as usize]],
            &mut cap_points,
        );
        for index in 1..polygon.len().saturating_sub(1) {
            surface_triangles.push([polygon[0], polygon[index], polygon[index + 1]]);
        }
    }
    deduplicate_points(&mut cap_points);
    if cap_points.len() >= 3 {
        let center = cap_points.iter().copied().sum::<Vec3>() / f32_from_usize(cap_points.len());
        cap_points.sort_by(|left, right| {
            (left.y - center.y)
                .atan2(left.x - center.x)
                .total_cmp(&(right.y - center.y).atan2(right.x - center.x))
        });
        for index in 1..cap_points.len() - 1 {
            surface_triangles.push([cap_points[0], cap_points[index], cap_points[index + 1]]);
        }
    }

    let (mut volume, mut moment) =
        surface_triangles
            .iter()
            .fold((0.0, Vec3::ZERO), |(volume, moment), &[a, b, c]| {
                let tetrahedron_volume = a.dot(b.cross(c)) / 6.0;
                (
                    volume + tetrahedron_volume,
                    moment + (a + b + c) * (0.25 * tetrahedron_volume),
                )
            });
    if volume < 0.0 {
        volume = -volume;
        moment = -moment;
    }
    let full_volume = mesh_volume(&points, indices);
    let center = if volume > f32::EPSILON {
        let local = moment / volume;
        plane.origin + basis_x * local.x + basis_y * local.y + plane.normal * local.z
    } else {
        pose.translation
    };
    SubmergedGeometry {
        volume: volume.max(0.0),
        full_volume,
        center,
    }
}

fn clip_polygon_below_plane(source: &[Vec3], cap_points: &mut Vec<Vec3>) -> Vec<Vec3> {
    let mut output = Vec::with_capacity(source.len() + 2);
    for index in 0..source.len() {
        let current = source[index];
        let next = source[(index + 1) % source.len()];
        let current_inside = current.z <= 0.0;
        let next_inside = next.z <= 0.0;
        if current_inside {
            output.push(current);
        }
        if current_inside != next_inside {
            let mut intersection = current.lerp(next, -current.z / (next.z - current.z));
            intersection.z = 0.0;
            output.push(intersection);
            cap_points.push(intersection);
        }
    }
    output
}

fn deduplicate_points(points: &mut Vec<Vec3>) {
    let mut unique = Vec::<Vec3>::with_capacity(points.len());
    for point in points.drain(..) {
        if !unique
            .iter()
            .any(|candidate| candidate.distance_squared(point) <= 1.0e-10)
        {
            unique.push(point);
        }
    }
    *points = unique;
}

fn mesh_volume(vertices: &[Vec3], indices: &[[u32; 3]]) -> f32 {
    indices
        .iter()
        .map(|triangle| {
            vertices[triangle[0] as usize]
                .dot(vertices[triangle[1] as usize].cross(vertices[triangle[2] as usize]))
                / 6.0
        })
        .sum::<f32>()
        .abs()
}

fn mesh_medium_resistance(
    vertices: &[Vec3],
    indices: &[[u32; 3]],
    pose: PhysicsPose,
    plane: FluidPlane,
    motion: MediumMotion,
) -> MediumResistance {
    indices
        .iter()
        .fold(MediumResistance::default(), |mut total, triangle| {
            let points = [
                pose.transform_point(vertices[triangle[0] as usize]),
                pose.transform_point(vertices[triangle[1] as usize]),
                pose.transform_point(vertices[triangle[2] as usize]),
            ];
            let normal = (points[1] - points[0])
                .cross(points[2] - points[0])
                .normalize_or_zero();
            let contribution = polygon_medium_resistance(&points, normal, plane, motion);
            total.linear += contribution.linear;
            total.angular += contribution.angular;
            total
        })
}

/// Direct port of Cry's shared `CalcMediumResistance` polygon integral.
#[allow(clippy::many_single_char_names)]
fn polygon_medium_resistance(
    source: &[Vec3],
    normal: Vec3,
    plane: FluidPlane,
    motion: MediumMotion,
) -> MediumResistance {
    let submerged = crop_polygon(source, plane.normal, plane.origin.dot(plane.normal));
    let relative = submerged
        .into_iter()
        .map(|point| point - motion.center_of_mass)
        .collect::<Vec<_>>();
    let loaded = crop_polygon(
        &relative,
        motion.angular_velocity.cross(normal),
        motion.linear_velocity.dot(normal),
    );
    if loaded.is_empty() {
        return MediumResistance::default();
    }

    let basis_x = normal.any_orthonormal_vector();
    let basis_y = normal.cross(basis_x);
    let v = Vec3::new(
        motion.linear_velocity.dot(basis_x),
        motion.linear_velocity.dot(basis_y),
        motion.linear_velocity.dot(normal),
    );
    let w = Vec3::new(
        motion.angular_velocity.dot(basis_x),
        motion.angular_velocity.dot(basis_y),
        motion.angular_velocity.dot(normal),
    );
    let points = loaded
        .iter()
        .map(|point| Vec3::new(point.dot(basis_x), point.dot(basis_y), point.dot(normal)))
        .collect::<Vec<_>>();

    let mut pressure = Vec3::ZERO;
    let mut torque = Vec3::ZERO;
    let mut doubled_area = 0.0;
    for index in 0..points.len() {
        let p0 = points[index];
        let p1 = points[(index + 1) % points.len()];
        let dx = p1.x - p0.x;
        let dy = p1.y - p0.y;
        doubled_area += p1.x.mul_add(-p0.y, p0.x * p1.y);
        let moments = SegmentMoments {
            xy: (dx * dy).mul_add(
                1.0f32 / 3.0,
                dy.mul_add(p0.x, dx * p0.y).mul_add(0.5, p0.x * p0.y),
            ),
            xx: (dx * dx).mul_add(1.0f32 / 3.0, dx.mul_add(p0.x, p0.x * p0.x)),
            xxy: (p0.x * p0.x).mul_add(
                p0.y,
                (p0.x * p0.x).mul_add(dy, p0.x * p0.y * dx * 2.0).mul_add(
                    0.5,
                    (dx * dy * p0.x)
                        .mul_add(2.0, dx * dx * p0.y)
                        .mul_add(1.0f32 / 3.0, dx * dx * dy * 0.25),
                ),
            ),
            xyy: (p0.y * p0.y).mul_add(
                p0.x,
                (p0.y * p0.y).mul_add(dx, p0.y * p0.x * dy * 2.0).mul_add(
                    0.5,
                    (dy * dx * p0.y)
                        .mul_add(2.0, dy * dy * p0.x)
                        .mul_add(1.0f32 / 3.0, dy * dy * dx * 0.25),
                ),
            ),
            xxx: (dx * p0.x * p0.x).mul_add(1.5, (dx * dx).mul_add(p0.x, dx.powi(3) * 0.25))
                + p0.x.powi(3),
        };
        pressure.z = dy.mul_add(
            (w.y * 0.5).mul_add(-moments.xx, w.x * moments.xy),
            pressure.z,
        );
        torque.x = (v.z * dy).mul_add(moments.xy, torque.x);
        torque.y = (v.z * dy * 0.5).mul_add(-moments.xx, torque.y);
        torque.x = dy.mul_add(
            (w.y * 0.5).mul_add(-moments.xxy, w.x * moments.xyy),
            torque.x,
        );
        torque.y = dy.mul_add(
            -(w.y * (1.0f32 / 3.0)).mul_add(-moments.xxx, w.x * 0.5 * moments.xxy),
            torque.y,
        );
    }
    pressure.z = (v.z * doubled_area).mul_add(0.5, pressure.z);
    MediumResistance {
        linear: -(basis_x * pressure.x + basis_y * pressure.y + normal * pressure.z),
        angular: -(basis_x * torque.x + basis_y * torque.y + normal * torque.z),
    }
}

fn crop_polygon(source: &[Vec3], normal: Vec3, distance: f32) -> Vec<Vec3> {
    if source.is_empty() {
        return Vec::new();
    }
    let mut output = Vec::with_capacity(source.len() + 2);
    for i in 0..source.len() {
        let current = source[i];
        let next = source[(i + 1) % source.len()];
        let current_distance = current.dot(normal) - distance;
        let next_distance = next.dot(normal) - distance;
        if current_distance < 0.0 {
            output.push(current);
        }
        if current_distance * next_distance < 0.0 {
            output.push(
                current + (next - current) * (-current_distance / (next - current).dot(normal)),
            );
        }
    }
    output
}

#[allow(clippy::many_single_char_names)]
fn sphere_medium_resistance(
    radius: f32,
    center: Vec3,
    plane: FluidPlane,
    motion: MediumMotion,
) -> MediumResistance {
    let v = motion.linear_velocity
        + motion
            .angular_velocity
            .cross(center - motion.center_of_mass);
    let x = (plane.origin - center).dot(plane.normal);
    if x.abs() > radius {
        return if x > radius {
            MediumResistance {
                linear: -v * (core::f32::consts::PI * radius * radius),
                angular: (center - motion.center_of_mass)
                    .cross(-v * (core::f32::consts::PI * radius * radius)),
            }
        } else {
            MediumResistance::default()
        };
    }
    if v.length_squared() <= f32::EPSILON {
        return MediumResistance::default();
    }
    let vn = v.normalize();
    let unnormalized_axis_y = vn.cross(plane.normal);
    let v_cross_n = unnormalized_axis_y.length();
    let (axis_y, l) = if v_cross_n < 0.01 {
        (
            vn.any_orthonormal_vector(),
            x.signum().copysign(1.0) * radius,
        )
    } else {
        (unnormalized_axis_y / v_cross_n, x / v_cross_n)
    };
    let axis_x = axis_y.cross(vn);
    let nv = plane.normal.dot(v);
    let ellipse_center = x * v_cross_n;
    let ellipse_radius = x.mul_add(-x, radius * radius).max(0.0).sqrt().max(0.001);
    let circle_x = l.clamp(-radius * 0.999, radius * 0.999);
    let circle_y = radius
        .mul_add(radius, -(circle_x * circle_x))
        .max(0.0)
        .sqrt();
    let circle_area = (radius * radius).mul_add(
        core::f32::consts::FRAC_PI_2 + (circle_x / radius).asin(),
        circle_x * circle_y,
    ) * 0.5;
    let circle_moment = -circle_y.powi(3) / 3.0;
    let ellipse_x = ((l - ellipse_center) / nv.abs().max(0.001))
        .clamp(-ellipse_radius * 0.999, ellipse_radius * 0.999);
    let ellipse_y = (ellipse_radius * ellipse_radius - ellipse_x * ellipse_x)
        .max(0.0)
        .sqrt();
    let ellipse_area = (ellipse_radius * ellipse_radius).mul_add(
        (ellipse_x / ellipse_radius)
            .asin()
            .mul_add(nv.signum().copysign(1.0), core::f32::consts::FRAC_PI_2),
        ellipse_x * ellipse_y,
    ) * nv.abs()
        * 0.5;
    let ellipse_moment = (-ellipse_y.powi(3) / 3.0).mul_add(nv, ellipse_center);
    let area = circle_area + ellipse_area;
    if area <= f32::EPSILON {
        return MediumResistance::default();
    }
    let center_x = (circle_moment * circle_area + ellipse_moment * ellipse_area) / area;
    let linear = -v * area;
    let pressure_center = center
        + axis_x * center_x
        + vn * radius
            .mul_add(radius, -(center_x * center_x))
            .max(0.0)
            .sqrt();
    MediumResistance {
        linear,
        angular: (pressure_center - motion.center_of_mass).cross(linear),
    }
}

fn revolved_medium_resistance(
    radius: f32,
    half_height: f32,
    local_axis: Vec3,
    capsule: bool,
    pose: PhysicsPose,
    plane: FluidPlane,
    motion: MediumMotion,
) -> MediumResistance {
    // Cry evaluates the curved side as exactly twelve quads.
    let segments = 12usize;
    let axis = pose.rotation * local_axis;
    let radial_x = axis.any_orthonormal_vector();
    let radial_y = axis.cross(radial_x);
    let top = pose.translation + axis * half_height;
    let bottom = pose.translation - axis * half_height;
    let mut total = MediumResistance::default();
    let ring = (0..segments)
        .map(|index| {
            let angle = core::f32::consts::TAU * f32_from_usize(index) / f32_from_usize(segments);
            radial_x * (angle.cos() * radius) + radial_y * (angle.sin() * radius)
        })
        .collect::<Vec<_>>();
    for index in 0..segments {
        let next = (index + 1) % segments;
        let face = [
            top + ring[next],
            top + ring[index],
            bottom + ring[index],
            bottom + ring[next],
        ];
        let normal = (ring[index] + ring[next]).normalize_or_zero();
        add_resistance(
            &mut total,
            polygon_medium_resistance(&face, normal, plane, motion),
        );
    }
    if capsule {
        add_resistance(
            &mut total,
            sphere_medium_resistance(radius, top, plane, motion),
        );
        add_resistance(
            &mut total,
            sphere_medium_resistance(radius, bottom, plane, motion),
        );
    } else {
        add_resistance(
            &mut total,
            disk_medium_resistance(radius, top, axis, plane, motion),
        );
        add_resistance(
            &mut total,
            disk_medium_resistance(radius, bottom, -axis, plane, motion),
        );
    }
    total
}

#[derive(Debug, Clone, Copy)]
struct CircleBoundaryPoint {
    point: Vec2,
    /// `true` when the edge beginning at this point is a line; `false` for a
    /// counter-clockwise circular arc.
    line: bool,
}

/// Cry `crop_polyarc2d_with_line`, expressed with ordinary angles instead of
/// its comparison-only `quotientf` fake-angle representation.
fn crop_circle_boundary_with_line(
    boundary: &[CircleBoundaryPoint],
    radius: f32,
    line_point: Vec2,
    line_direction: Vec2,
) -> Vec<CircleBoundaryPoint> {
    if boundary.is_empty() || line_direction.length_squared() <= f32::EPSILON {
        return boundary.to_vec();
    }
    let mut output = Vec::with_capacity(boundary.len() + 2);
    let line_length_squared = line_direction.length_squared();
    let projection = line_point.dot(line_direction);
    let discriminant = line_length_squared.mul_add(
        -radius.mul_add(-radius, line_point.length_squared()),
        projection * projection,
    );
    let circle_intersections = if discriminant >= 0.0 {
        let root = discriminant.sqrt();
        [
            line_point + line_direction * ((-projection - root) / line_length_squared),
            line_point + line_direction * ((-projection + root) / line_length_squared),
        ]
    } else {
        [Vec2::NAN, Vec2::NAN]
    };

    let mut inside = cross2(line_direction, boundary[0].point - line_point) >= 0.0;
    for index in 0..boundary.len() {
        let current = boundary[index];
        let next = boundary[(index + 1) % boundary.len()];
        let mut intersections = Vec::<(f32, Vec2)>::with_capacity(2);
        if current.line {
            let edge = next.point - current.point;
            let denominator = cross2(edge, line_direction);
            if denominator.abs() > f32::EPSILON {
                let t = cross2(line_direction, current.point - line_point) / denominator;
                if (0.0..=1.0).contains(&t) {
                    intersections.push((t, current.point + edge * t));
                }
            }
        } else if discriminant >= 0.0 {
            let start = normalized_angle(current.point.y.atan2(current.point.x));
            let end = normalized_angle(next.point.y.atan2(next.point.x));
            let span = ccw_angle_distance(start, end);
            for point in circle_intersections {
                let angle = normalized_angle(point.y.atan2(point.x));
                let progress = ccw_angle_distance(start, angle);
                if progress <= span + 1.0e-5 {
                    intersections.push((progress, point));
                }
            }
            intersections.sort_by(|left, right| left.0.total_cmp(&right.0));
            intersections.dedup_by(|left, right| left.1.distance_squared(right.1) <= 1.0e-10);
        }

        if inside {
            output.push(current);
        }
        for (_, point) in intersections {
            output.push(CircleBoundaryPoint {
                point,
                line: current.line || inside,
            });
            inside = !inside;
        }
    }
    output
}

/// Cry's `CalcMediumResistance` moments over one boundary segment of a disc:
/// a straight chord when the segment is a line, otherwise a counter-clockwise
/// arc of the disc's rim. The first element is the segment's signed area
/// contribution.
fn disk_segment_moments(
    current: CircleBoundaryPoint,
    next: CircleBoundaryPoint,
    r2: f32,
    inverse_radius: f32,
) -> (f32, SegmentMoments) {
    let x0 = current.point.x;
    let y0 = current.point.y;
    let x1 = next.point.x;
    let y1 = next.point.y;
    if current.line {
        let dx = x1 - x0;
        let dy = y1 - y0;
        (
            x1.mul_add(-y0, x0 * y1) * 0.5,
            SegmentMoments {
                xy: dy * (dy.mul_add(x0, dx * y0).mul_add(0.5, x0 * y0) + dx * dy / 3.0),
                xx: dy * (dx.mul_add(x0, x0 * x0) + dx * dx / 3.0),
                xxy: dy
                    * (x0 * x0).mul_add(
                        y0,
                        (x0 * x0).mul_add(dy, x0 * y0 * dx * 2.0).mul_add(
                            0.5,
                            (dx * dx * dy)
                                .mul_add(0.25, (dx * dy * x0).mul_add(2.0, dx * dx * y0) / 3.0),
                        ),
                    ),
                xyy: dy
                    * (y0 * y0).mul_add(
                        x0,
                        (y0 * y0).mul_add(dx, y0 * x0 * dy * 2.0).mul_add(
                            0.5,
                            (dy * dy * dx)
                                .mul_add(0.25, (dy * dx * y0).mul_add(2.0, dy * dy * x0) / 3.0),
                        ),
                    ),
                xxx: dy
                    * ((dx * x0 * x0).mul_add(1.5, (dx * dx).mul_add(x0, dx.powi(3) * 0.25))
                        + x0.powi(3)),
            },
        )
    } else {
        let alpha = (y1 * inverse_radius).clamp(-0.9999, 0.9999).asin()
            - (y0 * inverse_radius).clamp(-0.9999, 0.9999).asin();
        let sign_x = if y1 - y0 < 0.0 { -1.0 } else { 1.0 };
        (
            r2 * alpha * 0.5 * sign_x,
            SegmentMoments {
                xy: -(x1.powi(3) - x0.powi(3)) / 3.0,
                xx: r2.mul_add(y1 - y0, -((y1.powi(3) - y0.powi(3)) / 3.0)),
                xxy: (r2 * y0.mul_add(-y0, y1 * y1))
                    .mul_add(0.5, -((y1.powi(3) - y0.powi(3)) / 3.0)),
                xyy: (r2 * 0.125).mul_add(
                    (r2 * alpha).mul_add(sign_x, x0.mul_add(-y0, x1 * y1)),
                    -y0.mul_add(-x0.powi(3), y1 * x1.powi(3)) * 0.25,
                ),
                xxx: 0.25
                    * (1.5 * r2).mul_add(
                        (r2 * alpha).mul_add(sign_x, y0.mul_add(-x0, y1 * x1)),
                        y0.mul_add(-x0.powi(3), y1 * x1.powi(3)),
                    ),
            },
        )
    }
}

#[allow(clippy::many_single_char_names)]
fn disk_medium_resistance(
    radius: f32,
    center: Vec3,
    normal: Vec3,
    plane: FluidPlane,
    motion: MediumMotion,
) -> MediumResistance {
    let basis_x = normal.any_orthonormal_vector();
    let basis_y = normal.cross(basis_x);
    let mut boundary = vec![
        CircleBoundaryPoint {
            point: Vec2::new(0.0, radius),
            line: false,
        },
        CircleBoundaryPoint {
            point: Vec2::new(0.0, -radius),
            line: false,
        },
    ];

    let water_normal = Vec2::new(plane.normal.dot(basis_x), plane.normal.dot(basis_y));
    let water_distance = (plane.origin - center).dot(plane.normal);
    if water_normal.length_squared() > 0.001 {
        let line_point = water_normal * (water_distance / water_normal.length_squared());
        let line_direction = Vec2::new(-water_normal.y, water_normal.x);
        boundary = crop_circle_boundary_with_line(&boundary, radius, line_point, line_direction);
    } else if plane.signed_distance(center) > 0.0 {
        boundary.clear();
    }
    if boundary.is_empty() {
        return MediumResistance::default();
    }

    let velocity_normal = motion.angular_velocity.cross(normal);
    let velocity_halfspace = Vec2::new(velocity_normal.dot(basis_x), velocity_normal.dot(basis_y));
    let velocity_distance =
        motion.linear_velocity.dot(normal) - velocity_normal.dot(center - motion.center_of_mass);
    if velocity_halfspace.length_squared() > motion.angular_velocity.length_squared() * 0.001 {
        let line_point =
            velocity_halfspace * (velocity_distance / velocity_halfspace.length_squared());
        let line_direction = Vec2::new(-velocity_halfspace.y, velocity_halfspace.x);
        boundary = crop_circle_boundary_with_line(&boundary, radius, line_point, line_direction);
    } else if velocity_halfspace.dot(Vec2::ZERO) > velocity_distance {
        boundary.clear();
    }
    if boundary.is_empty() {
        return MediumResistance::default();
    }

    let relative_center = center - motion.center_of_mass;
    let center_velocity = motion.linear_velocity + motion.angular_velocity.cross(relative_center);
    let v = Vec3::new(
        center_velocity.dot(basis_x),
        center_velocity.dot(basis_y),
        center_velocity.dot(normal),
    );
    let w = Vec3::new(
        motion.angular_velocity.dot(basis_x),
        motion.angular_velocity.dot(basis_y),
        motion.angular_velocity.dot(normal),
    );
    let r2 = radius * radius;
    let inverse_radius = radius.recip();
    let mut area = 0.0;
    let mut pressure = Vec3::ZERO;
    let mut torque = Vec3::ZERO;

    for index in 0..boundary.len() {
        let current = boundary[index];
        let next = boundary[(index + 1) % boundary.len()];
        let (segment_area, moments) = disk_segment_moments(current, next, r2, inverse_radius);
        area += segment_area;
        pressure.z += (w.y * 0.5).mul_add(-moments.xx, w.x * moments.xy);
        torque.x = v.z.mul_add(moments.xy, torque.x);
        torque.y = (v.z * 0.5).mul_add(-moments.xx, torque.y);
        torque.x += (w.y * 0.5).mul_add(-moments.xxy, w.x * moments.xyy);
        torque.y -= (w.y * (1.0f32 / 3.0)).mul_add(-moments.xxx, w.x * 0.5 * moments.xxy);
    }
    pressure.z = v.z.mul_add(area, pressure.z);
    MediumResistance {
        linear: -(basis_x * pressure.x + basis_y * pressure.y + normal * pressure.z),
        angular: -(basis_x * torque.x + basis_y * torque.y + normal * torque.z),
    }
}

#[inline]
fn cross2(left: Vec2, right: Vec2) -> f32 {
    left.y.mul_add(-right.x, left.x * right.y)
}

#[inline]
fn normalized_angle(angle: f32) -> f32 {
    angle.rem_euclid(core::f32::consts::TAU)
}

#[inline]
fn ccw_angle_distance(start: f32, end: f32) -> f32 {
    (end - start).rem_euclid(core::f32::consts::TAU)
}

fn mesh_revolved_submerged(
    radius: f32,
    half_height: f32,
    local_axis: Vec3,
    pose: PhysicsPose,
    plane: FluidPlane,
    segments: usize,
) -> SubmergedGeometry {
    let rotation = Quat::from_rotation_arc(Vec3::Z, local_axis);
    let mut vertices = Vec::with_capacity(segments * 2 + 2);
    for z in [-half_height, half_height] {
        for index in 0..segments {
            let angle = core::f32::consts::TAU * f32_from_usize(index) / f32_from_usize(segments);
            vertices.push(rotation * Vec3::new(radius * angle.cos(), radius * angle.sin(), z));
        }
    }
    vertices.push(rotation * Vec3::new(0.0, 0.0, -half_height));
    vertices.push(rotation * Vec3::new(0.0, 0.0, half_height));
    let bottom_center = u32_from_usize(segments * 2);
    let top_center = bottom_center + 1;
    let mut indices = Vec::with_capacity(segments * 4);
    for index in 0..segments {
        let next = (index + 1) % segments;
        let bottom = u32_from_usize(index);
        let bottom_next = u32_from_usize(next);
        let top = u32_from_usize(segments + index);
        let top_next = u32_from_usize(segments + next);
        indices.push([bottom, top_next, top]);
        indices.push([bottom, bottom_next, top_next]);
        indices.push([bottom_center, bottom_next, bottom]);
        indices.push([top_center, top, top_next]);
    }
    mesh_submerged(&vertices, &indices, pose, plane)
}

fn add_resistance(total: &mut MediumResistance, value: MediumResistance) {
    total.linear += value.linear;
    total.angular += value.angular;
}

const fn axis_vector(axis: Axis3) -> Vec3 {
    match axis {
        Axis3::X => Vec3::X,
        Axis3::Y => Vec3::Y,
        Axis3::Z => Vec3::Z,
    }
}

fn cuboid_mesh(half: Vec3) -> ([Vec3; 8], [[u32; 3]; 12]) {
    let vertices = [
        Vec3::new(-half.x, -half.y, -half.z),
        Vec3::new(half.x, -half.y, -half.z),
        Vec3::new(half.x, half.y, -half.z),
        Vec3::new(-half.x, half.y, -half.z),
        Vec3::new(-half.x, -half.y, half.z),
        Vec3::new(half.x, -half.y, half.z),
        Vec3::new(half.x, half.y, half.z),
        Vec3::new(-half.x, half.y, half.z),
    ];
    let indices = [
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_submerged_sphere_has_half_volume_and_native_centroid() {
        let result = submerged_geometry(
            &ColliderShape::Sphere { radius: 1.0 },
            PhysicsPose::IDENTITY,
            FluidPlane::default(),
        );
        assert!((result.volume - 2.0 * core::f32::consts::PI / 3.0).abs() < 1.0e-5);
        assert!((result.center.z + 0.375).abs() < 1.0e-5);
    }

    #[test]
    fn half_submerged_box_preserves_exact_volume_and_centroid() {
        let result = submerged_geometry(
            &ColliderShape::Cuboid {
                half_extents: Vec3::ONE,
            },
            PhysicsPose::IDENTITY,
            FluidPlane::default(),
        );
        assert!((result.full_volume - 8.0).abs() < 1.0e-5);
        assert!((result.volume - 4.0).abs() < 1.0e-4, "{result:?}");
        assert!((result.center.z + 0.5).abs() < 1.0e-4, "{result:?}");
    }

    #[test]
    fn fully_submerged_box_drag_opposes_velocity() {
        let resistance = medium_resistance(
            &ColliderShape::Cuboid {
                half_extents: Vec3::ONE,
            },
            PhysicsPose {
                translation: -Vec3::Z * 2.0,
                rotation: Quat::IDENTITY,
            },
            FluidPlane::default(),
            MediumMotion {
                linear_velocity: Vec3::X,
                angular_velocity: Vec3::ZERO,
                center_of_mass: -Vec3::Z * 2.0,
            },
        );
        assert!(resistance.linear.x < 0.0, "{resistance:?}");
    }

    #[test]
    fn submerged_sphere_resistance_uses_point_velocity_and_center_of_mass_torque() {
        let resistance = medium_resistance(
            &ColliderShape::Sphere { radius: 1.0 },
            PhysicsPose {
                translation: Vec3::new(1.0, 0.0, -2.0),
                rotation: Quat::IDENTITY,
            },
            FluidPlane::default(),
            MediumMotion {
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::Z,
                center_of_mass: Vec3::new(0.0, 0.0, -2.0),
            },
        );
        let area = core::f32::consts::PI;
        assert!((resistance.linear - Vec3::new(0.0, -area, 0.0)).length() < 1.0e-5);
        assert!((resistance.angular - Vec3::new(0.0, 0.0, -area)).length() < 1.0e-5);
    }

    #[test]
    fn half_submerged_axial_cylinder_is_exact() {
        let result = submerged_geometry(
            &ColliderShape::Cylinder {
                axis: Axis3::Z,
                half_height: 1.0,
                radius: 1.0,
            },
            PhysicsPose::IDENTITY,
            FluidPlane::default(),
        );
        assert!(
            2.0f32
                .mul_add(-core::f32::consts::PI, result.full_volume)
                .abs()
                < 1.0e-5
        );
        assert!((result.volume - core::f32::consts::PI).abs() < 1.0e-5);
        assert!((result.center.z + 0.5).abs() < 1.0e-5);
    }

    #[test]
    fn analytic_cylinder_cap_drag_preserves_native_arc_endpoint_clamp() {
        let resistance = medium_resistance(
            &ColliderShape::Cylinder {
                axis: Axis3::Z,
                half_height: 1.0,
                radius: 1.0,
            },
            PhysicsPose {
                translation: -Vec3::Z * 3.0,
                rotation: Quat::IDENTITY,
            },
            FluidPlane::default(),
            MediumMotion {
                linear_velocity: Vec3::Z,
                angular_velocity: Vec3::ZERO,
                center_of_mass: -Vec3::Z * 3.0,
            },
        );
        let native_clamped_disk_area = 2.0 * 0.9999f32.asin();
        assert!(
            (resistance.linear.z + native_clamped_disk_area).abs() < 1.0e-4,
            "{resistance:?}"
        );
        assert!(resistance.angular.length() < 1.0e-5, "{resistance:?}");
    }
}
