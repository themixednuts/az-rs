//! LmbrCentral/Cry component materialization into backend-neutral physics.

use az_core::EntityId as AzEntityId;
use std::collections::{HashMap, HashSet, VecDeque};

use az_physics::{
    Axis3, BodyDescriptor, BodyKind, ColliderConfiguration, ColliderShape, CollisionFilterRegistry,
    ImpulseAction, PhysicsAction, PhysicsBodyHandle, PhysicsColliderSet, PhysicsEntityId,
    PhysicsInteraction, PhysicsInteractionKind, PhysicsInteractionPhase, PhysicsMaterial,
    PhysicsMaterialRegistry, PhysicsMeshGeometry, PhysicsPose, PhysicsSet, PhysicsShapeInstance,
    PhysicsShapeSet, PhysicsWorld, RigidBodyConfiguration,
};
use bevy::math::bounding::BoundingVolume;
use bevy::prelude::*;

use super::{
    CharacterPhysicsComponent, ForceMode, ForceSpace, ForceVolumeComponent, MassOrDensity,
    MeshColliderComponent, PrimitiveColliderComponent, RigidPhysicsComponent,
    StaticPhysicsComponent,
};
use crate::{
    BoxShapeComponent, CapsuleShapeComponent, CompoundShapeComponent, CylinderShapeComponent,
    PolygonPrismShapeComponent, SphereShapeComponent, TagComponent,
    TriggerAreaActivationEntityType, TriggerAreaComponent, TriggerAreaFilters,
    TriggerAreaRelevanceChanged, TriggerAreaRequest, TriggerAreaRequestKind, TriggerAreasRespawned,
};

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct LmbrCentralPhysicsError(pub String);

/// Stable ordering boundaries for engine and project physics authoring systems.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LmbrCentralAuthoringSet {
    Initialize,
    Cleanup,
    Geometry,
    Shapes,
    Colliders,
    Bodies,
}

/// Runtime enable state shared by the rigid/static request-bus surface.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub struct PhysicsEnabled(pub bool);

impl PhysicsEnabled {
    pub const ENABLED: Self = Self(true);
    pub const DISABLED: Self = Self(false);

    pub const fn enable(&mut self) {
        self.0 = true;
    }

    pub const fn disable(&mut self) {
        self.0 = false;
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct LmbrRigidProduct;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct LmbrStaticProduct;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct LmbrLivingProduct;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct LmbrTriggerAreaProduct;

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LmbrPrimitiveShapeProduct;

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LmbrMeshShapeProduct;

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LmbrCompoundShapeProduct;

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LmbrPolygonPrismShapeProduct;

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LmbrPrimitiveColliderProduct;

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LmbrMeshColliderProduct;

/// Runtime state corresponding to the native trigger area's inside/excluded
/// lists. Counts preserve one logical entrant when compound collider pairs
/// generate more than one solver interaction.
#[derive(Component, Debug, Clone)]
pub struct TriggerAreaState {
    overlaps: HashMap<Entity, u32>,
    occupants: HashSet<Entity>,
    arming: TriggerAreaArming,
    relevant: bool,
    rematerialize: bool,
}

/// The three reachable combinations of the native trigger area's
/// `proximity trigger requested` and `already consumed` flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TriggerAreaArming {
    /// A proximity trigger is requested and the area has not fired yet.
    #[default]
    Armed,
    /// The proximity trigger was withdrawn; the area can still re-arm.
    Idle,
    /// A `trigger once` area already fired and never re-arms.
    Consumed,
}

impl TriggerAreaState {
    #[must_use]
    pub fn contains(&self, entity: Entity) -> bool {
        self.occupants.contains(&entity)
    }

    #[must_use]
    pub const fn proximity_trigger_requested(&self) -> bool {
        matches!(self.arming, TriggerAreaArming::Armed)
    }

    #[must_use]
    pub const fn is_relevant(&self) -> bool {
        self.relevant
    }

    const fn is_consumed(&self) -> bool {
        matches!(self.arming, TriggerAreaArming::Consumed)
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.relevant && matches!(self.arming, TriggerAreaArming::Armed)
    }

    fn clear_entrants(&mut self) {
        self.overlaps.clear();
        self.occupants.clear();
    }

    fn remove_proximity_trigger(&mut self) {
        if matches!(self.arming, TriggerAreaArming::Armed) {
            self.arming = TriggerAreaArming::Idle;
        }
        self.clear_entrants();
    }

    const fn add_proximity_trigger(&mut self) {
        if matches!(self.arming, TriggerAreaArming::Idle) {
            self.arming = TriggerAreaArming::Armed;
        }
    }

    fn set_relevant(&mut self, relevant: bool) {
        self.relevant = relevant;
        if !relevant {
            self.clear_entrants();
        }
    }

    fn cycle_for_respawn(&mut self) {
        self.clear_entrants();
        self.rematerialize = true;
    }

    const fn consume_once(&mut self) {
        self.arming = TriggerAreaArming::Consumed;
    }

    fn take_rematerialize(&mut self) -> bool {
        std::mem::take(&mut self.rematerialize)
    }
}

impl Default for TriggerAreaState {
    fn default() -> Self {
        Self {
            overlaps: HashMap::new(),
            occupants: HashSet::new(),
            arming: TriggerAreaArming::Armed,
            relevant: true,
            rematerialize: false,
        }
    }
}

/// Typed replacement for `TriggerAreaNotificationBus` enter/exit callbacks.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerAreaEvent {
    pub area: Entity,
    pub entrant: Entity,
    pub phase: PhysicsInteractionPhase,
}

/// Marks the client-owned player used by `only_active_for_local_player`.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalTriggerEntity;

/// Bounded collision log corresponding to Cry's `maxLoggedCollisions`.
#[derive(Component, Debug, Clone, Default)]
pub struct RecordedPhysicsCollisions {
    capacity: usize,
    entries: VecDeque<PhysicsInteraction>,
}

impl RecordedPhysicsCollisions {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        }
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &PhysicsInteraction> + ExactSizeIterator {
        self.entries.iter()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn push(&mut self, interaction: PhysicsInteraction) {
        if self.capacity == 0 {
            return;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(interaction);
    }
}

pub fn configure(app: &mut App) {
    app.init_resource::<PhysicsMaterialRegistry>()
        .init_resource::<CollisionFilterRegistry>()
        .register_type::<PhysicsEnabled>()
        .register_type::<PhysicsMeshGeometry>()
        .register_type::<PhysicsShapeSet>()
        .add_message::<TriggerAreaRequest>()
        .add_message::<TriggerAreaRelevanceChanged>()
        .add_message::<TriggerAreasRespawned>()
        .add_message::<TriggerAreaEvent>()
        .add_message::<PhysicsInteraction>()
        .configure_sets(
            Update,
            (
                LmbrCentralAuthoringSet::Initialize,
                LmbrCentralAuthoringSet::Cleanup,
                LmbrCentralAuthoringSet::Geometry,
                LmbrCentralAuthoringSet::Shapes,
                LmbrCentralAuthoringSet::Colliders,
                LmbrCentralAuthoringSet::Bodies,
            )
                .chain()
                .in_set(PhysicsSet::Authoring),
        )
        .add_systems(
            Update,
            (
                initialize_rigid_enabled,
                initialize_static_enabled,
                initialize_trigger_area_state,
                apply_trigger_area_requests,
                apply_trigger_area_relevance,
                reset_trigger_areas_on_respawn,
                configure_collision_recording,
            )
                .chain()
                .in_set(LmbrCentralAuthoringSet::Initialize),
        )
        .add_systems(
            Update,
            (
                cleanup_removed_primitive_shapes,
                cleanup_removed_polygon_prism_shapes,
                cleanup_removed_compound_shapes,
                cleanup_removed_mesh_geometry,
                cleanup_removed_primitive_colliders,
                cleanup_removed_mesh_colliders,
            )
                .chain()
                .in_set(LmbrCentralAuthoringSet::Cleanup),
        )
        .add_systems(
            Update,
            (
                materialize_primitive_shapes,
                materialize_polygon_prism_shapes,
                materialize_mesh_shapes,
                materialize_compound_shapes,
            )
                .chain()
                .in_set(LmbrCentralAuthoringSet::Shapes),
        )
        .add_systems(
            Update,
            (materialize_primitive_colliders, materialize_mesh_colliders)
                .chain()
                .in_set(LmbrCentralAuthoringSet::Colliders),
        )
        .add_systems(
            Update,
            (
                materialize_trigger_areas,
                materialize_rigid_bodies,
                materialize_static_bodies,
                materialize_living_bodies,
                cleanup_removed_rigid_bodies,
                cleanup_removed_static_bodies,
                cleanup_removed_living_bodies,
                cleanup_removed_trigger_areas,
            )
                .chain()
                .in_set(LmbrCentralAuthoringSet::Bodies),
        );
    app.add_systems(
        Update,
        (
            apply_force_volumes.in_set(PhysicsSet::Forces),
            update_trigger_areas
                .after(PhysicsSet::Interactions)
                .before(PhysicsSet::Writeback),
            record_physics_collisions
                .after(PhysicsSet::Interactions)
                .before(PhysicsSet::Writeback),
        ),
    );
}

fn initialize_trigger_area_state(
    mut commands: Commands,
    areas: Query<Entity, (With<TriggerAreaComponent>, Without<TriggerAreaState>)>,
) {
    for area in &areas {
        commands.entity(area).insert(TriggerAreaState::default());
    }
}

fn apply_trigger_area_requests(
    mut requests: MessageReader<TriggerAreaRequest>,
    mut areas: Query<(&mut TriggerAreaComponent, &mut TriggerAreaState)>,
) {
    for request in requests.read() {
        let Ok((mut configuration, mut state)) = areas.get_mut(request.area) else {
            continue;
        };
        match request.kind {
            TriggerAreaRequestKind::AddRequiredTag(tag) => {
                let _ = configuration.try_add_required_tag(tag);
            }
            TriggerAreaRequestKind::RemoveRequiredTag(tag) => {
                configuration.remove_required_tag(tag);
            }
            TriggerAreaRequestKind::AddExcludedTag(tag) => {
                let _ = configuration.try_add_excluded_tag(tag);
            }
            TriggerAreaRequestKind::RemoveExcludedTag(tag) => {
                configuration.remove_excluded_tag(tag);
            }
            TriggerAreaRequestKind::AddProximityTrigger => state.add_proximity_trigger(),
            TriggerAreaRequestKind::RemoveProximityTrigger => state.remove_proximity_trigger(),
        }
    }
}

fn apply_trigger_area_relevance(
    mut relevance: MessageReader<TriggerAreaRelevanceChanged>,
    mut areas: Query<&mut TriggerAreaState>,
) {
    for changed in relevance.read() {
        if let Ok(mut state) = areas.get_mut(changed.area) {
            state.set_relevant(changed.is_relevant);
        }
    }
}

fn reset_trigger_areas_on_respawn(
    mut respawned: MessageReader<TriggerAreasRespawned>,
    mut areas: Query<(&TriggerAreaComponent, &mut TriggerAreaState)>,
) {
    if respawned.read().next().is_none() {
        return;
    }
    for (configuration, mut state) in &mut areas {
        // The native runtime only cycles a trigger that currently exists.
        if configuration.reset_trigger_on_respawn && state.is_active() {
            state.cycle_for_respawn();
        }
    }
}

fn configure_collision_recording(
    mut commands: Commands,
    bodies: Query<(
        Entity,
        Ref<RigidPhysicsComponent>,
        Option<&RecordedPhysicsCollisions>,
    )>,
) {
    for (entity, body, current) in &bodies {
        let capacity = body.configuration.recorded_collision_capacity();
        if capacity == 0 {
            if current.is_some() {
                commands
                    .entity(entity)
                    .remove::<RecordedPhysicsCollisions>();
            }
        } else if body.is_changed() || current.is_none_or(|current| current.capacity != capacity) {
            commands
                .entity(entity)
                .insert(RecordedPhysicsCollisions::with_capacity(capacity));
        }
    }
}

fn record_physics_collisions(
    mut interactions: MessageReader<PhysicsInteraction>,
    handles: Query<(Entity, &PhysicsBodyHandle)>,
    mut records: Query<&mut RecordedPhysicsCollisions>,
) {
    let entities = handles
        .iter()
        .map(|(entity, handle)| (*handle, entity))
        .collect::<HashMap<_, _>>();
    for interaction in interactions.read() {
        if interaction.kind != PhysicsInteractionKind::Contact
            || interaction.phase == PhysicsInteractionPhase::Stopped
        {
            continue;
        }
        for handle in [interaction.body_a, interaction.body_b] {
            if let Some(&entity) = entities.get(&handle)
                && let Ok(mut record) = records.get_mut(entity)
            {
                record.push(*interaction);
            }
        }
    }
}

fn initialize_rigid_enabled(
    mut commands: Commands,
    bodies: Query<(Entity, Ref<RigidPhysicsComponent>, Option<&PhysicsEnabled>)>,
) {
    for (entity, body, enabled) in &bodies {
        if enabled.is_none() || body.is_changed() {
            commands
                .entity(entity)
                .insert(PhysicsEnabled(body.configuration.enabled_initially));
        }
    }
}

fn initialize_static_enabled(
    mut commands: Commands,
    bodies: Query<(Entity, Ref<StaticPhysicsComponent>, Option<&PhysicsEnabled>)>,
) {
    for (entity, body, enabled) in &bodies {
        if enabled.is_none() || body.is_changed() {
            commands
                .entity(entity)
                .insert(PhysicsEnabled(body.configuration.enabled_initially));
        }
    }
}

fn cleanup_removed_primitive_shapes(
    mut commands: Commands,
    mut removed_boxes: RemovedComponents<BoxShapeComponent>,
    mut removed_spheres: RemovedComponents<SphereShapeComponent>,
    mut removed_cylinders: RemovedComponents<CylinderShapeComponent>,
    mut removed_capsules: RemovedComponents<CapsuleShapeComponent>,
    shape_products: Query<(), With<LmbrPrimitiveShapeProduct>>,
    collider_products: Query<(), With<LmbrPrimitiveColliderProduct>>,
) {
    let mut removed = HashSet::new();
    removed.extend(removed_boxes.read());
    removed.extend(removed_spheres.read());
    removed.extend(removed_cylinders.read());
    removed.extend(removed_capsules.read());
    for entity in removed {
        if shape_products.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsShapeSet>()
                .remove::<LmbrPrimitiveShapeProduct>();
        }
        if collider_products.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsColliderSet>()
                .remove::<LmbrPrimitiveColliderProduct>();
        }
    }
}

fn cleanup_removed_polygon_prism_shapes(
    mut commands: Commands,
    mut removed: RemovedComponents<PolygonPrismShapeComponent>,
    shape_products: Query<(), With<LmbrPolygonPrismShapeProduct>>,
    collider_products: Query<(), With<LmbrPrimitiveColliderProduct>>,
) {
    for entity in removed.read() {
        if shape_products.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsShapeSet>()
                .remove::<LmbrPolygonPrismShapeProduct>();
        }
        if collider_products.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsColliderSet>()
                .remove::<LmbrPrimitiveColliderProduct>();
        }
    }
}

fn cleanup_removed_compound_shapes(
    mut commands: Commands,
    mut removed: RemovedComponents<CompoundShapeComponent>,
    shape_products: Query<(), With<LmbrCompoundShapeProduct>>,
    collider_products: Query<(), With<LmbrPrimitiveColliderProduct>>,
) {
    for entity in removed.read() {
        if shape_products.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsShapeSet>()
                .remove::<LmbrCompoundShapeProduct>();
        }
        if collider_products.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsColliderSet>()
                .remove::<LmbrPrimitiveColliderProduct>();
        }
    }
}

fn cleanup_removed_mesh_geometry(
    mut commands: Commands,
    mut removed: RemovedComponents<PhysicsMeshGeometry>,
    shape_products: Query<(), With<LmbrMeshShapeProduct>>,
    collider_products: Query<(), With<LmbrMeshColliderProduct>>,
) {
    for entity in removed.read() {
        if shape_products.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsShapeSet>()
                .remove::<LmbrMeshShapeProduct>();
        }
        if collider_products.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsColliderSet>()
                .remove::<LmbrMeshColliderProduct>();
        }
    }
}

fn cleanup_removed_primitive_colliders(
    mut commands: Commands,
    mut removed: RemovedComponents<PrimitiveColliderComponent>,
    products: Query<(), With<LmbrPrimitiveColliderProduct>>,
) {
    for entity in removed.read() {
        if products.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsColliderSet>()
                .remove::<LmbrPrimitiveColliderProduct>();
        }
    }
}

fn cleanup_removed_mesh_colliders(
    mut commands: Commands,
    mut removed: RemovedComponents<MeshColliderComponent>,
    shape_products: Query<(), With<LmbrMeshShapeProduct>>,
    collider_products: Query<(), With<LmbrMeshColliderProduct>>,
) {
    for entity in removed.read() {
        let mut entity_commands = commands.entity(entity);
        if shape_products.contains(entity) {
            entity_commands
                .remove::<PhysicsShapeSet>()
                .remove::<LmbrMeshShapeProduct>();
        }
        if collider_products.contains(entity) {
            entity_commands
                .remove::<PhysicsColliderSet>()
                .remove::<LmbrMeshColliderProduct>();
        }
    }
}

#[allow(
    clippy::type_complexity,
    reason = "one query enforces the native one-shape service"
)]
fn materialize_primitive_shapes(
    mut commands: Commands,
    shapes: Query<
        (
            Entity,
            Option<&GlobalTransform>,
            Option<&BoxShapeComponent>,
            Option<&SphereShapeComponent>,
            Option<&CylinderShapeComponent>,
            Option<&CapsuleShapeComponent>,
            Option<&PhysicsShapeSet>,
            Option<&CompoundShapeComponent>,
        ),
        Or<(
            With<BoxShapeComponent>,
            With<SphereShapeComponent>,
            With<CylinderShapeComponent>,
            With<CapsuleShapeComponent>,
        )>,
    >,
) {
    for (entity, transform, box_shape, sphere, cylinder, capsule, previous, compound) in &shapes {
        if compound.is_some() {
            continue;
        }
        let result = primitive_shape(
            transform_scale(transform),
            box_shape,
            sphere,
            cylinder,
            capsule,
        )
        .map(|shape| PhysicsShapeSet(vec![shape.into()]));
        set_shape_product::<LmbrPrimitiveShapeProduct>(&mut commands, entity, result, previous);
    }
}

fn materialize_polygon_prism_shapes(
    mut commands: Commands,
    shapes: Query<(
        Entity,
        &PolygonPrismShapeComponent,
        Option<&GlobalTransform>,
        Option<&PhysicsShapeSet>,
    )>,
) {
    for (entity, component, transform, previous) in &shapes {
        let scale = transform_scale(transform);
        let prism = &component.configuration.polygon_prism;
        let result = valid_scale(scale)
            .and_then(|()| prism.decompose().map_err(|error| error.to_string()))
            .and_then(|decomposition| {
                decomposition
                    .into_iter()
                    .map(|face| {
                        let points = face
                            .into_iter()
                            .flat_map(|vertex| {
                                [
                                    Vec3::new(vertex.x, vertex.y, 0.0) * scale,
                                    Vec3::new(vertex.x, vertex.y, prism.height) * scale,
                                ]
                            })
                            .collect();
                        let shape = ColliderShape::ConvexHull {
                            points,
                            border_radius: 0.0,
                        };
                        shape.validate().map_err(|error| error.to_string())?;
                        Ok(PhysicsShapeInstance::from(shape))
                    })
                    .collect::<Result<PhysicsShapeSet, String>>()
            });
        set_shape_product::<LmbrPolygonPrismShapeProduct>(&mut commands, entity, result, previous);
    }
}

/// A mesh collider's authored geometry plus the shape set it last produced.
type MeshShapeSourceData = (
    Entity,
    &'static MeshColliderComponent,
    &'static PhysicsMeshGeometry,
    Option<&'static GlobalTransform>,
    Option<&'static PhysicsShapeSet>,
);

fn materialize_mesh_shapes(mut commands: Commands, meshes: Query<MeshShapeSourceData>) {
    for (entity, _, geometry, transform, previous) in &meshes {
        let scale = transform_scale(transform);
        let result = geometry
            .validate()
            .map_err(|error| error.to_string())
            .and_then(|()| {
                valid_scale(scale)?;
                let vertices = geometry
                    .vertices
                    .iter()
                    .map(|vertex| *vertex * scale)
                    .collect();
                Ok(PhysicsShapeSet(vec![PhysicsShapeInstance::from(
                    ColliderShape::TriangleMesh {
                        vertices,
                        indices: geometry.indices.clone(),
                    },
                )]))
            });
        set_shape_product::<LmbrMeshShapeProduct>(&mut commands, entity, result, previous);
    }
}

fn materialize_compound_shapes(
    mut commands: Commands,
    compounds: Query<(
        Entity,
        &CompoundShapeComponent,
        Option<&GlobalTransform>,
        Option<&PhysicsShapeSet>,
    )>,
    children: Query<(&AzEntityId, &PhysicsShapeSet, Option<&GlobalTransform>)>,
) {
    for (entity, compound, root_transform, previous) in &compounds {
        let root_pose = world_pose(root_transform);
        let mut output = Vec::new();
        let mut error = if compound.configuration.child_shape_entities.is_empty() {
            Some("compound shape contains no child shape entities".to_owned())
        } else {
            None
        };
        for child_id in &compound.configuration.child_shape_entities {
            let Some((_, child_shapes, child_transform)) = children
                .iter()
                .find(|(candidate, _, _)| **candidate == *child_id)
            else {
                error = Some(format!(
                    "compound child entity id {} has no materialized collider",
                    child_id.value()
                ));
                break;
            };
            let child_pose = root_pose.inverse() * world_pose(child_transform);
            output.extend(child_shapes.0.iter().cloned().map(|mut shape| {
                shape.local_pose = child_pose * shape.local_pose;
                shape
            }));
        }
        let result = error.map_or_else(|| Ok(PhysicsShapeSet(output)), Err);
        set_shape_product::<LmbrCompoundShapeProduct>(&mut commands, entity, result, previous);
    }
}

fn set_shape_product<M: Component + Default>(
    commands: &mut Commands,
    entity: Entity,
    result: Result<PhysicsShapeSet, String>,
    previous: Option<&PhysicsShapeSet>,
) {
    match result {
        Ok(product) => {
            let mut entity_commands = commands.entity(entity);
            if previous != Some(&product) {
                entity_commands.insert(product);
            }
            entity_commands
                .insert(M::default())
                .remove::<LmbrCentralPhysicsError>();
        }
        Err(error) => {
            commands
                .entity(entity)
                .insert(LmbrCentralPhysicsError(error))
                .remove::<PhysicsShapeSet>()
                .remove::<M>();
        }
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
fn materialize_primitive_colliders(
    mut commands: Commands,
    materials: Res<PhysicsMaterialRegistry>,
    colliders: Query<(
        Entity,
        &PrimitiveColliderComponent,
        &PhysicsShapeSet,
        Option<&PhysicsColliderSet>,
    )>,
) {
    for (entity, collider, shapes, previous) in &colliders {
        let surface_name = collider.configuration.surface_type_name();
        let result = materials
            .resolve(surface_name)
            .ok_or_else(|| {
                format!(
                    "physics surface '{}' is not registered",
                    surface_name.unwrap_or_default()
                )
            })
            .and_then(|material| colliders_from_shapes(shapes, material));
        set_collider_product::<LmbrPrimitiveColliderProduct>(
            &mut commands,
            entity,
            result,
            previous,
        );
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
fn materialize_mesh_colliders(
    mut commands: Commands,
    materials: Res<PhysicsMaterialRegistry>,
    colliders: Query<(
        Entity,
        &MeshColliderComponent,
        &PhysicsShapeSet,
        Option<&PhysicsColliderSet>,
    )>,
) {
    let material = materials
        .resolve(None)
        .expect("the explicit default physics material always exists");
    for (entity, _, shapes, previous) in &colliders {
        set_collider_product::<LmbrMeshColliderProduct>(
            &mut commands,
            entity,
            colliders_from_shapes(shapes, material),
            previous,
        );
    }
}

#[allow(
    clippy::type_complexity,
    reason = "the query rejects ambiguous body ownership"
)]
#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
fn materialize_trigger_areas(
    mut commands: Commands,
    materials: Res<PhysicsMaterialRegistry>,
    mut areas: Query<(
        Entity,
        &TriggerAreaComponent,
        &PhysicsShapeSet,
        Option<&AzEntityId>,
        Option<&GlobalTransform>,
        Option<&BodyDescriptor>,
        Option<&RigidPhysicsComponent>,
        Option<&StaticPhysicsComponent>,
        Option<&CharacterPhysicsComponent>,
        &mut TriggerAreaState,
    )>,
) {
    let material = materials
        .resolve(None)
        .expect("the explicit default physics material always exists");
    for (
        entity,
        _area,
        shapes,
        entity_id,
        transform,
        previous,
        rigid,
        static_body,
        living,
        mut state,
    ) in &mut areas
    {
        if !state.is_active() {
            if previous.is_some() {
                commands.entity(entity).remove::<BodyDescriptor>();
            }
            continue;
        }
        let result = if rigid.is_some() || static_body.is_some() || living.is_some() {
            Err(
                "trigger area cannot share an entity with another physics body component"
                    .to_owned(),
            )
        } else {
            colliders_from_shapes(shapes, material).and_then(|mut colliders| {
                for collider in &mut colliders.0 {
                    collider.sensor = true;
                }
                descriptor(entity_id, transform, BodyKind::Area, colliders)
            })
        };
        let force_rematerialize = state.take_rematerialize();
        set_body_product::<LmbrTriggerAreaProduct>(
            &mut commands,
            entity,
            result,
            (!force_rematerialize).then_some(previous).flatten(),
        );
    }
}

/// Every body a trigger area can filter on, with the tags the filter reads.
type TriggerCandidateData = (
    Entity,
    &'static PhysicsBodyHandle,
    Option<&'static AzEntityId>,
    Option<&'static TagComponent>,
    Option<&'static LocalTriggerEntity>,
);

fn update_trigger_areas(
    mut world: Option<ResMut<PhysicsWorld>>,
    mut interactions: MessageReader<PhysicsInteraction>,
    mut events: MessageWriter<TriggerAreaEvent>,
    mut areas: Query<(
        Entity,
        &PhysicsBodyHandle,
        &TriggerAreaComponent,
        &mut TriggerAreaState,
    )>,
    bodies: Query<TriggerCandidateData>,
) {
    let Some(world) = world.as_deref_mut() else {
        return;
    };
    let body_entities = bodies
        .iter()
        .map(|body| (*body.1, body))
        .collect::<HashMap<_, _>>();
    let area_entities = areas
        .iter()
        .map(|(entity, handle, _, _)| (*handle, entity))
        .collect::<HashMap<_, _>>();

    for interaction in interactions.read() {
        if interaction.kind != PhysicsInteractionKind::Trigger {
            continue;
        }
        let Some((area, entrant_handle)) = area_and_entrant(interaction, &area_entities) else {
            continue;
        };
        let Some(&(entrant, _, entrant_id, tags, local)) = body_entities.get(&entrant_handle)
        else {
            continue;
        };
        if area == entrant {
            continue;
        }
        let Ok((_, &area_handle, configuration, mut state)) = areas.get_mut(area) else {
            continue;
        };
        if configuration.trigger_once && state.is_consumed() {
            continue;
        }

        match interaction.phase {
            PhysicsInteractionPhase::Started => {
                *state.overlaps.entry(entrant).or_default() += 1;
                if reevaluate_trigger_entrant(
                    area,
                    entrant,
                    entrant_id,
                    tags,
                    local.is_some(),
                    configuration,
                    &mut state,
                    &mut events,
                ) {
                    let _ = world.apply_action(area_handle, PhysicsAction::SetSimulated(false));
                }
            }
            PhysicsInteractionPhase::Persisted => {
                if reevaluate_trigger_entrant(
                    area,
                    entrant,
                    entrant_id,
                    tags,
                    local.is_some(),
                    configuration,
                    &mut state,
                    &mut events,
                ) {
                    let _ = world.apply_action(area_handle, PhysicsAction::SetSimulated(false));
                }
            }
            PhysicsInteractionPhase::Stopped => {
                let remove_overlap = state.overlaps.get_mut(&entrant).is_some_and(|count| {
                    *count = count.saturating_sub(1);
                    *count == 0
                });
                if remove_overlap {
                    state.overlaps.remove(&entrant);
                    if state.occupants.remove(&entrant) {
                        events.write(TriggerAreaEvent {
                            area,
                            entrant,
                            phase: PhysicsInteractionPhase::Stopped,
                        });
                    }
                }
            }
        }
    }
}

fn area_and_entrant(
    interaction: &PhysicsInteraction,
    areas: &HashMap<PhysicsBodyHandle, Entity>,
) -> Option<(Entity, PhysicsBodyHandle)> {
    areas
        .get(&interaction.body_a)
        .map(|area| (*area, interaction.body_b))
        .or_else(|| {
            areas
                .get(&interaction.body_b)
                .map(|area| (*area, interaction.body_a))
        })
}

#[allow(
    clippy::too_many_arguments,
    reason = "arguments are the complete native trigger filter inputs"
)]
fn reevaluate_trigger_entrant(
    area: Entity,
    entrant: Entity,
    entrant_id: Option<&AzEntityId>,
    tags: Option<&TagComponent>,
    is_local: bool,
    configuration: &TriggerAreaComponent,
    state: &mut TriggerAreaState,
    events: &mut MessageWriter<TriggerAreaEvent>,
) -> bool {
    let accepted = trigger_accepts(configuration, entrant_id, tags, is_local);
    if accepted && state.occupants.insert(entrant) {
        events.write(TriggerAreaEvent {
            area,
            entrant,
            phase: PhysicsInteractionPhase::Started,
        });
        if configuration.trigger_once {
            state.consume_once();
            return true;
        }
    } else if !accepted && state.occupants.remove(&entrant) {
        events.write(TriggerAreaEvent {
            area,
            entrant,
            phase: PhysicsInteractionPhase::Stopped,
        });
    }
    false
}

fn trigger_accepts(
    configuration: &TriggerAreaComponent,
    entity_id: Option<&AzEntityId>,
    tags: Option<&TagComponent>,
    is_local: bool,
) -> bool {
    if configuration.only_active_for_local_player && !is_local {
        return false;
    }
    if configuration.activation_entity_type == TriggerAreaActivationEntityType::SpecificEntities
        && !entity_id.is_some_and(|entity_id| {
            configuration
                .specific_interact_entities
                .contains(&entity_id.value())
        })
    {
        return false;
    }
    let has_tag = |tag| tags.is_some_and(|tags| tags.has_tag(tag));
    configuration.required_tags.iter().copied().all(has_tag)
        && !configuration.excluded_tags.iter().copied().any(has_tag)
}

fn apply_force_volumes(
    mut world: Option<ResMut<PhysicsWorld>>,
    volumes: Query<(&ForceVolumeComponent, &PhysicsBodyHandle, &TriggerAreaState)>,
    bodies: Query<&PhysicsBodyHandle>,
) {
    let Some(world) = world.as_deref_mut() else {
        return;
    };
    for (volume, &volume_body, state) in &volumes {
        let Ok(volume_status) = world.body_status(volume_body) else {
            continue;
        };
        let Ok(volume_bounds) = world.body_aabb(volume_body) else {
            continue;
        };
        for &entity in &state.occupants {
            let Ok(&body) = bodies.get(entity) else {
                continue;
            };
            let Ok(status) = world.body_status(body) else {
                continue;
            };
            let Ok(bounds) = world.body_aabb(body) else {
                continue;
            };
            let impulse = force_volume_impulse(
                &volume.configuration,
                volume_status.pose,
                volume_bounds,
                status,
                bounds,
            );
            if impulse.abs().cmpgt(Vec3::splat(f32::EPSILON)).any() {
                let _ = world.apply_action(
                    body,
                    PhysicsAction::Impulse(ImpulseAction {
                        impulse,
                        point: None,
                        explosion: false,
                        apply_during_step: false,
                    }),
                );
            }
        }
    }
}

fn force_volume_impulse(
    configuration: &super::ForceVolumeConfiguration,
    volume_pose: PhysicsPose,
    volume_bounds: bevy::math::bounding::Aabb3d,
    body: az_physics::BodyStatus,
    body_bounds: bevy::math::bounding::Aabb3d,
) -> Vec3 {
    let mut impulse = Vec3::ZERO;
    if configuration.force_scale > 0.0 {
        let direction = match configuration.force_mode {
            ForceMode::Point => body.pose.translation - Vec3::from(volume_bounds.center()),
            ForceMode::Direction => match configuration.force_space {
                ForceSpace::Local => volume_pose.rotation * configuration.force_direction,
                ForceSpace::World => configuration.force_direction,
            },
        };
        let magnitude = configuration.force_scale
            * if configuration.force_mass_dependent {
                body.mass
            } else {
                1.0
            };
        impulse += direction.normalize_or_zero() * magnitude;
    }
    if configuration.volume_damping > 0.0 {
        impulse -= body.linear_velocity * configuration.volume_damping;
    }
    if configuration.volume_density > 0.0 {
        // The native implementation squares each
        // velocity lane, uses the squared radius of the AABB's enclosing
        // sphere, and multiplies by the baked drag-area coefficient 1.47655.
        let half_extents = Vec3::from((body_bounds.max - body_bounds.min) * 0.5);
        impulse += body.linear_velocity
            * body.linear_velocity
            * (-0.5 * configuration.volume_density * 1.47655 * half_extents.length_squared());
    }
    impulse
}

fn set_collider_product<M: Component + Default>(
    commands: &mut Commands,
    entity: Entity,
    result: Result<PhysicsColliderSet, String>,
    previous: Option<&PhysicsColliderSet>,
) {
    match result {
        Ok(product) => {
            let mut entity_commands = commands.entity(entity);
            if previous != Some(&product) {
                entity_commands.insert(product);
            }
            entity_commands
                .insert(M::default())
                .remove::<LmbrCentralPhysicsError>();
        }
        Err(error) => {
            commands
                .entity(entity)
                .insert(LmbrCentralPhysicsError(error))
                .remove::<PhysicsColliderSet>()
                .remove::<M>();
        }
    }
}

fn primitive_shape(
    scale: Vec3,
    box_shape: Option<&BoxShapeComponent>,
    sphere: Option<&SphereShapeComponent>,
    cylinder: Option<&CylinderShapeComponent>,
    capsule: Option<&CapsuleShapeComponent>,
) -> Result<ColliderShape, String> {
    valid_scale(scale)?;
    let shape_count = usize::from(box_shape.is_some())
        + usize::from(sphere.is_some())
        + usize::from(cylinder.is_some())
        + usize::from(capsule.is_some());
    if shape_count != 1 {
        return Err(format!(
            "primitive collider requires exactly one primitive shape, found {shape_count}"
        ));
    }
    let shape = if let Some(shape) = box_shape {
        ColliderShape::Cuboid {
            half_extents: shape.configuration.dimensions.abs() * scale * 0.5,
        }
    } else if let Some(shape) = sphere {
        require_uniform(scale, "sphere")?;
        ColliderShape::Sphere {
            radius: shape.configuration.radius * scale.x,
        }
    } else if let Some(shape) = cylinder {
        require_equal(scale.x, scale.y, "cylinder radial scale")?;
        ColliderShape::Cylinder {
            axis: Axis3::Z,
            half_height: shape.configuration.height * scale.z * 0.5,
            radius: shape.configuration.radius * scale.x,
        }
    } else {
        let shape = capsule.expect("shape count proved the capsule branch");
        require_equal(scale.x, scale.y, "capsule radial scale")?;
        ColliderShape::Capsule {
            axis: Axis3::Z,
            // The native converter uses max(height / 2 - radius, 0) * scale.z.
            half_height: shape
                .configuration
                .height
                .mul_add(0.5, -shape.configuration.radius)
                .max(0.0)
                * scale.z,
            radius: shape.configuration.radius * scale.x,
        }
    };
    shape.validate().map_err(|error| error.to_string())?;
    Ok(shape)
}

fn colliders_from_shapes(
    shapes: &PhysicsShapeSet,
    material: PhysicsMaterial,
) -> Result<PhysicsColliderSet, String> {
    if shapes.0.is_empty() {
        return Err("shape product contains no geometry".to_owned());
    }
    shapes
        .0
        .iter()
        .map(|instance| {
            collider_from_shape(instance.shape.clone(), material).map(|mut collider| {
                collider.local_pose = instance.local_pose;
                collider
            })
        })
        .collect()
}

fn collider_from_shape(
    shape: ColliderShape,
    material: PhysicsMaterial,
) -> Result<ColliderConfiguration, String> {
    shape.validate().map_err(|error| error.to_string())?;
    Ok(ColliderConfiguration {
        shape,
        surface_index: material.surface_index,
        surface_pierceability: material.pierceability,
        friction: material.friction,
        restitution: material.restitution,
        density: material.density,
        ..ColliderConfiguration::default()
    })
}

/// An authored rigid body, its enable state, and the descriptor it last built.
type RigidBodySourceData = (
    Entity,
    &'static RigidPhysicsComponent,
    &'static PhysicsEnabled,
    Option<&'static AzEntityId>,
    Option<&'static GlobalTransform>,
    Option<&'static PhysicsColliderSet>,
    Option<&'static BodyDescriptor>,
);

fn materialize_rigid_bodies(mut commands: Commands, bodies: Query<RigidBodySourceData>) {
    for (entity, body, enabled, entity_id, transform, colliders, previous) in &bodies {
        if !enabled.0 {
            commands
                .entity(entity)
                .remove::<BodyDescriptor>()
                .insert(LmbrRigidProduct);
            continue;
        }
        let result = colliders
            .ok_or_else(|| "rigid physics component has no materialized collider".to_owned())
            .and_then(|colliders| {
                let mut colliders = colliders.clone();
                for collider in &mut colliders.0 {
                    collider.sensor = !body.configuration.enable_collision_response;
                    collider.interacts_with_triggers = body.configuration.interacts_with_triggers;
                    if body.configuration.specify_mass_or_density == MassOrDensity::Density {
                        collider.density = body.configuration.density;
                        collider.mass = None;
                    }
                }
                if body.configuration.specify_mass_or_density == MassOrDensity::Mass {
                    colliders
                        .distribute_mass(body.configuration.mass)
                        .map_err(|error| error.to_string())?;
                }
                let configuration = RigidBodyConfiguration {
                    mass: body.configuration.mass,
                    density: body.configuration.density,
                    linear_damping: body.configuration.simulation_damping,
                    angular_damping: body.configuration.simulation_damping,
                    buoyancy: az_physics::RigidBodyBuoyancy {
                        density_scale: body.configuration.buoyancy_density,
                        resistance_scale: body.configuration.buoyancy_resistance,
                        damping: body.configuration.buoyancy_damping,
                    },
                    sleep_min_energy: body.configuration.simulation_min_energy,
                    sleep_policy: az_physics::RigidBodySleepPolicy::CryEnergy,
                    start_asleep: body.configuration.at_rest_initially,
                    // Both source modes assign mass through individual parts:
                    // density directly, or total mass distributed by volume.
                    compute_mass: true,
                    ..RigidBodyConfiguration::default()
                };
                descriptor(
                    entity_id,
                    transform,
                    BodyKind::Rigid(configuration),
                    colliders,
                )
            });
        set_body_product::<LmbrRigidProduct>(&mut commands, entity, result, previous);
    }
}

/// An authored static body, its enable state, and the descriptor it last built.
type StaticBodySourceData = (
    Entity,
    &'static StaticPhysicsComponent,
    &'static PhysicsEnabled,
    Option<&'static AzEntityId>,
    Option<&'static GlobalTransform>,
    Option<&'static PhysicsColliderSet>,
    Option<&'static BodyDescriptor>,
);

#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
fn materialize_static_bodies(
    mut commands: Commands,
    filters: Res<CollisionFilterRegistry>,
    bodies: Query<StaticBodySourceData>,
) {
    for (entity, body, enabled, entity_id, transform, colliders, previous) in &bodies {
        if !enabled.0 {
            commands
                .entity(entity)
                .remove::<BodyDescriptor>()
                .insert(LmbrStaticProduct);
            continue;
        }
        let result = colliders
            .ok_or_else(|| "static physics component has no materialized collider".to_owned())
            .and_then(|colliders| {
                let filter_name = body.collision_filter().ok_or_else(|| {
                    "static physics component has an empty collision filter".to_owned()
                })?;
                let collision_filter = filters
                    .resolve(filter_name)
                    .ok_or_else(|| format!("collision filter '{filter_name}' is not registered"))?;
                let mut colliders = colliders.clone();
                for collider in &mut colliders.0 {
                    collider.collision_filter = Some(collision_filter);
                    collider.interacts_with_triggers = body.configuration.interacts_with_triggers;
                }
                descriptor(
                    entity_id,
                    transform,
                    BodyKind::Static { terrain: false },
                    colliders,
                )
            });
        set_body_product::<LmbrStaticProduct>(&mut commands, entity, result, previous);
    }
}

/// An authored character body and the descriptor it last built.
type LivingBodySourceData = (
    Entity,
    &'static CharacterPhysicsComponent,
    Option<&'static AzEntityId>,
    Option<&'static GlobalTransform>,
    Option<&'static BodyDescriptor>,
);

fn materialize_living_bodies(mut commands: Commands, bodies: Query<LivingBodySourceData>) {
    for (entity, body, entity_id, transform, previous) in &bodies {
        let result = descriptor(
            entity_id,
            transform,
            BodyKind::Living(body.living_body_configuration()),
            PhysicsColliderSet::default(),
        );
        set_body_product::<LmbrLivingProduct>(&mut commands, entity, result, previous);
    }
}

fn descriptor(
    entity_id: Option<&AzEntityId>,
    transform: Option<&GlobalTransform>,
    kind: BodyKind,
    colliders: PhysicsColliderSet,
) -> Result<BodyDescriptor, String> {
    let descriptor = BodyDescriptor {
        entity_id: entity_id.map(|id| PhysicsEntityId(id.value())),
        pose: world_pose(transform),
        kind,
        colliders: colliders.0,
    };
    descriptor.validate().map_err(|error| error.to_string())?;
    Ok(descriptor)
}

fn set_body_product<M: Component + Default>(
    commands: &mut Commands,
    entity: Entity,
    result: Result<BodyDescriptor, String>,
    previous: Option<&BodyDescriptor>,
) {
    match result {
        Ok(descriptor) => {
            let mut entity_commands = commands.entity(entity);
            if previous != Some(&descriptor) {
                entity_commands.insert(descriptor);
            }
            entity_commands
                .insert(M::default())
                .remove::<LmbrCentralPhysicsError>();
        }
        Err(error) => {
            commands
                .entity(entity)
                .insert(LmbrCentralPhysicsError(error))
                .remove::<BodyDescriptor>()
                .remove::<M>();
        }
    }
}

fn cleanup_removed_rigid_bodies(
    mut commands: Commands,
    mut removed: RemovedComponents<RigidPhysicsComponent>,
    products: Query<(), With<LmbrRigidProduct>>,
) {
    cleanup_removed::<LmbrRigidProduct>(&mut commands, &mut removed, &products);
}

fn cleanup_removed_static_bodies(
    mut commands: Commands,
    mut removed: RemovedComponents<StaticPhysicsComponent>,
    products: Query<(), With<LmbrStaticProduct>>,
) {
    cleanup_removed::<LmbrStaticProduct>(&mut commands, &mut removed, &products);
}

fn cleanup_removed_living_bodies(
    mut commands: Commands,
    mut removed: RemovedComponents<CharacterPhysicsComponent>,
    products: Query<(), With<LmbrLivingProduct>>,
) {
    cleanup_removed::<LmbrLivingProduct>(&mut commands, &mut removed, &products);
}

fn cleanup_removed_trigger_areas(
    mut commands: Commands,
    mut removed: RemovedComponents<TriggerAreaComponent>,
    products: Query<(), With<LmbrTriggerAreaProduct>>,
) {
    for entity in removed.read() {
        if products.contains(entity) {
            commands
                .entity(entity)
                .remove::<BodyDescriptor>()
                .remove::<TriggerAreaState>()
                .remove::<LmbrTriggerAreaProduct>()
                .remove::<LmbrCentralPhysicsError>();
        }
    }
}

fn cleanup_removed<M: Component>(
    commands: &mut Commands,
    removed: &mut RemovedComponents<impl Component>,
    products: &Query<(), With<M>>,
) {
    for entity in removed.read() {
        if products.contains(entity) {
            commands
                .entity(entity)
                .remove::<BodyDescriptor>()
                .remove::<PhysicsEnabled>()
                .remove::<M>()
                .remove::<LmbrCentralPhysicsError>();
        }
    }
}

fn transform_scale(transform: Option<&GlobalTransform>) -> Vec3 {
    transform.map_or(Vec3::ONE, |transform| {
        transform.to_scale_rotation_translation().0.abs()
    })
}

fn world_pose(transform: Option<&GlobalTransform>) -> PhysicsPose {
    transform.map_or(PhysicsPose::IDENTITY, |transform| {
        let (_, rotation, translation) = transform.to_scale_rotation_translation();
        PhysicsPose {
            translation,
            rotation,
        }
    })
}

fn valid_scale(scale: Vec3) -> Result<(), String> {
    if scale.is_finite() && scale.cmpgt(Vec3::ZERO).all() {
        Ok(())
    } else {
        Err(format!("physics shape has invalid world scale {scale:?}"))
    }
}

fn require_uniform(scale: Vec3, shape: &str) -> Result<(), String> {
    require_equal(scale.x, scale.y, shape)?;
    require_equal(scale.x, scale.z, shape)
}

fn require_equal(left: f32, right: f32, field: &str) -> Result<(), String> {
    if (left - right).abs() <= 1.0e-5 * left.abs().max(right.abs()).max(1.0) {
        Ok(())
    } else {
        Err(format!(
            "{field} cannot represent non-uniform scale ({left}, {right}) without changing geometry"
        ))
    }
}

impl Default for LmbrRigidProduct {
    fn default() -> Self {
        Self
    }
}

impl Default for LmbrStaticProduct {
    fn default() -> Self {
        Self
    }
}

impl Default for LmbrLivingProduct {
    fn default() -> Self {
        Self
    }
}

impl Default for LmbrTriggerAreaProduct {
    fn default() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BoxShapeConfig, CapsuleShapeConfig, LmbrCentralPhysicsPlugin, PolygonPrism,
        PolygonPrismCommon,
    };

    #[test]
    fn unrelated_entities_do_not_receive_primitive_shape_errors() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins(LmbrCentralPhysicsPlugin);
        let entity = app.world_mut().spawn_empty().id();

        app.update();

        assert!(
            !app.world()
                .entity(entity)
                .contains::<LmbrCentralPhysicsError>()
        );
    }

    #[test]
    fn living_and_mesh_bodies_materialize_in_their_explicit_scene() {
        use az_physics::{CollisionCategoryMask, CollisionFilter, PhysicsSceneId, SimulationClass};
        use bevy::asset::AssetPlugin;

        let scene = PhysicsSceneId::new(11);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins((
                az_physics_rapier::RapierPhysicsPlugin::default(),
                LmbrCentralPhysicsPlugin,
            ));
        app.world_mut()
            .resource_mut::<PhysicsWorld>()
            .ensure_scene(scene);
        app.world_mut()
            .resource_mut::<CollisionFilterRegistry>()
            .insert(
                "Structure",
                CollisionFilter::new(
                    CollisionCategoryMask::EMPTY,
                    CollisionCategoryMask::EMPTY,
                    0,
                ),
            );
        let living = app
            .world_mut()
            .spawn((CharacterPhysicsComponent::default(), scene))
            .id();
        let mesh = app
            .world_mut()
            .spawn((
                MeshColliderComponent::default(),
                PhysicsMeshGeometry {
                    vertices: vec![
                        Vec3::new(-1.0, -1.0, 0.0),
                        Vec3::new(1.0, -1.0, 0.0),
                        Vec3::new(0.0, 1.0, 0.0),
                    ],
                    indices: vec![[0, 1, 2]],
                },
                StaticPhysicsComponent::default(),
                scene,
            ))
            .id();

        app.update();

        let living_body = *app
            .world()
            .entity(living)
            .get::<PhysicsBodyHandle>()
            .expect("living body");
        let mesh_body = *app
            .world()
            .entity(mesh)
            .get::<PhysicsBodyHandle>()
            .expect("mesh body");
        assert_eq!(living_body.scene(), scene);
        assert_eq!(mesh_body.scene(), scene);
        let physics = app.world().resource::<PhysicsWorld>();
        assert_eq!(
            physics
                .body_status(living_body)
                .expect("living status")
                .simulation_class,
            SimulationClass::Living
        );
        assert_eq!(
            physics
                .body_status(mesh_body)
                .expect("mesh status")
                .simulation_class,
            SimulationClass::Static
        );
    }

    #[test]
    fn removing_primitive_shape_cascades_through_body_descriptor() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins(LmbrCentralPhysicsPlugin);
        let entity = app
            .world_mut()
            .spawn((
                BoxShapeComponent::default(),
                PrimitiveColliderComponent::default(),
                RigidPhysicsComponent::default(),
            ))
            .id();

        app.update();
        assert!(app.world().entity(entity).contains::<PhysicsShapeSet>());
        assert!(app.world().entity(entity).contains::<PhysicsColliderSet>());
        assert!(app.world().entity(entity).contains::<BodyDescriptor>());

        app.world_mut()
            .entity_mut(entity)
            .remove::<BoxShapeComponent>();
        app.update();

        let entity_ref = app.world().entity(entity);
        assert!(!entity_ref.contains::<PhysicsShapeSet>());
        assert!(!entity_ref.contains::<PhysicsColliderSet>());
        assert!(!entity_ref.contains::<BodyDescriptor>());
    }

    #[test]
    fn removing_collider_cascades_through_body_descriptor_but_keeps_shape() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins(LmbrCentralPhysicsPlugin);
        let entity = app
            .world_mut()
            .spawn((
                BoxShapeComponent::default(),
                PrimitiveColliderComponent::default(),
                RigidPhysicsComponent::default(),
            ))
            .id();

        app.update();
        app.world_mut()
            .entity_mut(entity)
            .remove::<PrimitiveColliderComponent>();
        app.update();

        let entity_ref = app.world().entity(entity);
        assert!(entity_ref.contains::<PhysicsShapeSet>());
        assert!(!entity_ref.contains::<PhysicsColliderSet>());
        assert!(!entity_ref.contains::<BodyDescriptor>());
    }

    #[test]
    fn primitive_scaling_matches_native_converter() {
        let capsule = CapsuleShapeComponent {
            configuration: CapsuleShapeConfig {
                height: 2.0,
                radius: 0.25,
            },
            ..Default::default()
        };
        let product = primitive_shape(Vec3::splat(2.0), None, None, None, Some(&capsule)).unwrap();
        assert_eq!(
            product,
            ColliderShape::Capsule {
                axis: Axis3::Z,
                half_height: 1.5,
                radius: 0.5,
            }
        );

        let box_shape = BoxShapeComponent {
            configuration: BoxShapeConfig {
                dimensions: Vec3::new(2.0, 4.0, 6.0),
            },
            ..Default::default()
        };
        let product =
            primitive_shape(Vec3::new(2.0, 3.0, 4.0), Some(&box_shape), None, None, None).unwrap();
        assert_eq!(
            product,
            ColliderShape::Cuboid {
                half_extents: Vec3::new(2.0, 6.0, 12.0),
            }
        );
    }

    #[test]
    fn polygon_prism_materializes_convex_hulls_with_world_scale() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins(LmbrCentralPhysicsPlugin);
        let entity = app
            .world_mut()
            .spawn((
                PolygonPrismShapeComponent {
                    configuration: PolygonPrismCommon {
                        polygon_prism: PolygonPrism {
                            height: 2.0,
                            vertices: vec![
                                Vec2::new(0.0, 0.0),
                                Vec2::new(2.0, 0.0),
                                Vec2::new(2.0, 1.0),
                                Vec2::new(0.0, 1.0),
                            ],
                        },
                        reduced_vertices: Vec::new(),
                    },
                    ..Default::default()
                },
                GlobalTransform::from(Transform::from_scale(Vec3::new(2.0, 3.0, 4.0))),
            ))
            .id();

        app.update();

        let shapes = app.world().entity(entity).get::<PhysicsShapeSet>().unwrap();
        assert_eq!(shapes.0.len(), 1);
        let ColliderShape::ConvexHull { points, .. } = &shapes.0[0].shape else {
            panic!("polygon prism must materialize as a convex hull");
        };
        assert_eq!(points.len(), 8);
        assert!(points.contains(&Vec3::new(4.0, 3.0, 8.0)));
    }

    #[test]
    fn unsupported_nonuniform_round_shape_is_not_silently_approximated() {
        let result = primitive_shape(
            Vec3::new(1.0, 2.0, 1.0),
            None,
            Some(&SphereShapeComponent::default()),
            None,
            None,
        );
        assert!(result.unwrap_err().contains("non-uniform"));
    }

    #[test]
    fn trigger_area_state_matches_proximity_request_and_trigger_once_lifecycle() {
        let mut state = TriggerAreaState::default();
        assert!(state.is_active());

        state.remove_proximity_trigger();
        assert!(!state.is_active());
        state.add_proximity_trigger();
        assert!(state.is_active());

        state.consume_once();
        assert!(!state.is_active());
        state.add_proximity_trigger();
        assert!(!state.is_active());
    }

    #[test]
    fn trigger_area_relevance_and_respawn_cycle_are_independent_gates() {
        let mut state = TriggerAreaState::default();
        state.set_relevant(false);
        assert!(!state.is_active());
        state.set_relevant(true);
        assert!(state.is_active());

        state.cycle_for_respawn();
        assert!(state.is_active());
        assert!(state.take_rematerialize());
        assert!(!state.take_rematerialize());
    }

    #[test]
    fn force_volume_primary_and_damping_match_native_branches() {
        let configuration = super::super::ForceVolumeConfiguration {
            force_mode: ForceMode::Direction,
            force_space: ForceSpace::Local,
            force_mass_dependent: true,
            force_scale: 2.0,
            force_direction: Vec3::X,
            volume_damping: 0.5,
            volume_density: 0.0,
        };
        let body = az_physics::BodyStatus {
            pose: PhysicsPose::IDENTITY,
            linear_velocity: Vec3::new(2.0, -4.0, 0.0),
            angular_velocity: Vec3::ZERO,
            linear_acceleration: Vec3::ZERO,
            angular_acceleration: Vec3::ZERO,
            mass: 3.0,
            density: 0.0,
            kinetic_energy: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            sleep_min_energy: 0.0,
            buoyancy: az_physics::RigidBodyBuoyancy::default(),
            buoyancy_status: az_physics::BuoyancyStatus::default(),
            simulation_class: az_physics::SimulationClass::ActiveRigid,
            awake: true,
            kinematic: false,
            simulated: true,
        };
        let bounds = bevy::math::bounding::Aabb3d::from_min_max(Vec3::splat(-1.0), Vec3::ONE);
        let impulse = force_volume_impulse(
            &configuration,
            PhysicsPose {
                translation: Vec3::ZERO,
                rotation: Quat::from_rotation_z(core::f32::consts::FRAC_PI_2),
            },
            bounds,
            body,
            bounds,
        );
        assert!(impulse.abs_diff_eq(Vec3::new(-1.0, 8.0, 0.0), 1.0e-5));
    }

    #[test]
    fn force_volume_density_preserves_native_lane_squared_logic() {
        let configuration = super::super::ForceVolumeConfiguration {
            force_scale: 0.0,
            volume_density: 2.0,
            ..Default::default()
        };
        let body = az_physics::BodyStatus {
            pose: PhysicsPose::IDENTITY,
            linear_velocity: Vec3::new(-2.0, 1.0, 0.0),
            angular_velocity: Vec3::ZERO,
            linear_acceleration: Vec3::ZERO,
            angular_acceleration: Vec3::ZERO,
            mass: 1.0,
            density: 0.0,
            kinetic_energy: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            sleep_min_energy: 0.0,
            buoyancy: az_physics::RigidBodyBuoyancy::default(),
            buoyancy_status: az_physics::BuoyancyStatus::default(),
            simulation_class: az_physics::SimulationClass::ActiveRigid,
            awake: true,
            kinematic: false,
            simulated: true,
        };
        let volume_bounds = bevy::math::bounding::Aabb3d::from_min_max(Vec3::ZERO, Vec3::ONE);
        let body_bounds = bevy::math::bounding::Aabb3d::from_min_max(
            Vec3::new(-1.0, -2.0, -2.0),
            Vec3::new(1.0, 2.0, 2.0),
        );
        let impulse = force_volume_impulse(
            &configuration,
            PhysicsPose::IDENTITY,
            volume_bounds,
            body,
            body_bounds,
        );
        let scale = -0.5 * 2.0 * 1.47655 * 9.0;
        assert!(impulse.abs_diff_eq(Vec3::new(4.0, 1.0, 0.0) * scale, 1.0e-5));
    }
}
