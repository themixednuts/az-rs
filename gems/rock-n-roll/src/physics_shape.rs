//! Conversion from native `RockNRoll` shape products into solver-neutral
//! Azoth collider descriptors.

use az_physics::{
    Axis3, ColliderConfiguration, ColliderShape, PhysicsColliderSet, PhysicsError, PhysicsPose,
};
use bevy::prelude::{Mat3, Quat, Vec3};
use thiserror::Error;

use crate::{
    CompoundChild, HeightFieldData, MeshShape, PhysicalShape, PlaneShape, ScaledShape, ShapeAsset,
    ShapeData, ShapeTransform,
};

const TRANSFORM_EPSILON: f32 = 1.0e-4;

/// Converts a loaded `RockNRoll` shape product into runtime collider geometry.
pub trait ToPhysicsColliders {
    /// Converts every top-level shape while preserving compound child poses.
    ///
    /// # Errors
    ///
    /// Returns [`ShapeConversionError::InvalidTransform`] for a non-finite,
    /// singular or sheared shape transform,
    /// [`ShapeConversionError::InvalidMeshIndexCount`] or
    /// [`ShapeConversionError::MeshIndexOutOfRange`] for a malformed mesh,
    /// [`ShapeConversionError::MissingHeightFieldData`],
    /// [`ShapeConversionError::UnsupportedHeightSampleKind`] or
    /// [`ShapeConversionError::InvalidHeightFieldBytes`] for a malformed height
    /// field, [`ShapeConversionError::SoftBodyRequiresDeformable`] for a
    /// soft-body marker, and [`ShapeConversionError::Physics`] if a converted
    /// primitive is rejected by the physics layer.
    fn to_physics_colliders(&self) -> Result<PhysicsColliderSet, ShapeConversionError>;
}

impl ToPhysicsColliders for ShapeAsset {
    fn to_physics_colliders(&self) -> Result<PhysicsColliderSet, ShapeConversionError> {
        self.shapes
            .iter()
            .try_fold(PhysicsColliderSet::default(), |mut colliders, shape| {
                append_shape(shape, &mut colliders.0)?;
                Ok(colliders)
            })
    }
}

impl TryFrom<&ShapeAsset> for PhysicsColliderSet {
    type Error = ShapeConversionError;

    fn try_from(value: &ShapeAsset) -> Result<Self, Self::Error> {
        value.to_physics_colliders()
    }
}

/// Applies an entity-level scale to already converted collider geometry.
/// Unsupported non-uniform primitive scaling is rejected explicitly.
///
/// # Errors
///
/// Returns [`ShapeConversionError::InvalidTransform`] if `scale` is non-finite
/// or zero on any axis, [`ShapeConversionError::NonRepresentableScale`] if a
/// primitive (sphere, capsule, plane) cannot absorb a non-uniform scale without
/// changing shape class, and [`ShapeConversionError::Physics`] if the scaled
/// primitive is rejected by the physics layer.
pub fn scale_physics_colliders(
    colliders: &mut PhysicsColliderSet,
    scale: Vec3,
) -> Result<(), ShapeConversionError> {
    validate_scale(scale)?;
    for collider in &mut colliders.0 {
        scale_collider(collider, scale)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ShapeConversionError {
    #[error(transparent)]
    Physics(#[from] PhysicsError),
    #[error("RockNRoll mesh index count {0} is not divisible by three")]
    InvalidMeshIndexCount(usize),
    #[error("RockNRoll mesh contains an out-of-range vertex index")]
    MeshIndexOutOfRange,
    #[error("RockNRoll height field layout {0} has no sample payload")]
    MissingHeightFieldData(u32),
    #[error("RockNRoll height field sample kind {0} is unsupported")]
    UnsupportedHeightSampleKind(u32),
    #[error("RockNRoll height field byte length is inconsistent with its dimensions")]
    InvalidHeightFieldBytes,
    #[error("RockNRoll shape transform is non-finite, singular, or contains shear")]
    InvalidTransform,
    #[error("RockNRoll scale {scale:?} cannot represent {shape} without changing its shape class")]
    NonRepresentableScale { shape: &'static str, scale: Vec3 },
    #[error("RockNRoll soft-body marker must be materialized as a linked soft body")]
    SoftBodyRequiresDeformable,
}

fn append_shape(
    shape: &PhysicalShape,
    output: &mut Vec<ColliderConfiguration>,
) -> Result<(), ShapeConversionError> {
    match &shape.data {
        ShapeData::Box(shape) => output.push(collider(ColliderShape::RoundedCuboid {
            half_extents: shape.half_extents,
            border_radius: shape.convex_radius,
        })?),
        ShapeData::Sphere(shape) => output.push(collider(ColliderShape::Sphere {
            radius: shape.radius,
        })?),
        ShapeData::ConvexHull(shape) => output.push(collider(ColliderShape::ConvexHull {
            points: shape.vertices.to_vec(),
            border_radius: shape.convex_radius,
        })?),
        ShapeData::Cylinder(shape) => output.push(collider(ColliderShape::RoundedCylinder {
            axis: Axis3::Y,
            half_height: shape.half_height,
            radius: shape.radius,
            border_radius: shape.convex_radius,
        })?),
        ShapeData::CylinderUnaligned(shape) => {
            output.push(collider(ColliderShape::RoundedCylinderSegment {
                endpoint_a: shape.endpoint_a,
                endpoint_b: shape.endpoint_b,
                radius: shape.radius,
                border_radius: shape.convex_radius,
            })?);
        }
        ShapeData::Capsule(shape) => output.push(collider(ColliderShape::Capsule {
            axis: Axis3::Y,
            half_height: shape.half_height,
            radius: shape.radius,
        })?),
        ShapeData::CapsuleUnaligned(shape) => {
            output.push(collider(ColliderShape::CapsuleSegment {
                endpoint_a: shape.endpoint_a,
                endpoint_b: shape.endpoint_b,
                radius: shape.radius,
            })?);
        }
        ShapeData::Triangle(shape) => output.push(collider(ColliderShape::Triangle {
            vertices: [shape.a, shape.b, shape.c],
            border_radius: shape.convex_radius,
        })?),
        ShapeData::Mesh(shape) => output.push(collider(mesh(shape)?)?),
        ShapeData::Compound(shape) => {
            for child in &shape.children {
                append_transformed_child(child, output)?;
            }
        }
        ShapeData::Transform(shape) => {
            let start = output.len();
            append_shape(&shape.shape, output)?;
            apply_transform(&mut output[start..], &shape.transform)?;
        }
        ShapeData::SoftBody(_) => return Err(ShapeConversionError::SoftBodyRequiresDeformable),
        ShapeData::Plane(shape) => output.push(collider(plane(*shape))?),
        ShapeData::ScaleConvexPolytope(shape) | ShapeData::ScaleMesh(shape) => {
            append_scaled(shape, output)?;
        }
        ShapeData::HeightField(shape) => {
            let data = shape
                .data
                .as_ref()
                .ok_or(ShapeConversionError::MissingHeightFieldData(shape.layout))?;
            output.push(collider(height_field(data)?)?);
        }
    }
    Ok(())
}

fn collider(shape: ColliderShape) -> Result<ColliderConfiguration, ShapeConversionError> {
    shape.validate()?;
    Ok(ColliderConfiguration {
        shape,
        ..ColliderConfiguration::default()
    })
}

fn mesh(shape: &MeshShape) -> Result<ColliderShape, ShapeConversionError> {
    if !shape.indices.len().is_multiple_of(3) {
        return Err(ShapeConversionError::InvalidMeshIndexCount(
            shape.indices.len(),
        ));
    }
    let indices: Vec<_> = shape
        .indices
        .chunks_exact(3)
        .map(|triangle| {
            [
                u32::from(triangle[0]),
                u32::from(triangle[1]),
                u32::from(triangle[2]),
            ]
        })
        .collect();
    if indices
        .iter()
        .flatten()
        .any(|&index| index as usize >= shape.vertices.len())
    {
        return Err(ShapeConversionError::MeshIndexOutOfRange);
    }
    Ok(ColliderShape::TriangleMesh {
        vertices: shape.vertices.to_vec(),
        indices,
    })
}

fn plane(shape: PlaneShape) -> ColliderShape {
    ColliderShape::Plane {
        normal: shape.plane.truncate(),
        offset: shape.plane.w,
        aabb_min: shape.aabb_min,
        aabb_max: shape.aabb_max,
    }
}

fn height_field(data: &HeightFieldData) -> Result<ColliderShape, ShapeConversionError> {
    let count = (data.width as usize)
        .checked_mul(data.length as usize)
        .ok_or(ShapeConversionError::InvalidHeightFieldBytes)?;
    let base = data.aabb_min.y;
    let heights = match data.version {
        0 if data.samples.len() == count => data
            .samples
            .iter()
            .map(|&sample| f32::from(sample).mul_add(data.height_scale, base))
            .collect(),
        1 if data.samples.len() == count.saturating_mul(2) => data
            .samples
            .chunks_exact(2)
            .map(|sample| {
                f32::from(u16::from_le_bytes([sample[0], sample[1]]))
                    .mul_add(data.height_scale, base)
            })
            .collect(),
        2 if data.samples.len() == count.saturating_mul(4) => data
            .samples
            .chunks_exact(4)
            .map(|sample| {
                f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]])
                    .mul_add(data.height_scale, base)
            })
            .collect(),
        0..=2 => return Err(ShapeConversionError::InvalidHeightFieldBytes),
        sample_kind => {
            return Err(ShapeConversionError::UnsupportedHeightSampleKind(
                sample_kind,
            ));
        }
    };
    Ok(ColliderShape::HeightField {
        width: data.width,
        length: data.length,
        heights,
        aabb_min: data.aabb_min,
        aabb_max: data.aabb_max,
        // The native unreflected up-axis flag defaults to zero: X/Z are the
        // grid axes and Y contains height.
        up_axis: Axis3::Y,
    })
}

fn append_transformed_child(
    child: &CompoundChild,
    output: &mut Vec<ColliderConfiguration>,
) -> Result<(), ShapeConversionError> {
    let start = output.len();
    append_shape(&child.shape, output)?;
    apply_transform(&mut output[start..], &child.transform)
}

fn append_scaled(
    scaled: &ScaledShape,
    output: &mut Vec<ColliderConfiguration>,
) -> Result<(), ShapeConversionError> {
    validate_scale(scaled.scale)?;
    let start = output.len();
    append_shape(&scaled.shape, output)?;
    for collider in &mut output[start..] {
        scale_collider(collider, scaled.scale)?;
    }
    Ok(())
}

fn apply_transform(
    colliders: &mut [ColliderConfiguration],
    transform: &ShapeTransform,
) -> Result<(), ShapeConversionError> {
    let (pose, scale) = decompose_transform(transform)?;
    for collider in colliders {
        if scale != Vec3::ONE {
            scale_collider(collider, scale)?;
        }
        collider.local_pose = pose * collider.local_pose;
    }
    Ok(())
}

fn decompose_transform(rows: &ShapeTransform) -> Result<(PhysicsPose, Vec3), ShapeConversionError> {
    if rows.iter().any(|row| !row.is_finite()) {
        return Err(ShapeConversionError::InvalidTransform);
    }
    let mut columns = [
        Vec3::new(rows[0].x, rows[1].x, rows[2].x),
        Vec3::new(rows[0].y, rows[1].y, rows[2].y),
        Vec3::new(rows[0].z, rows[1].z, rows[2].z),
    ];
    let mut scale = Vec3::new(
        columns[0].length(),
        columns[1].length(),
        columns[2].length(),
    );
    validate_scale(scale)?;
    if Mat3::from_cols(columns[0], columns[1], columns[2]).determinant() < 0.0 {
        scale.x = -scale.x;
    }
    columns[0] /= scale.x;
    columns[1] /= scale.y;
    columns[2] /= scale.z;
    if columns[0].dot(columns[1]).abs() > TRANSFORM_EPSILON
        || columns[0].dot(columns[2]).abs() > TRANSFORM_EPSILON
        || columns[1].dot(columns[2]).abs() > TRANSFORM_EPSILON
    {
        return Err(ShapeConversionError::InvalidTransform);
    }
    let rotation = Mat3::from_cols(columns[0], columns[1], columns[2]);
    if (rotation.determinant() - 1.0).abs() > TRANSFORM_EPSILON {
        return Err(ShapeConversionError::InvalidTransform);
    }
    Ok((
        PhysicsPose {
            translation: Vec3::new(rows[0].w, rows[1].w, rows[2].w),
            rotation: Quat::from_mat3(&rotation).normalize(),
        },
        scale,
    ))
}

fn validate_scale(scale: Vec3) -> Result<(), ShapeConversionError> {
    if scale.is_finite() && scale.abs().cmpgt(Vec3::ZERO).all() {
        Ok(())
    } else {
        Err(ShapeConversionError::InvalidTransform)
    }
}

fn scale_collider(
    collider: &mut ColliderConfiguration,
    scale: Vec3,
) -> Result<(), ShapeConversionError> {
    collider.local_pose.translation *= scale;
    collider.shape = scale_shape(collider.shape.clone(), scale)?;
    collider.shape.validate()?;
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "each arm preserves one native RockNRoll shape class"
)]
fn scale_shape(shape: ColliderShape, scale: Vec3) -> Result<ColliderShape, ShapeConversionError> {
    let absolute = scale.abs();
    let uniform = (absolute.max_element() - absolute.min_element()).abs() <= TRANSFORM_EPSILON;
    let scale_radius = absolute.min_element();
    let scaled = match shape {
        ColliderShape::Sphere { radius } if uniform => ColliderShape::Sphere {
            radius: radius * scale_radius,
        },
        ColliderShape::Sphere { .. } => {
            return Err(ShapeConversionError::NonRepresentableScale {
                shape: "sphere",
                scale,
            });
        }
        ColliderShape::Cuboid { half_extents } => ColliderShape::Cuboid {
            half_extents: half_extents * absolute,
        },
        ColliderShape::RoundedCuboid {
            half_extents,
            border_radius,
        } => ColliderShape::RoundedCuboid {
            half_extents: half_extents * absolute,
            border_radius: border_radius * scale_radius,
        },
        ColliderShape::Capsule {
            axis,
            half_height,
            radius,
        } if uniform => ColliderShape::Capsule {
            axis,
            half_height: half_height * scale_radius,
            radius: radius * scale_radius,
        },
        ColliderShape::Cylinder {
            axis,
            half_height,
            radius,
        } => scale_cylinder(axis, half_height, radius, 0.0, scale, false)?,
        ColliderShape::RoundedCylinder {
            axis,
            half_height,
            radius,
            border_radius,
        } => scale_cylinder(axis, half_height, radius, border_radius, scale, true)?,
        ColliderShape::Capsule { .. } => {
            return Err(ShapeConversionError::NonRepresentableScale {
                shape: "capsule",
                scale,
            });
        }
        ColliderShape::CapsuleSegment {
            endpoint_a,
            endpoint_b,
            radius,
        } if uniform => ColliderShape::CapsuleSegment {
            endpoint_a: endpoint_a * scale,
            endpoint_b: endpoint_b * scale,
            radius: radius * scale_radius,
        },
        ColliderShape::CapsuleSegment { .. } => {
            return Err(ShapeConversionError::NonRepresentableScale {
                shape: "unaligned capsule",
                scale,
            });
        }
        ColliderShape::RoundedCylinderSegment {
            endpoint_a,
            endpoint_b,
            radius,
            border_radius,
        } if uniform => ColliderShape::RoundedCylinderSegment {
            endpoint_a: endpoint_a * scale,
            endpoint_b: endpoint_b * scale,
            radius: radius * scale_radius,
            border_radius: border_radius * scale_radius,
        },
        ColliderShape::RoundedCylinderSegment { .. } => {
            return Err(ShapeConversionError::NonRepresentableScale {
                shape: "unaligned cylinder",
                scale,
            });
        }
        ColliderShape::ConvexHull {
            points,
            border_radius,
        } => ColliderShape::ConvexHull {
            points: points.into_iter().map(|point| point * scale).collect(),
            border_radius: border_radius * scale_radius,
        },
        ColliderShape::Triangle {
            vertices,
            border_radius,
        } => ColliderShape::Triangle {
            vertices: vertices.map(|vertex| vertex * scale),
            border_radius: border_radius * scale_radius,
        },
        ColliderShape::TriangleMesh { vertices, indices } => ColliderShape::TriangleMesh {
            vertices: vertices.into_iter().map(|vertex| vertex * scale).collect(),
            indices,
        },
        ColliderShape::Plane {
            normal,
            offset,
            aabb_min,
            aabb_max,
        } => {
            let inverse_scale = scale.recip();
            let raw_normal = normal * inverse_scale;
            let length = raw_normal.length();
            let first = aabb_min * scale;
            let second = aabb_max * scale;
            ColliderShape::Plane {
                normal: raw_normal / length,
                offset: offset / length,
                aabb_min: first.min(second),
                aabb_max: first.max(second),
            }
        }
        ColliderShape::HeightField {
            width,
            length,
            heights,
            aabb_min,
            aabb_max,
            up_axis,
        } => {
            let up_scale = match up_axis {
                Axis3::X => scale.x,
                Axis3::Y => scale.y,
                Axis3::Z => scale.z,
            };
            let first = aabb_min * scale;
            let second = aabb_max * scale;
            ColliderShape::HeightField {
                width,
                length,
                heights: heights
                    .into_iter()
                    .map(|height| height * up_scale)
                    .collect(),
                aabb_min: first.min(second),
                aabb_max: first.max(second),
                up_axis,
            }
        }
    };
    Ok(scaled)
}

fn scale_cylinder(
    axis: Axis3,
    half_height: f32,
    radius: f32,
    border_radius: f32,
    scale: Vec3,
    rounded: bool,
) -> Result<ColliderShape, ShapeConversionError> {
    let absolute = scale.abs();
    let (axial, radial_a, radial_b) = match axis {
        Axis3::X => (absolute.x, absolute.y, absolute.z),
        Axis3::Y => (absolute.y, absolute.x, absolute.z),
        Axis3::Z => (absolute.z, absolute.x, absolute.y),
    };
    if (radial_a - radial_b).abs() > TRANSFORM_EPSILON {
        return Err(ShapeConversionError::NonRepresentableScale {
            shape: "cylinder",
            scale,
        });
    }
    if rounded {
        Ok(ColliderShape::RoundedCylinder {
            axis,
            half_height: half_height * axial,
            radius: radius * radial_a,
            border_radius: border_radius * absolute.min_element(),
        })
    } else {
        Ok(ColliderShape::Cylinder {
            axis,
            half_height: half_height * axial,
            radius: radius * radial_a,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BoxShape, CompoundShape, HeightFieldShape, MaterialFilter, MeshShape, ShapeAsset,
        SphereShape,
    };
    use bevy::prelude::Vec4;

    fn physical(data: ShapeData) -> PhysicalShape {
        PhysicalShape::new(data, Box::new([]))
    }

    #[test]
    fn converts_primitives_and_compound_pose() {
        let asset = ShapeAsset::new(
            Box::new([]),
            MaterialFilter::default(),
            vec![physical(ShapeData::Compound(CompoundShape {
                children: vec![CompoundChild {
                    transform: [
                        Vec4::new(1.0, 0.0, 0.0, 2.0),
                        Vec4::new(0.0, 1.0, 0.0, 3.0),
                        Vec4::new(0.0, 0.0, 1.0, 4.0),
                    ],
                    shape: Box::new(physical(ShapeData::Sphere(SphereShape { radius: 0.5 }))),
                }]
                .into_boxed_slice(),
            }))]
            .into_boxed_slice(),
        );
        let colliders = asset.to_physics_colliders().unwrap();
        assert_eq!(colliders.0.len(), 1);
        assert_eq!(
            colliders.0[0].local_pose.translation,
            Vec3::new(2.0, 3.0, 4.0)
        );
        assert!(matches!(
            colliders.0[0].shape,
            ColliderShape::Sphere { radius: 0.5 }
        ));
    }

    #[test]
    fn converts_mesh_indices_without_widening_the_source_storage() {
        let shape = MeshShape {
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y].into_boxed_slice(),
            indices: vec![0, 1, 2].into_boxed_slice(),
            ..Default::default()
        };
        assert!(matches!(
            mesh(&shape).unwrap(),
            ColliderShape::TriangleMesh { indices, .. } if indices == [[0, 1, 2]]
        ));
    }

    #[test]
    fn decodes_all_native_height_sample_kinds() {
        for (kind, samples, expected) in [
            (0, vec![2u8, 3, 4, 5], 5.0),
            (1, vec![2u8, 0, 3, 0, 4, 0, 5, 0], 5.0),
            (
                2,
                [2.0f32, 3.0, 4.0, 5.0]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect(),
                5.0,
            ),
        ] {
            let shape = height_field(&HeightFieldData {
                version: kind,
                width: 2,
                length: 2,
                height_scale: 2.0,
                aabb_min: Vec3::new(0.0, 1.0, 0.0),
                aabb_max: Vec3::new(1.0, 20.0, 1.0),
                samples: samples.into_boxed_slice(),
            })
            .unwrap();
            assert!(matches!(
                shape,
                ColliderShape::HeightField { ref heights, up_axis: Axis3::Y, .. }
                    if (heights[0] - expected).abs() <= 1.0e-6
            ));
        }
    }

    #[test]
    fn rejects_missing_heightfield_payload_instead_of_falling_back() {
        let asset = ShapeAsset::new(
            Box::new([]),
            MaterialFilter::default(),
            vec![physical(ShapeData::HeightField(HeightFieldShape {
                layout: 0,
                data: None,
            }))]
            .into_boxed_slice(),
        );
        assert_eq!(
            asset.to_physics_colliders(),
            Err(ShapeConversionError::MissingHeightFieldData(0))
        );
    }

    #[test]
    fn rounded_box_keeps_native_convex_radius() {
        let shape = physical(ShapeData::Box(BoxShape {
            half_extents: Vec3::splat(1.0),
            convex_radius: 0.1,
        }));
        let mut output = Vec::new();
        append_shape(&shape, &mut output).unwrap();
        assert!(matches!(
            output[0].shape,
            ColliderShape::RoundedCuboid {
                border_radius: 0.1,
                ..
            }
        ));
    }
}
