use bevy::asset::RenderAssetUsages;
use bevy::math::primitives::{Capsule3d, Cuboid, Cylinder, Sphere, Triangle3d};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;

use crate::{
    HeightFieldShape, MeshShape, PhysicalShape, PlaneShape, ScaledShape, ShapeAsset,
    ShapeAssetBinding, ShapeData, ShapeTransform,
};

const DEFAULT_SHAPE_COLOR: Color = Color::srgba(0.1, 0.58, 0.95, 0.26);
const DEFAULT_SHAPE_ROUGHNESS: f32 = 0.84;
const MIN_PREVIEW_EXTENT: f32 = 0.02;

/// Rendering settings for `RockNRoll` shape assets.
#[derive(Debug, Clone, PartialEq, Resource, Reflect)]
pub struct ShapeRenderConfig {
    pub base_color: Color,
    pub roughness: f32,
}

impl Default for ShapeRenderConfig {
    fn default() -> Self {
        Self {
            base_color: DEFAULT_SHAPE_COLOR,
            roughness: DEFAULT_SHAPE_ROUGHNESS,
        }
    }
}

impl ShapeRenderConfig {
    #[must_use]
    pub fn material(&self) -> StandardMaterial {
        StandardMaterial {
            base_color: self.base_color,
            perceptual_roughness: self.roughness,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            cull_mode: None,
            ..Default::default()
        }
    }
}

/// Child entities spawned for one `RockNRoll` shape asset binding.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct ShapeAssetChildren(pub Vec<Entity>);

/// Marker for mesh entities spawned from a `RockNRoll` shape asset.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct ShapeAssetVisual;

#[derive(Debug, Clone)]
pub struct ShapeVisualPrimitive {
    mesh: Mesh,
    transform: Transform,
}

#[derive(Resource, Debug, Clone, Default)]
pub struct ShapeRenderAssets {
    material: Option<Handle<StandardMaterial>>,
}

impl ShapeRenderAssets {
    fn material(
        &mut self,
        materials: &mut Assets<StandardMaterial>,
        config: &ShapeRenderConfig,
    ) -> Handle<StandardMaterial> {
        if let Some(handle) = &self.material {
            return handle.clone();
        }

        let handle = materials.add(config.material());
        self.material = Some(handle.clone());
        handle
    }
}

// Bevy system: `Res` is an owned parameter wrapper, so borrowing these would
// stop this function satisfying `IntoSystem` and it could not be registered.
#[allow(clippy::type_complexity, clippy::needless_pass_by_value)]
pub fn sync_shape_asset_visuals(
    mut commands: Commands,
    config: Res<ShapeRenderConfig>,
    mut render_assets: ResMut<ShapeRenderAssets>,
    shape_assets: Option<Res<Assets<ShapeAsset>>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    query: Query<(Entity, Ref<ShapeAssetBinding>, Option<&ShapeAssetChildren>)>,
) {
    let (Some(shape_assets), Some(meshes), Some(materials)) = (
        shape_assets.as_deref(),
        meshes.as_deref_mut(),
        materials.as_deref_mut(),
    ) else {
        return;
    };

    for (entity, binding, children) in &query {
        if !binding.is_changed() && children.is_some() {
            continue;
        }

        let Some(asset) = shape_assets.get(binding.handle()) else {
            if binding.is_changed() {
                despawn_shape_children(&mut commands, children);
                commands.entity(entity).remove::<ShapeAssetChildren>();
            }
            continue;
        };

        despawn_shape_children(&mut commands, children);
        spawn_shape_children(
            &mut commands,
            entity,
            asset,
            meshes,
            &render_assets.material(materials, &config),
        );
    }
}

pub fn cleanup_removed_shape_asset_bindings(
    mut commands: Commands,
    mut removed: RemovedComponents<ShapeAssetBinding>,
    children: Query<&ShapeAssetChildren>,
) {
    for entity in removed.read() {
        if let Ok(children) = children.get(entity) {
            despawn_shape_children(&mut commands, Some(children));
            commands.entity(entity).remove::<ShapeAssetChildren>();
        }
    }
}

fn spawn_shape_children(
    commands: &mut Commands,
    entity: Entity,
    asset: &ShapeAsset,
    meshes: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
) {
    let mut primitives = Vec::with_capacity(asset.shapes.len());
    for shape in &asset.shapes {
        collect_shape_primitives(shape, Mat4::IDENTITY, &mut primitives);
    }

    if primitives.is_empty() {
        commands.entity(entity).remove::<ShapeAssetChildren>();
        return;
    }

    let mut child_entities = Vec::with_capacity(primitives.len());
    commands.entity(entity).with_children(|parent| {
        for (index, primitive) in primitives.into_iter().enumerate() {
            let child = parent
                .spawn((
                    Name::new(format!("RockNRoll Shape {index}")),
                    ShapeAssetVisual,
                    Mesh3d(meshes.add(primitive.mesh)),
                    MeshMaterial3d(material.clone()),
                    primitive.transform,
                ))
                .id();
            child_entities.push(child);
        }
    });
    commands
        .entity(entity)
        .insert(ShapeAssetChildren(child_entities));
}

fn despawn_shape_children(commands: &mut Commands, children: Option<&ShapeAssetChildren>) {
    let Some(children) = children else {
        return;
    };
    for child in &children.0 {
        commands.entity(*child).despawn();
    }
}

fn collect_shape_primitives(
    shape: &PhysicalShape,
    transform: Mat4,
    primitives: &mut Vec<ShapeVisualPrimitive>,
) {
    match &shape.data {
        ShapeData::Box(value) => {
            let size = clamp_extent(value.half_extents.abs() * 2.0);
            primitives.push(ShapeVisualPrimitive {
                mesh: Mesh::from(Cuboid::from_size(size)),
                transform: Transform::from_matrix(transform),
            });
        }
        ShapeData::Sphere(value) => {
            if finite_positive(value.radius) {
                primitives.push(ShapeVisualPrimitive {
                    mesh: Mesh::from(Sphere::new(value.radius)),
                    transform: Transform::from_matrix(transform),
                });
            }
        }
        ShapeData::ConvexHull(value) => {
            if let Some(primitive) = bounds_primitive(&value.vertices, transform) {
                primitives.push(primitive);
            }
        }
        ShapeData::Cylinder(value) => {
            if let Some(mesh) = cylinder_mesh(value.radius, value.half_height * 2.0) {
                primitives.push(ShapeVisualPrimitive {
                    mesh,
                    transform: Transform::from_matrix(transform),
                });
            }
        }
        ShapeData::CylinderUnaligned(value) => {
            if let Some(primitive) =
                cylinder_between(value.endpoint_a, value.endpoint_b, value.radius, transform)
            {
                primitives.push(primitive);
            }
        }
        ShapeData::Capsule(value) => {
            if finite_positive(value.radius) && value.half_height.is_finite() {
                primitives.push(ShapeVisualPrimitive {
                    mesh: Mesh::from(Capsule3d::new(value.radius, value.half_height * 2.0)),
                    transform: Transform::from_matrix(transform),
                });
            }
        }
        ShapeData::CapsuleUnaligned(value) => {
            if let Some(primitive) =
                capsule_between(value.endpoint_a, value.endpoint_b, value.radius, transform)
            {
                primitives.push(primitive);
            }
        }
        ShapeData::Triangle(value) => {
            primitives.push(ShapeVisualPrimitive {
                mesh: Mesh::from(Triangle3d::new(value.a, value.b, value.c)),
                transform: Transform::from_matrix(transform),
            });
        }
        ShapeData::Mesh(value) => {
            if let Some(mesh) = mesh_shape_mesh(value) {
                primitives.push(ShapeVisualPrimitive {
                    mesh,
                    transform: Transform::from_matrix(transform),
                });
            }
        }
        ShapeData::Compound(value) => {
            for child in &value.children {
                collect_shape_primitives(
                    &child.shape,
                    transform * shape_transform_matrix(&child.transform),
                    primitives,
                );
            }
        }
        ShapeData::Transform(value) => {
            collect_shape_primitives(
                &value.shape,
                transform * shape_transform_matrix(&value.transform),
                primitives,
            );
        }
        // Kind 13 is a marker only. Its deformable vertices/faces live on the
        // linked soft-body instance, so the static shape asset has no geometry
        // that this fallback visualizer can materialize.
        ShapeData::SoftBody(_) => {}
        ShapeData::Plane(value) => {
            if let Some(primitive) = plane_primitive(value, transform) {
                primitives.push(primitive);
            }
        }
        ShapeData::ScaleConvexPolytope(value) | ShapeData::ScaleMesh(value) => {
            collect_scaled_shape(value, transform, primitives);
        }
        ShapeData::HeightField(value) => {
            if let Some(primitive) = height_field_primitive(value, transform) {
                primitives.push(primitive);
            }
        }
    }
}

fn collect_scaled_shape(
    value: &ScaledShape,
    transform: Mat4,
    primitives: &mut Vec<ShapeVisualPrimitive>,
) {
    if value.scale.is_finite() {
        collect_shape_primitives(
            &value.shape,
            transform * Mat4::from_scale(value.scale),
            primitives,
        );
    }
}

fn mesh_shape_mesh(value: &MeshShape) -> Option<Mesh> {
    if value.vertices.is_empty() || value.indices.len() < 3 {
        return None;
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        value
            .vertices
            .iter()
            .map(Vec3::to_array)
            .collect::<Vec<_>>(),
    )
    .with_inserted_indices(Indices::U32(
        value
            .indices
            .iter()
            .map(|index| u32::from(*index))
            .collect(),
    ));
    mesh.compute_smooth_normals();
    Some(mesh)
}

fn bounds_primitive(vertices: &[Vec3], transform: Mat4) -> Option<ShapeVisualPrimitive> {
    let (center, size) = bounds_center_size(vertices)?;
    Some(ShapeVisualPrimitive {
        mesh: Mesh::from(Cuboid::from_size(clamp_extent(size))),
        transform: Transform::from_matrix(transform * Mat4::from_translation(center)),
    })
}

fn bounds_center_size(vertices: &[Vec3]) -> Option<(Vec3, Vec3)> {
    let (&first, rest) = vertices.split_first()?;
    if !first.is_finite() {
        return None;
    }

    let mut min = first;
    let mut max = first;
    for &vertex in rest {
        if !vertex.is_finite() {
            return None;
        }
        min = min.min(vertex);
        max = max.max(vertex);
    }

    Some(((min + max) * 0.5, max - min))
}

fn cylinder_mesh(radius: f32, height: f32) -> Option<Mesh> {
    (finite_positive(radius) && finite_positive(height))
        .then(|| Mesh::from(Cylinder::new(radius, height)))
}

fn cylinder_between(
    endpoint_a: Vec3,
    endpoint_b: Vec3,
    radius: f32,
    transform: Mat4,
) -> Option<ShapeVisualPrimitive> {
    let (midpoint, rotation, length) = segment_transform(endpoint_a, endpoint_b)?;
    Some(ShapeVisualPrimitive {
        mesh: cylinder_mesh(radius, length)?,
        transform: Transform::from_matrix(
            transform * Mat4::from_scale_rotation_translation(Vec3::ONE, rotation, midpoint),
        ),
    })
}

fn capsule_between(
    endpoint_a: Vec3,
    endpoint_b: Vec3,
    radius: f32,
    transform: Mat4,
) -> Option<ShapeVisualPrimitive> {
    if !finite_positive(radius) {
        return None;
    }
    let (midpoint, rotation, length) = segment_transform(endpoint_a, endpoint_b)?;
    Some(ShapeVisualPrimitive {
        mesh: Mesh::from(Capsule3d::new(radius, length)),
        transform: Transform::from_matrix(
            transform * Mat4::from_scale_rotation_translation(Vec3::ONE, rotation, midpoint),
        ),
    })
}

fn segment_transform(endpoint_a: Vec3, endpoint_b: Vec3) -> Option<(Vec3, Quat, f32)> {
    if !endpoint_a.is_finite() || !endpoint_b.is_finite() {
        return None;
    }

    let axis = endpoint_b - endpoint_a;
    let length = axis.length();
    if !finite_positive(length) {
        return None;
    }
    let rotation = Quat::from_rotation_arc(Vec3::Y, axis / length);
    Some(((endpoint_a + endpoint_b) * 0.5, rotation, length))
}

fn plane_primitive(value: &PlaneShape, transform: Mat4) -> Option<ShapeVisualPrimitive> {
    if !value.plane.is_finite() || !value.aabb_min.is_finite() || !value.aabb_max.is_finite() {
        return None;
    }

    let normal = value.plane.truncate().normalize_or_zero();
    if normal == Vec3::ZERO {
        return None;
    }

    let bounds_center = (value.aabb_min + value.aabb_max) * 0.5;
    let center = bounds_center - normal * (normal.dot(bounds_center) - value.plane.w);
    let a = normal.any_orthonormal_vector();
    let b = normal.cross(a);
    let half_extent = ((value.aabb_max - value.aabb_min).length() * 0.5).max(0.5);
    Some(ShapeVisualPrimitive {
        mesh: quad_mesh(
            center - a * half_extent - b * half_extent,
            center + a * half_extent - b * half_extent,
            center + a * half_extent + b * half_extent,
            center - a * half_extent + b * half_extent,
        ),
        transform: Transform::from_matrix(transform),
    })
}

fn height_field_primitive(
    value: &HeightFieldShape,
    transform: Mat4,
) -> Option<ShapeVisualPrimitive> {
    let data = value.data.as_ref()?;
    let vertices = [data.aabb_min, data.aabb_max];
    bounds_primitive(&vertices, transform)
}

fn quad_mesh(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![a.to_array(), b.to_array(), c.to_array(), d.to_array()],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    mesh.compute_smooth_normals();
    mesh
}

fn shape_transform_matrix(transform: &ShapeTransform) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(transform[0].x, transform[1].x, transform[2].x, 0.0),
        Vec4::new(transform[0].y, transform[1].y, transform[2].y, 0.0),
        Vec4::new(transform[0].z, transform[1].z, transform[2].z, 0.0),
        Vec4::new(transform[0].w, transform[1].w, transform[2].w, 1.0),
    )
}

fn clamp_extent(size: Vec3) -> Vec3 {
    size.max(Vec3::splat(MIN_PREVIEW_EXTENT))
}

const fn finite_positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoxShape, CompoundChild, CompoundShape};

    #[test]
    fn shape_transform_uses_matrix34_translation() {
        let matrix = shape_transform_matrix(&[
            Vec4::new(1.0, 0.0, 0.0, 10.0),
            Vec4::new(0.0, 1.0, 0.0, 20.0),
            Vec4::new(0.0, 0.0, 1.0, 30.0),
        ]);

        assert_eq!(
            matrix.transform_point3(Vec3::ZERO),
            Vec3::new(10.0, 20.0, 30.0)
        );
        assert_eq!(
            matrix.transform_point3(Vec3::X),
            Vec3::new(11.0, 20.0, 30.0)
        );
    }

    #[test]
    fn collect_shape_primitives_flattens_compounds() {
        let shape = PhysicalShape::new(
            ShapeData::Compound(CompoundShape {
                children: vec![CompoundChild {
                    transform: [
                        Vec4::new(1.0, 0.0, 0.0, 2.0),
                        Vec4::new(0.0, 1.0, 0.0, 0.0),
                        Vec4::new(0.0, 0.0, 1.0, 0.0),
                    ],
                    shape: Box::new(PhysicalShape::new(
                        ShapeData::Box(BoxShape {
                            half_extents: Vec3::splat(0.5),
                            convex_radius: 0.0,
                        }),
                        Box::new([]),
                    )),
                }]
                .into_boxed_slice(),
            }),
            Box::new([]),
        );
        let mut primitives = Vec::new();

        collect_shape_primitives(&shape, Mat4::IDENTITY, &mut primitives);

        assert_eq!(primitives.len(), 1);
        assert_eq!(
            primitives[0].transform.translation,
            Vec3::new(2.0, 0.0, 0.0)
        );
    }
}
