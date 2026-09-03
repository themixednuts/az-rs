use std::{ops::Range, sync::Arc};

use az_core::AssetId as AzAssetId;
use az_framework::asset::{AssetCatalog, AssetRefLoadError};
use az_gem_animation::{
    CharacterAttachmentNode, CharacterClothCollision, CharacterSkeleton, ClothCharacterAttachment,
};
use az_mesh::SKINNED_MESH_ASSET_TYPE;
use az_nv_cloth::{
    ClothFabricAsset, ClothMaterialAsset, ClothMaterialBinding, ClothRenderMapping,
    ClothSimulationVertex,
};
use az_physics::{PhysicsSceneId, PhysicsWorld};
use bevy::{
    app::{AnimationSystems, PostUpdate},
    asset::{AssetEvent, AssetServer, Assets, Handle},
    camera::primitives::{Aabb, MeshAabb},
    ecs::hierarchy::{ChildOf, Children},
    gltf::GltfAssetLabel,
    math::{Affine3A, Mat4},
    mesh::{
        Mesh, Mesh3d, VertexAttributeValues,
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
    prelude::*,
    transform::TransformSystems,
    world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady},
};

use crate::{
    fabric::SharedFabricCache,
    solver::{
        ClothAdvanceResult, ClothCapsuleCollider, ClothParticleTarget, ClothSimulationFrame,
        ClothSolver,
    },
};

#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct ClothWorldParameters {
    pub gravity: Vec3,
    pub wind: Vec3,
}

impl Default for ClothWorldParameters {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, 0.0, -9.81),
            wind: Vec3::ZERO,
        }
    }
}

#[derive(Component, Debug, Clone)]
struct PendingClothInstance {
    fabric: Handle<ClothFabricAsset>,
    material: Option<Handle<ClothMaterialAsset>>,
}

#[derive(Component, Debug, Clone, Copy)]
struct ClothRenderRoot {
    cloth: Entity,
}

#[derive(Debug, Clone)]
enum BoundRenderMapping {
    Direct,
    Barycentric { entries: Range<usize> },
}

#[derive(Debug, Clone)]
struct BoundRenderMesh {
    entity: Entity,
    mesh: Handle<Mesh>,
    mapping: BoundRenderMapping,
    mapped_positions: Box<[Vec3]>,
    mapped_normals: Box<[Vec3]>,
    tangent_data: Option<BoundTangentData>,
}

#[derive(Debug, Clone)]
struct BoundTangentData {
    uvs: Box<[Vec2]>,
    indices: Box<[u32]>,
    handedness: Box<[f32]>,
    accumulated: Box<[Vec3]>,
}

#[derive(Component, Debug)]
pub struct ClothInstance {
    fabric_asset_id: AzAssetId,
    fabric: Handle<ClothFabricAsset>,
    material: Option<Handle<ClothMaterialAsset>>,
    solver: ClothSolver,
    render_root: Entity,
    render_meshes: Vec<BoundRenderMesh>,
    joints: Box<[Entity]>,
    inverse_bindposes: Option<Handle<SkinnedMeshInverseBindposes>>,
    targets: Box<[ClothParticleTarget]>,
    collider_history: Vec<(Entity, Transform)>,
    collider_entities: Vec<Entity>,
    colliders: Vec<ClothCapsuleCollider>,
    binding_ready: bool,
    render_dirty: bool,
}

impl ClothInstance {
    #[must_use]
    pub const fn fabric_asset_id(&self) -> AzAssetId {
        self.fabric_asset_id
    }

    #[must_use]
    pub fn positions(&self) -> &[Vec3] {
        self.solver.positions()
    }
}

#[derive(Component, Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClothInstanceError {
    #[error("cloth fabric: {0}")]
    Fabric(AssetRefLoadError),
    #[error("cloth material: {0}")]
    Material(AssetRefLoadError),
    #[error("cloth render model: {0}")]
    RenderModel(AssetRefLoadError),
    #[error("cloth render model contains no skinned mesh")]
    MissingSkinnedMesh,
    #[error("cloth render skin references missing character joint {0}")]
    MissingJoint(Arc<str>),
    #[error(
        "cloth simulation vertex {vertex} references skin joint {joint}, but the skin has {joints} joints"
    )]
    InvalidSimulationJoint {
        vertex: usize,
        joint: u16,
        joints: usize,
    },
    #[error("cloth render model has no mesh matching its cooked render mapping")]
    MissingRenderMapping,
}

pub fn register_runtime(app: &mut App) {
    app.init_resource::<ClothWorldParameters>()
        .init_resource::<SharedFabricCache>()
        .add_systems(
            Update,
            (
                resolve_cloth_assets,
                initialize_loaded_cloth,
                invalidate_changed_fabrics,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            (simulate_cloth, upload_cloth_meshes)
                .chain()
                .after(AnimationSystems)
                .after(TransformSystems::Propagate),
        )
        .add_observer(bind_cloth_render_model);
}

/// Query filter matching attachments that have not started resolving yet.
type UnresolvedClothAttachment = (
    Without<PendingClothInstance>,
    Without<ClothInstance>,
    Without<ClothInstanceError>,
);

fn resolve_cloth_assets(
    mut commands: Commands,
    catalog: Option<Res<AssetCatalog>>,
    asset_server: Option<Res<AssetServer>>,
    attachments: Query<(Entity, &ClothCharacterAttachment), UnresolvedClothAttachment>,
) {
    let (Some(catalog), Some(asset_server)) = (catalog, asset_server) else {
        return;
    };
    for (entity, attachment) in &attachments {
        match catalog.load_asset_id::<ClothFabricAsset>(
            attachment.asset_id,
            az_nv_cloth::ids::CLOTH_FABRIC,
            &asset_server,
        ) {
            Ok(fabric) => {
                commands.entity(entity).insert(PendingClothInstance {
                    fabric,
                    material: None,
                });
            }
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(ClothInstanceError::Fabric(error));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn initialize_loaded_cloth(
    mut commands: Commands,
    catalog: Option<Res<AssetCatalog>>,
    asset_server: Option<Res<AssetServer>>,
    fabrics: Option<Res<Assets<ClothFabricAsset>>>,
    materials: Option<Res<Assets<ClothMaterialAsset>>>,
    mut fabric_cache: ResMut<SharedFabricCache>,
    mut pending_instances: Query<(Entity, &ClothCharacterAttachment, &mut PendingClothInstance)>,
) {
    let (Some(catalog), Some(asset_server), Some(fabrics), Some(materials)) =
        (catalog, asset_server, fabrics, materials)
    else {
        return;
    };

    for (entity, attachment, mut pending) in &mut pending_instances {
        let Some(asset) = fabrics.get(&pending.fabric) else {
            continue;
        };
        if let ClothMaterialBinding::Asset(material_id) = &asset.fabric().material
            && pending.material.is_none()
        {
            match catalog.load_asset_id::<ClothMaterialAsset>(
                *material_id,
                az_nv_cloth::ids::CLOTH_MATERIAL,
                &asset_server,
            ) {
                Ok(material) => pending.material = Some(material),
                Err(error) => {
                    commands
                        .entity(entity)
                        .insert(ClothInstanceError::Material(error))
                        .remove::<PendingClothInstance>();
                }
            }
            continue;
        }
        let material = match (&asset.fabric().material, &pending.material) {
            (ClothMaterialBinding::Asset(_), Some(handle)) => {
                let Some(material) = materials.get(handle) else {
                    continue;
                };
                *material.material()
            }
            (ClothMaterialBinding::Embedded(material), None) => *material,
            (ClothMaterialBinding::Asset(_), None) => continue,
            (ClothMaterialBinding::Embedded(_), Some(_)) => {
                unreachable!("embedded cloth material cannot have an external material handle")
            }
        };
        let render_entry =
            match catalog.resolve_asset_id(asset.fabric().render_model, SKINNED_MESH_ASSET_TYPE) {
                Ok(entry) => entry,
                Err(error) => {
                    commands
                        .entity(entity)
                        .insert(ClothInstanceError::RenderModel(error))
                        .remove::<PendingClothInstance>();
                    continue;
                }
            };
        let scene_path =
            GltfAssetLabel::Scene(0).from_asset(render_entry.relative_path().to_path_buf());
        let scene: Handle<WorldAsset> = asset_server.load(scene_path);
        let render_root = commands
            .spawn((
                Name::new("cloth-render-model"),
                WorldAssetRoot(scene),
                ClothRenderRoot { cloth: entity },
                Transform::IDENTITY,
                if attachment.parameters.hidden {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                },
                ChildOf(entity),
            ))
            .id();
        let shared = fabric_cache.get_or_insert(attachment.asset_id, asset);
        let particle_count = asset.fabric().mesh.vertices.len();
        commands
            .entity(entity)
            .insert(ClothInstance {
                fabric_asset_id: attachment.asset_id,
                fabric: pending.fabric.clone(),
                material: pending.material.clone(),
                solver: ClothSolver::new(asset, shared, material),
                render_root,
                render_meshes: Vec::new(),
                joints: Box::new([]),
                inverse_bindposes: None,
                targets: vec![ClothParticleTarget::new(Vec3::ZERO, Vec3::Z); particle_count]
                    .into_boxed_slice(),
                collider_history: Vec::with_capacity(16),
                collider_entities: Vec::with_capacity(16),
                colliders: Vec::with_capacity(16),
                binding_ready: false,
                render_dirty: false,
            })
            .remove::<PendingClothInstance>();
    }
}

// Bevy observer: `On`, `Res` and `Query` are owned parameter wrappers, so
// borrowing them here would stop this function satisfying `IntoObserverSystem`.
#[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)]
fn bind_cloth_render_model(
    trigger: On<WorldInstanceReady>,
    mut commands: Commands,
    roots: Query<&ClothRenderRoot>,
    mut instances: Query<(&ClothCharacterAttachment, &mut ClothInstance)>,
    nodes: Query<&CharacterAttachmentNode>,
    skeletons: Query<&CharacterSkeleton>,
    children: Query<&Children>,
    names: Query<&Name>,
    skinned_meshes: Query<&SkinnedMesh>,
    mesh_handles: Query<&Mesh3d>,
    mut meshes: ResMut<Assets<Mesh>>,
    fabrics: Res<Assets<ClothFabricAsset>>,
) {
    let Ok(root) = roots.get(trigger.entity) else {
        return;
    };
    let Ok((_, mut instance)) = instances.get_mut(root.cloth) else {
        return;
    };
    let Ok(node) = nodes.get(root.cloth) else {
        return;
    };
    let Ok(skeleton) = skeletons.get(node.character) else {
        return;
    };
    let Some(asset) = fabrics.get(&instance.fabric) else {
        return;
    };

    let skinned_entities = children
        .iter_descendants(trigger.entity)
        .filter(|entity| skinned_meshes.get(*entity).is_ok())
        .collect::<Vec<_>>();
    let Some(reference_skin) = skinned_entities
        .iter()
        .filter_map(|entity| skinned_meshes.get(*entity).ok())
        .max_by_key(|skin| skin.joints.len())
    else {
        commands
            .entity(root.cloth)
            .insert(ClothInstanceError::MissingSkinnedMesh);
        return;
    };
    let mapped_joints = match map_simulation_joints(reference_skin, skeleton, &names, asset) {
        Ok(mapped_joints) => mapped_joints,
        Err(error) => {
            commands.entity(root.cloth).insert(error);
            return;
        }
    };

    let candidates = render_mesh_candidates(&skinned_entities, &mesh_handles, &names, &meshes);
    let bound_meshes = bind_render_meshes(
        &mut commands,
        &mut meshes,
        &asset.fabric().mesh.render_mapping,
        candidates,
    );
    if bound_meshes.is_empty() {
        commands
            .entity(root.cloth)
            .insert(ClothInstanceError::MissingRenderMapping);
        return;
    }
    instance.joints = mapped_joints.into_boxed_slice();
    instance.inverse_bindposes = Some(reference_skin.inverse_bindposes.clone());
    instance.render_meshes = bound_meshes;
    instance.binding_ready = true;
}

/// Map the reference skin's joints onto the character skeleton and check that
/// every simulation vertex indexes a joint that survived the mapping.
fn map_simulation_joints(
    reference_skin: &SkinnedMesh,
    skeleton: &CharacterSkeleton,
    names: &Query<&Name>,
    asset: &ClothFabricAsset,
) -> Result<Vec<Entity>, ClothInstanceError> {
    let mut mapped_joints = Vec::with_capacity(reference_skin.joints.len());
    for joint in &reference_skin.joints {
        let Ok(name) = names.get(*joint) else {
            return Err(ClothInstanceError::MissingJoint(Arc::from("<unnamed>")));
        };
        let Some(mapped) = skeleton.joint_by_name(name.as_str()) else {
            return Err(ClothInstanceError::MissingJoint(Arc::from(name.as_str())));
        };
        mapped_joints.push(mapped);
    }
    for (vertex, simulation_vertex) in asset.fabric().mesh.vertices.iter().enumerate() {
        if let Some(joint) = simulation_vertex
            .joint_indices
            .iter()
            .copied()
            .find(|joint| *joint as usize >= mapped_joints.len())
        {
            return Err(ClothInstanceError::InvalidSimulationJoint {
                vertex,
                joint,
                joints: mapped_joints.len(),
            });
        }
    }
    Ok(mapped_joints)
}

/// One skinned descendant that could carry a cloth render mapping, as
/// `(entity, source mesh, vertex count, name)`.
type RenderMeshCandidate = (Entity, Handle<Mesh>, usize, Arc<str>);

/// Collect the skinned descendants that still have a mesh, ordered by name so
/// mapping ranges are assigned deterministically.
fn render_mesh_candidates(
    skinned_entities: &[Entity],
    mesh_handles: &Query<&Mesh3d>,
    names: &Query<&Name>,
    meshes: &Assets<Mesh>,
) -> Vec<RenderMeshCandidate> {
    let mut candidates = skinned_entities
        .iter()
        .filter_map(|entity| {
            let mesh_handle = mesh_handles.get(*entity).ok()?;
            let mesh = meshes.get(&mesh_handle.0)?;
            let name: Arc<str> = names
                .get(*entity)
                .map_or_else(|_| Arc::from(""), |name| Arc::from(name.as_str()));
            Some((*entity, mesh_handle.0.clone(), mesh.count_vertices(), name))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by(|left, right| {
        left.3
            .cmp(&right.3)
            .then_with(|| left.0.to_bits().cmp(&right.0.to_bits()))
    });
    candidates
}

/// Give each matching candidate a private copy of its mesh and record how the
/// simulation feeds it.
fn bind_render_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    render_mapping: &ClothRenderMapping,
    candidates: Vec<RenderMeshCandidate>,
) -> Vec<BoundRenderMesh> {
    let mut bound_meshes = Vec::with_capacity(candidates.len());
    let mut used_ranges = Vec::new();
    for (entity, source_handle, vertex_count, _) in candidates {
        let mapping = match render_mapping {
            ClothRenderMapping::Direct { particle_indices }
                if particle_indices.len() == vertex_count =>
            {
                Some(BoundRenderMapping::Direct)
            }
            ClothRenderMapping::Barycentric { ranges, .. } => ranges
                .iter()
                .enumerate()
                .find(|(index, range)| {
                    range.vertex_count as usize == vertex_count && !used_ranges.contains(index)
                })
                .map(|(index, range)| {
                    used_ranges.push(index);
                    BoundRenderMapping::Barycentric {
                        entries: range.first_vertex as usize
                            ..range.first_vertex as usize + range.vertex_count as usize,
                    }
                }),
            ClothRenderMapping::Direct { .. } => None,
        };
        let Some(mapping) = mapping else {
            continue;
        };
        let Some(source) = meshes.get(&source_handle).cloned() else {
            continue;
        };
        let tangent_data = bound_tangent_data(&source, vertex_count);
        let source_bounds = source.compute_aabb();
        let private_mesh = meshes.add(source);
        let mut entity_commands = commands.entity(entity);
        entity_commands
            .insert(Mesh3d(private_mesh.clone()))
            .remove::<SkinnedMesh>();
        if let Some(bounds) = source_bounds {
            entity_commands.insert(bounds);
        }
        bound_meshes.push(BoundRenderMesh {
            entity,
            mesh: private_mesh,
            mapping,
            mapped_positions: vec![Vec3::ZERO; vertex_count].into_boxed_slice(),
            mapped_normals: vec![Vec3::Z; vertex_count].into_boxed_slice(),
            tangent_data,
        });
    }
    bound_meshes
}

// Bevy system: `Res` is an owned parameter wrapper, so borrowing these would
// stop this function satisfying `IntoSystem` and it could not be registered.
#[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)]
fn simulate_cloth(
    time: Res<Time>,
    world_parameters: Res<ClothWorldParameters>,
    fabrics: Res<Assets<ClothFabricAsset>>,
    materials: Res<Assets<ClothMaterialAsset>>,
    inverse_bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    physics_world: Option<Res<PhysicsWorld>>,
    mut instances: Query<(
        Entity,
        &ClothCharacterAttachment,
        &CharacterAttachmentNode,
        &GlobalTransform,
        &mut ClothInstance,
    )>,
    joint_transforms: Query<&GlobalTransform>,
    physics_scenes: Query<&PhysicsSceneId>,
    collision_nodes: Query<(
        Entity,
        &CharacterAttachmentNode,
        &CharacterClothCollision,
        &GlobalTransform,
    )>,
) {
    let frame_delta = time.delta_secs();
    instances
        .par_iter_mut()
        .for_each(|(entity, attachment, node, cloth_global, mut instance)| {
            if !instance.binding_ready {
                return;
            }
            let Some(asset) = fabrics.get(&instance.fabric) else {
                return;
            };
            let Some(inverse_handle) = instance.inverse_bindposes.as_ref() else {
                return;
            };
            let Some(inverse_bindposes) = inverse_bindposes.get(inverse_handle) else {
                return;
            };
            match (&asset.fabric().material, &instance.material) {
                (ClothMaterialBinding::Asset(_), Some(material_handle)) => {
                    if let Some(material) = materials.get(material_handle) {
                        instance.solver.set_material(*material.material());
                    }
                }
                (ClothMaterialBinding::Embedded(material), None) => {
                    instance.solver.set_material(*material);
                }
                (ClothMaterialBinding::Asset(_), None) => {}
                (ClothMaterialBinding::Embedded(_), Some(_)) => {
                    unreachable!("embedded cloth material cannot have an external material handle");
                }
            }

            {
                let ClothInstance {
                    joints, targets, ..
                } = &mut *instance;
                if !calculate_particle_targets(
                    &asset.fabric().mesh.vertices,
                    joints,
                    inverse_bindposes,
                    cloth_global,
                    &joint_transforms,
                    targets,
                ) {
                    return;
                }
            }
            {
                let ClothInstance {
                    collider_history,
                    collider_entities,
                    colliders,
                    ..
                } = &mut *instance;
                collect_colliders(
                    entity,
                    node.character,
                    attachment.parameters.collision_layer_mask,
                    cloth_global,
                    &collision_nodes,
                    collider_history,
                    collider_entities,
                    colliders,
                );
            }
            let (_, root_rotation, root_translation) = cloth_global.to_scale_rotation_translation();
            let root = Transform::from_rotation(root_rotation).with_translation(root_translation);
            let physics_scene = physics_scenes
                .get(node.character)
                .or_else(|_| physics_scenes.get(entity))
                .copied()
                .unwrap_or_default();
            let world_gravity = physics_world
                .as_deref()
                .and_then(|world| world.scene(physics_scene).ok())
                .map_or(world_parameters.gravity, az_physics::PhysicsScene::gravity);
            let gravity = root_rotation.inverse() * world_gravity;
            let wind =
                root_rotation.inverse() * world_parameters.wind + attachment.parameters.local_wind;
            let result = {
                let ClothInstance {
                    solver,
                    targets,
                    colliders,
                    ..
                } = &mut *instance;
                let frame = ClothSimulationFrame {
                    particle_targets: targets,
                    colliders,
                    root,
                    gravity,
                    local_wind: wind,
                    max_simulation_distance: attachment.parameters.max_simulation_distance,
                };
                solver.advance(frame_delta, frame)
            };
            instance.render_dirty |= !matches!(
                result,
                ClothAdvanceResult::Simulated { substeps: 0 }
                    | ClothAdvanceResult::WaitingForTargets
            );
        });
}

// Bevy system: `Res` is an owned parameter wrapper, so borrowing it here would
// stop this function satisfying `IntoSystem` and it could not be registered.
#[allow(clippy::needless_pass_by_value)]
fn upload_cloth_meshes(
    fabrics: Res<Assets<ClothFabricAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut instances: Query<(&GlobalTransform, &mut ClothInstance)>,
    render_transforms: Query<&GlobalTransform>,
    mut bounds: Query<&mut Aabb>,
) {
    for (cloth_global, mut instance) in &mut instances {
        if !instance.render_dirty {
            continue;
        }
        let Some(asset) = fabrics.get(&instance.fabric) else {
            continue;
        };
        update_render_meshes(
            asset,
            cloth_global,
            &mut instance,
            &render_transforms,
            &mut meshes,
            &mut bounds,
        );
        instance.render_dirty = false;
    }
}

fn calculate_particle_targets(
    vertices: &[ClothSimulationVertex],
    joints: &[Entity],
    inverse_bindposes: &[Mat4],
    cloth_global: &GlobalTransform,
    joint_transforms: &Query<&GlobalTransform>,
    targets: &mut [ClothParticleTarget],
) -> bool {
    if vertices.len() != targets.len() || joints.len() > inverse_bindposes.len() {
        return false;
    }
    let cloth_inverse = cloth_global.affine().inverse();
    for (index, vertex) in vertices.iter().enumerate() {
        let bind_normal = vertex.tangent_frame * Vec3::Z;
        let mut position = Vec3::ZERO;
        let mut normal = Vec3::ZERO;
        let mut accumulated_weight = 0.0;
        for (&joint_index, &joint_weight) in vertex.joint_indices.iter().zip(&vertex.joint_weights)
        {
            if joint_weight == 0 {
                continue;
            }
            let joint_index = joint_index as usize;
            let Some(&joint) = joints.get(joint_index) else {
                return false;
            };
            let Ok(joint_global) = joint_transforms.get(joint) else {
                return false;
            };
            let Some(inverse_bindpose) = inverse_bindposes.get(joint_index) else {
                return false;
            };
            let skin =
                cloth_inverse * joint_global.affine() * Affine3A::from_mat4(*inverse_bindpose);
            let weight = f32::from(joint_weight) / 255.0;
            position += skin.transform_point3(vertex.position) * weight;
            normal += skin.transform_vector3(bind_normal) * weight;
            accumulated_weight += weight;
        }
        if accumulated_weight <= 0.0 || !position.is_finite() || !normal.is_finite() {
            return false;
        }
        targets[index] = ClothParticleTarget::new(position, normal);
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn collect_colliders(
    cloth_entity: Entity,
    character: Entity,
    collision_layer_mask: u32,
    cloth_global: &GlobalTransform,
    collision_nodes: &Query<(
        Entity,
        &CharacterAttachmentNode,
        &CharacterClothCollision,
        &GlobalTransform,
    )>,
    history: &mut Vec<(Entity, Transform)>,
    collider_entities: &mut Vec<Entity>,
    output: &mut Vec<ClothCapsuleCollider>,
) {
    output.clear();
    collider_entities.clear();
    let cloth_inverse = cloth_global.affine().inverse();
    for (entity, node, collision, collision_global) in collision_nodes {
        if entity == cloth_entity || node.character != character {
            continue;
        }
        let layer_bit = 1_u32.checked_shl(collision.0.collision_layer).unwrap_or(0);
        if collision_layer_mask & layer_bit == 0 {
            continue;
        }
        let local = cloth_inverse * collision_global.affine();
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        let current = Transform {
            translation,
            rotation,
            scale,
        };
        let previous = history
            .iter()
            .find_map(|(candidate, previous)| (*candidate == entity).then_some(*previous))
            .unwrap_or(current);
        output.push(ClothCapsuleCollider::from_proxy_parameters(
            collision.0.parameters,
            current,
            previous,
        ));
        collider_entities.push(entity);
    }
    history.clear();
    history.extend(
        collider_entities
            .iter()
            .copied()
            .zip(output.iter().map(|collider| collider.current)),
    );
}

fn update_render_meshes(
    asset: &ClothFabricAsset,
    cloth_global: &GlobalTransform,
    instance: &mut ClothInstance,
    render_transforms: &Query<&GlobalTransform>,
    meshes: &mut Assets<Mesh>,
    bounds: &mut Query<&mut Aabb>,
) {
    let ClothInstance {
        solver,
        render_meshes,
        ..
    } = instance;
    let particle_positions = solver.positions();
    let particle_normals = solver.normals();
    for bound in render_meshes {
        let Ok(mesh_global) = render_transforms.get(bound.entity) else {
            continue;
        };
        let cloth_to_mesh = mesh_global.affine().inverse() * cloth_global.affine();
        let normal_matrix = cloth_to_mesh.matrix3.inverse().transpose();
        let mapped_count = render_mapping_len(asset, &bound.mapping);
        if bound.mapped_positions.len() != mapped_count
            || bound.mapped_normals.len() != mapped_count
        {
            continue;
        }
        for index in 0..mapped_count {
            let Some((position, normal)) = map_render_vertex(
                asset,
                particle_positions,
                particle_normals,
                &bound.mapping,
                index,
            ) else {
                break;
            };
            bound.mapped_positions[index] = cloth_to_mesh.transform_point3(position);
            bound.mapped_normals[index] = (normal_matrix * normal).normalize_or(Vec3::Z);
        }
        update_tangent_accumulator(bound);
        let Some(mut mesh) = meshes.get_mut(&bound.mesh) else {
            continue;
        };
        if let Some(VertexAttributeValues::Float32x3(values)) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
            && values.len() == mapped_count
        {
            for (destination, source) in values.iter_mut().zip(&bound.mapped_positions) {
                *destination = source.to_array();
            }
        }
        if let Some(VertexAttributeValues::Float32x3(values)) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_NORMAL)
            && values.len() == mapped_count
        {
            for (destination, source) in values.iter_mut().zip(&bound.mapped_normals) {
                *destination = source.to_array();
            }
        }
        if let (Some(tangent_data), Some(VertexAttributeValues::Float32x4(values))) = (
            bound.tangent_data.as_ref(),
            mesh.attribute_mut(Mesh::ATTRIBUTE_TANGENT),
        ) && values.len() == mapped_count
        {
            for (((destination, tangent), normal), handedness) in values
                .iter_mut()
                .zip(&tangent_data.accumulated)
                .zip(&bound.mapped_normals)
                .zip(&tangent_data.handedness)
            {
                let tangent = (*tangent - *normal * tangent.dot(*normal))
                    .normalize_or(orthogonal_tangent(*normal));
                *destination = tangent.extend(*handedness).to_array();
            }
        }
        if let Ok(mut aabb) = bounds.get_mut(bound.entity)
            && let Some(updated) = Aabb::enclosing(bound.mapped_positions.iter())
        {
            *aabb = updated;
        }
    }
}

fn bound_tangent_data(mesh: &Mesh, vertex_count: usize) -> Option<BoundTangentData> {
    let VertexAttributeValues::Float32x2(uvs) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)? else {
        return None;
    };
    let VertexAttributeValues::Float32x4(tangents) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT)?
    else {
        return None;
    };
    if uvs.len() != vertex_count || tangents.len() != vertex_count {
        return None;
    }
    let indices = match mesh.indices() {
        Some(indices) => indices
            .iter()
            .map(u32::try_from)
            .collect::<Result<Vec<_>, _>>()
            .ok()?,
        None => (0..vertex_count)
            .map(u32::try_from)
            .collect::<Result<Vec<_>, _>>()
            .ok()?,
    };
    if indices.len() % 3 != 0 {
        return None;
    }
    Some(BoundTangentData {
        uvs: uvs.iter().copied().map(Vec2::from_array).collect(),
        indices: indices.into_boxed_slice(),
        handedness: tangents.iter().map(|tangent| tangent[3]).collect(),
        accumulated: vec![Vec3::ZERO; vertex_count].into_boxed_slice(),
    })
}

fn update_tangent_accumulator(bound: &mut BoundRenderMesh) {
    let BoundRenderMesh {
        mapped_positions,
        tangent_data: Some(tangent_data),
        ..
    } = bound
    else {
        return;
    };
    tangent_data.accumulated.fill(Vec3::ZERO);
    for triangle in tangent_data.indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let (Some(&pa), Some(&pb), Some(&pc), Some(&uva), Some(&uvb), Some(&uvc)) = (
            mapped_positions.get(a),
            mapped_positions.get(b),
            mapped_positions.get(c),
            tangent_data.uvs.get(a),
            tangent_data.uvs.get(b),
            tangent_data.uvs.get(c),
        ) else {
            continue;
        };
        let first_edge = pb - pa;
        let second_edge = pc - pa;
        let first_uv_delta = uvb - uva;
        let second_uv_delta = uvc - uva;
        let determinant = first_uv_delta
            .y
            .mul_add(-second_uv_delta.x, first_uv_delta.x * second_uv_delta.y);
        if determinant.abs() <= f32::EPSILON {
            continue;
        }
        let tangent =
            (first_edge * second_uv_delta.y - second_edge * first_uv_delta.y) / determinant;
        for vertex in [a, b, c] {
            if let Some(accumulated) = tangent_data.accumulated.get_mut(vertex) {
                *accumulated += tangent;
            }
        }
    }
}

#[inline]
fn orthogonal_tangent(normal: Vec3) -> Vec3 {
    let axis = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    normal.cross(axis).normalize_or(Vec3::Z)
}

fn render_mapping_len(asset: &ClothFabricAsset, mapping: &BoundRenderMapping) -> usize {
    match (&asset.fabric().mesh.render_mapping, mapping) {
        (ClothRenderMapping::Direct { particle_indices }, BoundRenderMapping::Direct) => {
            particle_indices.len()
        }
        (ClothRenderMapping::Barycentric { .. }, BoundRenderMapping::Barycentric { entries }) => {
            entries.len()
        }
        _ => 0,
    }
}

fn map_render_vertex(
    asset: &ClothFabricAsset,
    positions: &[Vec3],
    normals: &[Vec3],
    mapping: &BoundRenderMapping,
    vertex: usize,
) -> Option<(Vec3, Vec3)> {
    match (&asset.fabric().mesh.render_mapping, mapping) {
        (ClothRenderMapping::Direct { particle_indices }, BoundRenderMapping::Direct) => {
            let particle = *particle_indices.get(vertex)? as usize;
            Some((*positions.get(particle)?, *normals.get(particle)?))
        }
        (
            ClothRenderMapping::Barycentric { entries, .. },
            BoundRenderMapping::Barycentric { entries: range },
        ) => {
            let entry = entries.get(range.start.checked_add(vertex)?)?;
            let triangle = entry.triangle as usize * 3;
            let indices = asset.fabric().mesh.indices.get(triangle..triangle + 3)?;
            let [a, b, c] = [
                indices[0] as usize,
                indices[1] as usize,
                indices[2] as usize,
            ];
            let normal = (*normals.get(a)? * entry.barycentric.x
                + *normals.get(b)? * entry.barycentric.y
                + *normals.get(c)? * entry.barycentric.z)
                .normalize_or(Vec3::Z);
            let position = *positions.get(a)? * entry.barycentric.x
                + *positions.get(b)? * entry.barycentric.y
                + *positions.get(c)? * entry.barycentric.z
                + normal * entry.height;
            Some((position, normal))
        }
        _ => None,
    }
}

fn invalidate_changed_fabrics(
    mut commands: Commands,
    mut events: MessageReader<AssetEvent<ClothFabricAsset>>,
    mut cache: ResMut<SharedFabricCache>,
    instances: Query<(Entity, &ClothInstance)>,
) {
    for event in events.read() {
        let changed = match event {
            AssetEvent::Modified { id }
            | AssetEvent::Removed { id }
            | AssetEvent::Unused { id } => Some(*id),
            AssetEvent::Added { .. } | AssetEvent::LoadedWithDependencies { .. } => None,
        };
        let Some(changed) = changed else {
            continue;
        };
        for (entity, instance) in &instances {
            if instance.fabric.id() != changed {
                continue;
            }
            cache.remove(instance.fabric_asset_id);
            commands.entity(instance.render_root).try_despawn();
            commands
                .entity(entity)
                .insert(PendingClothInstance {
                    fabric: instance.fabric.clone(),
                    material: instance.material.clone(),
                })
                .remove::<ClothInstance>();
        }
    }
}
