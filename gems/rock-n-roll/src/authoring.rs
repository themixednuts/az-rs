//! Canonical `RockNRoll` component-to-runtime materialization.

use std::collections::{HashMap, HashSet};

use az_core::EntityId as AzEntityId;
use az_physics::{
    BodyDescriptor, BodyKind, CharacterBodyConfiguration, PhysicsAction, PhysicsBodyHandle,
    PhysicsColliderSet, PhysicsEntityId, PhysicsPose, PhysicsSet, PhysicsStepReport, PhysicsWorld,
    RigidBodyConfiguration as RuntimeRigidBodyConfiguration, RigidBodyMotion,
};
use bevy::prelude::*;

use crate::{
    CharacterControllerComponent, CharacterControllerShapeSource, ContinuousPhysicsMode,
    RigidBodyComponent, RigidBodyShapeSource, RockNRollSystemComponent, ShapeAssetPhysicsBinding,
    ShapeAssetRef, ShapeAssetReference, SleepState, TimeStep, scale_physics_colliders,
};

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct RockNRollBodyError(pub String);

/// Stable ordering boundaries for systems that depend on `RockNRoll` products.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RockNRollAuthoringSet {
    References,
    ShapeAssets,
    Bodies,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct RockNRollRigidBodyProduct;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct RockNRollCharacterProduct;

#[derive(Component, Debug, Clone, Copy, Default, PartialEq)]
struct RockNRollSleepRuntime {
    state: SleepState,
    awake: Option<bool>,
}

pub fn configure(app: &mut App) {
    app.configure_sets(
        Update,
        (
            RockNRollAuthoringSet::References,
            RockNRollAuthoringSet::ShapeAssets,
            RockNRollAuthoringSet::Bodies,
        )
            .chain()
            .in_set(PhysicsSet::Authoring),
    )
    .add_systems(
        Update,
        (
            sync_rigid_shape_references,
            sync_character_shape_references,
            crate::binding::sync_shape_asset_references,
            crate::binding::cleanup_removed_shape_asset_references,
        )
            .chain()
            .in_set(RockNRollAuthoringSet::References),
    )
    .add_systems(
        Update,
        (
            crate::binding::sync_shape_asset_physics,
            crate::binding::cleanup_removed_shape_asset_bindings,
        )
            .chain()
            .in_set(RockNRollAuthoringSet::ShapeAssets),
    )
    .add_systems(
        Update,
        (
            materialize_rigid_bodies,
            materialize_characters,
            cleanup_removed_rigid_bodies,
            cleanup_removed_characters,
        )
            .chain()
            .in_set(RockNRollAuthoringSet::Bodies),
    )
    .add_systems(
        Update,
        evaluate_native_sleep
            .after(PhysicsSet::Step)
            .before(PhysicsSet::Writeback),
    );
}

fn sync_rigid_shape_references(
    mut commands: Commands,
    bodies: Query<(
        Entity,
        Ref<RigidBodyComponent>,
        Option<&ShapeAssetReference>,
    )>,
) {
    for (entity, body, current) in &bodies {
        if !body.is_changed() && current.is_some() {
            continue;
        }
        sync_shape_reference(
            &mut commands,
            entity,
            match &body.configuration.shape {
                RigidBodyShapeSource::Asset(asset) if !asset.is_empty() => Some(asset),
                RigidBodyShapeSource::Asset(_) | RigidBodyShapeSource::Entity(_) => None,
            },
            current,
        );
    }
}

fn sync_character_shape_references(
    mut commands: Commands,
    controllers: Query<(
        Entity,
        Ref<CharacterControllerComponent>,
        Option<&ShapeAssetReference>,
    )>,
) {
    for (entity, controller, current) in &controllers {
        if !controller.is_changed() && current.is_some() {
            continue;
        }
        sync_shape_reference(
            &mut commands,
            entity,
            match &controller.configuration.shape {
                CharacterControllerShapeSource::Asset(asset) if !asset.is_empty() => Some(asset),
                CharacterControllerShapeSource::Asset(_)
                | CharacterControllerShapeSource::Entity(_) => None,
            },
            current,
        );
    }
}

fn sync_shape_reference(
    commands: &mut Commands,
    entity: Entity,
    asset: Option<&ShapeAssetRef>,
    current: Option<&ShapeAssetReference>,
) {
    match asset {
        Some(asset) if current.map(AsRef::as_ref) != Some(asset) => {
            commands
                .entity(entity)
                .insert(ShapeAssetReference::from(asset.clone()));
        }
        None if current.is_some() => {
            commands.entity(entity).remove::<ShapeAssetReference>();
        }
        // Already in sync: the authored asset matches, or there is none either side.
        _ => {}
    }
}

#[allow(
    clippy::type_complexity,
    reason = "the query mirrors the two native shape-source branches"
)]
fn materialize_rigid_bodies(
    mut commands: Commands,
    bodies: Query<(
        Entity,
        &RigidBodyComponent,
        Option<&AzEntityId>,
        Option<&GlobalTransform>,
        Option<&PhysicsColliderSet>,
        Option<&ShapeAssetPhysicsBinding>,
        Option<&BodyDescriptor>,
        Option<&RockNRollSleepRuntime>,
    )>,
    entity_shapes: Query<(&AzEntityId, &PhysicsColliderSet)>,
) {
    for (
        entity,
        component,
        entity_id,
        transform,
        local_shapes,
        asset_binding,
        previous,
        sleep_runtime,
    ) in &bodies
    {
        let result = build_rigid_descriptor(
            component,
            entity_id,
            transform,
            local_shapes,
            asset_binding,
            &entity_shapes,
        );
        match result {
            Ok(descriptor) => {
                let mut entity_commands = commands.entity(entity);
                if previous != Some(&descriptor) {
                    entity_commands.insert(descriptor);
                }
                entity_commands
                    .insert(RockNRollRigidBodyProduct)
                    .remove::<RockNRollBodyError>();
                if component
                    .configuration
                    .motion_type
                    .has_dynamic_mass_properties()
                {
                    if sleep_runtime.is_none() {
                        entity_commands.insert(RockNRollSleepRuntime::default());
                    }
                } else {
                    entity_commands.remove::<RockNRollSleepRuntime>();
                }
            }
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(RockNRollBodyError(error))
                    .remove::<BodyDescriptor>()
                    .remove::<RockNRollRigidBodyProduct>()
                    .remove::<RockNRollSleepRuntime>();
            }
        }
    }
}

fn build_rigid_descriptor(
    component: &RigidBodyComponent,
    entity_id: Option<&AzEntityId>,
    transform: Option<&GlobalTransform>,
    local_shapes: Option<&PhysicsColliderSet>,
    asset_binding: Option<&ShapeAssetPhysicsBinding>,
    entity_shapes: &Query<(&AzEntityId, &PhysicsColliderSet)>,
) -> Result<BodyDescriptor, String> {
    component
        .configuration
        .validate()
        .map_err(|error| error.to_string())?;
    let source = match &component.configuration.shape {
        RigidBodyShapeSource::Asset(asset) => {
            if asset.is_empty() {
                return Err("RockNRoll rigid body has an empty shape asset".to_owned());
            }
            if asset_binding.is_none_or(|binding| binding.asset_id() != asset.id()) {
                return Err("RockNRoll rigid-body shape asset is not loaded".to_owned());
            }
            local_shapes
                .ok_or_else(|| "RockNRoll shape asset has no collider product".to_owned())?
        }
        RigidBodyShapeSource::Entity(source_id) => {
            if entity_id.is_some_and(|entity_id| entity_id == source_id) {
                return Err("RockNRoll rigid body cannot source its shape from itself".to_owned());
            }
            entity_shapes
                .iter()
                .find_map(|(candidate, shapes)| (candidate == source_id).then_some(shapes))
                .ok_or_else(|| {
                    format!(
                        "RockNRoll rigid-body shape entity {} is unresolved",
                        source_id.value()
                    )
                })?
        }
    };
    let (scale, pose) = transform_parts(transform);
    let mut colliders = source.clone();
    scale_physics_colliders(&mut colliders, scale).map_err(|error| error.to_string())?;
    for collider in &mut colliders.0 {
        collider.friction = component.configuration.material.coefficients.friction();
        collider.restitution = component.configuration.material.coefficients.restitution();
    }

    let motion_type = component.configuration.motion_type;
    if motion_type.has_dynamic_mass_properties() {
        colliders
            .distribute_mass(component.configuration.mass)
            .map_err(|error| error.to_string())?;
    }
    let kind = if motion_type.has_pose_history_and_dynamics_state() {
        BodyKind::Rigid(runtime_rigid_body_configuration(component))
    } else {
        BodyKind::Static { terrain: false }
    };

    Ok(BodyDescriptor {
        entity_id: entity_id.map(|id| PhysicsEntityId(id.value())),
        pose,
        kind,
        colliders: colliders.0,
    })
}

/// The native runtime configuration for a rigid body that carries pose history
/// and dynamics state.
fn runtime_rigid_body_configuration(
    component: &RigidBodyComponent,
) -> RuntimeRigidBodyConfiguration {
    let motion_type = component.configuration.motion_type;
    let sleep = component.configuration.runtime_sleep_configuration();
    RuntimeRigidBodyConfiguration {
        initial_linear_velocity: component.configuration.initial_linear_velocity,
        initial_angular_velocity: component.configuration.initial_angular_velocity,
        mass: component.configuration.mass,
        // The native descriptor uses identity principal inertia when
        // automatic inertia is disabled. When enabled, the shape
        // dispatcher derives it and the adapter scales it to authored
        // mass.
        principal_inertia: (!component.configuration.auto_inertia_tensor).then_some(Vec3::ONE),
        linear_damping: component.configuration.damping.linear,
        angular_damping: component.configuration.damping.angular,
        damping_model: az_physics::RigidBodyDampingModel::LinearStep {
            low_speed_decrement: crate::LOW_SPEED_DECREMENT,
        },
        // RockNRoll does not route through Cry `pe_params_buoyancy`.
        buoyancy: az_physics::RigidBodyBuoyancy {
            density_scale: 0.0,
            resistance_scale: 0.0,
            damping: 0.0,
        },
        sleep_min_energy: sleep.energy_threshold,
        sleep_linear_velocity_threshold: sleep.linear_velocity_threshold,
        sleep_angular_velocity_threshold: sleep.angular_velocity_threshold,
        sleep_duration: sleep.required_duration,
        sleep_policy: az_physics::RigidBodySleepPolicy::External,
        max_angular_velocity: f32::MAX,
        max_angular_displacement: Some(core::f32::consts::FRAC_PI_2),
        start_asleep: !component.configuration.initially_active,
        // RockNRoll's five-mode evaluator below owns sleeping exactly.
        can_sleep: false,
        gravity_enabled: motion_type.has_dynamic_mass_properties(),
        simulated: true,
        motion: match motion_type.value() {
            2 => RigidBodyMotion::KinematicVelocity,
            3 => RigidBodyMotion::KinematicPosition,
            4 => RigidBodyMotion::Dynamic,
            _ => unreachable!("dynamics-state motion types are exactly 2..=4"),
        },
        continuous_collision_mode: match component.configuration.continuous.mode {
            ContinuousPhysicsMode::Disabled => az_physics::ContinuousCollisionMode::Disabled,
            ContinuousPhysicsMode::Mode1 => az_physics::ContinuousCollisionMode::Mode1,
            ContinuousPhysicsMode::Mode2 => az_physics::ContinuousCollisionMode::Mode2,
            ContinuousPhysicsMode::OrderedTimeOfImpact => {
                az_physics::ContinuousCollisionMode::OrderedTimeOfImpact
            }
            ContinuousPhysicsMode::ReverseDisplacementSweep => {
                az_physics::ContinuousCollisionMode::ReverseDisplacementSweep
            }
        },
        continuous_prediction_distance: 0.05,
        // The native descriptor copies only the reflected mode. The authored
        // scalars remain editable and
        // round-trippable, while simulation uses descriptor defaults.
        continuous_distance_factor: 0.3,
        continuous_sphere_radius: 1.0,
        compute_inertia_tensor: component.configuration.auto_inertia_tensor,
        compute_mass: false,
        independent: motion_type.participates_in_unconstrained_substep_integration(),
        ..RuntimeRigidBodyConfiguration::default()
    }
}

#[allow(
    clippy::type_complexity,
    reason = "the query mirrors the two native shape-source branches"
)]
fn materialize_characters(
    mut commands: Commands,
    controllers: Query<(
        Entity,
        &CharacterControllerComponent,
        Option<&AzEntityId>,
        Option<&GlobalTransform>,
        Option<&PhysicsColliderSet>,
        Option<&ShapeAssetPhysicsBinding>,
        Option<&BodyDescriptor>,
    )>,
    entity_shapes: Query<(&AzEntityId, &PhysicsColliderSet)>,
) {
    for (entity, component, entity_id, transform, local_shapes, asset_binding, previous) in
        &controllers
    {
        let result = build_character_descriptor(
            component,
            entity_id,
            transform,
            local_shapes,
            asset_binding,
            &entity_shapes,
        );
        match result {
            Ok(descriptor) => {
                let mut entity_commands = commands.entity(entity);
                if previous != Some(&descriptor) {
                    entity_commands.insert(descriptor);
                }
                entity_commands
                    .insert(RockNRollCharacterProduct)
                    .remove::<RockNRollBodyError>();
            }
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(RockNRollBodyError(error))
                    .remove::<BodyDescriptor>()
                    .remove::<RockNRollCharacterProduct>();
            }
        }
    }
}

fn build_character_descriptor(
    component: &CharacterControllerComponent,
    entity_id: Option<&AzEntityId>,
    transform: Option<&GlobalTransform>,
    local_shapes: Option<&PhysicsColliderSet>,
    asset_binding: Option<&ShapeAssetPhysicsBinding>,
    entity_shapes: &Query<(&AzEntityId, &PhysicsColliderSet)>,
) -> Result<BodyDescriptor, String> {
    let source = match &component.configuration.shape {
        CharacterControllerShapeSource::Asset(asset) => {
            if asset.is_empty() {
                return Err("RockNRoll character has an empty shape asset".to_owned());
            }
            if asset_binding.is_none_or(|binding| binding.asset_id() != asset.id()) {
                return Err("RockNRoll character shape asset is not loaded".to_owned());
            }
            local_shapes
                .ok_or_else(|| "RockNRoll shape asset has no collider product".to_owned())?
        }
        CharacterControllerShapeSource::Entity(source_id) => {
            if entity_id.is_some_and(|entity_id| entity_id == source_id) {
                return Err("RockNRoll character cannot source its shape from itself".to_owned());
            }
            entity_shapes
                .iter()
                .find_map(|(candidate, shapes)| (candidate == source_id).then_some(shapes))
                .ok_or_else(|| {
                    format!(
                        "RockNRoll character shape entity {} is unresolved",
                        source_id.value()
                    )
                })?
        }
    };
    let (scale, pose) = transform_parts(transform);
    let mut colliders = source.clone();
    scale_physics_colliders(&mut colliders, scale).map_err(|error| error.to_string())?;
    Ok(BodyDescriptor {
        entity_id: entity_id.map(|id| PhysicsEntityId(id.value())),
        pose,
        kind: BodyKind::Character(CharacterBodyConfiguration {
            up_direction: component.configuration.character.up_direction.normalize(),
            max_slope: component.configuration.character.max_slope,
            contact_distance: component.configuration.character.contact_distance,
            solver_max_iterations: component.configuration.character.solver_max_iterations,
            asynchronous: component.configuration.character.asynchronous,
            ..CharacterBodyConfiguration::default()
        }),
        colliders: colliders.0,
    })
}

fn transform_parts(transform: Option<&GlobalTransform>) -> (Vec3, PhysicsPose) {
    transform.map_or((Vec3::ONE, PhysicsPose::IDENTITY), |transform| {
        let (scale, rotation, translation) = transform.to_scale_rotation_translation();
        (
            scale,
            PhysicsPose {
                translation,
                rotation,
            },
        )
    })
}

/// One body's sleep verdict for this step, keyed by handle while islands are
/// walked.
#[derive(Clone, Copy)]
struct Candidate {
    entity: Entity,
    awake: bool,
    eligible: bool,
}

// Bevy system: `Res` is an owned parameter wrapper, so borrowing it here would
// stop this function satisfying `IntoSystem` and it could not be registered.
#[allow(clippy::needless_pass_by_value)]
fn evaluate_native_sleep(
    step: Option<Res<PhysicsStepReport>>,
    mut physics: Option<ResMut<PhysicsWorld>>,
    listeners: Res<crate::SleepListeners>,
    mut bodies: Query<(
        Entity,
        &RigidBodyComponent,
        &PhysicsBodyHandle,
        &mut RockNRollSleepRuntime,
    )>,
) {
    let Some(step) = step else {
        return;
    };
    let Some(physics) = physics.as_deref_mut() else {
        return;
    };
    let Ok(time_step) = TimeStep::try_from(step.simulated_time) else {
        return;
    };
    let mut candidates = HashMap::new();
    for (entity, component, &body, mut sleep_runtime) in &mut bodies {
        let configuration = component.configuration.runtime_sleep_configuration();
        let Ok(status) = physics.body_status(body) else {
            continue;
        };

        if sleep_runtime.awake == Some(false) && status.awake {
            sleep_runtime.state.veto();
            listeners.notify_wake(entity, body);
        } else if sleep_runtime.awake == Some(true) && !status.awake {
            listeners.notify_sleep(entity, body);
        }
        sleep_runtime.awake = Some(status.awake);

        let mut eligible = status.awake
            && sleep_runtime
                .state
                .update(&status, configuration, time_step);
        if eligible && !listeners.can_sleep(entity, body, &status) {
            sleep_runtime.state.veto();
            eligible = false;
        }
        candidates.insert(
            body,
            Candidate {
                entity,
                awake: status.awake,
                eligible,
            },
        );
    }

    let mut visited = HashSet::new();
    let mut island = Vec::new();
    let mut slept = HashSet::new();
    for (&seed, seed_candidate) in &candidates {
        if !seed_candidate.eligible || !visited.insert(seed) {
            continue;
        }
        if physics.connected_bodies(seed, &mut island).is_err() {
            continue;
        }
        visited.extend(island.iter().copied());
        let all_eligible = island.iter().all(|member| {
            candidates.get(member).map_or_else(
                || {
                    physics
                        .body_status(*member)
                        .is_ok_and(|status| !status.awake)
                },
                |candidate| !candidate.awake || candidate.eligible,
            )
        });
        if !all_eligible {
            continue;
        }
        for &member in &island {
            let Some(candidate) = candidates.get(&member) else {
                continue;
            };
            if candidate.awake
                && physics
                    .apply_action(member, PhysicsAction::Wake(false))
                    .is_ok()
            {
                slept.insert(member);
                listeners.notify_sleep(candidate.entity, member);
            }
        }
    }

    for (_, _, &body, mut sleep_runtime) in &mut bodies {
        if slept.contains(&body) {
            sleep_runtime.awake = Some(false);
        }
    }
}

fn cleanup_removed_rigid_bodies(
    mut commands: Commands,
    mut removed: RemovedComponents<RigidBodyComponent>,
    products: Query<(), With<RockNRollRigidBodyProduct>>,
) {
    for entity in removed.read() {
        if products.contains(entity) {
            commands
                .entity(entity)
                .remove::<BodyDescriptor>()
                .remove::<RockNRollRigidBodyProduct>()
                .remove::<RockNRollSleepRuntime>()
                .remove::<RockNRollBodyError>();
        }
    }
}

fn cleanup_removed_characters(
    mut commands: Commands,
    mut removed: RemovedComponents<CharacterControllerComponent>,
    products: Query<(), With<RockNRollCharacterProduct>>,
) {
    for entity in removed.read() {
        if products.contains(entity) {
            commands
                .entity(entity)
                .remove::<BodyDescriptor>()
                .remove::<RockNRollCharacterProduct>()
                .remove::<RockNRollBodyError>();
        }
    }
}

pub fn apply_system_gravity(
    configuration: Query<&RockNRollSystemComponent, Changed<RockNRollSystemComponent>>,
    mut physics: Option<ResMut<PhysicsWorld>>,
) {
    let Some(physics) = physics.as_deref_mut() else {
        return;
    };
    for configuration in &configuration {
        physics.set_gravity_all(configuration.default_gravity);
    }
}

#[cfg(test)]
mod tests {
    use az_core::EntityId as AzEntityId;
    use az_physics::{
        ColliderConfiguration, ColliderShape, PhysicsBodyHandle, PhysicsColliderSet,
        PhysicsSceneId, PhysicsWorld, SimulationClass,
    };

    use super::*;
    use crate::{
        CharacterControllerConfig, CharacterControllerShapeSource, RockNRollAssetPlugin,
        RockNRollPlugin,
    };

    #[test]
    fn canonical_character_component_materializes_in_its_explicit_scene() {
        let scene = PhysicsSceneId::new(7);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .add_plugins((
                az_physics_rapier::RapierPhysicsPlugin::default(),
                RockNRollAssetPlugin,
                RockNRollPlugin,
            ));
        app.world_mut()
            .resource_mut::<PhysicsWorld>()
            .ensure_scene(scene);

        let shape_entity = AzEntityId::new(11);
        app.world_mut().spawn((
            shape_entity,
            PhysicsColliderSet(vec![ColliderConfiguration {
                shape: ColliderShape::CapsuleSegment {
                    endpoint_a: -Vec3::Z * 0.4,
                    endpoint_b: Vec3::Z * 0.4,
                    radius: 0.41,
                },
                ..ColliderConfiguration::default()
            }]),
        ));
        let actor = app
            .world_mut()
            .spawn((
                CharacterControllerComponent {
                    configuration: CharacterControllerConfig {
                        shape: CharacterControllerShapeSource::Entity(shape_entity),
                        ..CharacterControllerConfig::default()
                    },
                    ..CharacterControllerComponent::default()
                },
                scene,
            ))
            .id();

        app.update();

        let body = *app
            .world()
            .entity(actor)
            .get::<PhysicsBodyHandle>()
            .expect("character backend body");
        assert_eq!(body.scene(), scene);
        assert_eq!(
            app.world()
                .resource::<PhysicsWorld>()
                .body_status(body)
                .expect("character status")
                .simulation_class,
            SimulationClass::Living
        );
    }
}
