//! Static model engine asset loading and rendering.

use std::collections::HashSet;

use bevy::asset::{AssetEvent, AssetId, LoadState};
use bevy::mesh::Mesh;
use bevy::prelude::*;
use thiserror::Error;

use az_asset::normalize_source_path;
use az_core::AssetType;
use az_framework::asset::{AssetCatalog, AssetRefLoadError};
use az_gem_animation::StaticCharacterAttachment;
use az_mesh_builder::{MESH_ASSET_TYPE, MESH_PRODUCT_EXTENSION, MeshAsset, MeshPrimitive};
use az_physics::{PhysicsError, PhysicsMeshGeometry};

use crate::{MATERIAL_ASSET_TYPE_ID, MaterialAsset, MeshColliderComponent};

/// Resolve a static-model source-path reference to the on-disk product path.
///
/// Legacy component references and glTF authoring paths resolve to the one
/// native `.azmesh` product emitted by `az-mesh-builder`.
#[must_use]
pub fn static_model_engine_asset_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized
        .rsplit_once('.')
        .filter(|(_, extension)| matches!(*extension, "cgf" | "cga" | "gltf" | "glb" | "azmesh"))
        .map_or(normalized.as_str(), |(stem, _)| stem);
    format!("{stem}.{MESH_PRODUCT_EXTENSION}")
}

#[must_use]
pub fn is_static_model_engine_asset_path(path: &str) -> bool {
    path.ends_with(".azmesh")
}

#[must_use]
pub fn is_static_model_source_asset_path(path: &str) -> bool {
    [".cgf", ".cga", ".gltf", ".glb", ".azmesh"]
        .iter()
        .any(|extension| path.ends_with(extension))
}

/// Failure while deriving the product-side triangle geometry consumed by the
/// `LmbrCentral` mesh collider.
#[derive(Component, Debug, Clone, PartialEq, Error)]
pub enum StaticModelPhysicsError {
    #[error("static model contains no triangle geometry")]
    NoTriangleGeometry,
    #[error("static model primitive {primitive} has {count} indices, not a triangle list")]
    InvalidIndexCount { primitive: usize, count: usize },
    #[error(
        "static model primitive {primitive} index {index} is outside its {vertex_count} vertices"
    )]
    IndexOutOfBounds {
        primitive: usize,
        index: u32,
        vertex_count: usize,
    },
    #[error("static model vertex count exceeds the u32 triangle-index range")]
    TooManyVertices,
    #[error(transparent)]
    InvalidGeometry(#[from] PhysicsError),
}

fn physics_mesh_geometry(
    model: &MeshAsset,
) -> Result<PhysicsMeshGeometry, StaticModelPhysicsError> {
    let vertex_capacity = model
        .primitives
        .iter()
        .try_fold(0usize, |total, primitive| {
            total.checked_add(primitive.positions.len())
        })
        .ok_or(StaticModelPhysicsError::TooManyVertices)?;
    if vertex_capacity > u32::MAX as usize {
        return Err(StaticModelPhysicsError::TooManyVertices);
    }
    let triangle_capacity = model
        .primitives
        .iter()
        .try_fold(0usize, |total, primitive| {
            total.checked_add(primitive.indices.len() / 3)
        })
        .ok_or(StaticModelPhysicsError::TooManyVertices)?;
    let mut geometry = PhysicsMeshGeometry {
        vertices: Vec::with_capacity(vertex_capacity),
        indices: Vec::with_capacity(triangle_capacity),
    };
    for (primitive_index, primitive) in model.primitives.iter().enumerate() {
        let base = u32::try_from(geometry.vertices.len())
            .map_err(|_| StaticModelPhysicsError::TooManyVertices)?;
        geometry
            .vertices
            .extend(primitive.positions.iter().copied().map(Vec3::from_array));
        if primitive.indices.len() % 3 != 0 {
            return Err(StaticModelPhysicsError::InvalidIndexCount {
                primitive: primitive_index,
                count: primitive.indices.len(),
            });
        }
        for triangle in primitive.indices.chunks_exact(3) {
            for &index in triangle {
                if index as usize >= primitive.positions.len() {
                    return Err(StaticModelPhysicsError::IndexOutOfBounds {
                        primitive: primitive_index,
                        index,
                        vertex_count: primitive.positions.len(),
                    });
                }
            }
            geometry
                .indices
                .push([base + triangle[0], base + triangle[1], base + triangle[2]]);
        }
    }
    if geometry.indices.is_empty() {
        return Err(StaticModelPhysicsError::NoTriangleGeometry);
    }
    geometry.validate()?;
    Ok(geometry)
}

/// Resolved static model asset for a renderable entity.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct StaticModelBinding {
    engine_path: String,
    handle: Handle<MeshAsset>,
    material_override_path: Option<String>,
    material_override: Option<Handle<MaterialAsset>>,
    spawned: bool,
}

/// Identifies the static-model revision that produced an entity's
/// [`PhysicsMeshGeometry`].
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticModelPhysicsBinding(pub AssetId<MeshAsset>);

/// Requests product-side triangle geometry from a loaded static model.
///
/// Carrying the request as its own marker keeps a particular collider component
/// out of the picture, so physics components in other gems can compose their own
/// material/filter/body semantics on top of the same geometry.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StaticModelPhysicsGeometryRequest;

impl StaticModelBinding {
    #[must_use]
    pub const fn new(engine_path: String, handle: Handle<MeshAsset>) -> Self {
        Self {
            engine_path,
            handle,
            material_override_path: None,
            material_override: None,
            spawned: false,
        }
    }

    #[must_use]
    pub fn engine_path(&self) -> &str {
        &self.engine_path
    }

    #[must_use]
    pub const fn handle(&self) -> &Handle<MeshAsset> {
        &self.handle
    }

    #[must_use]
    pub fn material_override_path(&self) -> Option<&str> {
        self.material_override_path.as_deref()
    }

    #[must_use]
    pub const fn material_override(&self) -> Option<&Handle<MaterialAsset>> {
        self.material_override.as_ref()
    }

    #[must_use]
    pub fn with_material_override(
        mut self,
        path: Option<String>,
        handle: Option<Handle<MaterialAsset>>,
    ) -> Self {
        self.material_override_path = path;
        self.material_override = handle;
        self
    }

    #[must_use]
    pub const fn spawned(&self) -> bool {
        self.spawned
    }

    pub const fn mark_pending(&mut self) {
        self.spawned = false;
    }

    pub const fn mark_spawned(&mut self) {
        self.spawned = true;
    }
}

/// Child entities spawned for one static model binding.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticModelChildren(pub Vec<Entity>);

/// Marker for entities spawned from a static model asset.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StaticModelPrimitiveVisual;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StaticCharacterAttachmentError {
    #[error("static attachment model: {0}")]
    Model(AssetRefLoadError),
    #[error("static attachment material: {0}")]
    Material(AssetRefLoadError),
}

/// Attachments that have neither been bound nor already failed to bind.
type UnboundStaticAttachmentFilter = (
    Without<StaticModelBinding>,
    Without<StaticCharacterAttachmentError>,
);

#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
pub fn resolve_static_character_attachments(
    mut commands: Commands,
    catalog: Option<Res<AssetCatalog>>,
    asset_server: Option<Res<AssetServer>>,
    material_assets: Option<Res<Assets<MaterialAsset>>>,
    attachments: Query<(Entity, &StaticCharacterAttachment), UnboundStaticAttachmentFilter>,
) {
    let (Some(catalog), Some(asset_server)) = (catalog, asset_server) else {
        return;
    };
    for (entity, attachment) in &attachments {
        let entry = match catalog.resolve_asset_id(attachment.asset_id, MESH_ASSET_TYPE) {
            Ok(entry) => entry,
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(StaticCharacterAttachmentError::Model(error));
                continue;
            }
        };
        let handle: Handle<MeshAsset> = asset_server.load(entry.relative_path().to_path_buf());
        let mut binding =
            StaticModelBinding::new(entry.relative_path().to_string_lossy().into_owned(), handle);
        if material_assets.is_some()
            && let Some(material_id) = attachment.material
        {
            let material_entry = match catalog
                .resolve_asset_id(material_id, AssetType::from(MATERIAL_ASSET_TYPE_ID))
            {
                Ok(entry) => entry,
                Err(error) => {
                    commands
                        .entity(entity)
                        .insert(StaticCharacterAttachmentError::Material(error));
                    continue;
                }
            };
            let material: Handle<MaterialAsset> =
                asset_server.load(material_entry.relative_path().to_path_buf());
            binding = binding.with_material_override(
                Some(
                    material_entry
                        .relative_path()
                        .to_string_lossy()
                        .into_owned(),
                ),
                Some(material),
            );
        }
        commands.entity(entity).insert(binding);
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
pub fn spawn_static_model_instances(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    static_models: Option<Res<Assets<MeshAsset>>>,
    material_assets: Option<Res<Assets<MaterialAsset>>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut query: Query<(
        Entity,
        &mut StaticModelBinding,
        Option<&StaticModelChildren>,
    )>,
) {
    let (Some(static_models), Some(meshes), Some(materials)) = (
        static_models.as_deref(),
        meshes.as_deref_mut(),
        materials.as_deref_mut(),
    ) else {
        return;
    };
    let asset_server = asset_server.as_deref();
    let material_assets = material_assets.as_deref();

    for (entity, mut binding, children) in &mut query {
        if binding.spawned() {
            continue;
        }
        let Some(model) = static_models.get(binding.handle()) else {
            continue;
        };
        let material_override = binding
            .material_override()
            .and_then(|handle| material_assets.and_then(|assets| assets.get(handle)));
        if binding.material_override().is_some_and(|handle| {
            material_assets.is_some()
                && material_override.is_none()
                && material_handle_pending(handle, asset_server)
        }) {
            continue;
        }
        if material_override.is_none()
            && model.primitives.iter().any(|primitive| {
                primitive_material_pending(model, primitive, asset_server, material_assets)
            })
        {
            continue;
        }
        despawn_static_model_children(&mut commands, children);

        let mut child_entities = Vec::with_capacity(model.primitives.len());
        commands.entity(entity).with_children(|parent| {
            for primitive in &model.primitives {
                let material_asset = material_override.or_else(|| {
                    primitive_material_asset(model, primitive, asset_server, material_assets)
                });
                let child = parent
                    .spawn((
                        Name::new(primitive.label.clone()),
                        StaticModelPrimitiveVisual,
                        Mesh3d(meshes.add(primitive.to_bevy_mesh())),
                        MeshMaterial3d(materials.add(static_model_material(
                            primitive.material_slot,
                            material_asset,
                            asset_server,
                        ))),
                        Transform::IDENTITY,
                    ))
                    .id();
                child_entities.push(child);
            }
        });
        commands
            .entity(entity)
            .insert(StaticModelChildren(child_entities));
        binding.mark_spawned();
    }
}

/// The binding revision each physics-bearing static model was last built from.
type StaticModelPhysicsData = (
    Entity,
    Ref<'static, StaticModelBinding>,
    Option<&'static StaticModelPhysicsBinding>,
);

/// Entities that either carry a mesh collider or explicitly asked for geometry.
type StaticModelPhysicsFilter = Or<(
    With<MeshColliderComponent>,
    With<StaticModelPhysicsGeometryRequest>,
)>;

/// Mirrors `MeshColliderComponent::OnMeshCreated`.
///
/// Once the mesh asset is available, derive the collider geometry from the same
/// runtime model product and notify physics through an ordinary changed
/// component.
#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
pub fn sync_static_model_physics_geometry(
    mut commands: Commands,
    static_models: Option<Res<Assets<MeshAsset>>>,
    mut asset_events: MessageReader<AssetEvent<MeshAsset>>,
    meshes: Query<StaticModelPhysicsData, StaticModelPhysicsFilter>,
) {
    let Some(static_models) = static_models.as_deref() else {
        return;
    };
    let changed = asset_events
        .read()
        .filter_map(|event| match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::LoadedWithDependencies { id }
            | AssetEvent::Removed { id } => Some(*id),
            AssetEvent::Unused { .. } => None,
        })
        .collect::<HashSet<_>>();

    for (entity, binding, materialized) in &meshes {
        let id = binding.handle().id();
        if !binding.is_changed()
            && materialized.is_some_and(|materialized| materialized.0 == id)
            && !changed.contains(&id)
        {
            continue;
        }
        let Some(model) = static_models.get(id) else {
            if materialized.is_some() {
                commands
                    .entity(entity)
                    .remove::<PhysicsMeshGeometry>()
                    .remove::<StaticModelPhysicsBinding>();
            }
            continue;
        };
        match physics_mesh_geometry(model) {
            Ok(geometry) => {
                commands
                    .entity(entity)
                    .insert((geometry, StaticModelPhysicsBinding(id)))
                    .remove::<StaticModelPhysicsError>();
            }
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(error)
                    .remove::<PhysicsMeshGeometry>()
                    .remove::<StaticModelPhysicsBinding>();
            }
        }
    }
}

pub fn cleanup_removed_static_model_physics_bindings(
    mut commands: Commands,
    mut removed: RemovedComponents<StaticModelBinding>,
    materialized: Query<(), With<StaticModelPhysicsBinding>>,
) {
    for entity in removed.read() {
        if materialized.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsMeshGeometry>()
                .remove::<StaticModelPhysicsBinding>()
                .remove::<StaticModelPhysicsError>();
        }
    }
}

pub fn cleanup_removed_mesh_collider_geometry(
    mut commands: Commands,
    mut removed: RemovedComponents<MeshColliderComponent>,
    materialized: Query<(), With<StaticModelPhysicsBinding>>,
    requests: Query<(), With<StaticModelPhysicsGeometryRequest>>,
) {
    for entity in removed.read() {
        if materialized.contains(entity) && !requests.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsMeshGeometry>()
                .remove::<StaticModelPhysicsBinding>()
                .remove::<StaticModelPhysicsError>();
        }
    }
}

pub fn cleanup_removed_static_model_physics_geometry_requests(
    mut commands: Commands,
    mut removed: RemovedComponents<StaticModelPhysicsGeometryRequest>,
    materialized: Query<(), With<StaticModelPhysicsBinding>>,
    mesh_colliders: Query<(), With<MeshColliderComponent>>,
) {
    for entity in removed.read() {
        if materialized.contains(entity) && !mesh_colliders.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsMeshGeometry>()
                .remove::<StaticModelPhysicsBinding>()
                .remove::<StaticModelPhysicsError>();
        }
    }
}

pub fn despawn_static_model_children(
    commands: &mut Commands,
    children: Option<&StaticModelChildren>,
) {
    let Some(children) = children else {
        return;
    };
    for child in &children.0 {
        commands.entity(*child).despawn();
    }
}

/// Grey tints cycled per material slot when a primitive resolves no material,
/// so adjacent slots stay visually distinguishable in an unmaterialed model.
const PLACEHOLDER_SLOT_TINTS: [f32; 5] = [0.0, 0.035, 0.07, 0.105, 0.14];

fn static_model_material(
    material_slot: u32,
    material_asset: Option<&MaterialAsset>,
    asset_server: Option<&AssetServer>,
) -> StandardMaterial {
    if let Some(material) = material_asset {
        if let Some(asset_server) = asset_server {
            return material.standard_material_for_slot_with_images(Some(material_slot), |path| {
                asset_server.load(path.to_owned())
            });
        }
        return material.standard_material_for_slot(Some(material_slot));
    }

    let tint = PLACEHOLDER_SLOT_TINTS[(material_slot as usize) % PLACEHOLDER_SLOT_TINTS.len()];
    StandardMaterial {
        base_color: Color::srgb(0.62 + tint, 0.64 + tint, 0.66 + tint),
        perceptual_roughness: 0.9,
        ..Default::default()
    }
}

fn primitive_material_pending(
    model: &MeshAsset,
    primitive: &MeshPrimitive,
    asset_server: Option<&AssetServer>,
    material_assets: Option<&Assets<MaterialAsset>>,
) -> bool {
    let Some(path) = primitive_material_path(model, primitive) else {
        return false;
    };
    let (Some(asset_server), Some(material_assets)) = (asset_server, material_assets) else {
        return false;
    };
    let handle: Handle<MaterialAsset> = asset_server.load(path.to_owned());
    material_assets.get(&handle).is_none() && material_handle_pending(&handle, Some(asset_server))
}

fn primitive_material_asset<'a>(
    model: &MeshAsset,
    primitive: &MeshPrimitive,
    asset_server: Option<&AssetServer>,
    material_assets: Option<&'a Assets<MaterialAsset>>,
) -> Option<&'a MaterialAsset> {
    let path = primitive_material_path(model, primitive)?;
    let asset_server = asset_server?;
    let material_assets = material_assets?;
    let handle: Handle<MaterialAsset> = asset_server.load(path.to_owned());
    material_assets.get(&handle)
}

fn primitive_material_path<'a>(model: &'a MeshAsset, primitive: &MeshPrimitive) -> Option<&'a str> {
    model
        .material_slots
        .iter()
        .find(|slot| slot.id == primitive.material_slot)
        .and_then(|slot| slot.default_material.as_deref())
}

fn material_handle_pending(
    handle: &Handle<MaterialAsset>,
    asset_server: Option<&AssetServer>,
) -> bool {
    let Some(asset_server) = asset_server else {
        return false;
    };
    matches!(
        asset_server.load_state(handle),
        LoadState::NotLoaded | LoadState::Loading
    )
}

#[cfg(test)]
mod tests {
    use bevy::asset::AssetPlugin;

    use super::*;

    #[test]
    fn static_model_engine_asset_paths_resolve_to_native_products() {
        assert_eq!(
            static_model_engine_asset_path("Objects/Nature/Oak.cgf"),
            "objects/nature/oak.azmesh"
        );
        assert_eq!(
            static_model_engine_asset_path("objects/props/cart.cga"),
            "objects/props/cart.azmesh"
        );
        assert_eq!(
            static_model_engine_asset_path("meshes/props/cart.glb"),
            "meshes/props/cart.azmesh"
        );
    }

    #[test]
    fn physics_geometry_merges_primitives_and_applies_local_transforms() {
        let model = MeshAsset {
            name: "two triangles".to_owned(),
            bounds_min: [0.0; 3],
            bounds_max: [2.0; 3],
            material_slots: Vec::new(),
            primitives: vec![
                MeshPrimitive {
                    label: "xy".to_owned(),
                    material_slot: 0,
                    positions: vec![[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                    normals: Vec::new(),
                    tangents: Vec::new(),
                    uv0: Vec::new(),
                    indices: vec![0, 1, 2],
                },
                MeshPrimitive {
                    label: "xz".to_owned(),
                    material_slot: 0,
                    positions: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.0, 2.0]],
                    normals: Vec::new(),
                    tangents: Vec::new(),
                    uv0: Vec::new(),
                    indices: vec![0, 1, 2],
                },
            ],
        };

        let geometry = physics_mesh_geometry(&model).unwrap();

        assert_eq!(
            geometry.vertices,
            vec![
                Vec3::X,
                Vec3::X * 2.0,
                Vec3::X + Vec3::Y,
                Vec3::ZERO,
                Vec3::X * 2.0,
                Vec3::Z * 2.0
            ]
        );
        assert_eq!(geometry.indices, vec![[0, 1, 2], [3, 4, 5]]);
    }

    #[test]
    fn mesh_collider_tracks_loaded_static_model_geometry_lifecycle() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<MeshAsset>();
        crate::mesh::register_static_model_physics_components(&mut app);

        let handle = app
            .world_mut()
            .resource_mut::<Assets<MeshAsset>>()
            .add(MeshAsset {
                name: "triangle".to_owned(),
                bounds_min: [0.0, 0.0, 0.0],
                bounds_max: [1.0, 1.0, 0.0],
                material_slots: Vec::new(),
                primitives: vec![MeshPrimitive {
                    label: "triangle".to_owned(),
                    material_slot: 0,
                    positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    normals: Vec::new(),
                    tangents: Vec::new(),
                    uv0: Vec::new(),
                    indices: vec![0, 1, 2],
                }],
            });
        let entity = app
            .world_mut()
            .spawn((
                MeshColliderComponent::default(),
                StaticModelBinding::new("objects/test.azmesh".to_owned(), handle.clone()),
            ))
            .id();

        app.update();

        let entity_ref = app.world().entity(entity);
        assert_eq!(
            entity_ref.get::<PhysicsMeshGeometry>(),
            Some(&PhysicsMeshGeometry {
                vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
                indices: vec![[0, 1, 2]],
            })
        );
        assert_eq!(
            entity_ref.get::<StaticModelPhysicsBinding>(),
            Some(&StaticModelPhysicsBinding(handle.id()))
        );

        app.world_mut()
            .entity_mut(entity)
            .remove::<StaticModelBinding>();
        app.update();

        let entity_ref = app.world().entity(entity);
        assert!(!entity_ref.contains::<PhysicsMeshGeometry>());
        assert!(!entity_ref.contains::<StaticModelPhysicsBinding>());
    }
}
