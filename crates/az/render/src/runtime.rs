use std::collections::HashMap;

use az_material::MaterialPropertyValue;
use az_material_builder::loader::{MaterialAssetLoader, MaterialTypeAssetLoader};
use az_material_builder::{MaterialAsset, MaterialTypeAsset};
use az_mesh_builder::{MeshAsset, MeshAssetPlugin};
use bevy::asset::{AssetEvent, AssetId, AssetServer, Assets, Handle, LoadState};
use bevy::camera::primitives::Aabb;
use bevy::ecs::message::MessageReader;
use bevy::math::{Vec3, Vec3A, primitives::Cuboid};
use bevy::mesh::{Mesh as BevyMesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::{
    Added, Alpha, AlphaMode, App, Changed, ChildOf, Color, Commands, Component, Entity,
    IntoScheduleConfigs, Or, Plugin, Query, Res, ResMut, Resource, Transform, Update, Visibility,
    With,
};

use crate::{MaterialAssignment, Mesh};

/// Observable lifecycle of an authored mesh asset reference.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MeshRenderState {
    #[default]
    Missing,
    Loading,
    Ready,
    Failed,
}

/// Marks renderer-owned primitive/placeholder children. The authored entity is
/// never respawned during asset reconciliation.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderMeshChild {
    pub owner: Entity,
    pub primitive_index: Option<usize>,
}

/// Material-slot decision retained on each render child for diagnostics and
/// editor readout.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMaterialSlot {
    pub slot_id: u32,
    pub slot_label: String,
    pub material_path: Option<String>,
}

#[derive(Component, Debug, Clone)]
struct MeshLoadHandle(Handle<MeshAsset>);

#[derive(Component, Debug, Clone, Copy)]
struct MeshRebuildRequested;

#[derive(Resource)]
struct RenderFallbacks {
    placeholder_mesh: Handle<BevyMesh>,
    missing_material: Handle<StandardMaterial>,
    loading_material: Handle<StandardMaterial>,
    failed_material: Handle<StandardMaterial>,
    default_material: Handle<StandardMaterial>,
}

impl bevy::ecs::world::FromWorld for RenderFallbacks {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let placeholder_mesh = world
            .resource_mut::<Assets<BevyMesh>>()
            .add(Cuboid::new(0.8, 0.8, 0.8));
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        let missing_material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.08, 0.55, 0.95),
            emissive: bevy::color::LinearRgba::rgb(0.02, 0.18, 0.42),
            unlit: true,
            ..Default::default()
        });
        let loading_material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.72, 0.12),
            emissive: bevy::color::LinearRgba::rgb(0.40, 0.22, 0.01),
            unlit: true,
            ..Default::default()
        });
        let failed_material = materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.02, 0.62),
            emissive: bevy::color::LinearRgba::rgb(0.55, 0.0, 0.24),
            unlit: true,
            ..Default::default()
        });
        let default_material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.58, 0.60, 0.66),
            perceptual_roughness: 0.72,
            ..Default::default()
        });
        Self {
            placeholder_mesh,
            missing_material,
            loading_material,
            failed_material,
            default_material,
        }
    }
}

struct CachedMaterial {
    source: Handle<MaterialAsset>,
    standard: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
struct MaterialCache {
    by_path: HashMap<String, CachedMaterial>,
}

/// Installs cooked mesh/material loaders plus asset-backed render resolution.
pub struct AzothRenderPlugin;

impl Plugin for AzothRenderPlugin {
    fn build(&self, app: &mut App) {
        use bevy::asset::AssetApp as _;

        if !app.is_plugin_added::<MeshAssetPlugin>() {
            app.add_plugins(MeshAssetPlugin);
        }
        app.init_asset::<MaterialTypeAsset>()
            .init_asset_loader::<MaterialTypeAssetLoader>()
            .init_asset::<MaterialAsset>()
            .init_asset_loader::<MaterialAssetLoader>()
            .init_resource::<RenderFallbacks>()
            .init_resource::<MaterialCache>()
            .add_systems(
                Update,
                (
                    start_mesh_loads,
                    mark_changed_assignments,
                    mark_mesh_asset_events,
                    reconcile_mesh_states,
                    reconcile_material_assets,
                    rebuild_meshes,
                )
                    .chain(),
            );
    }
}

/// Entities whose `Mesh` reference was just added or changed.
type ChangedMeshReferences<'w, 's> =
    Query<'w, 's, (Entity, &'static Mesh), Or<(Added<Mesh>, Changed<Mesh>)>>;

/// Entities flagged for a rebuild, with the render state and handles the
/// rebuild reads.
type MeshRebuildRequests<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static MeshRenderState,
        Option<&'static MeshLoadHandle>,
        Option<&'static MaterialAssignment>,
    ),
    With<MeshRebuildRequested>,
>;

#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn start_mesh_loads(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    references: ChangedMeshReferences<'_, '_>,
) {
    for (entity, reference) in &references {
        let visibility = if reference.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let path = &reference.mesh;
        if path.as_str().is_empty() {
            commands.entity(entity).remove::<MeshLoadHandle>().insert((
                MeshRenderState::Missing,
                MeshRebuildRequested,
                visibility,
            ));
            continue;
        }
        let handle = asset_server.load::<MeshAsset>(path.as_str().to_owned());
        tracing::info!(entity = ?entity, asset = path.as_str(), "mesh asset load requested");
        commands.entity(entity).insert((
            MeshLoadHandle(handle),
            MeshRenderState::Loading,
            MeshRebuildRequested,
            visibility,
        ));
    }
}

fn mark_changed_assignments(
    mut commands: Commands,
    changed: Query<Entity, Changed<MaterialAssignment>>,
) {
    for entity in &changed {
        commands.entity(entity).insert(MeshRebuildRequested);
    }
}

fn mark_mesh_asset_events(
    mut commands: Commands,
    mut events: MessageReader<AssetEvent<MeshAsset>>,
    handles: Query<(Entity, &MeshLoadHandle)>,
) {
    for event in events.read() {
        let id = match *event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::Removed { id }
            | AssetEvent::LoadedWithDependencies { id } => id,
            AssetEvent::Unused { .. } => continue,
        };
        for (entity, handle) in &handles {
            if handle.0.id() == id {
                if matches!(event, AssetEvent::Modified { .. }) {
                    tracing::info!(entity = ?entity, asset = ?id, "mesh asset hot reload reconciled");
                }
                commands.entity(entity).insert(MeshRebuildRequested);
            }
        }
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn reconcile_mesh_states(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    meshes: Res<Assets<MeshAsset>>,
    query: Query<(Entity, &MeshLoadHandle, &MeshRenderState)>,
) {
    for (entity, handle, current) in &query {
        let next = if meshes.get(&handle.0).is_some() {
            MeshRenderState::Ready
        } else {
            match asset_server.load_state(handle.0.id()) {
                LoadState::Failed(_) => MeshRenderState::Failed,
                LoadState::Loaded => MeshRenderState::Ready,
                LoadState::NotLoaded | LoadState::Loading => MeshRenderState::Loading,
            }
        };
        if *current != next {
            match next {
                MeshRenderState::Ready => {
                    tracing::info!(entity = ?entity, "mesh asset became render-ready");
                }
                MeshRenderState::Failed => {
                    if let LoadState::Failed(error) = asset_server.load_state(handle.0.id()) {
                        tracing::info!(entity = ?entity, %error, "mesh asset load failed");
                    }
                }
                MeshRenderState::Missing | MeshRenderState::Loading => {}
            }
            commands.entity(entity).insert((next, MeshRebuildRequested));
        }
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn reconcile_material_assets(
    mut events: MessageReader<AssetEvent<MaterialAsset>>,
    mut cache: ResMut<MaterialCache>,
    sources: Res<Assets<MaterialAsset>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    let changed = events
        .read()
        .filter_map(|event| match *event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::LoadedWithDependencies { id } => Some(id),
            AssetEvent::Removed { id } => {
                reset_removed_material(id, &mut cache, &mut standards);
                None
            }
            AssetEvent::Unused { .. } => None,
        })
        .collect::<Vec<_>>();
    for entry in cache.by_path.values_mut() {
        if !changed.contains(&entry.source.id()) {
            continue;
        }
        let Some(source) = sources.get(&entry.source) else {
            continue;
        };
        if let Some(mut material) = standards.get_mut(&entry.standard) {
            *material = standard_material(source);
        }
        tracing::info!(asset = ?entry.source.id(), "material asset reconciled in place");
    }
}

fn reset_removed_material(
    id: AssetId<MaterialAsset>,
    cache: &mut MaterialCache,
    standards: &mut Assets<StandardMaterial>,
) {
    for entry in cache
        .by_path
        .values_mut()
        .filter(|entry| entry.source.id() == id)
    {
        if let Some(mut material) = standards.get_mut(&entry.standard) {
            *material = StandardMaterial {
                base_color: Color::srgb(1.0, 0.02, 0.62),
                emissive: bevy::color::LinearRgba::rgb(0.55, 0.0, 0.24),
                unlit: true,
                ..Default::default()
            };
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn rebuild_meshes(
    mut commands: Commands,
    requested: MeshRebuildRequests<'_, '_>,
    existing: Query<(Entity, &RenderMeshChild)>,
    meshes: Res<Assets<MeshAsset>>,
    mut gpu_meshes: ResMut<Assets<BevyMesh>>,
    asset_server: Res<AssetServer>,
    mut standards: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<MaterialCache>,
    fallbacks: Res<RenderFallbacks>,
) {
    for (owner, state, mesh_handle, assignment) in &requested {
        for (entity, child) in &existing {
            if child.owner == owner {
                commands.entity(entity).despawn();
            }
        }
        match *state {
            MeshRenderState::Ready => {
                let Some(mesh_asset) = mesh_handle.and_then(|handle| meshes.get(&handle.0)) else {
                    spawn_placeholder(&mut commands, owner, &fallbacks, MeshRenderState::Loading);
                    commands.entity(owner).remove::<MeshRebuildRequested>();
                    continue;
                };
                commands.entity(owner).insert(product_aabb(mesh_asset));
                for (primitive_index, primitive) in mesh_asset.primitives.iter().enumerate() {
                    let slot = mesh_asset
                        .material_slots
                        .iter()
                        .find(|slot| slot.id == primitive.material_slot)
                        .or_else(|| mesh_asset.material_slots.first());
                    let material_path = slot.and_then(|slot| {
                        assigned_material_path(assignment, slot.id, &slot.label)
                            .or_else(|| slot.default_material.clone())
                    });
                    let material = material_path.as_deref().map_or_else(
                        || fallbacks.default_material.clone(),
                        |path| {
                            cached_material(
                                path,
                                &asset_server,
                                &mut standards,
                                &mut material_cache,
                            )
                        },
                    );
                    commands.spawn((
                        Mesh3d(gpu_meshes.add(primitive.to_bevy_mesh())),
                        MeshMaterial3d(material),
                        Transform::default(),
                        Visibility::default(),
                        RenderMeshChild {
                            owner,
                            primitive_index: Some(primitive_index),
                        },
                        ResolvedMaterialSlot {
                            slot_id: slot.map_or(0, |slot| slot.id),
                            slot_label: slot
                                .map_or_else(|| "default".to_owned(), |slot| slot.label.clone()),
                            material_path,
                        },
                        ChildOf(owner),
                    ));
                }
            }
            other => spawn_placeholder(&mut commands, owner, &fallbacks, other),
        }
        commands.entity(owner).remove::<MeshRebuildRequested>();
    }
}

fn spawn_placeholder(
    commands: &mut Commands<'_, '_>,
    owner: Entity,
    fallbacks: &RenderFallbacks,
    state: MeshRenderState,
) {
    let material = match state {
        MeshRenderState::Missing => fallbacks.missing_material.clone(),
        MeshRenderState::Loading | MeshRenderState::Ready => fallbacks.loading_material.clone(),
        MeshRenderState::Failed => fallbacks.failed_material.clone(),
    };
    commands.entity(owner).insert(Aabb {
        center: Vec3A::ZERO,
        half_extents: Vec3A::splat(0.4),
    });
    commands.spawn((
        Mesh3d(fallbacks.placeholder_mesh.clone()),
        MeshMaterial3d(material),
        Transform::default(),
        Visibility::default(),
        RenderMeshChild {
            owner,
            primitive_index: None,
        },
        ChildOf(owner),
    ));
}

fn product_aabb(asset: &MeshAsset) -> Aabb {
    let min = Vec3::from_array(asset.bounds_min);
    let max = Vec3::from_array(asset.bounds_max);
    Aabb {
        center: Vec3A::from((min + max) * 0.5),
        half_extents: Vec3A::from((max - min) * 0.5),
    }
}

fn assigned_material_path(
    assignment: Option<&MaterialAssignment>,
    slot_id: u32,
    slot_label: &str,
) -> Option<String> {
    let assignment = assignment?;
    assignment
        .slots
        .iter()
        .find(|override_| override_.slot_id == Some(slot_id))
        .or_else(|| {
            assignment
                .slots
                .iter()
                .find(|override_| override_.slot_label.as_deref() == Some(slot_label))
        })
        .map(|override_| override_.material.as_str().to_owned())
        .or_else(|| {
            assignment
                .default
                .as_ref()
                .map(|path| path.as_str().to_owned())
        })
}

fn cached_material(
    path: &str,
    asset_server: &AssetServer,
    standards: &mut Assets<StandardMaterial>,
    cache: &mut MaterialCache,
) -> Handle<StandardMaterial> {
    if let Some(entry) = cache.by_path.get(path) {
        return entry.standard.clone();
    }
    let source = asset_server.load::<MaterialAsset>(path.to_owned());
    let standard = standards.add(StandardMaterial {
        base_color: Color::srgb(0.82, 0.62, 0.12),
        perceptual_roughness: 0.65,
        ..Default::default()
    });
    cache.by_path.insert(
        path.to_owned(),
        CachedMaterial {
            source,
            standard: standard.clone(),
        },
    );
    standard
}

fn standard_material(asset: &MaterialAsset) -> StandardMaterial {
    let mut material = StandardMaterial::default();
    for binding in &asset.property_values {
        let property = binding.property.to_ascii_lowercase();
        match (property.as_str(), &binding.value) {
            ("base.color" | "base_color" | "color", MaterialPropertyValue::Color(value)) => {
                material.base_color =
                    Color::srgba(value.rgba.x, value.rgba.y, value.rgba.z, value.rgba.w);
            }
            ("base.color" | "base_color" | "color", MaterialPropertyValue::Vec4(value)) => {
                material.base_color = Color::srgba(value.x, value.y, value.z, value.w);
            }
            ("base.color" | "base_color" | "color", MaterialPropertyValue::Vec3(value)) => {
                material.base_color = Color::srgb(value.x, value.y, value.z);
            }
            ("base.roughness" | "roughness", MaterialPropertyValue::Float(value)) => {
                material.perceptual_roughness = value.clamp(0.0, 1.0);
            }
            ("base.metallic" | "metallic", MaterialPropertyValue::Float(value)) => {
                material.metallic = value.clamp(0.0, 1.0);
            }
            ("unlit", MaterialPropertyValue::Bool(value)) => material.unlit = *value,
            _ => {}
        }
    }
    if material.base_color.alpha() < 1.0 {
        material.alpha_mode = AlphaMode::Blend;
    }
    material
}

#[cfg(test)]
mod tests {
    use az_core::AssetPathBuf;
    use az_mesh_builder::{MeshMaterialSlot, MeshPrimitive};
    use bevy::asset::{AssetApp as _, AssetPlugin};
    use bevy::prelude::MinimalPlugins;

    use super::*;
    use crate::MaterialSlotOverride;

    fn override_(id: Option<u32>, label: Option<&str>, material: &str) -> MaterialSlotOverride {
        MaterialSlotOverride {
            slot_id: id,
            slot_label: label.map(str::to_owned),
            material: AssetPathBuf::new(material).unwrap(),
        }
    }

    #[test]
    fn material_fallback_is_id_then_label_then_default_then_imported() {
        let assignment = MaterialAssignment {
            default: Some(AssetPathBuf::new("materials/default.azmaterial").unwrap()),
            slots: vec![
                override_(None, Some("body"), "materials/by-label.azmaterial"),
                override_(Some(7), None, "materials/by-id.azmaterial"),
            ],
        };
        assert_eq!(
            assigned_material_path(Some(&assignment), 7, "body").as_deref(),
            Some("materials/by-id.azmaterial")
        );
        assert_eq!(
            assigned_material_path(Some(&assignment), 8, "body").as_deref(),
            Some("materials/by-label.azmaterial")
        );
        assert_eq!(
            assigned_material_path(Some(&assignment), 8, "other").as_deref(),
            Some("materials/default.azmaterial")
        );
        assert_eq!(assigned_material_path(None, 7, "body"), None);
    }

    #[test]
    fn mesh_owner_visibility_tracks_authored_reference() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<BevyMesh>()
            .init_asset::<StandardMaterial>()
            .add_plugins(AzothRenderPlugin);
        let owner = app
            .world_mut()
            .spawn(Mesh {
                mesh: AssetPathBuf::default(),
                visible: false,
                ..Default::default()
            })
            .id();

        app.update();
        assert_eq!(
            app.world().get::<Visibility>(owner),
            Some(&Visibility::Hidden)
        );

        app.world_mut()
            .entity_mut(owner)
            .get_mut::<Mesh>()
            .unwrap()
            .visible = true;
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(owner),
            Some(&Visibility::Inherited)
        );
    }

    #[test]
    fn mesh_hot_reload_rebuilds_children_and_bounds_without_respawning_owner() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<BevyMesh>()
            .init_asset::<StandardMaterial>()
            .add_plugins(AzothRenderPlugin);

        let mesh_asset = MeshAsset {
            name: "phase2-pyramid".to_string(),
            bounds_min: [-1.5, 0.0, -0.5],
            bounds_max: [1.5, 2.5, 0.5],
            material_slots: vec![MeshMaterialSlot {
                id: 7,
                label: "body".to_string(),
                default_material: None,
            }],
            primitives: vec![MeshPrimitive {
                label: "pyramid".to_string(),
                material_slot: 7,
                positions: vec![[-1.5, 0.0, -0.5], [1.5, 0.0, -0.5], [0.0, 2.5, 0.5]],
                normals: Vec::new(),
                tangents: Vec::new(),
                uv0: Vec::new(),
                indices: vec![0, 1, 2],
            }],
        };
        let handle = app
            .world_mut()
            .resource_mut::<Assets<MeshAsset>>()
            .add(mesh_asset);
        let owner = app
            .world_mut()
            .spawn((
                Transform::default(),
                MeshLoadHandle(handle.clone()),
                MeshRenderState::Ready,
                MeshRebuildRequested,
            ))
            .id();

        app.update();
        let bounds = app.world().get::<Aabb>(owner).unwrap();
        assert_eq!(Vec3::from(bounds.center), Vec3::new(0.0, 1.25, 0.0));
        assert_eq!(Vec3::from(bounds.half_extents), Vec3::new(1.5, 1.25, 0.5));
        let first_child = {
            let world = app.world_mut();
            let mut query = world.query::<(Entity, &RenderMeshChild)>();
            query
                .iter(world)
                .find_map(|(entity, child)| (child.owner == owner).then_some(entity))
                .unwrap()
        };

        app.world_mut()
            .resource_mut::<Assets<MeshAsset>>()
            .get_mut(&handle)
            .unwrap()
            .bounds_max = [4.5, 5.0, 0.5];
        app.world_mut()
            .write_message(AssetEvent::Modified { id: handle.id() });
        app.update();

        assert!(app.world().get_entity(owner).is_ok());
        assert!(app.world().get_entity(first_child).is_err());
        let bounds = app.world().get::<Aabb>(owner).unwrap();
        assert_eq!(Vec3::from(bounds.center), Vec3::new(1.5, 2.5, 0.0));
        assert_eq!(Vec3::from(bounds.half_extents), Vec3::new(3.0, 2.5, 0.5));
    }
}
