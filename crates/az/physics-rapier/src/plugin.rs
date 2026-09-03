use az_physics::{
    BodyDescriptor, PhysicsAction, PhysicsBackend, PhysicsBackendFactory, PhysicsBodyError,
    PhysicsBodyHandle, PhysicsInteraction, PhysicsSceneId, PhysicsSet, PhysicsStepConfiguration,
    PhysicsStepReport, PhysicsTimeStepSelector, PhysicsWorld,
};
use bevy::prelude::*;

use crate::RapierPhysicsBackend;

/// Schedule used for solver materialization, stepping, and writeback.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PhysicsSchedule {
    #[default]
    Update,
    FixedUpdate,
}

/// Installs the Rapier implementation behind Azoth's solver-neutral world.
#[derive(Debug, Clone, Copy)]
pub struct RapierPhysicsPlugin {
    gravity: Vec3,
    schedule: PhysicsSchedule,
}

impl RapierPhysicsPlugin {
    #[must_use]
    pub const fn new(gravity: Vec3) -> Self {
        Self {
            gravity,
            schedule: PhysicsSchedule::Update,
        }
    }

    #[must_use]
    pub const fn on_fixed_update(mut self) -> Self {
        self.schedule = PhysicsSchedule::FixedUpdate;
        self
    }
}

impl Default for RapierPhysicsPlugin {
    fn default() -> Self {
        Self::new(Vec3::new(0.0, 0.0, -9.81))
    }
}

#[derive(Debug, Clone, Copy)]
struct RapierBackendFactory {
    gravity: Vec3,
}

impl PhysicsBackendFactory for RapierBackendFactory {
    fn create(&self, scene: PhysicsSceneId) -> Box<dyn PhysicsBackend> {
        Box::new(RapierPhysicsBackend::in_scene(scene, self.gravity))
    }
}

impl Plugin for RapierPhysicsPlugin {
    fn build(&self, app: &mut App) {
        let mut world = PhysicsWorld::new(RapierBackendFactory {
            gravity: self.gravity,
        });
        world.ensure_scene(PhysicsSceneId::DEFAULT);
        app.insert_resource(world)
            .init_resource::<PhysicsStepConfiguration>()
            .init_resource::<PhysicsTimeStepSelector>()
            .init_resource::<PhysicsStepReport>()
            .add_message::<PhysicsInteraction>()
            .register_type::<PhysicsStepConfiguration>()
            .register_type::<PhysicsStepReport>()
            .register_type::<BodyDescriptor>()
            .register_type::<PhysicsBodyHandle>()
            .register_type::<PhysicsSceneId>();
        match self.schedule {
            PhysicsSchedule::Update => configure_schedule(app, Update),
            PhysicsSchedule::FixedUpdate => configure_schedule(app, FixedUpdate),
        }

        app.world_mut()
            .register_component_hooks::<PhysicsBodyHandle>()
            .on_remove(|mut world, context| {
                let Some(&body) = world.get::<PhysicsBodyHandle>(context.entity) else {
                    return;
                };
                let _ = world.resource_mut::<PhysicsWorld>().remove_body(body);
            });
    }
}

fn configure_schedule(app: &mut App, schedule: impl bevy::ecs::schedule::ScheduleLabel + Clone) {
    app.configure_sets(
        schedule.clone(),
        (
            PhysicsSet::Authoring,
            PhysicsSet::Materialization,
            PhysicsSet::PoseSync,
            PhysicsSet::Forces,
            PhysicsSet::Step,
            PhysicsSet::Interactions,
            PhysicsSet::Writeback,
        )
            .chain(),
    )
    .add_systems(
        schedule.clone(),
        (materialize_changed_bodies, cleanup_removed_descriptors)
            .chain()
            .in_set(PhysicsSet::Materialization),
    )
    .add_systems(
        schedule,
        (
            sync_authored_poses.in_set(PhysicsSet::PoseSync),
            step_world.in_set(PhysicsSet::Step),
            publish_interactions.in_set(PhysicsSet::Interactions),
            sync_simulated_poses.in_set(PhysicsSet::Writeback),
        )
            .chain(),
    );
}

/// Everything the materialization pass reads about one authored body.
type AuthoredBody = (
    Entity,
    Ref<'static, BodyDescriptor>,
    Option<&'static PhysicsSceneId>,
    Option<&'static PhysicsBodyHandle>,
);

fn materialize_changed_bodies(
    mut commands: Commands,
    mut world: ResMut<PhysicsWorld>,
    bodies: Query<AuthoredBody>,
) {
    for (entity, descriptor, scene, previous) in &bodies {
        let scene = scene.copied().unwrap_or_default();
        if !descriptor.is_changed() && previous.is_some_and(|body| body.scene() == scene) {
            continue;
        }
        match world.create_body(scene, &descriptor) {
            Ok(body) => {
                if let Some(&previous) = previous
                    && previous != body
                {
                    let _ = world.remove_body(previous);
                }
                commands
                    .entity(entity)
                    .insert(body)
                    .remove::<PhysicsBodyError>();
            }
            Err(error) => {
                commands.entity(entity).insert(PhysicsBodyError(error));
            }
        }
    }
}

fn cleanup_removed_descriptors(
    mut commands: Commands,
    mut removed: RemovedComponents<BodyDescriptor>,
    bodies: Query<(), With<PhysicsBodyHandle>>,
) {
    for entity in removed.read() {
        if bodies.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsBodyHandle>()
                .remove::<PhysicsBodyError>();
        }
    }
}

fn sync_authored_poses(
    mut world: ResMut<PhysicsWorld>,
    bodies: Query<(&PhysicsBodyHandle, &GlobalTransform), Changed<GlobalTransform>>,
) {
    for (&body, transform) in &bodies {
        let Ok(status) = world.body_status(body) else {
            continue;
        };
        if status.kinematic
            || matches!(status.simulation_class, az_physics::SimulationClass::Static)
        {
            let (_, rotation, translation) = transform.to_scale_rotation_translation();
            let _ = world.apply_action(
                body,
                PhysicsAction::SetPose(az_physics::PhysicsPose {
                    translation,
                    rotation,
                }),
            );
        }
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn step_world(
    time: Res<Time>,
    configuration: Res<PhysicsStepConfiguration>,
    mut selector: ResMut<PhysicsTimeStepSelector>,
    mut report: ResMut<PhysicsStepReport>,
    mut world: ResMut<PhysicsWorld>,
) {
    *report = PhysicsStepReport::default();
    if configuration.paused {
        return;
    }

    let requested_time_step = time.delta_secs();
    report.requested_time_step = requested_time_step;
    selector.reconfigure(*configuration);
    let Ok(substeps) = selector.substeps(requested_time_step) else {
        return;
    };
    for time_step in substeps {
        if world.step_all(time_step).is_err() {
            break;
        }
        report.simulated_time += time_step;
        report.substep_count += 1;
    }
}

fn publish_interactions(
    mut world: ResMut<PhysicsWorld>,
    mut interactions: MessageWriter<PhysicsInteraction>,
) {
    interactions.write_batch(world.drain_interactions());
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn sync_simulated_poses(
    world: Res<PhysicsWorld>,
    mut bodies: Query<(&PhysicsBodyHandle, &mut Transform)>,
) {
    for (&body, mut transform) in &mut bodies {
        let Ok(status) = world.body_status(body) else {
            continue;
        };
        if status.simulated
            && !matches!(status.simulation_class, az_physics::SimulationClass::Static)
        {
            transform.translation = status.pose.translation;
            transform.rotation = status.pose.rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use az_physics::{
        BodyDescriptor, BodyKind, ColliderConfiguration, ColliderShape, PhysicsEntityId,
        PhysicsPose, PhysicsSceneId,
    };
    use bevy::{math::Vec3, prelude::App};

    use super::{PhysicsWorld, RapierPhysicsPlugin};

    fn static_body(entity_id: u64) -> BodyDescriptor {
        BodyDescriptor {
            entity_id: Some(PhysicsEntityId(entity_id)),
            pose: PhysicsPose::IDENTITY,
            kind: BodyKind::Static { terrain: false },
            colliders: vec![ColliderConfiguration {
                shape: ColliderShape::Cuboid {
                    half_extents: Vec3::splat(0.5),
                },
                ..ColliderConfiguration::default()
            }],
        }
    }

    #[test]
    fn physics_world_keeps_identical_backend_handles_isolated_by_scene() {
        let mut app = App::new();
        app.add_plugins(RapierPhysicsPlugin::default());

        let primary = PhysicsSceneId::DEFAULT;
        let secondary = PhysicsSceneId::new(1);
        let mut world = app.world_mut().resource_mut::<PhysicsWorld>();
        world.ensure_scene(secondary);

        let primary_body = world
            .create_body(primary, &static_body(1))
            .expect("primary body");
        let secondary_body = world
            .create_body(secondary, &static_body(2))
            .expect("secondary body");

        assert_eq!(primary_body.get(), secondary_body.get());
        assert_ne!(primary_body, secondary_body);
        assert_eq!(primary_body.scene(), primary);
        assert_eq!(secondary_body.scene(), secondary);

        world
            .remove_body(primary_body)
            .expect("remove primary body");
        assert!(world.body_status(primary_body).is_err());
        assert!(world.body_status(secondary_body).is_ok());
    }
}
