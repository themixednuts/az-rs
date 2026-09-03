use std::sync::Arc;

use az_animation::{
    character::{
        attachment::AttachmentVisibility,
        definition::{
            AttachmentBinding, AttachmentFlags, AttachmentMaterials, CharacterAttachment,
            CharacterAttachmentKind, CharacterDefinitionAsset, ClothAttachment,
            ClothCollisionAttachment, PendulumRowAttachment, ProxyAttachment, SocketSimulation,
        },
    },
    controller_target::AnimationControllerNodeExtras,
};
use az_core::{AssetId, crc::Crc32};
use az_framework::asset::{AssetCatalog, AssetRefLoadError};
use az_mesh::{MESH_ASSET_TYPE, SKINNED_MESH_ASSET_TYPE};
use bevy::{
    animation::{AnimatedBy, graph::AnimationGraph},
    asset::{AssetServer, Assets, Handle},
    ecs::hierarchy::{ChildOf, Children},
    gltf::{GltfAssetLabel, GltfExtras},
    math::Affine3A,
    mesh::skinning::SkinnedMesh,
    prelude::*,
    transform::commands::BuildChildrenTransformExt,
    world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady},
};

use crate::{
    CryAnimationPlayerBundle, controller_animation_target_id, controller_animation_target_root_id,
};

/// Selects one cooked character definition for an ECS entity.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CharacterDefinitionId(AssetId);

impl CharacterDefinitionId {
    #[must_use]
    pub const fn new(asset_id: AssetId) -> Self {
        Self(asset_id)
    }

    #[must_use]
    pub const fn asset_id(self) -> AssetId {
        self.0
    }
}

impl From<AssetId> for CharacterDefinitionId {
    fn from(asset_id: AssetId) -> Self {
        Self::new(asset_id)
    }
}

impl AsRef<AssetId> for CharacterDefinitionId {
    fn as_ref(&self) -> &AssetId {
        &self.0
    }
}

#[derive(Component, Debug, Clone)]
struct PendingCharacterDefinition(Handle<CharacterDefinitionAsset>);

/// Loaded character state retained on the caller-owned entity.
#[derive(Component, Debug, Clone)]
pub struct CharacterInstance {
    definition: Handle<CharacterDefinitionAsset>,
    attachments: Vec<Entity>,
}

impl CharacterInstance {
    #[must_use]
    pub const fn definition(&self) -> &Handle<CharacterDefinitionAsset> {
        &self.definition
    }

    #[must_use]
    pub fn attachments(&self) -> &[Entity] {
        &self.attachments
    }
}

/// Terminal resolution failure for a character or one of its model products.
#[derive(Component, Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CharacterInstanceError {
    #[error("character definition: {0}")]
    Definition(AssetRefLoadError),
    #[error("character model: {0}")]
    Model(AssetRefLoadError),
    #[error("attachment {attachment} model: {source}")]
    AttachmentModel {
        attachment: Arc<str>,
        #[source]
        source: AssetRefLoadError,
    },
    #[error("character model contains no skinned skeleton")]
    MissingSkeleton,
    #[error("attachment {attachment} references missing joint {joint}")]
    MissingJoint {
        attachment: Arc<str>,
        joint: Arc<str>,
    },
    #[error("skinned attachment {attachment} references missing base joint {joint}")]
    MissingSkinnedJoint {
        attachment: Arc<str>,
        joint: Arc<str>,
    },
    #[error("attachment {attachment} has a binding incompatible with kind {kind}")]
    BindingKindMismatch {
        attachment: Arc<str>,
        kind: &'static str,
    },
}

/// Stable attachment index within one character definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CharacterAttachmentId(u32);

impl CharacterAttachmentId {
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Runtime socket shared by every attachment kind.
#[derive(Component, Debug, Clone)]
pub struct CharacterAttachmentNode {
    pub character: Entity,
    pub id: CharacterAttachmentId,
    pub name: Option<Arc<str>>,
    pub name_crc: Crc32,
    pub flags: AttachmentFlags,
    pub visibility: AttachmentVisibility,
}

/// Static-model request consumed by the engine's static mesh renderer.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct StaticCharacterAttachment {
    pub asset_id: AssetId,
    pub material: Option<AssetId>,
}

/// Skinned-model instance that must use its owning character's skeleton.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkinnedCharacterAttachment {
    pub character: Entity,
    pub id: CharacterAttachmentId,
}

/// Cloth product and instance parameters handed to the cloth gem.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct ClothCharacterAttachment {
    pub asset_id: AssetId,
    pub materials: AttachmentMaterials<AssetId>,
    pub parameters: ClothAttachment,
}

#[derive(Component, Debug, Clone, PartialEq)]
pub struct CharacterSocketSimulation(pub SocketSimulation);

#[derive(Component, Debug, Clone, PartialEq)]
pub struct CharacterPendulumRow {
    pub attachment: PendulumRowAttachment,
}

#[derive(Component, Debug, Clone, PartialEq)]
pub struct CharacterAttachmentProxy(pub ProxyAttachment);

#[derive(Component, Debug, Clone, PartialEq)]
pub struct CharacterClothCollision(pub ClothCollisionAttachment);

/// Sorted, allocation-stable skeleton lookup for attachment and skin binding.
#[derive(Component, Debug, Clone)]
pub struct CharacterSkeleton {
    animation_root: Entity,
    joints: Box<[(Name, Entity)]>,
    controllers: Box<[(u32, Entity)]>,
}

impl CharacterSkeleton {
    #[must_use]
    pub const fn animation_root(&self) -> Entity {
        self.animation_root
    }

    #[must_use]
    pub fn joint_by_name(&self, name: &str) -> Option<Entity> {
        self.joints
            .binary_search_by(|(candidate, _)| candidate.as_str().cmp(name))
            .ok()
            .map(|index| self.joints[index].1)
    }

    #[must_use]
    pub fn joints(&self) -> &[(Name, Entity)] {
        &self.joints
    }

    #[must_use]
    pub fn joint_by_controller_id(&self, controller_id: u32) -> Option<Entity> {
        self.controllers
            .binary_search_by_key(&controller_id, |(candidate, _)| *candidate)
            .ok()
            .map(|index| self.controllers[index].1)
    }
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "lib.rs re-exports this module with `pub use`, so `pub(crate)` is what actually keeps the registration hook out of the crate API"
)]
pub(crate) fn register_character_runtime(app: &mut App) {
    app.add_systems(
        Update,
        (resolve_character_definitions, spawn_resolved_characters).chain(),
    )
    .add_observer(assemble_character_instance)
    .add_observer(bind_skinned_character_attachment);
}

#[expect(
    clippy::type_complexity,
    reason = "a Bevy query filter tuple has no shorter spelling"
)]
fn resolve_character_definitions(
    mut commands: Commands,
    catalog: Option<Res<AssetCatalog>>,
    asset_server: Option<Res<AssetServer>>,
    characters: Query<
        (Entity, &CharacterDefinitionId),
        (
            Without<PendingCharacterDefinition>,
            Without<CharacterInstance>,
            Without<CharacterInstanceError>,
        ),
    >,
) {
    let (Some(catalog), Some(asset_server)) = (catalog, asset_server) else {
        return;
    };

    for (entity, definition) in &characters {
        match catalog.load_asset_id::<CharacterDefinitionAsset>(
            definition.asset_id(),
            az_animation::ids::CHARACTER_DEFINITION,
            &asset_server,
        ) {
            Ok(handle) => {
                commands
                    .entity(entity)
                    .insert(PendingCharacterDefinition(handle));
            }
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(CharacterInstanceError::Definition(error));
            }
        }
    }
}

fn spawn_resolved_characters(
    mut commands: Commands,
    catalog: Option<Res<AssetCatalog>>,
    asset_server: Option<Res<AssetServer>>,
    definitions: Option<Res<Assets<CharacterDefinitionAsset>>>,
    pending: Query<(Entity, &PendingCharacterDefinition)>,
) {
    let (Some(catalog), Some(asset_server), Some(definitions)) =
        (catalog, asset_server, definitions)
    else {
        return;
    };

    for (entity, pending) in &pending {
        let Some(asset) = definitions.get(&pending.0) else {
            continue;
        };
        let model = asset.definition().model;
        let entry = match catalog.resolve_asset_id(model, SKINNED_MESH_ASSET_TYPE) {
            Ok(entry) => entry,
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(CharacterInstanceError::Model(error));
                commands
                    .entity(entity)
                    .remove::<PendingCharacterDefinition>();
                continue;
            }
        };
        let scene_path = GltfAssetLabel::Scene(0).from_asset(entry.relative_path().to_path_buf());
        let scene: Handle<WorldAsset> = asset_server.load(scene_path);
        commands.entity(entity).insert((
            WorldAssetRoot(scene),
            CharacterInstance {
                definition: pending.0.clone(),
                attachments: Vec::with_capacity(asset.definition().attachments.len()),
            },
        ));
        commands
            .entity(entity)
            .remove::<PendingCharacterDefinition>();
    }
}

#[allow(clippy::too_many_arguments)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer parameters are taken by value"
)]
fn assemble_character_instance(
    trigger: On<WorldInstanceReady>,
    mut commands: Commands,
    mut characters: Query<&mut CharacterInstance>,
    definitions: Res<Assets<CharacterDefinitionAsset>>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    extras: Query<&GltfExtras>,
    transforms: Query<&Transform>,
    skinned_meshes: Query<&SkinnedMesh>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    asset_server: Res<AssetServer>,
    catalog: Res<AssetCatalog>,
) {
    let Ok(mut instance) = characters.get_mut(trigger.entity) else {
        return;
    };
    let Some(asset) = definitions.get(&instance.definition) else {
        return;
    };

    for entity in instance.attachments.drain(..) {
        commands.entity(entity).try_despawn();
    }

    let Some(skeleton) = build_character_skeleton(
        trigger.entity,
        &children,
        &parents,
        &names,
        &extras,
        &skinned_meshes,
    ) else {
        commands
            .entity(trigger.entity)
            .insert(CharacterInstanceError::MissingSkeleton);
        return;
    };

    install_animation_targets(&mut commands, &skeleton);
    commands
        .entity(skeleton.animation_root)
        .insert(CryAnimationPlayerBundle::new(&mut graphs));

    let definition = asset.definition();
    for attachment in &definition.attachments {
        if let Err(error) = validate_character_attachment(attachment, &skeleton, &catalog) {
            commands.entity(trigger.entity).insert(error);
            return;
        }
    }
    for (index, attachment) in definition.attachments.iter().enumerate() {
        let Ok(index) = u32::try_from(index) else {
            break;
        };
        match spawn_character_attachment(
            &mut commands,
            trigger.entity,
            CharacterAttachmentId::new(index),
            attachment,
            &skeleton,
            &parents,
            &transforms,
            &asset_server,
            &catalog,
        ) {
            Ok(entity) => instance.attachments.push(entity),
            Err(error) => {
                commands.entity(trigger.entity).insert(error);
                return;
            }
        }
    }
    commands.entity(trigger.entity).insert(skeleton);
}

fn build_character_skeleton(
    character: Entity,
    children: &Query<&Children>,
    parents: &Query<&ChildOf>,
    names: &Query<&Name>,
    extras: &Query<&GltfExtras>,
    skinned_meshes: &Query<&SkinnedMesh>,
) -> Option<CharacterSkeleton> {
    let skinned_mesh = children
        .iter_descendants(character)
        .filter_map(|entity| skinned_meshes.get(entity).ok())
        .max_by_key(|skinned_mesh| skinned_mesh.joints.len())?;
    let animation_root = skinned_mesh.joints.iter().copied().find(|joint| {
        parents.get(*joint).map_or(true, |parent| {
            !skinned_mesh.joints.contains(&parent.parent())
        })
    })?;
    let mut joints = skinned_mesh
        .joints
        .iter()
        .filter_map(|joint| names.get(*joint).ok().cloned().map(|name| (name, *joint)))
        .collect::<Vec<_>>();
    joints.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    let mut controllers = skinned_mesh
        .joints
        .iter()
        .map(|joint| {
            let extras = extras.get(*joint).ok()?;
            let extras =
                serde_json::from_str::<AnimationControllerNodeExtras>(&extras.value).ok()?;
            Some((extras.azoth_animation_controller_id, *joint))
        })
        .collect::<Option<Vec<_>>>()?;
    controllers.sort_unstable_by_key(|(controller_id, _)| *controller_id);
    if controllers.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    Some(CharacterSkeleton {
        animation_root,
        joints: joints.into_boxed_slice(),
        controllers: controllers.into_boxed_slice(),
    })
}

fn validate_character_attachment(
    attachment: &CharacterAttachment<AssetId>,
    skeleton: &CharacterSkeleton,
    catalog: &AssetCatalog,
) -> Result<(), CharacterInstanceError> {
    let attachment_name = attachment
        .name
        .clone()
        .unwrap_or_else(|| Arc::from("<unnamed>"));
    let required_joint = match &attachment.kind {
        CharacterAttachmentKind::Bone(value) => Some(value.joint.as_ref()),
        CharacterAttachmentKind::Proxy(value) => Some(value.joint.as_ref()),
        CharacterAttachmentKind::PendulumRow(value) => Some(value.row_joint.as_ref()),
        CharacterAttachmentKind::ClothCollision(value) => Some(value.joint.as_ref()),
        CharacterAttachmentKind::Face(_)
        | CharacterAttachmentKind::Skin(_)
        | CharacterAttachmentKind::Cloth(_) => None,
    };
    if let Some(joint) = required_joint {
        let Some(joint) = joint else {
            return Err(CharacterInstanceError::MissingJoint {
                attachment: attachment_name,
                joint: Arc::from("<unset>"),
            });
        };
        if skeleton.joint_by_name(joint).is_none() {
            return Err(CharacterInstanceError::MissingJoint {
                attachment: attachment_name,
                joint: joint.clone(),
            });
        }
    }

    let expected = match &attachment.binding {
        Some(AttachmentBinding::Character(_)) => Some(az_animation::ids::CHARACTER_DEFINITION),
        Some(AttachmentBinding::StaticMesh(_)) => Some(MESH_ASSET_TYPE),
        Some(AttachmentBinding::SkinnedMesh(_)) => Some(SKINNED_MESH_ASSET_TYPE),
        Some(AttachmentBinding::Cloth(_)) => Some(az_nv_cloth::ids::CLOTH_FABRIC),
        None => None,
    };
    if let (Some(binding), Some(expected)) = (&attachment.binding, expected) {
        catalog
            .resolve_asset_id(*binding.asset(), expected)
            .map_err(|source| CharacterInstanceError::AttachmentModel {
                attachment: attachment_name.clone(),
                source,
            })?;
    }

    match (&attachment.kind, &attachment.binding) {
        (CharacterAttachmentKind::Skin(_), Some(AttachmentBinding::SkinnedMesh(_)))
        | (CharacterAttachmentKind::Cloth(_), Some(AttachmentBinding::Cloth(_)))
        | (CharacterAttachmentKind::Face(_) | CharacterAttachmentKind::Bone(_), _)
        | (
            CharacterAttachmentKind::Proxy(_)
            | CharacterAttachmentKind::PendulumRow(_)
            | CharacterAttachmentKind::ClothCollision(_),
            None,
        ) => Ok(()),
        (kind, _) => Err(CharacterInstanceError::BindingKindMismatch {
            attachment: attachment_name,
            kind: attachment_kind_name(kind),
        }),
    }
}

const fn attachment_kind_name(kind: &CharacterAttachmentKind) -> &'static str {
    match kind {
        CharacterAttachmentKind::Bone(_) => "bone",
        CharacterAttachmentKind::Face(_) => "face",
        CharacterAttachmentKind::Skin(_) => "skin",
        CharacterAttachmentKind::Proxy(_) => "proxy",
        CharacterAttachmentKind::PendulumRow(_) => "pendulum-row",
        CharacterAttachmentKind::Cloth(_) => "cloth",
        CharacterAttachmentKind::ClothCollision(_) => "cloth-collision",
    }
}

fn install_animation_targets(commands: &mut Commands, skeleton: &CharacterSkeleton) {
    for &(controller_id, entity) in skeleton.controllers.as_ref() {
        commands.entity(entity).insert((
            controller_animation_target_id(controller_id),
            AnimatedBy(skeleton.animation_root),
        ));
    }
    commands.spawn((
        Name::new(az_animation::controller_target::CONTROLLER_TARGET_ROOT_NAME),
        controller_animation_target_root_id(),
        AnimatedBy(skeleton.animation_root),
        Transform::IDENTITY,
        ChildOf(skeleton.animation_root),
    ));
}

#[allow(clippy::too_many_arguments)]
fn spawn_character_attachment(
    commands: &mut Commands,
    character: Entity,
    id: CharacterAttachmentId,
    attachment: &CharacterAttachment<AssetId>,
    skeleton: &CharacterSkeleton,
    parents: &Query<&ChildOf>,
    transforms: &Query<&Transform>,
    asset_server: &AssetServer,
    catalog: &AssetCatalog,
) -> Result<Entity, CharacterInstanceError> {
    let (parent, transform) =
        attachment_parent_and_transform(character, attachment, skeleton, parents, transforms)?;
    let name = attachment.name.clone();
    let name_crc = name.as_deref().map_or(Crc32::ZERO, Crc32::from_str_lower);
    let mut hidden = attachment.flags.contains(AttachmentFlags::HIDDEN);
    if let CharacterAttachmentKind::Cloth(cloth) = &attachment.kind {
        hidden |= cloth.hidden;
    }
    let visibility = if hidden {
        AttachmentVisibility::MAIN_PASS
    } else {
        AttachmentVisibility::empty()
    };
    let mut entity = commands.spawn((
        Name::new(
            name.as_deref()
                .map_or_else(|| format!("attachment-{}", id.index()), str::to_owned),
        ),
        CharacterAttachmentNode {
            character,
            id,
            name,
            name_crc,
            flags: attachment.flags,
            visibility,
        },
        transform,
        if hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        },
        ChildOf(parent),
    ));

    match &attachment.kind {
        CharacterAttachmentKind::Bone(bone) => {
            if let Some(simulation) = &bone.simulation {
                entity.insert(CharacterSocketSimulation(simulation.clone()));
            }
        }
        CharacterAttachmentKind::Proxy(proxy) => {
            entity.insert(CharacterAttachmentProxy(proxy.clone()));
        }
        CharacterAttachmentKind::PendulumRow(row) => {
            entity.insert(CharacterPendulumRow {
                attachment: row.clone(),
            });
        }
        CharacterAttachmentKind::ClothCollision(collision) => {
            entity.insert(CharacterClothCollision(collision.clone()));
        }
        CharacterAttachmentKind::Face(_)
        | CharacterAttachmentKind::Skin(_)
        | CharacterAttachmentKind::Cloth(_) => {}
    }

    let socket = entity.id();
    if let Some(binding) = &attachment.binding {
        match binding {
            AttachmentBinding::Character(asset_id) => {
                commands.spawn((
                    Name::new(format!("character-binding-{}", id.index())),
                    CharacterDefinitionId::new(*asset_id),
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    ChildOf(socket),
                ));
            }
            AttachmentBinding::StaticMesh(asset_id) => {
                commands.entity(socket).insert(StaticCharacterAttachment {
                    asset_id: *asset_id,
                    material: attachment.materials.shared,
                });
            }
            AttachmentBinding::SkinnedMesh(asset_id) => {
                spawn_skinned_attachment(
                    commands,
                    socket,
                    character,
                    id,
                    *asset_id,
                    asset_server,
                    catalog,
                )?;
            }
            AttachmentBinding::Cloth(asset_id) => {
                let CharacterAttachmentKind::Cloth(parameters) = &attachment.kind else {
                    return Ok(socket);
                };
                commands.entity(socket).insert(ClothCharacterAttachment {
                    asset_id: *asset_id,
                    materials: attachment.materials.clone(),
                    parameters: *parameters,
                });
            }
        }
    }
    Ok(socket)
}

fn attachment_parent_and_transform(
    character: Entity,
    attachment: &CharacterAttachment<AssetId>,
    skeleton: &CharacterSkeleton,
    parents: &Query<&ChildOf>,
    transforms: &Query<&Transform>,
) -> Result<(Entity, Transform), CharacterInstanceError> {
    let joint = match &attachment.kind {
        CharacterAttachmentKind::Bone(value) => value.joint.as_ref(),
        CharacterAttachmentKind::Proxy(value) => value.joint.as_ref(),
        CharacterAttachmentKind::PendulumRow(value) => value.row_joint.as_ref(),
        CharacterAttachmentKind::ClothCollision(value) => value.joint.as_ref(),
        CharacterAttachmentKind::Face(_)
        | CharacterAttachmentKind::Skin(_)
        | CharacterAttachmentKind::Cloth(_) => None,
    };
    let Some(joint_name) = joint else {
        let transform = match &attachment.kind {
            CharacterAttachmentKind::Face(_) => Transform {
                translation: attachment.absolute.translation,
                rotation: attachment.absolute.rotation,
                scale: Vec3::ONE,
            },
            CharacterAttachmentKind::Skin(_) | CharacterAttachmentKind::Cloth(_) => {
                Transform::IDENTITY
            }
            CharacterAttachmentKind::Bone(_)
            | CharacterAttachmentKind::Proxy(_)
            | CharacterAttachmentKind::PendulumRow(_)
            | CharacterAttachmentKind::ClothCollision(_) => {
                return Err(CharacterInstanceError::MissingJoint {
                    attachment: attachment
                        .name
                        .clone()
                        .unwrap_or_else(|| Arc::from("<unnamed>")),
                    joint: Arc::from("<unset>"),
                });
            }
        };
        return Ok((character, transform));
    };
    let Some(joint_entity) = skeleton.joint_by_name(joint_name) else {
        return Err(CharacterInstanceError::MissingJoint {
            attachment: attachment
                .name
                .clone()
                .unwrap_or_else(|| Arc::from("<unnamed>")),
            joint: joint_name.clone(),
        });
    };
    let joint_model = model_affine(joint_entity, character, parents, transforms);
    let absolute = Affine3A::from_scale_rotation_translation(
        Vec3::ONE,
        attachment.absolute.rotation,
        attachment.absolute.translation,
    );
    let relative = joint_model.inverse() * absolute;
    let (_, mut rotation, mut translation) = relative.to_scale_rotation_translation();
    if let Some(value) = attachment.relative.rotation {
        rotation = value;
    }
    if let Some(value) = attachment.relative.translation {
        translation = value;
    }
    Ok((
        joint_entity,
        Transform {
            translation,
            rotation,
            scale: Vec3::ONE,
        },
    ))
}

fn model_affine(
    entity: Entity,
    character: Entity,
    parents: &Query<&ChildOf>,
    transforms: &Query<&Transform>,
) -> Affine3A {
    let mut chain = Vec::new();
    let mut current = entity;
    while current != character {
        chain.push(current);
        let Ok(parent) = parents.get(current) else {
            break;
        };
        current = parent.parent();
    }
    chain
        .into_iter()
        .rev()
        .filter_map(|entity| transforms.get(entity).ok())
        .fold(Affine3A::IDENTITY, |model, local| {
            model * local.compute_affine()
        })
}

fn spawn_skinned_attachment(
    commands: &mut Commands,
    parent: Entity,
    character: Entity,
    id: CharacterAttachmentId,
    asset_id: AssetId,
    asset_server: &AssetServer,
    catalog: &AssetCatalog,
) -> Result<(), CharacterInstanceError> {
    let entry = catalog
        .resolve_asset_id(asset_id, SKINNED_MESH_ASSET_TYPE)
        .map_err(|source| CharacterInstanceError::AttachmentModel {
            attachment: Arc::from(format!("{}", id.index())),
            source,
        })?;
    let scene_path = GltfAssetLabel::Scene(0).from_asset(entry.relative_path().to_path_buf());
    let scene: Handle<WorldAsset> = asset_server.load(scene_path);
    commands.spawn((
        Name::new(format!("skinned-binding-{}", id.index())),
        WorldAssetRoot(scene),
        SkinnedCharacterAttachment { character, id },
        Transform::IDENTITY,
        Visibility::Inherited,
        ChildOf(parent),
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy observer parameters are taken by value"
)]
fn bind_skinned_character_attachment(
    trigger: On<WorldInstanceReady>,
    mut commands: Commands,
    attachments: Query<&SkinnedCharacterAttachment>,
    skeletons: Query<&CharacterSkeleton>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    mut skinned_meshes: Query<(Entity, &mut SkinnedMesh)>,
    attachment_nodes: Query<&CharacterAttachmentNode>,
) {
    let Ok(attachment) = attachments.get(trigger.entity) else {
        return;
    };
    let Ok(skeleton) = skeletons.get(attachment.character) else {
        return;
    };
    let attachment_name = parents
        .get(trigger.entity)
        .ok()
        .and_then(|parent| attachment_nodes.get(parent.parent()).ok())
        .and_then(|node| node.name.clone())
        .unwrap_or_else(|| Arc::from("<unnamed>"));

    let mesh_entities = children
        .iter_descendants(trigger.entity)
        .filter(|entity| skinned_meshes.get(*entity).is_ok())
        .collect::<Vec<_>>();
    let mut old_joints = Vec::new();
    let mut remapped = Vec::with_capacity(mesh_entities.len());
    for entity in &mesh_entities {
        let Ok((_, mesh)) = skinned_meshes.get(*entity) else {
            continue;
        };
        let mut joints = Vec::with_capacity(mesh.joints.len());
        for joint in &mesh.joints {
            let Ok(name) = names.get(*joint) else {
                continue;
            };
            let Some(base_joint) = skeleton.joint_by_name(name.as_str()) else {
                commands.entity(attachment.character).insert(
                    CharacterInstanceError::MissingSkinnedJoint {
                        attachment: attachment_name,
                        joint: Arc::from(name.as_str()),
                    },
                );
                return;
            };
            joints.push(base_joint);
            old_joints.push(*joint);
        }
        if joints.len() != mesh.joints.len() {
            return;
        }
        remapped.push((*entity, joints));
    }

    for (entity, joints) in remapped {
        if let Ok((_, mut mesh)) = skinned_meshes.get_mut(entity) {
            mesh.joints = joints;
        }
    }

    old_joints.sort_unstable();
    old_joints.dedup();
    for mesh in mesh_entities {
        if ancestor_in_set(mesh, &old_joints, &parents) {
            commands.entity(mesh).set_parent_in_place(trigger.entity);
        }
    }
    for joint in old_joints.iter().copied().filter(|joint| {
        parents.get(*joint).map_or(true, |parent| {
            old_joints.binary_search(&parent.parent()).is_err()
        })
    }) {
        commands.entity(joint).try_despawn();
    }
}

fn ancestor_in_set(entity: Entity, set: &[Entity], parents: &Query<&ChildOf>) -> bool {
    let mut current = entity;
    while let Ok(parent) = parents.get(current) {
        current = parent.parent();
        if set.binary_search(&current).is_ok() {
            return true;
        }
    }
    false
}
