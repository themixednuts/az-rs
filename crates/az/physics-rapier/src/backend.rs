use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
};

use az_physics::{
    AabbQuery, Axis3, BodyDescriptor, BodyKind, BodyStatus, BuoyancyStatus,
    CharacterBodyConfiguration, CharacterSupportInfo, CharacterSupportState, ColliderConfiguration,
    ColliderShape, ColliderTag, CollisionClass, CollisionFilter, ConstraintAxis,
    ConstraintAxisMask, ConstraintBreakReason, ConstraintCoupling, ConstraintDescriptor,
    ConstraintMotion, ConstraintSolverModel, ConstraintStatus, ConstraintTarget,
    DeformableTargetVertices, FluidAreaConfiguration, ImpulseAction, LinkedSoftBodyConfiguration,
    LinkedSoftBodyStatusRef, LivingBodyConfiguration, LivingDimensions, LivingDynamics,
    LivingMoveAction, LivingMoveMode, LivingStanceCheck, LivingStatus, OverlapHit,
    ParticleBodyConfiguration, ParticleFlags, ParticleStatus, PhysicalEntityType,
    PhysicalEntityTypes, PhysicsAction, PhysicsBackend, PhysicsBodyHandle,
    PhysicsConstraintBackend, PhysicsConstraintHandle, PhysicsEntityId, PhysicsError,
    PhysicsInteraction, PhysicsInteractionKind, PhysicsInteractionPhase,
    PhysicsLinkedSoftBodyBackend, PhysicsParticleBackend, PhysicsPose, PhysicsRopeBackend,
    PhysicsSceneId, PhysicsSoftBodyBackend, PhysicsVehicleBackend, RayCastConfiguration,
    RayCastHit, RayCastResult, RigidBodyBuoyancy, RigidBodyConfiguration, RigidBodyDampingModel,
    RigidBodyMotion, RigidBodySleepPolicy, RockNRollSleepMode, RopeBodyConfiguration, RopeFlags,
    RopeStatus, RopeVolumetricPressure, ShapeCastConfiguration,
    ShapeCastHit as PhysicsShapeCastHit, ShapeOverlapConfiguration, SimulationClass,
    SoftBodyAttachmentUpdate, SoftBodyConfiguration, SoftBodyImpulse, SoftBodyPressure,
    SoftBodySlice, SoftBodySliceResult, SoftBodyStatus, SpatialQueryFilter, SurfaceIndex,
    SyncLivingAction, VehicleAbilities, VehicleDriveAction, VehicleStatus,
    VehicleWheelConfiguration, VehicleWheelStatus, WheeledVehicleConfiguration,
    continuous_hit_projection,
};
use bevy_math::bounding::Aabb3d;
use glam::{Mat3, Quat, Vec3};
use rapier3d::{
    control::{
        CharacterCollision, CharacterLength, DynamicRayCastVehicleController,
        KinematicCharacterController, WheelTuning,
    },
    geometry::{Aabb, BoundingVolume},
    parry::query::{NonlinearRigidMotion, ShapeCastOptions, cast_shapes_nonlinear},
    prelude::*,
};
use smallvec::SmallVec;

use crate::buoyancy::{self, MediumMotion};
use crate::convert::{self, f32_from_i32, f32_from_usize, i32_from_usize, u32_from_usize};
use crate::deformable::{
    AttachmentFrame, DeformableContact, DeformableReaction, LinkedSoftBodyState, MediumSample,
    RopeState, SoftBodyState, deformable_bounds, solve_linked_soft_body_pair,
};

#[derive(Debug, Clone, Copy)]
struct ColliderMetadata {
    body: PhysicsBodyHandle,
    entity_id: Option<PhysicsEntityId>,
    query_type: PhysicalEntityTypes,
    surface_index: SurfaceIndex,
    surface_pierceability: u8,
    sensor: bool,
    simulated: bool,
    in_scene_queries: bool,
    tag: ColliderTag,
    rest_offset: f32,
    contact_offset: f32,
    collision_filter: Option<CollisionFilter>,
    continuous_collision_mode: az_physics::ContinuousCollisionMode,
    continuous_prediction_distance: f32,
}

impl ColliderMetadata {
    #[inline]
    const fn blocks_motion(self) -> bool {
        self.simulated && !self.sensor
    }

    #[inline]
    const fn participates_in_trigger_pairs(self) -> bool {
        self.simulated || self.sensor
    }

    #[inline]
    const fn positive_contact_range(self) -> f32 {
        let continuous_range = if self.continuous_collision_mode.uses_hit_projection()
            || self
                .continuous_collision_mode
                .uses_speculative_normal_constraints()
        {
            self.continuous_prediction_distance
        } else {
            0.0
        };
        self.contact_offset.max(continuous_range)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct InteractionKey {
    first: (u32, u32),
    second: (u32, u32),
    kind: PhysicsInteractionKind,
}

impl InteractionKey {
    fn new(
        first: ColliderHandle,
        second: ColliderHandle,
        kind: PhysicsInteractionKind,
    ) -> (Self, bool) {
        let first = first.into_raw_parts();
        let second = second.into_raw_parts();
        if first <= second {
            (
                Self {
                    first,
                    second,
                    kind,
                },
                false,
            )
        } else {
            (
                Self {
                    first: second,
                    second: first,
                    kind,
                },
                true,
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ColliderOwner {
    body: PhysicsBodyHandle,
    rigid_body: RigidBodyHandle,
    entity_id: Option<PhysicsEntityId>,
    query_type: PhysicalEntityTypes,
    continuous_collision_mode: az_physics::ContinuousCollisionMode,
    continuous_prediction_distance: f32,
}

#[derive(Debug, Clone)]
struct NativeBody {
    rigid_body: RigidBodyHandle,
    entity_id: Option<PhysicsEntityId>,
    physical_type: PhysicalEntityType,
    query_type: PhysicalEntityTypes,
    rigid_configuration: Option<RigidBodyConfiguration>,
    collider_configurations: Vec<ColliderConfiguration>,
    linear_acceleration: Vec3,
    angular_acceleration: Vec3,
    /// Pose at the beginning of the current substep. `RockNRoll` keeps the same
    /// history for continuous pair casts, modes 1/2 hit projection, and the
    /// mode-4 broadphase proxy.
    previous_pose: PhysicsPose,
    rock_n_roll_sleep_eligible_time: f32,
    rock_n_roll_smoothed_linear_speed_squared: f32,
    rock_n_roll_smoothed_angular_speed_squared: f32,
    rock_n_roll_smoothed_energy: f32,
}

#[derive(Debug, Clone)]
struct NativeConstraint {
    descriptor: ConstraintDescriptor,
    joint: Option<NativeConstraintHandle>,
    world_anchor: Option<RigidBodyHandle>,
    broken: bool,
    break_reason: Option<ConstraintBreakReason>,
    linear_impulse: Vec3,
    angular_impulse: Vec3,
}

#[derive(Debug, Clone, Copy)]
enum NativeConstraintHandle {
    Impulse(ImpulseJointHandle),
    ReducedCoordinate(MultibodyJointHandle),
}

/// Which bodies a deformable sweep is allowed to hit.
#[derive(Debug, Clone, Copy)]
struct DeformableFilter<'a> {
    physical_entity_types: PhysicalEntityTypes,
    collision_class: CollisionClass,
    ignored_bodies: &'a [PhysicsBodyHandle],
}

/// The moving character, shape, and exclusions of one character shape cast.
#[derive(Clone, Copy)]
struct CharacterSweep<'a> {
    body: PhysicsBodyHandle,
    native_body: RigidBodyHandle,
    collision_class: CollisionClass,
    pose: &'a Pose,
    shape: &'a dyn Shape,
    excluded: &'a [ColliderHandle],
}

/// The instantaneous motion one `RockNRoll` sleeping condition is tested
/// against.
#[derive(Debug, Clone, Copy)]
struct RockNRollMotion {
    linear_speed_squared: f32,
    angular_speed_squared: f32,
    energy: f32,
    mass: f32,
}

/// Evaluates one `RockNRoll` sleeping-condition mode, advancing the body's
/// smoothed speed and energy trackers as the native evaluator does.
fn rock_n_roll_sleep_condition(
    native: &mut NativeBody,
    mode: RockNRollSleepMode,
    configuration: RigidBodyConfiguration,
    motion: RockNRollMotion,
    time_step: f32,
) -> bool {
    let RockNRollMotion {
        linear_speed_squared,
        angular_speed_squared,
        energy,
        mass,
    } = motion;
    match mode {
        RockNRollSleepMode::Disabled => {
            native.rock_n_roll_sleep_eligible_time = 0.0;
            false
        }
        RockNRollSleepMode::InstantaneousVelocity => {
            if linear_speed_squared == 0.0 && angular_speed_squared == 0.0 {
                native.rock_n_roll_sleep_eligible_time =
                    3.0f32.mul_add(time_step, native.rock_n_roll_sleep_eligible_time);
            }
            linear_speed_squared < configuration.sleep_linear_velocity_threshold.powi(2)
                && angular_speed_squared < configuration.sleep_angular_velocity_threshold.powi(2)
        }
        RockNRollSleepMode::SmoothedVelocity => {
            native.rock_n_roll_smoothed_linear_speed_squared = 0.9f32.mul_add(
                native.rock_n_roll_smoothed_linear_speed_squared,
                0.1 * linear_speed_squared,
            );
            native.rock_n_roll_smoothed_angular_speed_squared = 0.9f32.mul_add(
                native.rock_n_roll_smoothed_angular_speed_squared,
                0.1 * angular_speed_squared,
            );
            if native.rock_n_roll_smoothed_linear_speed_squared == 0.0
                && native.rock_n_roll_smoothed_angular_speed_squared == 0.0
            {
                native.rock_n_roll_sleep_eligible_time =
                    3.0f32.mul_add(time_step, native.rock_n_roll_sleep_eligible_time);
            }
            native.rock_n_roll_smoothed_linear_speed_squared
                < configuration.sleep_linear_velocity_threshold.powi(2)
                && native.rock_n_roll_smoothed_angular_speed_squared
                    < configuration.sleep_angular_velocity_threshold.powi(2)
        }
        RockNRollSleepMode::InstantaneousEnergy => {
            if energy == 0.0 {
                native.rock_n_roll_sleep_eligible_time =
                    3.0f32.mul_add(time_step, native.rock_n_roll_sleep_eligible_time);
            }
            energy < configuration.sleep_min_energy * mass
        }
        RockNRollSleepMode::SmoothedEnergy => {
            native.rock_n_roll_smoothed_energy =
                0.9f32.mul_add(native.rock_n_roll_smoothed_energy, 0.1 * energy);
            if native.rock_n_roll_smoothed_energy == 0.0 {
                native.rock_n_roll_sleep_eligible_time =
                    3.0f32.mul_add(time_step, native.rock_n_roll_sleep_eligible_time);
            }
            native.rock_n_roll_smoothed_energy < configuration.sleep_min_energy * mass
        }
    }
}

/// What a body's retained contact pairs say about how well it is supported.
#[derive(Debug, Clone, Copy)]
struct CryContactSupport {
    pair_count: usize,
    contact_count: usize,
    support_bodies: usize,
    awake_dynamic_support: bool,
}

/// The solved motion of a character body for one step.
#[derive(Debug, Clone, Copy)]
struct CharacterMotion {
    rigid_pose: Pose,
    translation: Vec3,
    time_step: f32,
}

/// One collider's share of a nonlinear continuous pair cast.
#[derive(Debug, Clone, Copy)]
struct ContinuousCast<'a> {
    motion: &'a NonlinearRigidMotion,
    collider_start: Pose,
    collider_end: Pose,
    witness: Vector,
    local_normal: Vector,
    hit_fraction: f32,
}

#[derive(Debug, Clone, Copy)]
struct HitProjectionRecord {
    rigid_body: RigidBodyHandle,
    previous_pose: PhysicsPose,
    current_pose: PhysicsPose,
    pose_fraction: f32,
    normal: Vector,
    normal_velocity_retained_fraction: f32,
}

#[derive(Debug, Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "these are independent Cry living-entity status flags, not one state enum"
)]
struct LivingState {
    configuration: LivingBodyConfiguration,
    primary_collider: ColliderHandle,
    velocity: Vec3,
    unconstrained_velocity: Vec3,
    requested_velocity: Vec3,
    force_flight: bool,
    jump_requested: bool,
    flying: bool,
    time_flying: f32,
    requested_time_step: Option<f32>,
    time_force_inertia: f32,
    ground_height: f32,
    ground_slope: Vec3,
    ground_velocity: Vec3,
    ground_surface: Option<SurfaceIndex>,
    ground_body: Option<PhysicsBodyHandle>,
    time_since_stance_change: f32,
    camera_vertical_offset: f32,
    camera_offset_speed: f32,
    camera_offset_acceleration: f32,
    stable_height_time: f32,
    stuck: bool,
    squashed: bool,
}

impl LivingState {
    const fn new(configuration: LivingBodyConfiguration, primary_collider: ColliderHandle) -> Self {
        Self {
            configuration,
            primary_collider,
            velocity: Vec3::ZERO,
            unconstrained_velocity: Vec3::ZERO,
            requested_velocity: Vec3::ZERO,
            force_flight: false,
            jump_requested: false,
            flying: true,
            time_flying: 0.0,
            requested_time_step: None,
            time_force_inertia: 0.0,
            ground_height: 0.0,
            ground_slope: Vec3::Z,
            ground_velocity: Vec3::ZERO,
            ground_surface: None,
            ground_body: None,
            time_since_stance_change: 0.0,
            camera_vertical_offset: 0.0,
            camera_offset_speed: 0.0,
            camera_offset_acceleration: 0.0,
            stable_height_time: 1.0,
            stuck: false,
            squashed: false,
        }
    }
}

#[derive(Debug, Clone)]
struct CharacterState {
    configuration: CharacterBodyConfiguration,
    primary_collider: ColliderHandle,
    velocity: Vec3,
    requested_velocity: Vec3,
    flying: bool,
    time_flying: f32,
    ground_surface: Option<SurfaceIndex>,
    ground_body: Option<PhysicsBodyHandle>,
    support: CharacterSupportInfo,
}

impl CharacterState {
    fn new(configuration: CharacterBodyConfiguration, primary_collider: ColliderHandle) -> Self {
        Self {
            configuration,
            primary_collider,
            velocity: Vec3::ZERO,
            requested_velocity: Vec3::ZERO,
            flying: true,
            time_flying: 0.0,
            ground_surface: None,
            ground_body: None,
            support: CharacterSupportInfo::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CharacterContactPlane {
    normal: Vec3,
    velocity: Vec3,
    point: Vec3,
    distance: f32,
    body: PhysicsBodyHandle,
    entity_id: Option<PhysicsEntityId>,
    surface_index: SurfaceIndex,
}

#[derive(Debug, Clone)]
struct ParticleState {
    configuration: ParticleBodyConfiguration,
    primary_collider: ColliderHandle,
    gravity: Vec3,
    water_gravity: Vec3,
    heading: Vec3,
    velocity: Vec3,
    angular_velocity: Vec3,
    spin_orientation: Quat,
    lift_per_speed: f32,
    sliding: bool,
    slide_normal: Vec3,
    submerged_depth: f32,
    medium_velocity: Vec3,
    force_awake: u8,
    time_force_awake: f32,
    sleep_time: f32,
    collision_pending: bool,
    recent_collisions: u8,
    area_step_count: u8,
}

impl ParticleState {
    fn new(
        configuration: ParticleBodyConfiguration,
        primary_collider: ColliderHandle,
        scene_gravity: Vec3,
    ) -> Self {
        let gravity = configuration.gravity.unwrap_or(scene_gravity);
        let velocity = configuration.heading * configuration.speed;
        Self {
            configuration,
            primary_collider,
            gravity,
            water_gravity: configuration.water_gravity.unwrap_or(gravity * 0.8),
            heading: configuration.heading,
            velocity,
            angular_velocity: configuration.angular_velocity,
            spin_orientation: configuration.initial_orientation,
            lift_per_speed: if configuration.speed == 0.0 {
                0.0
            } else {
                configuration.lift_acceleration / configuration.speed.abs()
            },
            sliding: false,
            slide_normal: Vec3::Z,
            submerged_depth: 0.0,
            medium_velocity: Vec3::ZERO,
            force_awake: if velocity.length_squared() > 0.0 {
                1
            } else {
                2
            },
            time_force_awake: 0.0,
            sleep_time: 0.0,
            collision_pending: false,
            recent_collisions: 0,
            area_step_count: 0,
        }
    }

    fn acceleration(&self) -> Vec3 {
        let (gravity, resistance) = if self.submerged_depth < 0.0 {
            (
                self.water_gravity
                    * (-self.submerged_depth / (self.configuration.size * 0.5)).min(1.0),
                self.configuration.water_resistance,
            )
        } else {
            (self.gravity, self.configuration.air_resistance)
        };
        gravity
            + self.heading * self.configuration.thrust_acceleration
            + (self.medium_velocity - self.velocity) * resistance
            + particle_lift(self.heading, self.gravity)
                * (self.lift_per_speed * self.velocity.length())
    }

    fn is_awake(&self, position: Vec3) -> bool {
        let gravity = if self.submerged_depth < 0.0 {
            self.water_gravity
        } else {
            self.gravity
        };
        position.z > -1_000.0
            && self.force_awake != 0
            && (self.force_awake == 1
                || self.velocity.length_squared()
                    > self.configuration.minimum_speed * self.configuration.minimum_speed
                || (self.recent_collisions == 0 && !self.sliding && gravity.length_squared() > 0.0))
    }
}

#[derive(Debug, Clone, Copy)]
struct VehicleWheelContact {
    collider: ColliderHandle,
    point: Vector,
    normal: Vector,
    suspension_length: f32,
}

#[derive(Debug, Clone, Copy)]
struct VehicleWheelRuntime {
    angular_velocity: f32,
    previous_rotation: f32,
    torque: f32,
    slipping: bool,
    slip_velocity: Vec3,
    friction: f32,
    spring_stiffness: f32,
    spring_damping: f32,
    contact: Option<VehicleWheelContact>,
}

impl VehicleWheelRuntime {
    const fn new(angular_velocity: f32, spring_stiffness: f32, spring_damping: f32) -> Self {
        Self {
            angular_velocity,
            previous_rotation: 0.0,
            torque: 0.0,
            slipping: false,
            slip_velocity: Vec3::ZERO,
            friction: 0.0,
            spring_stiffness,
            spring_damping,
            contact: None,
        }
    }
}

#[derive(Debug, Clone)]
struct VehicleState {
    configuration: WheeledVehicleConfiguration,
    controller: DynamicRayCastVehicleController,
    wheels: Vec<VehicleWheelRuntime>,
    pedal: f32,
    steer: f32,
    ackermann_offset: f32,
    hand_brake: bool,
    clutch: f32,
    current_gear: i32,
    engine_angular_velocity: f32,
    driving_torque: f32,
    active_colliders: u32,
    time_without_chassis_contacts: f32,
    has_chassis_contacts: bool,
}

impl VehicleState {
    /// Current gear as an index into the authored gear-ratio table.
    ///
    /// Every path that writes `current_gear` clamps it into
    /// `0..gear_ratios.len()` first, so a negative gear cannot reach here.
    fn gear_index(&self) -> usize {
        usize::try_from(self.current_gear).unwrap_or(0)
    }

    fn apply_drive(&mut self, action: VehicleDriveAction) -> Result<(), PhysicsError> {
        action.validate()?;
        if let Some(delta) = action.pedal_delta {
            self.pedal = (self.pedal + delta).clamp(-1.0, 1.0);
        }
        if let Some(pedal) = action.pedal {
            self.pedal = pedal;
        }
        if let Some(clutch) = action.clutch {
            self.clutch = clutch;
        }
        if let Some(gear) = action.gear {
            if !usize::try_from(gear).is_ok_and(|gear| gear < self.configuration.gear_ratios.len())
            {
                return Err(PhysicsError::InvalidVehicleConfiguration {
                    field: "drive gear",
                });
            }
            self.current_gear = gear;
        }

        let steering_changed = action.steer.is_some() || action.steer_delta.is_some();
        if let Some(steer) = action.steer {
            self.steer = steer;
        }
        if let Some(delta) = action.steer_delta {
            self.steer += delta;
        }
        if let Some(offset) = action.ackermann_offset {
            self.ackermann_offset = offset;
        }
        self.steer = self.steer.clamp(
            -self.configuration.maximum_steer,
            self.configuration.maximum_steer,
        );
        if steering_changed {
            self.update_steering();
        }
        if let Some(hand_brake) = action.hand_brake {
            self.hand_brake = hand_brake;
        }
        Ok(())
    }

    fn update_steering(&mut self) {
        if self.configuration.tracked_neutral_turn_steer != 0.0 {
            for wheel in self.controller.wheels_mut() {
                wheel.steering = 0.0;
            }
            return;
        }
        if self.steer == 0.0 {
            for wheel in self.controller.wheels_mut() {
                wheel.steering = 0.0;
            }
            return;
        }

        let mut maximum_x = 0.0_f32;
        let mut maximum_y = -10.0_f32;
        let mut minimum_y = 10.0_f32;
        for wheel in &self.configuration.wheels {
            if wheel.axle >= 0 {
                minimum_y = minimum_y.min(wheel.connection.y);
                maximum_y = maximum_y.max(wheel.connection.y);
                maximum_x = maximum_x.max(wheel.connection.x.abs());
            }
        }
        let ackermann_line = self
            .ackermann_offset
            .mul_add(maximum_y - minimum_y, minimum_y);
        let longitudinal_extent = (maximum_y - ackermann_line).max(ackermann_line - minimum_y);
        if longitudinal_extent <= 0.01 {
            return;
        }
        let tangent = self.steer.abs().tan();
        let direction = sign_nonzero(self.steer);
        for (wheel, controller_wheel) in self
            .configuration
            .wheels
            .iter()
            .zip(self.controller.wheels_mut())
        {
            controller_wheel.steering = if wheel.axle >= 0 && wheel.can_steer {
                let y = wheel.connection.y - ackermann_line;
                direction
                    * (y * tangent
                        / tangent.mul_add(
                            wheel.connection.x.mul_add(-direction, maximum_x),
                            longitudinal_extent,
                        ))
                    .atan()
            } else {
                0.0
            };
        }
    }

    /// Fastest driven wheel speed at the current gear, together with the total
    /// torque share of the driven wheels.
    ///
    /// A tracked chassis scales one side's wheels by its neutral-turn ratio, so
    /// a neutral turn still reports a sensible engine speed.
    fn driven_wheel_speed(&self) -> (f32, f32) {
        let gear_ratio = self.configuration.gear_ratios[self.gear_index()];
        let mut side_speed_scale = [1.0_f32; 2];
        let tracked_scale = if self.configuration.tracked_neutral_turn_steer == 0.0 {
            0.0
        } else {
            2.0 / self.configuration.tracked_neutral_turn_steer
        };
        if tracked_scale != 0.0 && self.steer.abs() > 0.01 {
            let side = usize::from(self.steer < 0.0);
            let scale = (1.0 - (self.steer * tracked_scale).abs()).max(-1.0);
            side_speed_scale[side] = if scale.abs() < 0.05 {
                sign_nonzero(scale) * 0.05
            } else {
                scale
            }
            .recip();
        }

        let mut wheel_speed = 0.0_f32;
        let mut torque_scale_sum = 0.0_f32;
        for (wheel, runtime) in self.configuration.wheels.iter().zip(&self.wheels) {
            if wheel.axle >= 0 && wheel.driving {
                let speed = runtime.angular_velocity
                    * side_speed_scale[usize::from(wheel.connection.x < 0.0)]
                    * wheel.torque_scale;
                wheel_speed = wheel_speed.max(speed * gear_ratio);
                torque_scale_sum += wheel.torque_scale;
            }
        }
        (wheel_speed, torque_scale_sum)
    }

    /// Pulls the engine speed toward the driven wheels while the clutch closes,
    /// and drops back to neutral idle when the engine stalls below its minimum.
    fn engage_clutch(
        &mut self,
        wheel_speed: f32,
        minimum_speed: f32,
        idle_speed: f32,
        time_step: f32,
    ) {
        if self.current_gear != 1 {
            if self.clutch > 0.0 {
                self.engine_angular_velocity = (wheel_speed - self.engine_angular_velocity)
                    .mul_add(
                        self.configuration.clutch_speed * 2.0 * time_step,
                        self.engine_angular_velocity,
                    );
            }
            if self.engine_angular_velocity.abs() > minimum_speed {
                self.clutch = time_step
                    .mul_add(self.configuration.clutch_speed, self.clutch)
                    .min(1.0);
                if self.clutch >= 1.0 {
                    self.engine_angular_velocity = wheel_speed;
                }
            }
        }

        if self.clutch > 0.0 && self.engine_angular_velocity < minimum_speed {
            if self.pedal * f32_from_i32(self.current_gear - 1) <= 0.0 {
                self.current_gear = 1;
                self.clutch = 0.0;
            }
            self.engine_angular_velocity = idle_speed;
        }
    }

    /// Picks the gear for this step: direction changes out of neutral, then the
    /// shift-up and shift-down bands, then the authored gear clamps.
    fn select_gear(&mut self, wheel_speed: f32, idle_speed: f32) {
        let mut new_gear = self.current_gear;
        let contacts = self
            .wheels
            .iter()
            .filter(|wheel| wheel.contact.is_some())
            .count();
        if self.pedal != 0.0
            && self.current_gear == 1
            && wheel_speed * -sign_nonzero(self.pedal)
                < rpm_to_angular(self.configuration.gear_direction_switch_rpm)
        {
            new_gear = if self.pedal < 0.0 { 0 } else { 2 };
            self.engine_angular_velocity = rpm_to_angular(self.configuration.engine_start_rpm);
        } else if self.clutch > 0.99 && self.current_gear > 1 && contacts > 1 {
            if self.engine_angular_velocity > rpm_to_angular(self.configuration.engine_shift_up_rpm)
            {
                new_gear = (new_gear + 1).min(i32_from_usize(
                    self.configuration.gear_ratios.len().saturating_sub(1),
                ));
            } else if self.engine_angular_velocity
                < rpm_to_angular(self.configuration.engine_shift_down_rpm)
                && (self.pedal <= 0.0 || self.current_gear > 2)
            {
                new_gear = (new_gear - 1).max(1);
            }
        }
        new_gear = new_gear
            .clamp(
                self.configuration.minimum_gear,
                self.configuration.maximum_gear,
            )
            .clamp(
                0,
                i32_from_usize(self.configuration.gear_ratios.len().saturating_sub(1)),
            );
        if new_gear != self.current_gear {
            self.clutch = 0.0;
            if new_gear == 1 {
                self.engine_angular_velocity = idle_speed;
            }
        }
        self.current_gear = new_gear;
    }

    /// Direction a tracked chassis pulls along: the authored forward axis,
    /// tilted by `pull_tilt` while the vehicle drives forward.
    fn pull_direction(&self, chassis_pose: Pose) -> Vector {
        let local_pull = if self.pedal >= 0.0 {
            Vec3::new(
                0.0,
                self.configuration.pull_tilt.cos(),
                -self.configuration.pull_tilt.sin(),
            )
        } else {
            Vec3::Y
        };
        convert::vector(convert::physics_pose(&chassis_pose).rotation * local_pull)
            .normalize_or_zero()
    }

    fn compute_driving_torque(&mut self, time_step: f32) -> f32 {
        if self.configuration.gear_ratios.is_empty() {
            self.driving_torque = 0.0;
            return 0.0;
        }
        self.current_gear = self.current_gear.clamp(
            0,
            i32_from_usize(self.configuration.gear_ratios.len().saturating_sub(1)),
        );
        let (wheel_speed, torque_scale_sum) = self.driven_wheel_speed();
        let reciprocal_torque_scale = torque_scale_sum.max(0.001).recip();
        let engine_power = self.configuration.engine_power * reciprocal_torque_scale;
        let minimum_speed = rpm_to_angular(self.configuration.engine_minimum_rpm);
        let idle_speed = rpm_to_angular(self.configuration.engine_idle_rpm);
        let maximum_speed = rpm_to_angular(self.configuration.engine_maximum_rpm);

        self.engage_clutch(wheel_speed, minimum_speed, idle_speed, time_step);
        self.select_gear(wheel_speed, idle_speed);

        let mut torque = 0.0;
        if self.pedal * sign_zero(f32_from_i32(self.current_gear - 1)) > 0.0 {
            let gear_ratio = self.configuration.gear_ratios[self.gear_index()];
            if self.engine_angular_velocity > 0.1 {
                let power = ((self.engine_angular_velocity.min(maximum_speed * 1.5)
                    / maximum_speed)
                    * core::f32::consts::PI)
                    .sin()
                    * engine_power
                    * self.pedal.abs();
                torque = power / self.engine_angular_velocity;
            } else {
                torque = self.pedal.abs() * engine_power * (core::f32::consts::PI / maximum_speed);
            }
            torque *= gear_ratio * self.clutch;
        }
        self.driving_torque = torque;
        torque
    }

    fn configure_wheel_forces(&mut self, time_step: f32) {
        let driving_torque = self.compute_driving_torque(time_step);
        let maximum_engine_speed = rpm_to_angular(self.configuration.engine_maximum_rpm);
        let tracked_scale = if self.configuration.tracked_neutral_turn_steer == 0.0 {
            0.0
        } else {
            2.0 / self.configuration.tracked_neutral_turn_steer
        };
        for ((configuration, runtime), wheel) in self
            .configuration
            .wheels
            .iter()
            .zip(&mut self.wheels)
            .zip(self.controller.wheels_mut())
        {
            let mut torque =
                -sign_zero(runtime.angular_velocity) * self.configuration.axle_friction;
            if self.pedal * sign_zero(f32_from_i32(self.current_gear - 1)) <= 0.0 {
                torque = self.configuration.brake_torque.mul_add(self.pedal, torque);
            }
            if configuration.driving {
                let mut drive = driving_torque;
                if tracked_scale != 0.0
                    && self.steer.abs() > 0.01
                    && self.steer * configuration.connection.x > 0.0
                {
                    if runtime.angular_velocity * self.pedal >= 0.0 {
                        drive = sign_nonzero(driving_torque)
                            * self.configuration.brake_torque.max(driving_torque.abs());
                    }
                    drive *= (1.0 - (self.steer * tracked_scale).abs()).max(-1.0);
                    drive *= sign_zero(maximum_engine_speed - self.engine_angular_velocity);
                }
                torque = drive.mul_add(configuration.torque_scale, torque);
            }
            runtime.torque = torque;
            let locked = (self.hand_brake && configuration.can_brake) || configuration.blocked;
            wheel.engine_force = if locked {
                0.0
            } else {
                torque / configuration.radius
            };
            wheel.brake = if locked {
                f32::MAX
            } else if torque * runtime.angular_velocity < 0.0 {
                torque.abs() * time_step / configuration.radius
            } else {
                0.0
            };
        }
    }
}

/// Direct Rapier implementation of Azoth's physics scene contract.
///
/// This intentionally uses Rapier without `bevy_rapier3d`: the adapter owns no
/// Bevy ECS policy and can be driven identically by a server, client, test, or
/// an ECS synchronization plugin.
pub struct RapierPhysicsBackend {
    scene: PhysicsSceneId,
    pipeline: PhysicsPipeline,
    gravity: Vec3,
    integration_parameters: IntegrationParameters,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    query_broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    rigid_bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    next_body_handle: u64,
    next_constraint_handle: u64,
    bodies: HashMap<PhysicsBodyHandle, NativeBody>,
    constraints: HashMap<PhysicsConstraintHandle, NativeConstraint>,
    collider_metadata: HashMap<ColliderHandle, ColliderMetadata>,
    living: HashMap<PhysicsBodyHandle, LivingState>,
    characters: HashMap<PhysicsBodyHandle, CharacterState>,
    particles: HashMap<PhysicsBodyHandle, ParticleState>,
    ropes: HashMap<PhysicsBodyHandle, RopeState>,
    soft_bodies: HashMap<PhysicsBodyHandle, SoftBodyState>,
    linked_soft_bodies: HashMap<PhysicsBodyHandle, LinkedSoftBodyState>,
    vehicles: HashMap<PhysicsBodyHandle, VehicleState>,
    active_interactions: HashMap<InteractionKey, PhysicsInteraction>,
    pending_interactions: Vec<PhysicsInteraction>,
    fluid_areas: HashMap<PhysicsBodyHandle, FluidAreaConfiguration>,
    buoyancy_status: HashMap<PhysicsBodyHandle, BuoyancyStatus>,
    physics_time: f32,
}

impl std::fmt::Debug for RapierPhysicsBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RapierPhysicsBackend")
            .field("scene", &self.scene)
            .field("gravity", &self.gravity)
            .field("body_count", &self.bodies.len())
            .field("collider_count", &self.collider_metadata.len())
            .field("living_body_count", &self.living.len())
            .field("character_body_count", &self.characters.len())
            .field("rope_body_count", &self.ropes.len())
            .field("soft_body_count", &self.soft_bodies.len())
            .field("linked_soft_body_count", &self.linked_soft_bodies.len())
            .field("vehicle_body_count", &self.vehicles.len())
            .finish_non_exhaustive()
    }
}

impl Default for RapierPhysicsBackend {
    fn default() -> Self {
        let integration_parameters = IntegrationParameters {
            max_ccd_substeps: 20,
            ..IntegrationParameters::default()
        };
        Self {
            scene: PhysicsSceneId::DEFAULT,
            pipeline: PhysicsPipeline::new(),
            gravity: Vec3::new(0.0, 0.0, -9.81),
            integration_parameters,
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            query_broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            rigid_bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            next_body_handle: 1,
            next_constraint_handle: 1,
            bodies: HashMap::new(),
            constraints: HashMap::new(),
            collider_metadata: HashMap::new(),
            living: HashMap::new(),
            characters: HashMap::new(),
            particles: HashMap::new(),
            ropes: HashMap::new(),
            soft_bodies: HashMap::new(),
            linked_soft_bodies: HashMap::new(),
            vehicles: HashMap::new(),
            active_interactions: HashMap::new(),
            pending_interactions: Vec::new(),
            fluid_areas: HashMap::new(),
            buoyancy_status: HashMap::new(),
            physics_time: 0.0,
        }
    }
}

impl RapierPhysicsBackend {
    const ROCK_N_ROLL_BROADPHASE_MARGIN: f32 = 0.05;

    #[must_use]
    pub fn new(gravity: Vec3) -> Self {
        Self::in_scene(PhysicsSceneId::DEFAULT, gravity)
    }

    #[must_use]
    pub fn in_scene(scene: PhysicsSceneId, gravity: Vec3) -> Self {
        Self {
            scene,
            gravity,
            ..Self::default()
        }
    }

    fn allocate_body_handle(&mut self) -> Result<PhysicsBodyHandle, PhysicsError> {
        let value =
            NonZeroU64::new(self.next_body_handle).ok_or(PhysicsError::BodyHandleExhausted)?;
        self.next_body_handle = self
            .next_body_handle
            .checked_add(1)
            .ok_or(PhysicsError::BodyHandleExhausted)?;
        Ok(PhysicsBodyHandle::in_scene(self.scene, value))
    }

    fn allocate_constraint_handle(&mut self) -> Result<PhysicsConstraintHandle, PhysicsError> {
        let value = NonZeroU64::new(self.next_constraint_handle)
            .ok_or(PhysicsError::ConstraintHandleExhausted)?;
        self.next_constraint_handle = self
            .next_constraint_handle
            .checked_add(1)
            .ok_or(PhysicsError::ConstraintHandleExhausted)?;
        Ok(PhysicsConstraintHandle::in_scene(self.scene, value))
    }

    fn native_body(&self, body: PhysicsBodyHandle) -> Result<&NativeBody, PhysicsError> {
        self.bodies
            .get(&body)
            .ok_or(PhysicsError::BodyNotFound(body))
    }

    fn rigid_body_mut(&mut self, body: PhysicsBodyHandle) -> Result<&mut RigidBody, PhysicsError> {
        let native = self.native_body(body)?.rigid_body;
        self.rigid_bodies
            .get_mut(native)
            .ok_or(PhysicsError::BackendInvariant(
                "engine body references a missing Rapier rigid body",
            ))
    }

    fn rigid_body(&self, body: PhysicsBodyHandle) -> Result<&RigidBody, PhysicsError> {
        let native = self.native_body(body)?.rigid_body;
        self.rigid_bodies
            .get(native)
            .ok_or(PhysicsError::BackendInvariant(
                "engine body references a missing Rapier rigid body",
            ))
    }

    fn rapier_constraint(descriptor: &ConstraintDescriptor) -> GenericJoint {
        let mut locked_axes = JointAxesMask::empty();
        for axis in ConstraintAxis::ALL {
            if descriptor.axes.get(axis).motion == ConstraintMotion::Locked {
                locked_axes |= JointAxesMask::from(Self::rapier_joint_axis(axis));
            }
        }

        let mut joint = GenericJoint::new(locked_axes);
        joint
            .set_local_frame1(convert::pose(descriptor.parent_frame))
            .set_local_frame2(convert::pose(descriptor.child_frame))
            .set_contacts_enabled(descriptor.contacts_enabled);
        joint.set_enabled(descriptor.enabled);

        for axis in ConstraintAxis::ALL {
            let rapier_axis = Self::rapier_joint_axis(axis);
            let configuration = descriptor.axes.get(axis);
            if let ConstraintMotion::Limited(limit) = configuration.motion {
                joint.set_limits(rapier_axis, [limit.minimum, limit.maximum]);
            }
            if let Some(drive) = configuration.drive {
                joint
                    .set_motor(
                        rapier_axis,
                        drive.target_position,
                        drive.target_velocity,
                        drive.stiffness,
                        drive.damping,
                    )
                    .set_motor_max_force(rapier_axis, drive.maximum_force);
            }
        }
        Self::configure_coupling(&mut joint, descriptor.linear_coupling);
        Self::configure_coupling(&mut joint, descriptor.angular_coupling);
        joint
    }

    fn configure_coupling(joint: &mut GenericJoint, coupling: Option<ConstraintCoupling>) {
        let Some(coupling) = coupling else {
            return;
        };
        joint.coupled_axes |= Self::rapier_joint_axes(coupling.axes);
        let Some(first_axis) = coupling.axes.first() else {
            return;
        };
        let rapier_axis = Self::rapier_joint_axis(first_axis);
        if let Some(limit) = coupling.limit {
            joint.set_limits(rapier_axis, [limit.minimum, limit.maximum]);
        }
        if let Some(drive) = coupling.drive {
            joint
                .set_motor(
                    rapier_axis,
                    drive.target_position,
                    drive.target_velocity,
                    drive.stiffness,
                    drive.damping,
                )
                .set_motor_max_force(rapier_axis, drive.maximum_force);
        }
    }

    fn rapier_joint_axes(axes: ConstraintAxisMask) -> JointAxesMask {
        let mut result = JointAxesMask::empty();
        for axis in ConstraintAxis::ALL {
            if axes.contains(axis) {
                result |= JointAxesMask::from(Self::rapier_joint_axis(axis));
            }
        }
        result
    }

    const fn rapier_joint_axis(axis: ConstraintAxis) -> JointAxis {
        match axis {
            ConstraintAxis::LinearX => JointAxis::LinX,
            ConstraintAxis::LinearY => JointAxis::LinY,
            ConstraintAxis::LinearZ => JointAxis::LinZ,
            ConstraintAxis::AngularX => JointAxis::AngX,
            ConstraintAxis::AngularY => JointAxis::AngY,
            ConstraintAxis::AngularZ => JointAxis::AngZ,
        }
    }

    fn create_native_constraint(
        &mut self,
        descriptor: &ConstraintDescriptor,
    ) -> Result<NativeConstraint, PhysicsError> {
        let child = self.native_body(descriptor.child)?.rigid_body;
        let (parent, world_anchor) = match descriptor.parent {
            ConstraintTarget::Body(parent) => (self.native_body(parent)?.rigid_body, None),
            ConstraintTarget::World => {
                let anchor = self.rigid_bodies.insert(RigidBodyBuilder::fixed());
                (anchor, Some(anchor))
            }
        };
        let joint = match descriptor.solver_model {
            ConstraintSolverModel::Impulse => {
                NativeConstraintHandle::Impulse(self.impulse_joints.insert(
                    parent,
                    child,
                    Self::rapier_constraint(descriptor),
                    true,
                ))
            }
            ConstraintSolverModel::ReducedCoordinate => {
                let Some(joint) = self.multibody_joints.insert(
                    parent,
                    child,
                    Self::rapier_constraint(descriptor),
                    true,
                ) else {
                    if let Some(anchor) = world_anchor {
                        self.rigid_bodies.remove(
                            anchor,
                            &mut self.islands,
                            &mut self.colliders,
                            &mut self.impulse_joints,
                            &mut self.multibody_joints,
                            true,
                        );
                    }
                    return Err(PhysicsError::BackendInvariant(
                        "reduced-coordinate constraint would create an invalid articulation topology",
                    ));
                };
                NativeConstraintHandle::ReducedCoordinate(joint)
            }
        };
        Ok(NativeConstraint {
            descriptor: descriptor.clone(),
            joint: Some(joint),
            world_anchor,
            broken: false,
            break_reason: None,
            linear_impulse: Vec3::ZERO,
            angular_impulse: Vec3::ZERO,
        })
    }

    fn remove_native_constraint(&mut self, mut constraint: NativeConstraint) {
        if let Some(joint) = constraint.joint.take() {
            match joint {
                NativeConstraintHandle::Impulse(joint) => {
                    self.impulse_joints.remove(joint, true);
                }
                NativeConstraintHandle::ReducedCoordinate(joint) => {
                    self.multibody_joints.remove(joint, true);
                }
            }
        }
        if let Some(anchor) = constraint.world_anchor.take() {
            self.rigid_bodies.remove(
                anchor,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
        }
    }

    fn evaluate_constraint_breakage(&mut self, time_step: f32) {
        let mut broken = Vec::new();
        for (handle, state) in &self.constraints {
            let Some(NativeConstraintHandle::Impulse(joint)) = state.joint else {
                continue;
            };
            let Some(native) = self.impulse_joints.get(joint) else {
                continue;
            };
            let total_impulse = |index: usize| {
                native.impulses[index]
                    + native.data.limits[index].impulse
                    + native.data.motors[index].impulse
            };
            let impulses = [
                total_impulse(0),
                total_impulse(1),
                total_impulse(2),
                total_impulse(3),
                total_impulse(4),
                total_impulse(5),
            ];
            let linear = Vec3::new(impulses[0], impulses[1], impulses[2]);
            let angular = Vec3::new(impulses[3], impulses[4], impulses[5]);
            let maximum_row_impulse = impulses.into_iter().map(f32::abs).fold(0.0_f32, f32::max);
            let reason = if state
                .descriptor
                .break_impulse
                .is_some_and(|limit| maximum_row_impulse > limit)
            {
                Some(ConstraintBreakReason::Impulse)
            } else if state
                .descriptor
                .break_force
                .is_some_and(|limit| linear.length() / time_step > limit)
            {
                Some(ConstraintBreakReason::Force)
            } else if state
                .descriptor
                .break_torque
                .is_some_and(|limit| angular.length() / time_step > limit)
            {
                Some(ConstraintBreakReason::Torque)
            } else {
                None
            };
            broken.push((*handle, linear, angular, reason));
        }

        for (handle, linear, angular, reason) in broken {
            let (joint, anchor) = {
                let Some(state) = self.constraints.get_mut(&handle) else {
                    continue;
                };
                state.linear_impulse = linear;
                state.angular_impulse = angular;
                if reason.is_none() {
                    continue;
                }
                state.broken = true;
                state.break_reason = reason;
                (state.joint.take(), state.world_anchor.take())
            };
            if let Some(joint) = joint {
                match joint {
                    NativeConstraintHandle::Impulse(joint) => {
                        self.impulse_joints.remove(joint, true);
                    }
                    NativeConstraintHandle::ReducedCoordinate(joint) => {
                        self.multibody_joints.remove(joint, true);
                    }
                }
            }
            if let Some(anchor) = anchor {
                self.rigid_bodies.remove(
                    anchor,
                    &mut self.islands,
                    &mut self.colliders,
                    &mut self.impulse_joints,
                    &mut self.multibody_joints,
                    true,
                );
            }
        }
    }

    fn set_body_mass(&mut self, body: PhysicsBodyHandle, mass: f32) -> Result<(), PhysicsError> {
        let rigid_body_handle = self.native_body(body)?.rigid_body;
        let collider_handles = self
            .rigid_bodies
            .get(rigid_body_handle)
            .ok_or(PhysicsError::BackendInvariant(
                "engine body references a missing Rapier rigid body",
            ))?
            .colliders()
            .to_vec();
        let collider_mass: f32 = collider_handles
            .iter()
            .filter_map(|handle| self.colliders.get(*handle))
            .map(Collider::mass)
            .sum();

        if collider_mass > f32::EPSILON {
            let scale = mass / collider_mass;
            for handle in collider_handles {
                let collider =
                    self.colliders
                        .get_mut(handle)
                        .ok_or(PhysicsError::BackendInvariant(
                            "body references a missing Rapier collider",
                        ))?;
                collider.set_mass(collider.mass() * scale);
            }
            let rigid_body = self.rigid_bodies.get_mut(rigid_body_handle).ok_or(
                PhysicsError::BackendInvariant(
                    "engine body references a missing Rapier rigid body",
                ),
            )?;
            rigid_body.set_additional_mass(0.0, true);
            rigid_body.recompute_mass_properties_from_colliders(&self.colliders);
        } else {
            let rigid_body = self.rigid_bodies.get_mut(rigid_body_handle).ok_or(
                PhysicsError::BackendInvariant(
                    "engine body references a missing Rapier rigid body",
                ),
            )?;
            rigid_body.set_additional_mass(mass, true);
            rigid_body.recompute_mass_properties_from_colliders(&self.colliders);
        }
        Ok(())
    }

    fn living_unprojection_needed(
        &self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
        body_pose: Pose,
        direction: &mut Vec3,
    ) -> Result<Vec3, PhysicsError> {
        let native = self.native_body(body)?;
        let state = self
            .living
            .get(&body)
            .ok_or(PhysicsError::ActionRequiresLivingBody {
                action: "check_stance",
            })?;
        let (shape, local_pose) = living_collider_geometry(dimensions)?;
        let shape_pose = body_pose * local_pose;
        let collision_types = state.configuration.dynamics.collision_types();
        let collision_class = state.configuration.collision_class;
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let Some(metadata) = self.collider_metadata.get(&handle) else {
                return false;
            };
            metadata.body != body
                && metadata.blocks_motion()
                && collision_types.intersects(metadata.query_type)
                && collision_class.interacts_with(decode_collision_class(collider.user_data))
        };
        let filter = QueryFilter::default()
            .exclude_rigid_body(native.rigid_body)
            .predicate(&predicate);
        let queries = self.query_broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.rigid_bodies,
            &self.colliders,
            filter,
        );

        for (_, collider) in queries.intersect_shape(shape_pose, shape.as_ref()) {
            let Some(contact) = rapier3d::parry::query::contact(
                &shape_pose,
                shape.as_ref(),
                collider.position(),
                collider.shape(),
                0.0,
            )
            .map_err(|_| PhysicsError::BackendInvariant("unsupported living stance shape pair"))?
            else {
                continue;
            };
            let depth = (-contact.dist).max(0.0);
            if depth <= f32::EPSILON {
                continue;
            }
            let away = -convert::vec3(contact.normal1).normalize_or_zero();
            if direction.length_squared() <= f32::EPSILON {
                *direction = away;
            } else {
                *direction = direction.normalize();
            }
            let projected = direction.dot(away);
            if projected <= f32::EPSILON {
                return Ok(*direction * f32::INFINITY);
            }
            return Ok(*direction * (depth / projected));
        }
        Ok(Vec3::ZERO)
    }

    fn set_living_dimensions(
        &mut self,
        body: PhysicsBodyHandle,
        mut dimensions: LivingDimensions,
    ) -> Result<(), PhysicsError> {
        dimensions.validate()?;
        let native = self.native_body(body)?.clone();
        let state =
            self.living
                .get(&body)
                .cloned()
                .ok_or(PhysicsError::ActionRequiresLivingBody {
                    action: "set_living_dimensions",
                })?;
        dimensions.ground_contact_epsilon = dimensions
            .ground_contact_epsilon
            .max((dimensions.collider_half_height * 0.01).max(0.004));
        let geometry_changed = living_geometry_changed(state.configuration.dimensions, dimensions);
        let mut body_pose = *self
            .rigid_bodies
            .get(native.rigid_body)
            .ok_or(PhysicsError::BackendInvariant(
                "living body references a missing Rapier rigid body",
            ))?
            .position();
        let mut unprojection = Vec3::ZERO;
        if geometry_changed && state.configuration.dynamics.is_active {
            let mut direction = dimensions.unprojection_direction;
            let mut resolved = false;
            for _ in 0..30 {
                let mut test_pose = body_pose;
                test_pose.translation += convert::vector(unprojection);
                let step =
                    self.living_unprojection_needed(body, dimensions, test_pose, &mut direction)?;
                if step.length_squared() <= f32::EPSILON {
                    resolved = true;
                    break;
                }
                let required = unprojection + step * 1.01;
                if !required.is_finite() || required.length() > dimensions.max_unprojection {
                    return Err(PhysicsError::LivingStanceBlocked {
                        required,
                        maximum: dimensions.max_unprojection,
                    });
                }
                unprojection = required;
            }
            if !resolved {
                return Err(PhysicsError::LivingStanceBlocked {
                    required: unprojection,
                    maximum: dimensions.max_unprojection,
                });
            }
        }

        body_pose.translation += convert::vector(unprojection);
        let (shape, local_pose) = living_collider_geometry(dimensions)?;
        {
            let collider = self.colliders.get_mut(state.primary_collider).ok_or(
                PhysicsError::BackendInvariant("living body references a missing primary collider"),
            )?;
            collider.set_shape(shape);
            collider.set_position_wrt_parent(local_pose);
        }
        self.rigid_bodies
            .get_mut(native.rigid_body)
            .ok_or(PhysicsError::BackendInvariant(
                "living body references a missing Rapier rigid body",
            ))?
            .set_position(body_pose, true);
        let living = self
            .living
            .get_mut(&body)
            .ok_or(PhysicsError::BackendInvariant(
                "living body state disappeared during dimension update",
            ))?;
        living.configuration.dimensions = dimensions;
        living.time_since_stance_change = 0.0;
        let mut ignored_events = Vec::new();
        self.query_broad_phase.update(
            &self.integration_parameters,
            &self.colliders,
            &self.rigid_bodies,
            &[state.primary_collider],
            &[],
            &mut ignored_events,
        );
        Ok(())
    }

    fn set_living_dynamics(
        &mut self,
        body: PhysicsBodyHandle,
        dynamics: LivingDynamics,
    ) -> Result<(), PhysicsError> {
        dynamics.validate()?;
        let state =
            self.living
                .get(&body)
                .cloned()
                .ok_or(PhysicsError::ActionRequiresLivingBody {
                    action: "set_living_dynamics",
                })?;
        self.set_body_mass(body, dynamics.mass)?;
        self.collider_metadata
            .get_mut(&state.primary_collider)
            .ok_or(PhysicsError::BackendInvariant(
                "living body references untracked primary collider",
            ))?
            .surface_index = dynamics.surface_index;
        let living = self
            .living
            .get_mut(&body)
            .ok_or(PhysicsError::BackendInvariant(
                "living body state disappeared during dynamics update",
            ))?;
        if !living.configuration.dynamics.is_active && dynamics.is_active {
            living.flying = true;
        }
        living.configuration.dynamics = dynamics;
        Ok(())
    }

    /// Cry `CRigidEntity::SetParams(pe_simulation_params::density)` assigns
    /// `shape_volume * density` to every part, then recomputes the complete
    /// mass distribution.
    fn set_body_density(
        &mut self,
        body: PhysicsBodyHandle,
        density: f32,
    ) -> Result<(), PhysicsError> {
        if !density.is_finite() || density < 0.0 {
            return Err(PhysicsError::InvalidRigidBodyScalar { field: "density" });
        }
        let rigid_body_handle = self.native_body(body)?.rigid_body;
        let collider_handles = self
            .rigid_bodies
            .get(rigid_body_handle)
            .ok_or(PhysicsError::BackendInvariant(
                "engine body references a missing Rapier rigid body",
            ))?
            .colliders()
            .to_vec();
        for handle in collider_handles {
            self.colliders
                .get_mut(handle)
                .ok_or(PhysicsError::BackendInvariant(
                    "body references a missing Rapier collider",
                ))?
                .set_density(density);
        }
        let rigid_body =
            self.rigid_bodies
                .get_mut(rigid_body_handle)
                .ok_or(PhysicsError::BackendInvariant(
                    "engine body references a missing Rapier rigid body",
                ))?;
        rigid_body.set_additional_mass(0.0, true);
        rigid_body.recompute_mass_properties_from_colliders(&self.colliders);

        let native = self
            .bodies
            .get_mut(&body)
            .ok_or(PhysicsError::BodyNotFound(body))?;
        for collider in &mut native.collider_configurations {
            collider.density = density;
            collider.mass = None;
        }
        if let Some(configuration) = &mut native.rigid_configuration {
            configuration.density = density;
        }
        Ok(())
    }

    fn body_velocities(&self, body: PhysicsBodyHandle) -> Result<(Vec3, Vec3), PhysicsError> {
        if let Some(state) = self.living.get(&body) {
            return Ok((state.velocity, Vec3::ZERO));
        }
        if let Some(state) = self.characters.get(&body) {
            return Ok((state.velocity, Vec3::ZERO));
        }
        if let Some(state) = self.particles.get(&body) {
            return Ok((state.velocity, state.angular_velocity));
        }
        let native = self.native_body(body)?;
        let rigid_body =
            self.rigid_bodies
                .get(native.rigid_body)
                .ok_or(PhysicsError::BackendInvariant(
                    "engine body references a missing Rapier rigid body",
                ))?;
        Ok((
            convert::vec3(rigid_body.linvel()),
            convert::vec3(rigid_body.angvel()),
        ))
    }

    fn update_accelerations(
        &mut self,
        previous: &[(PhysicsBodyHandle, Vec3, Vec3)],
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        let mut current = Vec::with_capacity(previous.len());
        for &(body, previous_linear, previous_angular) in previous {
            let (linear, angular) = self.body_velocities(body)?;
            current.push((
                body,
                (linear - previous_linear) / time_step,
                (angular - previous_angular) / time_step,
            ));
        }
        for (body, linear, angular) in current {
            let native = self
                .bodies
                .get_mut(&body)
                .ok_or(PhysicsError::BodyNotFound(body))?;
            native.linear_acceleration = linear;
            native.angular_acceleration = angular;
        }
        Ok(())
    }

    fn insert_collider(
        &mut self,
        owner: ColliderOwner,
        configuration: &ColliderConfiguration,
        contributes_mass: bool,
        force_sensor: bool,
    ) -> Result<ColliderHandle, PhysicsError> {
        let (builder, shape_pose) = convert::collider(&configuration.shape)?;
        let local_pose = configuration.local_pose * shape_pose;
        let builder = builder
            .position(convert::pose(local_pose))
            .friction(configuration.friction)
            .friction_combine_rule(CoefficientCombineRule::Multiply)
            .restitution(configuration.restitution)
            .restitution_combine_rule(CoefficientCombineRule::Multiply);
        let builder = if contributes_mass {
            match configuration.mass {
                Some(mass) => builder.mass(mass),
                None => builder.density(configuration.density),
            }
        } else {
            builder.density(0.0)
        };
        let builder = builder
            .sensor(force_sensor || configuration.sensor)
            .enabled(
                force_sensor
                    || configuration.sensor
                    || configuration.simulated
                    || configuration.in_scene_queries,
            )
            .active_hooks(
                ActiveHooks::FILTER_CONTACT_PAIRS
                    | ActiveHooks::FILTER_INTERSECTION_PAIR
                    | ActiveHooks::MODIFY_SOLVER_CONTACTS,
            )
            .user_data(encode_collision_filter(configuration));
        let collider =
            self.colliders
                .insert_with_parent(builder, owner.rigid_body, &mut self.rigid_bodies);
        self.collider_metadata.insert(
            collider,
            ColliderMetadata {
                body: owner.body,
                entity_id: owner.entity_id,
                query_type: owner.query_type,
                surface_index: configuration.surface_index,
                surface_pierceability: configuration.surface_pierceability,
                sensor: force_sensor || configuration.sensor,
                simulated: configuration.simulated,
                in_scene_queries: configuration.in_scene_queries,
                tag: configuration.tag,
                rest_offset: configuration.rest_offset,
                contact_offset: configuration.contact_offset,
                collision_filter: configuration.collision_filter,
                continuous_collision_mode: owner.continuous_collision_mode,
                continuous_prediction_distance: owner.continuous_prediction_distance,
            },
        );
        let mut ignored_events = Vec::new();
        self.query_broad_phase.update(
            &self.integration_parameters,
            &self.colliders,
            &self.rigid_bodies,
            &[collider],
            &[],
            &mut ignored_events,
        );
        Ok(collider)
    }

    fn collect_interactions(&mut self) {
        let mut current = HashMap::new();

        self.collect_contact_interactions(&mut current);

        self.collect_trigger_interactions(&mut current);

        let mut events = Vec::with_capacity(current.len() + self.active_interactions.len());
        for (key, mut interaction) in current.iter().map(|(key, value)| (*key, *value)) {
            interaction.phase = if self.active_interactions.contains_key(&key) {
                PhysicsInteractionPhase::Persisted
            } else {
                PhysicsInteractionPhase::Started
            };
            events.push(interaction);
        }
        for (key, interaction) in &self.active_interactions {
            if current.contains_key(key) {
                continue;
            }
            let mut interaction = *interaction;
            interaction.phase = PhysicsInteractionPhase::Stopped;
            interaction.point = None;
            interaction.normal = None;
            interaction.penetration_depth = 0.0;
            interaction.impulse = Vec3::ZERO;
            events.push(interaction);
        }
        events.sort_by_key(|interaction| {
            (
                interaction.body_a.get(),
                interaction.body_b.get(),
                interaction.kind as u8,
                interaction.phase as u8,
            )
        });
        self.active_interactions = current;
        self.pending_interactions.extend(events);
    }

    fn snapshot_body_poses(&mut self) -> Result<(), PhysicsError> {
        for native in self.bodies.values_mut() {
            let rigid_body =
                self.rigid_bodies
                    .get(native.rigid_body)
                    .ok_or(PhysicsError::BackendInvariant(
                        "physics body references a missing Rapier rigid body",
                    ))?;
            native.previous_pose = convert::physics_pose(rigid_body.position());
        }
        Ok(())
    }

    fn collider_motion(
        &self,
        metadata: ColliderMetadata,
        collider_handle: ColliderHandle,
    ) -> Result<(NonlinearRigidMotion, Pose, Pose), PhysicsError> {
        let native = self
            .bodies
            .get(&metadata.body)
            .ok_or(PhysicsError::BackendInvariant(
                "collider references a missing physics body",
            ))?;
        let collider =
            self.colliders
                .get(collider_handle)
                .ok_or(PhysicsError::BackendInvariant(
                    "continuous pair references a missing Rapier collider",
                ))?;
        let local_pose = collider
            .position_wrt_parent()
            .copied()
            .unwrap_or_else(Pose::identity);
        let previous = convert::pose(native.previous_pose) * local_pose;
        let current = *collider.position();
        let linear_displacement = current.translation - previous.translation;
        let angular_displacement =
            (current.rotation * previous.rotation.inverse()).to_scaled_axis();
        let motion = NonlinearRigidMotion::new(
            previous,
            Vector::ZERO,
            linear_displacement,
            angular_displacement,
        );
        Ok((motion, previous, current))
    }

    fn collect_hit_projection_record(
        &self,
        records: &mut HashMap<PhysicsBodyHandle, HitProjectionRecord>,
        metadata: ColliderMetadata,
        cast: ContinuousCast<'_>,
    ) -> Result<(), PhysicsError> {
        let ContinuousCast {
            motion,
            collider_start,
            collider_end,
            witness,
            local_normal,
            hit_fraction,
        } = cast;
        if !metadata.continuous_collision_mode.uses_hit_projection() {
            return Ok(());
        }
        let native = self
            .bodies
            .get(&metadata.body)
            .ok_or(PhysicsError::BackendInvariant(
                "continuous collider references a missing physics body",
            ))?;
        let configuration = native
            .rigid_configuration
            .ok_or(PhysicsError::BackendInvariant(
                "hit-projection collider has no rigid-body configuration",
            ))?;
        let rigid_body =
            self.rigid_bodies
                .get(native.rigid_body)
                .ok_or(PhysicsError::BackendInvariant(
                    "continuous body references a missing Rapier rigid body",
                ))?;
        if !rigid_body.is_dynamic() {
            return Ok(());
        }

        let current_pose = convert::physics_pose(rigid_body.position());
        let body_displacement =
            (current_pose.translation - native.previous_pose.translation).length();
        let reference_point_displacement =
            (collider_end * witness - collider_start * witness).length();
        let projection = continuous_hit_projection(
            hit_fraction,
            body_displacement,
            reference_point_displacement,
            configuration.continuous_distance_factor,
        );
        let impact_pose = motion.position_at_time(hit_fraction);
        let normal = impact_pose.rotation * local_normal;
        if !normal.is_finite() {
            return Err(PhysicsError::BackendInvariant(
                "continuous shape cast produced a non-finite normal",
            ));
        }
        let candidate = HitProjectionRecord {
            rigid_body: native.rigid_body,
            previous_pose: native.previous_pose,
            current_pose,
            pose_fraction: projection.pose_fraction,
            normal,
            normal_velocity_retained_fraction: projection.normal_velocity_retained_fraction,
        };
        match records.entry(metadata.body) {
            std::collections::hash_map::Entry::Occupied(mut entry)
                if candidate.pose_fraction < entry.get().pose_fraction =>
            {
                entry.insert(candidate);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(candidate);
            }
            std::collections::hash_map::Entry::Occupied(_) => {}
        }
        Ok(())
    }

    /// Replays `RockNRoll`'s modes 1/2 pair cast and post-hit projection over
    /// Rapier's retained contact pairs. The nonlinear cast supplies the native
    /// record's hit fraction, witnesses, and normal; the cleanup scalar logic
    /// is shared with `az-physics`.
    fn apply_hit_projections(&mut self) -> Result<(), PhysicsError> {
        let mut records = HashMap::new();
        for pair in self.narrow_phase.contact_pairs() {
            let Some(left) = self.collider_metadata.get(&pair.collider1).copied() else {
                continue;
            };
            let Some(right) = self.collider_metadata.get(&pair.collider2).copied() else {
                continue;
            };
            if !left.continuous_collision_mode.uses_hit_projection()
                && !right.continuous_collision_mode.uses_hit_projection()
            {
                continue;
            }
            let left_collider =
                self.colliders
                    .get(pair.collider1)
                    .ok_or(PhysicsError::BackendInvariant(
                        "continuous pair references a missing left Rapier collider",
                    ))?;
            let right_collider =
                self.colliders
                    .get(pair.collider2)
                    .ok_or(PhysicsError::BackendInvariant(
                        "continuous pair references a missing right Rapier collider",
                    ))?;
            let (left_motion, left_start, left_end) = self.collider_motion(left, pair.collider1)?;
            let (right_motion, right_start, right_end) =
                self.collider_motion(right, pair.collider2)?;
            let Some(hit) = cast_shapes_nonlinear(
                &left_motion,
                left_collider.shape(),
                &right_motion,
                right_collider.shape(),
                0.0,
                1.0,
                true,
            )
            .map_err(|_| {
                PhysicsError::BackendInvariant(
                    "Rapier does not support nonlinear casting for a continuous shape pair",
                )
            })?
            else {
                continue;
            };
            self.collect_hit_projection_record(
                &mut records,
                left,
                ContinuousCast {
                    motion: &left_motion,
                    collider_start: left_start,
                    collider_end: left_end,
                    witness: hit.witness1,
                    local_normal: hit.normal1,
                    hit_fraction: hit.time_of_impact,
                },
            )?;
            self.collect_hit_projection_record(
                &mut records,
                right,
                ContinuousCast {
                    motion: &right_motion,
                    collider_start: right_start,
                    collider_end: right_end,
                    witness: hit.witness2,
                    local_normal: hit.normal2,
                    hit_fraction: hit.time_of_impact,
                },
            )?;
        }

        self.apply_hit_projection_records(records)
    }

    /// Replaces Rapier's ordinary final broadphase bounds with `RockNRoll`'s
    /// mode-4 proxy. Narrow-phase/query geometry remains the real collider;
    /// only candidate-pair retention sees the speculative forward extent.
    fn apply_reverse_displacement_broadphase_proxies(&mut self) -> Result<(), PhysicsError> {
        for native in self.bodies.values().filter(|native| {
            native.rigid_configuration.is_some_and(|configuration| {
                configuration
                    .continuous_collision_mode
                    .reverses_sweep_displacement()
            })
        }) {
            let rigid_body =
                self.rigid_bodies
                    .get(native.rigid_body)
                    .ok_or(PhysicsError::BackendInvariant(
                        "continuous body references a missing Rapier rigid body",
                    ))?;
            let displacement =
                rigid_body.translation() - convert::vector(native.previous_pose.translation);
            for &handle in rigid_body.colliders() {
                let collider = self
                    .colliders
                    .get(handle)
                    .ok_or(PhysicsError::BackendInvariant(
                        "continuous body references a missing Rapier collider",
                    ))?;
                let proxy = reverse_displacement_broadphase_aabb(
                    collider,
                    displacement,
                    Self::ROCK_N_ROLL_BROADPHASE_MARGIN,
                );
                self.broad_phase
                    .set_aabb(&self.integration_parameters, handle, proxy);
            }
        }
        Ok(())
    }

    /// Configures Rapier's global narrow-phase prediction window large enough
    /// for every authored per-collider contact offset. The contact hook then
    /// prunes each pair back to its exact summed range.
    fn update_contact_prediction_distance(&mut self) {
        let maximum_collider_range = self
            .collider_metadata
            .values()
            .copied()
            .filter(|metadata| metadata.blocks_motion())
            .map(ColliderMetadata::positive_contact_range)
            .fold(0.0, f32::max);
        self.integration_parameters.normalized_prediction_distance =
            (2.0 * maximum_collider_range) / self.integration_parameters.length_unit;
    }

    fn apply_particle_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        let mut state =
            self.particles
                .remove(&body)
                .ok_or(PhysicsError::OperationRequiresParticleBody {
                    operation: "apply_particle_action",
                })?;
        let result = (|| {
            match action {
                PhysicsAction::Impulse(action) => {
                    let position = convert::vec3(self.rigid_body_mut(body)?.translation());
                    if state.configuration.mass > 0.0 {
                        state.velocity += action.impulse / state.configuration.mass;
                    }
                    let angular_impulse = action
                        .point
                        .map_or(Vec3::ZERO, |point| (point - position).cross(action.impulse));
                    let angular_mass =
                        0.4 * state.configuration.mass * (state.configuration.size * 0.5);
                    if angular_mass > 0.0
                        && !state
                            .configuration
                            .flags
                            .contains(ParticleFlags::CONSTANT_ORIENTATION)
                    {
                        state.angular_velocity += angular_impulse / angular_mass;
                    }
                    state.force_awake = 1;
                }
                PhysicsAction::AngularImpulse(impulse) => {
                    let angular_mass =
                        0.4 * state.configuration.mass * (state.configuration.size * 0.5);
                    if angular_mass > 0.0
                        && !state
                            .configuration
                            .flags
                            .contains(ParticleFlags::CONSTANT_ORIENTATION)
                    {
                        state.angular_velocity += impulse / angular_mass;
                    }
                    state.force_awake = 1;
                }
                PhysicsAction::Reset => {
                    state.velocity = Vec3::ZERO;
                    state.angular_velocity = Vec3::ZERO;
                }
                PhysicsAction::Wake(awake) => {
                    state.force_awake = u8::from(awake);
                    if !awake {
                        state.velocity = Vec3::ZERO;
                    }
                }
                PhysicsAction::SetPose(pose) => {
                    self.rigid_body_mut(body)?
                        .set_position(convert::pose(pose), true);
                }
                PhysicsAction::SetVelocity(velocity) => {
                    state.velocity = velocity;
                    state.heading = velocity.normalize_or_zero();
                    if velocity.length_squared() > 0.0 {
                        state.force_awake = 1;
                    }
                }
                PhysicsAction::SetAngularVelocity(velocity) => {
                    state.angular_velocity = velocity;
                }
                PhysicsAction::SetMass(mass) => {
                    if !mass.is_finite() || mass < 0.0 {
                        return Err(PhysicsError::InvalidParticleConfiguration { field: "mass" });
                    }
                    state.configuration.mass = mass;
                }
                PhysicsAction::SetSimulated(simulated) => {
                    self.rigid_body_mut(body)?.set_enabled(simulated);
                }
                PhysicsAction::Force(_)
                | PhysicsAction::Torque(_)
                | PhysicsAction::SetDensity(_)
                | PhysicsAction::SetLinearDamping(_)
                | PhysicsAction::SetAngularDamping(_)
                | PhysicsAction::SetSleepMinEnergy(_)
                | PhysicsAction::SetBuoyancy(_) => {
                    return Err(PhysicsError::UnsupportedParticleAction {
                        action: rigid_only_action_name(action),
                    });
                }
                PhysicsAction::Move(_)
                | PhysicsAction::SetLivingDimensions(_)
                | PhysicsAction::SetLivingDynamics(_)
                | PhysicsAction::SyncLiving(_) => {
                    return Err(PhysicsError::ActionRequiresLivingBody {
                        action: "living action on particle",
                    });
                }
            }
            Ok(())
        })();
        self.particles.insert(body, state);
        result
    }

    fn particle_support_hit(
        &self,
        body: PhysicsBodyHandle,
        state: &ParticleState,
        position: Vec3,
    ) -> Result<Option<RayCastHit>, PhysicsError> {
        let direction = -state.slide_normal.normalize_or_zero();
        if direction == Vec3::ZERO {
            return Ok(None);
        }
        let mut ignore_bodies = vec![body];
        if let Some(ignored) = state.configuration.ignored_collider {
            ignore_bodies.push(ignored);
        }
        let hits = self.ray_cast(&RayCastConfiguration {
            origin: position,
            direction,
            max_distance: state.configuration.size * 0.55,
            ignore_entity_ids: Vec::new(),
            ignore_bodies,
            max_hits: 8,
            pierces_surfaces_greater_than: i32::from(state.configuration.pierceability),
            physical_entity_types: state.configuration.collision_types,
            include_sensors: false,
            collision_class: Some(state.configuration.collision_class),
            collision_filter: None,
        })?;
        Ok(hits.iter().copied().find(|hit| {
            !self.particle_pair_is_suppressed(state, hit.body)
                && hit.surface_pierceability <= state.configuration.pierceability
        }))
    }

    fn particle_sweep_hits(
        &self,
        body: PhysicsBodyHandle,
        state: &ParticleState,
        pose: PhysicsPose,
        displacement: Vec3,
    ) -> Result<Vec<PhysicsShapeCastHit>, PhysicsError> {
        let distance = displacement.length();
        if distance <= f32::EPSILON {
            return Ok(Vec::new());
        }
        let mut ignore_bodies = vec![body];
        if let Some(ignored) = state.configuration.ignored_collider {
            ignore_bodies.push(ignored);
        }
        let hits = self.cast_shape_all(&ShapeCastConfiguration {
            shape: ColliderShape::Cuboid {
                half_extents: Vec3::new(
                    state.configuration.size * 0.5,
                    state.configuration.size * 0.5,
                    state.configuration.thickness * 0.5,
                ),
            },
            pose,
            direction: displacement / distance,
            max_distance: distance,
            target_distance: 0.0,
            stop_at_penetration: false,
            filter: SpatialQueryFilter {
                ignore_entity_ids: Vec::new(),
                ignore_bodies,
                physical_entity_types: state.configuration.collision_types,
                include_sensors: false,
                collision_class: Some(state.configuration.collision_class),
                collision_filter: None,
            },
            max_results: 8,
        })?;
        Ok(hits
            .into_iter()
            .filter(|hit| !self.particle_pair_is_suppressed(state, hit.body))
            .collect())
    }

    fn particle_pair_is_suppressed(&self, state: &ParticleState, other: PhysicsBodyHandle) -> bool {
        state
            .configuration
            .flags
            .contains(ParticleFlags::NO_SELF_COLLISIONS)
            && self.particles.get(&other).is_some_and(|other| {
                other
                    .configuration
                    .flags
                    .contains(ParticleFlags::NO_SELF_COLLISIONS)
            })
    }

    fn velocity_at_body_point(
        &self,
        body: PhysicsBodyHandle,
        point: Vec3,
    ) -> Result<Vec3, PhysicsError> {
        if let Some(particle) = self.particles.get(&body) {
            let position = convert::vec3(self.rigid_body(body)?.translation());
            return Ok(particle.velocity + particle.angular_velocity.cross(point - position));
        }
        let native = self.native_body(body)?;
        let rigid_body =
            self.rigid_bodies
                .get(native.rigid_body)
                .ok_or(PhysicsError::BackendInvariant(
                    "particle contact references a missing rigid body",
                ))?;
        Ok(convert::vec3(
            rigid_body.velocity_at_point(convert::vector(point)),
        ))
    }

    fn particle_contact_mass(&self, body: PhysicsBodyHandle) -> Result<f32, PhysicsError> {
        if let Some(particle) = self.particles.get(&body) {
            return Ok(particle.configuration.mass);
        }
        let rigid_body = self.rigid_body(body)?;
        Ok(if rigid_body.is_dynamic() {
            rigid_body.mass()
        } else {
            0.0
        })
    }

    fn contact_coefficients(&self, body: PhysicsBodyHandle, surface: SurfaceIndex) -> (f32, f32) {
        self.bodies
            .get(&body)
            .and_then(|native| {
                native
                    .collider_configurations
                    .iter()
                    .find(|collider| collider.surface_index == surface)
                    .or_else(|| native.collider_configurations.first())
            })
            .map_or((0.5, 0.0), |collider| {
                (collider.friction, collider.restitution)
            })
    }

    fn record_particle_hit(
        &mut self,
        body: PhysicsBodyHandle,
        state: &ParticleState,
        hit: PhysicsShapeCastHit,
        velocity: Vec3,
        is_piercing: bool,
    ) -> Result<(), PhysicsError> {
        let target_velocity = self.velocity_at_body_point(hit.body, hit.position)?;
        let relative_velocity = velocity - target_velocity;
        let mut target_mass = self.particle_contact_mass(hit.body)?;
        if target_mass <= 1.0e-10 {
            target_mass = state.configuration.mass * 100.0;
        }
        let pierce_scale = if is_piercing {
            let difference =
                i32::from(hit.surface_pierceability) - i32::from(state.configuration.pierceability);
            0.5 * (1.0 - f32_from_i32(difference) / 15.0)
        } else {
            1.0
        };
        let reduced_mass = if state.configuration.mass + target_mass > 0.0 {
            state.configuration.mass * target_mass * pierce_scale
                / (state.configuration.mass + target_mass)
        } else {
            0.0
        };
        let impulse = relative_velocity * reduced_mass;
        let approaches_surface = relative_velocity.dot(hit.normal) < 0.0;
        if approaches_surface
            && velocity.length_squared()
                > state.configuration.minimum_bounce_speed
                    * state.configuration.minimum_bounce_speed
            && !state
                .configuration
                .flags
                .contains(ParticleFlags::NO_IMPULSE)
        {
            let native = self.native_body(hit.body)?.rigid_body;
            if let Some(target) = self.rigid_bodies.get_mut(native)
                && target.is_dynamic()
            {
                target.apply_impulse_at_point(
                    convert::vector(impulse),
                    convert::vector(hit.position),
                    true,
                );
            }
        }

        let particle_metadata = self
            .collider_metadata
            .get(&state.primary_collider)
            .copied()
            .ok_or(PhysicsError::BackendInvariant(
                "particle primary collider has no metadata",
            ))?;
        self.pending_interactions.push(PhysicsInteraction {
            phase: PhysicsInteractionPhase::Started,
            kind: PhysicsInteractionKind::Contact,
            body_a: body,
            body_b: hit.body,
            entity_a: particle_metadata.entity_id,
            entity_b: hit.entity_id,
            surface_a: state.configuration.surface_index,
            surface_b: hit.surface_index,
            tag_a: particle_metadata.tag,
            tag_b: hit.collider_tag,
            point: Some(hit.position),
            normal: Some(hit.normal),
            penetration_depth: 0.0,
            impulse: if approaches_surface {
                impulse
            } else {
                Vec3::ZERO
            },
        });
        Ok(())
    }

    fn refresh_particle_medium(
        &self,
        state: &mut ParticleState,
        position: Vec3,
    ) -> Result<(), PhysicsError> {
        if state.area_step_count != 0 {
            state.area_step_count -= 1;
            return Ok(());
        }
        let mut water_flow = Vec3::ZERO;
        let mut air_flow = Vec3::ZERO;
        let mut depth = 0.0_f32;
        for (&area_body, area) in &self.fluid_areas {
            let native = self.native_body(area_body)?;
            let overlaps = self.rigid_bodies[native.rigid_body]
                .colliders()
                .iter()
                .filter_map(|handle| self.colliders.get(*handle))
                .map(Collider::compute_aabb)
                .any(|bounds| {
                    position.x >= bounds.mins.x
                        && position.x <= bounds.maxs.x
                        && position.y >= bounds.mins.y
                        && position.y <= bounds.maxs.y
                        && position.z >= bounds.mins.z
                        && position.z <= bounds.maxs.z
                });
            if !overlaps {
                continue;
            }
            match area.medium {
                az_physics::FluidMedium::Water => {
                    water_flow += area.flow;
                    depth = depth.min(area.plane.signed_distance(position));
                }
                az_physics::FluidMedium::Air => air_flow += area.flow,
            }
        }
        state.submerged_depth = depth;
        state.medium_velocity = if depth < 0.0 { water_flow } else { air_flow };
        state.area_step_count = state.configuration.area_check_period.saturating_sub(1);
        Ok(())
    }

    fn step_particle_body(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        let mut state =
            self.particles
                .remove(&body)
                .ok_or(PhysicsError::OperationRequiresParticleBody {
                    operation: "step_particle_body",
                })?;
        let result = self.step_particle_state(body, &mut state, time_step);
        self.particles.insert(body, state);
        result
    }

    fn step_particle_state(
        &mut self,
        body: PhysicsBodyHandle,
        state: &mut ParticleState,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        let rigid_body = self.rigid_body(body)?;
        if !rigid_body.is_enabled() {
            return Ok(());
        }
        let current_pose = convert::physics_pose(rigid_body.position());
        if !state.is_awake(current_pose.translation) {
            state.sleep_time += time_step;
            return Ok(());
        }

        let mut step = ParticleStep::begin(state, current_pose, time_step);
        state.recent_collisions = state.recent_collisions.saturating_sub(1);

        if state.sliding {
            self.resolve_particle_slide(body, state, &mut step, time_step)?;
        }

        step.integrate_drag(state, time_step);
        step.integrate_orientation(state, time_step);

        let hits = self.particle_sweep_hits(
            body,
            state,
            PhysicsPose {
                translation: step.previous_position,
                rotation: current_pose.rotation,
            },
            step.position - step.previous_position,
        )?;
        let mut blocking_hit = None;
        for hit in hits {
            let piercing = hit.surface_pierceability > state.configuration.pierceability;
            self.record_particle_hit(body, state, hit, step.collision_velocity, piercing)?;
            if !piercing {
                blocking_hit = Some(hit);
                break;
            }
        }
        if let Some(hit) = blocking_hit {
            self.resolve_particle_impact(body, state, &mut step, hit, time_step)?;
        }

        let forced = state.force_awake & 1;
        state.time_force_awake = if forced != 0 {
            state.time_force_awake + time_step
        } else {
            0.0
        };
        state.sleep_time = 0.0;
        state.velocity = step.velocity;
        self.rigid_body_mut(body)?.set_position(
            convert::pose(PhysicsPose {
                translation: step.position,
                rotation: step.orientation.normalize(),
            }),
            true,
        );
        self.refresh_particle_medium(state, step.position)
    }

    /// Cry's resting branch: re-probes the supporting surface, sits the
    /// particle on it, and converts the tangential motion into sliding or
    /// rolling.
    ///
    /// # Errors
    ///
    /// Returns whatever the support ray cast, the point-velocity lookup, or the
    /// contact-mass lookup returns.
    fn resolve_particle_slide(
        &self,
        body: PhysicsBodyHandle,
        state: &mut ParticleState,
        step: &mut ParticleStep,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        let Some(hit) = self.particle_support_hit(body, state, step.position)? else {
            state.sliding = false;
            if !state
                .configuration
                .flags
                .contains(ParticleFlags::CONSTANT_ORIENTATION)
            {
                let spin = state.heading.cross(state.gravity);
                let length = spin.length();
                state.angular_velocity = if length > 0.01 {
                    spin * (0.5 * state.gravity.length() / (step.radius * length))
                } else {
                    Vec3::ZERO
                };
            }
            return Ok(());
        };

        state.slide_normal = hit.normal;
        step.position = hit.position + hit.normal * step.lying_radius;
        let target_velocity = self.velocity_at_body_point(hit.body, step.position)?;
        step.collision_velocity -= target_velocity;
        let normal_speed = hit.normal.dot(step.collision_velocity);
        let tangent = step.collision_velocity - hit.normal * normal_speed;
        if state.configuration.flags.contains(ParticleFlags::NO_ROLL) || hit.normal.z < 0.5 {
            let (particle_friction, _) =
                self.contact_coefficients(body, state.configuration.surface_index);
            let (target_friction, _) = self.contact_coefficients(hit.body, hit.surface_index);
            let friction = (particle_friction * target_friction).max(0.0);
            let tangent_length = tangent.length();
            let tangent_scale = if tangent_length > 1.0e-4 {
                (-step
                    .gravity
                    .dot(hit.normal)
                    .mul_add(time_step, normal_speed))
                .max(0.0)
                .mul_add(-friction, tangent_length)
                .max(0.0)
                    / tangent_length
            } else {
                0.0
            };
            step.velocity =
                target_velocity + hit.normal * normal_speed.max(0.0) + tangent * tangent_scale;
            step.collision_velocity = step.velocity;
            state.angular_velocity = Vec3::ZERO;
            state.spin_orientation = Quat::IDENTITY;
            step.orientation = align_axis(
                step.orientation,
                state.configuration.alignment_normal,
                hit.normal,
            );
            step.flags |= ParticleFlags::CONSTANT_ORIENTATION;
        } else {
            let (friction, _) = self.contact_coefficients(body, state.configuration.surface_index);
            step.velocity = target_velocity
                + (step.collision_velocity
                    - state.slide_normal * step.collision_velocity.dot(state.slide_normal))
                    * time_step.mul_add(-friction, 1.0).max(0.0);
            step.collision_velocity = step.velocity;
            state.angular_velocity =
                state.slide_normal.cross(step.velocity - target_velocity) / step.radius;
            if state.configuration.roll_axis.length_squared() > 0.0
                && state.angular_velocity.length_squared() > 1.0e-20
            {
                state.spin_orientation = align_axis(
                    state.spin_orientation,
                    state.configuration.roll_axis,
                    state.angular_velocity.normalize(),
                );
            }
        }
        step.gravity = if state
            .configuration
            .flags
            .contains(ParticleFlags::SINGLE_CONTACT)
        {
            Vec3::ZERO
        } else {
            step.gravity - state.slide_normal * step.gravity.dot(state.slide_normal)
        };
        state.force_awake =
            if self.particle_contact_mass(hit.body)? == 0.0 || state.time_force_awake > 40.0 {
                2
            } else {
                1
            };
        Ok(())
    }

    /// Cry's bounce branch: places the particle at its blocking hit, splits the
    /// relative velocity into a restituted normal part and a damped tangential
    /// part, and decides whether the particle comes to rest.
    ///
    /// # Errors
    ///
    /// Returns whatever the point-velocity or contact-mass lookup returns.
    fn resolve_particle_impact(
        &self,
        body: PhysicsBodyHandle,
        state: &mut ParticleState,
        step: &mut ParticleStep,
        hit: PhysicsShapeCastHit,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        state.collision_pending = true;
        state.recent_collisions = if hit.normal.dot(step.gravity) < 0.001 {
            3
        } else {
            state.recent_collisions
        };
        let direction = (step.position - step.previous_position).normalize_or_zero();
        step.position = if hit.distance > step.radius {
            hit.position - direction * step.radius
        } else {
            step.previous_position
        };
        let target_velocity = self.velocity_at_body_point(hit.body, step.position)?;
        step.collision_velocity -= target_velocity;
        let target_mass = self.particle_contact_mass(hit.body)?;
        let inverse_target_mass = if target_mass > 0.0 {
            target_mass.recip()
        } else {
            0.0
        };
        let mass_fraction = state.configuration.mass * inverse_target_mass
            / state.configuration.mass.mul_add(inverse_target_mass, 1.0);
        let (_, particle_restitution) =
            self.contact_coefficients(body, state.configuration.surface_index);
        let (_, target_restitution) = self.contact_coefficients(hit.body, hit.surface_index);
        let mut restitution = ((particle_restitution + target_restitution) * 0.5).clamp(0.0, 1.0);
        let normal_speed = hit.normal.dot(step.collision_velocity);
        let tangent = step.collision_velocity - hit.normal * normal_speed;
        if normal_speed > -state.configuration.minimum_bounce_speed
            || step.lying_radius < step.radius * 0.3
        {
            restitution = 0.0;
        }
        step.velocity = step.collision_velocity
            - hit.normal * (normal_speed * (1.0 - mass_fraction) * (1.0 + restitution))
            - tangent * (1.0 - mass_fraction) * (1.0 - restitution);
        let supported = hit.normal.dot(step.gravity) < 0.001;
        if (step.velocity.length_squared()
            < state.configuration.minimum_speed * state.configuration.minimum_speed
            && supported)
            || step.flags.contains(ParticleFlags::SINGLE_CONTACT)
        {
            step.velocity = Vec3::ZERO;
            state.angular_velocity = Vec3::ZERO;
            state.spin_orientation = Quat::IDENTITY;
            step.settle_on(state, hit.position, hit.normal);
            if !step.flags.contains(ParticleFlags::CONSTANT_ORIENTATION)
                && state.configuration.alignment_normal.length_squared() > 0.0
            {
                step.orientation = align_axis(
                    step.orientation,
                    state.configuration.alignment_normal,
                    hit.normal,
                );
            }
        } else {
            let next_velocity = step.velocity
                + (step.gravity + state.heading * state.configuration.thrust_acceleration
                    - step.velocity * step.resistance
                    + particle_lift(state.heading, state.gravity)
                        * (state.lift_per_speed * step.velocity.length()))
                    * time_step;
            let supported_scale = if supported { 1.0 } else { 0.0 };
            if next_velocity.dot(hit.normal) * supported_scale
                < (state.configuration.minimum_speed + 0.001) * supported_scale
            {
                step.settle_on(state, hit.position, hit.normal);
            }
            if step.flags.contains(ParticleFlags::NO_ROLL) {
                state.angular_velocity = Vec3::ZERO;
            } else {
                state.angular_velocity = hit.normal.cross(tangent) / step.radius;
                if state.angular_velocity.length_squared() > 400.0 {
                    state.angular_velocity = state.angular_velocity.normalize() * 20.0;
                }
            }
        }
        step.velocity += target_velocity;
        state.force_awake =
            if hit.normal.z > 0.7 && (target_mass == 0.0 || state.time_force_awake > 40.0) {
                2
            } else {
                1
            };
        Ok(())
    }

    fn create_vehicle_state(
        &self,
        body: PhysicsBodyHandle,
        configuration: &WheeledVehicleConfiguration,
    ) -> Result<VehicleState, PhysicsError> {
        let native = self.native_body(body)?.rigid_body;
        let rigid_body = self
            .rigid_bodies
            .get(native)
            .ok_or(PhysicsError::BackendInvariant(
                "vehicle chassis references a missing Rapier rigid body",
            ))?;
        let mass = rigid_body.mass();
        if mass <= f32::EPSILON {
            return Err(PhysicsError::InvalidVehicleConfiguration {
                field: "chassis mass",
            });
        }
        let wheel_springs = self.vehicle_wheel_springs(configuration, rigid_body, mass);
        let controller =
            Self::build_vehicle_controller(native, configuration, &wheel_springs, mass);

        Ok(VehicleState {
            configuration: configuration.clone(),
            controller,
            wheels: configuration
                .wheels
                .iter()
                .zip(wheel_springs)
                .map(|(wheel, (stiffness, damping))| {
                    VehicleWheelRuntime::new(wheel.angular_velocity, stiffness, damping)
                })
                .collect(),
            pedal: 0.0,
            steer: 0.0,
            ackermann_offset: 0.0,
            hand_brake: true,
            clutch: 0.0,
            current_gear: 1,
            engine_angular_velocity: rpm_to_angular(configuration.engine_idle_rpm),
            driving_torque: 0.0,
            active_colliders: 0,
            time_without_chassis_contacts: 10.0,
            has_chassis_contacts: false,
        })
    }

    fn vehicle_has_chassis_contacts(&self, chassis: RigidBodyHandle) -> bool {
        self.rigid_bodies.get(chassis).is_some_and(|rigid_body| {
            rigid_body.colliders().iter().any(|&collider| {
                self.narrow_phase
                    .contact_pairs_with(collider)
                    .any(rapier3d::geometry::ContactPair::has_any_active_contact)
            })
        })
    }

    fn collect_vehicle_wheel_contacts(
        &self,
        body: PhysicsBodyHandle,
        chassis_handle: RigidBodyHandle,
        state: &VehicleState,
    ) -> Result<Vec<Option<VehicleWheelContact>>, PhysicsError> {
        let chassis =
            self.rigid_bodies
                .get(chassis_handle)
                .ok_or(PhysicsError::BackendInvariant(
                    "vehicle chassis disappeared while querying its wheels",
                ))?;
        let chassis_collision_classes = chassis
            .colliders()
            .iter()
            .filter_map(|handle| self.colliders.get(*handle))
            .map(|collider| decode_collision_class(collider.user_data))
            .collect::<SmallVec<[CollisionClass; 4]>>();
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            self.collider_metadata.get(&handle).is_some_and(|metadata| {
                metadata.body != body
                    && metadata.blocks_motion()
                    && chassis_collision_classes.iter().any(|collision_class| {
                        collision_class.interacts_with(decode_collision_class(collider.user_data))
                    })
            })
        };
        let filter = QueryFilter::default()
            .exclude_rigid_body(chassis_handle)
            .predicate(&predicate);
        let queries = self.query_broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.rigid_bodies,
            &self.colliders,
            filter,
        );
        let chassis_pose = chassis.position();
        let chassis_forward = chassis_pose.rotation * Vector::new(0.0, 1.0, 0.0);
        let mut contacts = Vec::with_capacity(state.configuration.wheels.len());

        for (configuration, controller_wheel) in state
            .configuration
            .wheels
            .iter()
            .zip(state.controller.wheels())
        {
            if configuration.axle < 0 {
                contacts.push(None);
                continue;
            }
            let hard_point = chassis_pose * convert::vector(configuration.connection);
            let suspension_direction =
                chassis_pose.rotation * convert::vector(configuration.suspension_direction);
            let steering =
                Rotation::from_scaled_axis(-suspension_direction * controller_wheel.steering);
            let axle =
                steering * (chassis_pose.rotation * convert::vector(configuration.axle_direction));

            let hit = if configuration.ray_cast {
                let maximum_distance = configuration.suspension_max_length + configuration.radius;
                let ray = Ray::new(hard_point, suspension_direction);
                queries
                    .cast_ray_and_get_normal(&ray, maximum_distance, true)
                    .map(|(collider, intersection)| VehicleWheelContact {
                        collider,
                        point: ray.point_at(intersection.time_of_impact),
                        normal: intersection.normal,
                        suspension_length: (intersection.time_of_impact - configuration.radius)
                            .clamp(0.0, configuration.suspension_max_length),
                    })
            } else {
                let local_axis = Vector::new(0.0, 1.0, 0.0);
                let rotation = if axle.length_squared() > f32::EPSILON {
                    Rotation::from_rotation_arc(local_axis, axle.normalize())
                } else {
                    Rotation::IDENTITY
                };
                let pose = Pose::from_parts(hard_point, rotation);
                let wheel_shape = Cylinder::new(configuration.half_width, configuration.radius);
                queries
                    .cast_shape(
                        &pose,
                        suspension_direction,
                        &wheel_shape,
                        ShapeCastOptions {
                            max_time_of_impact: configuration.suspension_max_length,
                            target_distance: 0.0,
                            stop_at_penetration: false,
                            compute_impact_geometry_on_penetration: true,
                        },
                    )
                    .map(|(collider, impact)| VehicleWheelContact {
                        collider,
                        point: impact.witness1,
                        normal: impact.normal1,
                        suspension_length: impact
                            .time_of_impact
                            .clamp(0.0, configuration.suspension_max_length),
                    })
            };

            contacts.push(hit.filter(|contact| {
                contact.normal.dot(chassis_forward).abs() < state.configuration.maximum_tilt_cosine
                    || state.configuration.keep_traction_when_tilted
            }));
        }
        Ok(contacts)
    }

    fn apply_vehicle_impulse_pair(
        &mut self,
        chassis: RigidBodyHandle,
        contact: VehicleWheelContact,
        impulse: Vector,
    ) {
        if impulse.length_squared() <= f32::EPSILON {
            return;
        }
        let ground = self
            .colliders
            .get(contact.collider)
            .and_then(Collider::parent)
            .filter(|ground| *ground != chassis);
        if let Some(chassis) = self.rigid_bodies.get_mut(chassis) {
            chassis.apply_impulse_at_point(impulse, contact.point, true);
        }
        if let Some(ground) = ground
            && let Some(ground) = self.rigid_bodies.get_mut(ground)
            && ground.is_dynamic()
        {
            ground.apply_impulse_at_point(-impulse, contact.point, true);
        }
    }

    fn apply_vehicle_wheel_overrides(
        &mut self,
        chassis_handle: RigidBodyHandle,
        state: &mut VehicleState,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        for index in 0..state.configuration.wheels.len() {
            let configuration = state.configuration.wheels[index];
            let tracked_pull =
                state.configuration.tracked_neutral_turn_steer != 0.0 && configuration.driving;
            if (configuration.ray_cast && !tracked_pull) || configuration.axle < 0 {
                continue;
            }
            let Some(contact) = state.wheels[index].contact else {
                if !configuration.ray_cast {
                    state.wheels[index].angular_velocity = (state.wheels[index].torque
                        * configuration.inverse_inertia)
                        .mul_add(time_step, state.wheels[index].angular_velocity);
                }
                continue;
            };
            let frame = self.vehicle_wheel_frame(chassis_handle, state, index, contact)?;
            if configuration.ray_cast {
                self.apply_tracked_pull(chassis_handle, state, frame, time_step);
            } else {
                self.solve_geometry_wheel(chassis_handle, state, frame, time_step)?;
            }
        }
        Ok(())
    }

    /// Builds one wheel's contact frame: its steered axle, the forward
    /// direction along the ground, and the chassis-relative contact velocity.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BackendInvariant`] when the chassis has gone
    /// missing between substeps.
    fn vehicle_wheel_frame(
        &self,
        chassis_handle: RigidBodyHandle,
        state: &VehicleState,
        index: usize,
        contact: VehicleWheelContact,
    ) -> Result<WheelFrame, PhysicsError> {
        let configuration = state.configuration.wheels[index];
        let chassis =
            self.rigid_bodies
                .get(chassis_handle)
                .ok_or(PhysicsError::BackendInvariant(
                    "vehicle chassis disappeared while solving a geometry wheel",
                ))?;
        let chassis_pose = *chassis.position();
        let suspension_up =
            -(chassis_pose.rotation * convert::vector(configuration.suspension_direction));
        let steering = state.controller.wheels()[index].steering;
        let steering_rotation = Rotation::from_scaled_axis(suspension_up * steering);
        let mut axle = steering_rotation
            * (chassis_pose.rotation * convert::vector(configuration.axle_direction));
        axle -= contact.normal * axle.dot(contact.normal);
        axle = axle.normalize_or_zero();
        let forward = contact.normal.cross(axle).normalize_or_zero();
        let ground = self
            .colliders
            .get(contact.collider)
            .and_then(Collider::parent)
            .filter(|ground| *ground != chassis_handle);
        let ground_velocity = ground
            .and_then(|ground| self.rigid_bodies.get(ground))
            .map_or(Vector::ZERO, |ground| {
                ground.velocity_at_point(contact.point)
            });
        Ok(WheelFrame {
            index,
            configuration,
            contact,
            chassis_pose,
            suspension_up,
            axle,
            forward,
            relative_velocity: chassis.velocity_at_point(contact.point) - ground_velocity,
            ground,
        })
    }

    /// Cry's tracked-vehicle pull: a ray-cast driving wheel of a tracked
    /// chassis drags along the tilted chassis forward axis instead of through
    /// the suspension.
    fn apply_tracked_pull(
        &mut self,
        chassis_handle: RigidBodyHandle,
        state: &VehicleState,
        frame: WheelFrame,
        time_step: f32,
    ) {
        let configuration = frame.configuration;
        let pull_direction = state.pull_direction(frame.chassis_pose);
        let friction = self
            .colliders
            .get(frame.contact.collider)
            .map_or(1.0, Collider::friction)
            .clamp(
                configuration.minimum_friction,
                configuration.maximum_friction,
            );
        let limit =
            state.controller.wheels()[frame.index].wheel_suspension_force * time_step * friction;
        let impulse = (state.wheels[frame.index].torque * time_step / configuration.radius)
            .clamp(-limit, limit);
        self.apply_vehicle_impulse_pair(chassis_handle, frame.contact, pull_direction * impulse);
    }

    /// Solves one geometry wheel: the suspension spring and stabilizer along
    /// the strut, then the clamped forward and lateral friction impulses, then
    /// the wheel's own angular velocity.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BackendInvariant`] when the chassis has gone
    /// missing between substeps.
    fn solve_geometry_wheel(
        &mut self,
        chassis_handle: RigidBodyHandle,
        state: &mut VehicleState,
        frame: WheelFrame,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        let index = frame.index;
        let configuration = frame.configuration;
        let contact = frame.contact;
        let mut forward = frame.forward;
        let buddy_length = state
            .configuration
            .wheels
            .iter()
            .enumerate()
            .find(|(buddy, wheel)| *buddy != index && wheel.axle == configuration.axle)
            .and_then(|(buddy, _)| state.wheels[buddy].contact)
            .map_or(contact.suspension_length, |buddy| buddy.suspension_length);
        let runtime = state.wheels[index];
        let mut normal_force = frame.relative_velocity.dot(frame.suspension_up).mul_add(
            -runtime.spring_damping,
            (configuration.suspension_max_length - contact.suspension_length)
                * runtime.spring_stiffness,
        );
        normal_force = ((contact.suspension_length - buddy_length) * runtime.spring_stiffness)
            .mul_add(-state.configuration.stabilizer, normal_force);
        let normal_impulse = normal_force.max(0.0) * time_step;

        let surface_friction = self
            .colliders
            .get(contact.collider)
            .map_or(1.0, Collider::friction)
            .clamp(
                configuration.minimum_friction,
                configuration.maximum_friction,
            );
        let friction = surface_friction
            * if runtime.slipping {
                state.configuration.dynamic_friction
            } else {
                1.0
            };
        let friction_limit = normal_impulse * friction.max(0.0);
        if state.configuration.tracked_neutral_turn_steer != 0.0 && configuration.driving {
            forward = state.pull_direction(frame.chassis_pose);
        }

        let chassis =
            self.rigid_bodies
                .get(chassis_handle)
                .ok_or(PhysicsError::BackendInvariant(
                    "vehicle chassis disappeared while solving a geometry wheel",
                ))?;
        let ground = frame
            .ground
            .and_then(|ground| self.rigid_bodies.get(ground));
        let forward_inverse_mass = directional_inverse_mass(chassis, contact.point, forward)
            + ground.map_or(0.0, |ground| {
                directional_inverse_mass(ground, contact.point, forward)
            });
        let lateral_inverse_mass = directional_inverse_mass(chassis, contact.point, frame.axle)
            + ground.map_or(0.0, |ground| {
                directional_inverse_mass(ground, contact.point, frame.axle)
            });
        let locked = (state.hand_brake && configuration.can_brake) || configuration.blocked;
        let wheel_surface_speed = if locked {
            0.0
        } else {
            runtime.angular_velocity * configuration.radius
        };
        let mut forward_impulse = if forward_inverse_mass > f32::EPSILON {
            -(frame.relative_velocity.dot(forward) - wheel_surface_speed) / forward_inverse_mass
        } else {
            0.0
        };
        if !locked {
            forward_impulse += runtime.torque * time_step / configuration.radius;
        }
        let lateral_impulse = if lateral_inverse_mass > f32::EPSILON {
            -frame.relative_velocity.dot(frame.axle) * configuration.lateral_friction
                / lateral_inverse_mass
        } else {
            0.0
        };
        let mut friction_impulse = forward * forward_impulse + frame.axle * lateral_impulse;
        friction_impulse = friction_impulse.clamp_length_max(friction_limit);
        forward_impulse = friction_impulse.dot(forward);
        self.apply_vehicle_impulse_pair(
            chassis_handle,
            contact,
            frame.suspension_up * normal_impulse,
        );
        self.apply_vehicle_impulse_pair(chassis_handle, contact, friction_impulse);
        state.wheels[index].angular_velocity = runtime
            .torque
            .mul_add(time_step, -(forward_impulse * configuration.radius))
            .mul_add(
                configuration.inverse_inertia,
                state.wheels[index].angular_velocity,
            );
        Ok(())
    }

    fn apply_vehicle_stabilizer(
        &mut self,
        chassis_handle: RigidBodyHandle,
        state: &VehicleState,
        time_step: f32,
    ) {
        if state.configuration.stabilizer <= 0.0 {
            return;
        }
        let Some(chassis) = self.rigid_bodies.get(chassis_handle) else {
            return;
        };
        let rotation = chassis.position().rotation;
        let corrections = state
            .configuration
            .wheels
            .iter()
            .enumerate()
            .filter_map(|(index, configuration)| {
                if !configuration.ray_cast || configuration.axle < 0 {
                    return None;
                }
                let contact = state.wheels[index].contact?;
                let (buddy_index, _) = state
                    .configuration
                    .wheels
                    .iter()
                    .enumerate()
                    .find(|(buddy, wheel)| *buddy != index && wheel.axle == configuration.axle)?;
                let buddy_contact = state.wheels[buddy_index].contact?;
                let runtime = state.wheels[index];
                let up = -(rotation * convert::vector(configuration.suspension_direction));
                let correction = (-(contact.suspension_length - buddy_contact.suspension_length)
                    * runtime.spring_stiffness
                    * state.configuration.stabilizer
                    * time_step)
                    .max(-state.controller.wheels()[index].wheel_suspension_force * time_step);
                Some((contact, up * correction))
            })
            .collect::<Vec<_>>();
        for (contact, impulse) in corrections {
            self.apply_vehicle_impulse_pair(chassis_handle, contact, impulse);
        }
    }

    fn step_vehicle_body(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        let native = self.native_body(body)?.rigid_body;
        let mut state =
            self.vehicles
                .remove(&body)
                .ok_or(PhysicsError::OperationRequiresVehicleBody {
                    operation: "step_vehicle_body",
                })?;
        let result = (|| {
            if self.vehicle_has_chassis_contacts(native) {
                state.time_without_chassis_contacts = 0.0;
            } else {
                state.time_without_chassis_contacts += time_step;
            }
            state.has_chassis_contacts = state.time_without_chassis_contacts < 0.5;

            let maximum_time_step = if state.has_chassis_contacts {
                state.configuration.rigid_body.maximum_time_step
            } else {
                state.configuration.maximum_time_step
            };
            let substep_count = convert::substeps(time_step, maximum_time_step);
            let substep = time_step / convert::f32_from_u32(substep_count);
            for _ in 0..substep_count {
                let contacts = self.collect_vehicle_wheel_contacts(body, native, &state)?;
                for (runtime, contact) in state.wheels.iter_mut().zip(contacts) {
                    runtime.contact = contact;
                }
                self.configure_controller_wheels(native, &mut state)?;
                state.configure_wheel_forces(substep);
                for (configuration, wheel) in state
                    .configuration
                    .wheels
                    .iter()
                    .zip(state.controller.wheels_mut())
                {
                    if !configuration.ray_cast
                        || (state.configuration.tracked_neutral_turn_steer != 0.0
                            && configuration.driving)
                    {
                        wheel.engine_force = 0.0;
                    }
                    if !configuration.ray_cast {
                        wheel.brake = 0.0;
                    }
                }
                let metadata = &self.collider_metadata;
                let predicate = |handle: ColliderHandle, _: &Collider| {
                    metadata
                        .get(&handle)
                        .is_some_and(|metadata| metadata.blocks_motion())
                };
                let filter = QueryFilter::default()
                    .exclude_rigid_body(native)
                    .predicate(&predicate);
                let queries = self.query_broad_phase.as_query_pipeline_mut(
                    self.narrow_phase.query_dispatcher(),
                    &mut self.rigid_bodies,
                    &mut self.colliders,
                    filter,
                );
                state.controller.update_vehicle(substep, queries);
                self.apply_vehicle_wheel_overrides(native, &mut state, substep)?;
                self.apply_vehicle_stabilizer(native, &state, substep);
            }

            self.publish_vehicle_wheel_state(body, native, &mut state, time_step)?;
            Ok(())
        })();
        self.vehicles.insert(body, state);
        result
    }

    fn deformable_attachment_frame(
        &self,
        body: Option<PhysicsBodyHandle>,
        point: Vec3,
        local: bool,
    ) -> Result<AttachmentFrame, PhysicsError> {
        let Some(body) = body else {
            return Ok(AttachmentFrame {
                position: point,
                body_position: point,
                center: point,
                rotation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            });
        };
        if body.scene() != self.scene {
            return Err(PhysicsError::ConstraintSceneMismatch {
                parent: body.scene(),
                child: self.scene,
            });
        }
        let rigid_body = self.rigid_body(body)?;
        let pose = convert::physics_pose(rigid_body.position());
        let position = if local {
            pose.transform_point(point)
        } else {
            point
        };
        let center = convert::vec3(rigid_body.center_of_mass());
        let linear_velocity = convert::vec3(rigid_body.linvel());
        let angular_velocity = convert::vec3(rigid_body.angvel());
        let velocity = linear_velocity + angular_velocity.cross(position - center);
        Ok(AttachmentFrame {
            position,
            body_position: pose.translation,
            center,
            rotation: pose.rotation,
            velocity,
            linear_velocity,
            angular_velocity,
        })
    }

    fn deformable_medium_sample(&self, position: Vec3, radius: f32) -> MediumSample {
        let mut water_fraction = 0.0_f32;
        let mut submerged_depth = 0.0_f32;
        let mut water_density = 0.0_f32;
        let mut water_flow = Vec3::ZERO;
        let mut air_flow = Vec3::ZERO;
        for (&area_body, area) in &self.fluid_areas {
            let Some(native) = self.bodies.get(&area_body) else {
                continue;
            };
            let overlaps = self.rigid_bodies[native.rigid_body]
                .colliders()
                .iter()
                .filter_map(|handle| self.colliders.get(*handle))
                .map(Collider::compute_aabb)
                .any(|bounds| {
                    position.x >= bounds.mins.x
                        && position.x <= bounds.maxs.x
                        && position.y >= bounds.mins.y
                        && position.y <= bounds.maxs.y
                        && position.z >= bounds.mins.z
                        && position.z <= bounds.maxs.z
                });
            if !overlaps {
                continue;
            }
            match area.medium {
                az_physics::FluidMedium::Water => {
                    let signed_distance = area.plane.signed_distance(position);
                    if signed_distance < radius {
                        water_fraction = water_fraction
                            .max(((radius - signed_distance) / (radius * 2.0)).clamp(0.0, 1.0));
                        let depth = (-signed_distance).max(0.0);
                        if depth >= submerged_depth {
                            submerged_depth = depth;
                            water_density = area.density;
                            water_flow = area.flow;
                        }
                    }
                }
                az_physics::FluidMedium::Air => air_flow += area.flow,
            }
        }
        MediumSample {
            submerged_fraction: water_fraction,
            submerged_depth,
            water_density,
            velocity: if water_fraction > 0.0 {
                water_flow
            } else {
                air_flow
            },
            gravity: None,
        }
    }

    fn deformable_contact(
        &self,
        body: PhysicsBodyHandle,
        from: Vec3,
        to: Vec3,
        radius: f32,
        filter: DeformableFilter<'_>,
    ) -> Result<Option<DeformableContact>, PhysicsError> {
        let DeformableFilter {
            physical_entity_types,
            collision_class,
            ignored_bodies,
        } = filter;
        let displacement = to - from;
        let distance = displacement.length();
        if physical_entity_types == PhysicalEntityTypes::NONE {
            return Ok(None);
        }
        let direction = if distance > f32::EPSILON {
            displacement / distance
        } else {
            Vec3::X
        };
        let mut excluded = Vec::with_capacity(ignored_bodies.len() + 1);
        excluded.push(body);
        excluded.extend_from_slice(ignored_bodies);
        let hit = self.cast_shape(&ShapeCastConfiguration {
            shape: ColliderShape::Sphere { radius },
            pose: PhysicsPose {
                translation: from,
                rotation: Quat::IDENTITY,
            },
            direction,
            max_distance: distance,
            target_distance: 0.0,
            stop_at_penetration: true,
            filter: SpatialQueryFilter {
                ignore_bodies: excluded,
                physical_entity_types,
                include_sensors: false,
                collision_class: Some(collision_class),
                ..SpatialQueryFilter::default()
            },
            max_results: 1,
        })?;
        let Some(hit) = hit else {
            return Ok(None);
        };
        let dynamic = self
            .bodies
            .get(&hit.body)
            .and_then(|body| self.rigid_bodies.get(body.rigid_body))
            .is_some_and(RigidBody::is_dynamic);
        let velocity = self
            .bodies
            .get(&hit.body)
            .and_then(|body| self.rigid_bodies.get(body.rigid_body))
            .map_or(Vec3::ZERO, |rigid_body| {
                let center = convert::vec3(rigid_body.center_of_mass());
                convert::vec3(rigid_body.linvel())
                    + convert::vec3(rigid_body.angvel()).cross(hit.position - center)
            });
        Ok(Some(DeformableContact {
            position: hit.position,
            normal: hit.normal,
            distance: hit.distance,
            velocity,
            body: hit.body,
            dynamic,
        }))
    }

    fn synchronize_deformable_query(
        &mut self,
        body: PhysicsBodyHandle,
        collider: ColliderHandle,
        points: &[Vec3],
        thickness: f32,
    ) -> Result<(), PhysicsError> {
        let (center, radius) = deformable_bounds(points);
        self.rigid_body_mut(body)?
            .set_next_kinematic_position(Pose::from_translation(convert::vector(center)));
        self.colliders
            .get_mut(collider)
            .ok_or(PhysicsError::BackendInvariant(
                "deformable body references a missing query collider",
            ))?
            .set_shape(SharedShape::ball((radius + thickness).max(1.0e-4)));
        Ok(())
    }

    fn apply_deformable_reactions(
        &mut self,
        reactions: impl IntoIterator<Item = DeformableReaction>,
    ) -> Result<(), PhysicsError> {
        for reaction in reactions {
            let rigid_body = self.native_body(reaction.body)?.rigid_body;
            let body =
                self.rigid_bodies
                    .get_mut(rigid_body)
                    .ok_or(PhysicsError::BackendInvariant(
                        "deformable contact references a missing rigid body",
                    ))?;
            if body.is_dynamic() {
                body.apply_impulse_at_point(
                    convert::vector(reaction.impulse),
                    convert::vector(reaction.point),
                    true,
                );
            }
        }
        Ok(())
    }

    fn create_soft_rigid_core(&mut self, body: PhysicsBodyHandle) -> Result<(), PhysicsError> {
        let Some(descriptor) = self
            .soft_bodies
            .get(&body)
            .and_then(SoftBodyState::rigid_core_descriptor)
        else {
            return Ok(());
        };
        let native_body = self.rigid_bodies.insert(
            RigidBodyBuilder::dynamic()
                .pose(convert::pose(descriptor.pose))
                .additional_mass(0.0)
                .gravity_scale(0.0)
                .linear_damping(
                    self.soft_bodies
                        .get(&body)
                        .map_or(0.0, |state| state.configuration.damping),
                )
                .angular_damping(
                    self.soft_bodies
                        .get(&body)
                        .map_or(0.0, |state| state.configuration.damping),
                )
                .can_sleep(false)
                .user_data(u128::from(body.get())),
        );
        let owner = ColliderOwner {
            body,
            rigid_body: native_body,
            entity_id: self.bodies.get(&body).and_then(|native| native.entity_id),
            query_type: PhysicalEntityTypes::DYNAMIC,
            continuous_collision_mode: az_physics::ContinuousCollisionMode::Disabled,
            continuous_prediction_distance: 0.0,
        };
        let native_collider = match self.insert_collider(owner, &descriptor.collider, true, false) {
            Ok(collider) => collider,
            Err(error) => {
                self.rigid_bodies.remove(
                    native_body,
                    &mut self.islands,
                    &mut self.colliders,
                    &mut self.impulse_joints,
                    &mut self.multibody_joints,
                    true,
                );
                return Err(error);
            }
        };
        self.soft_bodies
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresSoftBody {
                operation: "create_soft_rigid_core",
            })?
            .bind_rigid_core(native_body, native_collider)?;
        self.bodies
            .get_mut(&body)
            .ok_or(PhysicsError::BodyNotFound(body))?
            .collider_configurations
            .push(descriptor.collider);
        Ok(())
    }

    fn synchronize_soft_rigid_core(
        &self,
        body: PhysicsBodyHandle,
        state: &mut SoftBodyState,
    ) -> Result<(), PhysicsError> {
        let Some((native_body, native_collider)) = state.rigid_core_handles() else {
            return Ok(());
        };
        let rigid_body =
            self.rigid_bodies
                .get(native_body)
                .ok_or(PhysicsError::BackendInvariant(
                    "soft body references a missing rigid-core body",
                ))?;
        let has_contacts = self
            .narrow_phase
            .contact_pairs_with(native_collider)
            .any(|pair| {
                pair.has_any_active_contact()
                    && self
                        .collider_metadata
                        .get(&if pair.collider1 == native_collider {
                            pair.collider2
                        } else {
                            pair.collider1
                        })
                        .is_some_and(|metadata| metadata.body != body)
            });
        state.synchronize_rigid_core(convert::physics_pose(rigid_body.position()), has_contacts)
    }

    fn apply_soft_rigid_core_update(
        &mut self,
        update: crate::deformable::SoftRigidCoreUpdate,
    ) -> Result<(), PhysicsError> {
        let collider =
            self.colliders
                .get_mut(update.collider)
                .ok_or(PhysicsError::BackendInvariant(
                    "soft body references a missing rigid-core collider",
                ))?;
        if (collider.mass() - update.mass).abs() > f32::EPSILON {
            collider.set_mass(update.mass);
        }
        let rigid_body =
            self.rigid_bodies
                .get_mut(update.body)
                .ok_or(PhysicsError::BackendInvariant(
                    "soft body references a missing rigid-core body",
                ))?;
        if update.fit_to_soft_body {
            rigid_body.set_position(convert::pose(update.pose), true);
        }
        for reaction in update.reactions {
            rigid_body.apply_impulse_at_point(
                convert::vector(reaction.impulse),
                convert::vector(reaction.point),
                true,
            );
        }
        Ok(())
    }

    fn step_rope_body(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        if !self.rigid_body(body)?.is_enabled() {
            return Ok(());
        }
        let mut state =
            self.ropes
                .remove(&body)
                .ok_or(PhysicsError::OperationRequiresRopeBody {
                    operation: "step_rope_body",
                })?;
        let mut collision_types = PhysicalEntityTypes::NONE;
        if state.configuration.flags.contains(RopeFlags::COLLIDES) {
            collision_types |= state.configuration.collision_types;
        }
        if state
            .configuration
            .flags
            .contains(RopeFlags::COLLIDES_WITH_TERRAIN)
        {
            collision_types |= PhysicalEntityTypes::TERRAIN;
        }
        let ignored_attachments: Vec<_> = if state
            .configuration
            .flags
            .contains(RopeFlags::IGNORE_ATTACHMENTS)
        {
            state
                .configuration
                .attachments
                .iter()
                .flatten()
                .filter_map(|attachment| attachment.body)
                .collect()
        } else {
            Vec::new()
        };
        let collision_class = state.configuration.collision_class;
        let medium_radius = state.configuration.collision_distance.max(1.0e-4);
        let result = state.step(
            time_step,
            self.gravity,
            self.physics_time,
            |attachment| {
                self.deformable_attachment_frame(
                    attachment.body,
                    attachment.point,
                    attachment.local,
                )
            },
            |from, to, radius| {
                self.deformable_contact(
                    body,
                    from,
                    to,
                    radius,
                    DeformableFilter {
                        physical_entity_types: collision_types,
                        collision_class,
                        ignored_bodies: &ignored_attachments,
                    },
                )
            },
            |position| self.deformable_medium_sample(position, medium_radius),
        );
        let synchronization = if result.is_ok() {
            self.synchronize_deformable_query(
                body,
                state.query_collider,
                &state.points,
                state.configuration.collision_distance,
            )
        } else {
            Ok(())
        };
        let reactions = state.take_reactions();
        self.ropes.insert(body, state);
        result?;
        synchronization?;
        self.apply_deformable_reactions(reactions)
    }

    fn step_soft_body(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        if !self.rigid_body(body)?.is_enabled() {
            return Ok(());
        }
        let mut state =
            self.soft_bodies
                .remove(&body)
                .ok_or(PhysicsError::OperationRequiresSoftBody {
                    operation: "step_soft_body",
                })?;
        self.synchronize_soft_rigid_core(body, &mut state)?;
        let collision_types = state.configuration.collision_types;
        let collision_class = state.configuration.collision_class;
        let radius = state.configuration.thickness;
        let result = state.step(
            time_step,
            |attachment| {
                self.deformable_attachment_frame(
                    attachment.body,
                    attachment.point,
                    attachment.local,
                )
            },
            |from, to, radius| {
                self.deformable_contact(
                    body,
                    from,
                    to,
                    radius,
                    DeformableFilter {
                        physical_entity_types: collision_types,
                        collision_class,
                        ignored_bodies: &[],
                    },
                )
            },
            |position| self.deformable_medium_sample(position, radius),
        );
        let synchronization = if result.is_ok() {
            self.synchronize_deformable_query(
                body,
                state.query_collider,
                &state.vertices,
                state.configuration.thickness,
            )
        } else {
            Ok(())
        };
        let reactions = state.take_reactions();
        let rigid_core_update = state.take_rigid_core_update();
        self.soft_bodies.insert(body, state);
        result?;
        synchronization?;
        if let Some(update) = rigid_core_update {
            self.apply_soft_rigid_core_update(update)?;
        }
        self.apply_deformable_reactions(reactions)
    }

    fn step_linked_soft_body(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        if !self.rigid_body(body)?.is_enabled() {
            return Ok(());
        }
        let mut state = self.linked_soft_bodies.remove(&body).ok_or(
            PhysicsError::OperationRequiresLinkedSoftBody {
                operation: "step_linked_soft_body",
            },
        )?;
        let collision_types = state.configuration.collision_types;
        let collision_class = state.configuration.collision_class;
        let radius = state.configuration.collision_radius;
        let ignored_linked_soft_bodies = self
            .linked_soft_bodies
            .keys()
            .copied()
            .filter(|&other| other != body)
            .collect::<Vec<_>>();
        let result = state.step(
            time_step,
            self.gravity,
            |from, to, radius| {
                self.deformable_contact(
                    body,
                    from,
                    to,
                    radius,
                    DeformableFilter {
                        physical_entity_types: collision_types,
                        collision_class,
                        ignored_bodies: &ignored_linked_soft_bodies,
                    },
                )
            },
            |position| self.deformable_medium_sample(position, radius),
        );
        let reactions = state.take_reactions();
        self.linked_soft_bodies.insert(body, state);
        result?;
        self.apply_deformable_reactions(reactions)
    }

    fn solve_linked_soft_body_pairs(&mut self, time_step: f32) -> Result<(), PhysicsError> {
        let bodies = sorted_body_handles(self.linked_soft_bodies.keys().copied());
        for left_index in 0..bodies.len() {
            for right_index in (left_index + 1)..bodies.len() {
                let left_body = bodies[left_index];
                let right_body = bodies[right_index];
                let interacts = {
                    let left = &self.linked_soft_bodies[&left_body].configuration;
                    let right = &self.linked_soft_bodies[&right_body].configuration;
                    left.collision_types
                        .contains(PhysicalEntityTypes::INDEPENDENT)
                        && right
                            .collision_types
                            .contains(PhysicalEntityTypes::INDEPENDENT)
                        && left.collision_class.interacts_with(right.collision_class)
                };
                if !interacts {
                    continue;
                }
                let mut left = self.linked_soft_bodies.remove(&left_body).ok_or(
                    PhysicsError::BackendInvariant(
                        "linked-soft pair key disappeared during pair solving",
                    ),
                )?;
                let Some(mut right) = self.linked_soft_bodies.remove(&right_body) else {
                    self.linked_soft_bodies.insert(left_body, left);
                    return Err(PhysicsError::BackendInvariant(
                        "linked-soft pair key disappeared during pair solving",
                    ));
                };
                solve_linked_soft_body_pair(&mut left, &mut right, time_step);
                self.linked_soft_bodies.insert(left_body, left);
                self.linked_soft_bodies.insert(right_body, right);
            }
        }
        Ok(())
    }

    fn synchronize_linked_soft_body_query(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<(), PhysicsError> {
        let (collider, center, radius) = {
            let state = self.linked_soft_bodies.get(&body).ok_or(
                PhysicsError::OperationRequiresLinkedSoftBody {
                    operation: "synchronize_linked_soft_body_query",
                },
            )?;
            let (center, radius) = deformable_bounds(&state.vertices);
            (
                state.query_collider,
                center,
                radius + state.configuration.collision_radius,
            )
        };
        self.rigid_body_mut(body)?
            .set_next_kinematic_position(Pose::from_translation(convert::vector(center)));
        self.colliders
            .get_mut(collider)
            .ok_or(PhysicsError::BackendInvariant(
                "linked soft body references a missing query collider",
            ))?
            .set_shape(SharedShape::ball(radius.max(1.0e-4)));
        Ok(())
    }

    fn create_rigid_body_builder(descriptor: &BodyDescriptor) -> RigidBodyBuilder {
        let builder = match &descriptor.kind {
            BodyKind::Static { .. } | BodyKind::Area | BodyKind::FluidArea(_) => {
                RigidBodyBuilder::fixed()
            }
            BodyKind::Rigid(configuration) | BodyKind::Articulated(configuration) => {
                rigid_body_motion_builder(configuration.motion)
            }
            BodyKind::WheeledVehicle(configuration) => {
                rigid_body_motion_builder(configuration.rigid_body.motion)
            }
            BodyKind::Living(_)
            | BodyKind::Character(_)
            | BodyKind::Particle(_)
            | BodyKind::Rope(_)
            | BodyKind::Soft(_)
            | BodyKind::LinkedSoft(_) => RigidBodyBuilder::kinematic_position_based(),
            BodyKind::Query(configuration) => {
                if configuration.dynamic {
                    RigidBodyBuilder::kinematic_position_based()
                } else {
                    RigidBodyBuilder::fixed()
                }
            }
        };
        builder.pose(convert::pose(descriptor.pose))
    }

    fn apply_living_action(
        state: &mut LivingState,
        action: PhysicsAction,
    ) -> Option<SyncLivingAction> {
        match action {
            PhysicsAction::Move(action) => {
                apply_living_move(state, action);
                None
            }
            PhysicsAction::Impulse(action) => {
                apply_living_impulse(state, action);
                None
            }
            PhysicsAction::AngularImpulse(_)
            | PhysicsAction::Force(_)
            | PhysicsAction::Torque(_)
            | PhysicsAction::SetMass(_)
            | PhysicsAction::SetDensity(_)
            | PhysicsAction::SetLinearDamping(_)
            | PhysicsAction::SetAngularDamping(_)
            | PhysicsAction::SetSleepMinEnergy(_)
            | PhysicsAction::SetBuoyancy(_)
            | PhysicsAction::SetSimulated(_)
            | PhysicsAction::SetLivingDimensions(_)
            | PhysicsAction::SetLivingDynamics(_)
            | PhysicsAction::SetAngularVelocity(_)
            | PhysicsAction::Wake(_) => None,
            PhysicsAction::Reset => {
                state.velocity = Vec3::ZERO;
                state.unconstrained_velocity = Vec3::ZERO;
                state.requested_velocity = Vec3::ZERO;
                state.flying = true;
                state.time_flying = 0.0;
                state.force_flight = false;
                state.jump_requested = false;
                state.camera_vertical_offset = 0.0;
                state.camera_offset_speed = 0.0;
                state.stable_height_time = 1.0;
                None
            }
            PhysicsAction::SetVelocity(velocity) => {
                state.velocity = velocity;
                state.unconstrained_velocity = velocity;
                state.requested_velocity = velocity;
                state.jump_requested = true;
                None
            }
            PhysicsAction::SetPose(pose) => Some(SyncLivingAction {
                pose,
                velocity: state.velocity,
                requested_velocity: state.requested_velocity,
            }),
            PhysicsAction::SyncLiving(sync) => {
                state.velocity = sync.velocity;
                state.unconstrained_velocity = sync.velocity;
                state.requested_velocity = sync.requested_velocity;
                Some(sync)
            }
        }
    }

    fn apply_character_action(
        state: &mut CharacterState,
        action: PhysicsAction,
    ) -> Option<SyncLivingAction> {
        match action {
            PhysicsAction::Move(action) => {
                match action.mode {
                    LivingMoveMode::RequestedVelocity | LivingMoveMode::ForceFlight => {
                        state.requested_velocity = action.velocity;
                    }
                    LivingMoveMode::SetVelocity => {
                        state.velocity = action.velocity;
                        state.requested_velocity = action.velocity;
                    }
                    LivingMoveMode::AddVelocity => {
                        state.velocity += action.velocity;
                        state.requested_velocity = state.velocity;
                    }
                }
                None
            }
            PhysicsAction::Impulse(action) => {
                let impulse = if action.explosion {
                    action.impulse * 0.3
                } else {
                    action.impulse
                };
                state.velocity += impulse / state.configuration.mass;
                state.requested_velocity = state.velocity;
                None
            }
            PhysicsAction::Reset => {
                state.velocity = Vec3::ZERO;
                state.requested_velocity = Vec3::ZERO;
                state.flying = true;
                state.time_flying = 0.0;
                None
            }
            PhysicsAction::SetVelocity(velocity) => {
                state.velocity = velocity;
                state.requested_velocity = velocity;
                None
            }
            PhysicsAction::SetPose(pose) => Some(SyncLivingAction {
                pose,
                velocity: state.velocity,
                requested_velocity: state.requested_velocity,
            }),
            PhysicsAction::SyncLiving(sync) => {
                state.velocity = sync.velocity;
                state.requested_velocity = sync.requested_velocity;
                Some(sync)
            }
            PhysicsAction::AngularImpulse(_)
            | PhysicsAction::Force(_)
            | PhysicsAction::Torque(_)
            | PhysicsAction::Wake(_)
            | PhysicsAction::SetAngularVelocity(_)
            | PhysicsAction::SetMass(_)
            | PhysicsAction::SetDensity(_)
            | PhysicsAction::SetLinearDamping(_)
            | PhysicsAction::SetAngularDamping(_)
            | PhysicsAction::SetSleepMinEnergy(_)
            | PhysicsAction::SetBuoyancy(_)
            | PhysicsAction::SetSimulated(_)
            | PhysicsAction::SetLivingDimensions(_)
            | PhysicsAction::SetLivingDynamics(_) => None,
        }
    }

    fn step_character_body(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        let native = self.native_body(body)?.clone();
        let mut state = self.characters.get(&body).cloned().ok_or(
            PhysicsError::OperationRequiresCharacterBody {
                operation: "integrate",
            },
        )?;
        let rigid_pose = *self
            .rigid_bodies
            .get(native.rigid_body)
            .ok_or(PhysicsError::BackendInvariant(
                "character body references a missing Rapier rigid body",
            ))?
            .position();
        let collider =
            self.colliders
                .get(state.primary_collider)
                .ok_or(PhysicsError::BackendInvariant(
                    "character body references a missing primary collider",
                ))?;
        let character_pose = *collider.position();
        let character_shape = collider.shared_shape().clone();
        let collision_class = decode_collision_class(collider.user_data);
        let mut cast_pose = character_pose;
        let desired_velocity = state.requested_velocity;
        let mut projected_velocity = desired_velocity;
        let mut remaining_time = time_step;
        let mut translation = Vec3::ZERO;
        let mut contacts = SmallVec::<[CharacterContactPlane; 8]>::new();

        for _ in 0..state.configuration.solver_max_iterations {
            if remaining_time <= f32::EPSILON || projected_velocity.length_squared() <= f32::EPSILON
            {
                break;
            }
            let impact = self.cast_character_shape(
                CharacterSweep {
                    body,
                    native_body: native.rigid_body,
                    collision_class,
                    pose: &cast_pose,
                    shape: character_shape.as_ref(),
                    excluded: &[],
                },
                projected_velocity,
                remaining_time,
                state.configuration.contact_distance,
            );
            let Some((handle, hit)) = impact else {
                let advance = projected_velocity * remaining_time;
                cast_pose.translation += convert::vector(advance);
                translation += advance;
                break;
            };

            let advance_time = hit.time_of_impact.clamp(0.0, remaining_time);
            let advance = projected_velocity * advance_time;
            cast_pose.translation += convert::vector(advance);
            translation += advance;
            remaining_time -= advance_time;

            let contact = self.character_contact_plane(handle, hit)?;
            self.apply_character_contact_impulse(projected_velocity, contact)?;
            contacts.push(contact);

            let next_velocity = project_character_velocity(desired_velocity, &contacts);
            let stalled = advance_time <= f32::EPSILON
                && next_velocity.abs_diff_eq(projected_velocity, 1.0e-6);
            projected_velocity = next_velocity;
            if stalled {
                break;
            }
        }

        self.collect_character_support_contacts(
            CharacterSweep {
                body,
                native_body: native.rigid_body,
                collision_class,
                pose: &cast_pose,
                shape: character_shape.as_ref(),
                excluded: &[],
            },
            &state.configuration,
            &mut contacts,
        )?;

        self.publish_character_support(
            native.rigid_body,
            &mut state,
            &contacts,
            CharacterMotion {
                rigid_pose,
                translation,
                time_step,
            },
        )?;
        self.characters.insert(body, state);
        Ok(())
    }

    fn cast_character_shape(
        &self,
        sweep: CharacterSweep<'_>,
        velocity: Vec3,
        max_time_of_impact: f32,
        target_distance: f32,
    ) -> Option<(ColliderHandle, rapier3d::parry::query::ShapeCastHit)> {
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let Some(metadata) = self.collider_metadata.get(&handle) else {
                return false;
            };
            metadata.body != sweep.body
                && metadata.blocks_motion()
                && !sweep.excluded.contains(&handle)
                && sweep
                    .collision_class
                    .interacts_with(decode_collision_class(collider.user_data))
        };
        let filter = QueryFilter::default()
            .exclude_rigid_body(sweep.native_body)
            .predicate(&predicate);
        let queries = self.query_broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.rigid_bodies,
            &self.colliders,
            filter,
        );
        queries.cast_shape(
            sweep.pose,
            convert::vector(velocity),
            sweep.shape,
            ShapeCastOptions {
                max_time_of_impact,
                target_distance,
                stop_at_penetration: false,
                compute_impact_geometry_on_penetration: true,
            },
        )
    }

    fn collect_character_support_contacts(
        &self,
        sweep: CharacterSweep<'_>,
        configuration: &CharacterBodyConfiguration,
        contacts: &mut SmallVec<[CharacterContactPlane; 8]>,
    ) -> Result<(), PhysicsError> {
        let mut excluded = SmallVec::<[ColliderHandle; 8]>::new();
        for _ in 0..configuration.solver_max_iterations {
            let Some((handle, hit)) = self.cast_character_shape(
                CharacterSweep {
                    excluded: &excluded,
                    ..sweep
                },
                -configuration.up_direction,
                CHARACTER_SUPPORT_PROBE_TIME,
                configuration.contact_distance,
            ) else {
                break;
            };
            excluded.push(handle);
            let support_contact = self.character_contact_plane(handle, hit)?;
            if !contacts.iter().any(|contact| {
                contact.body == support_contact.body
                    && contact.normal.abs_diff_eq(support_contact.normal, 1.0e-5)
            }) {
                contacts.push(support_contact);
            }
        }
        Ok(())
    }

    fn character_contact_plane(
        &self,
        handle: ColliderHandle,
        hit: rapier3d::parry::query::ShapeCastHit,
    ) -> Result<CharacterContactPlane, PhysicsError> {
        let metadata =
            self.collider_metadata
                .get(&handle)
                .copied()
                .ok_or(PhysicsError::BackendInvariant(
                    "character cast returned an untracked collider",
                ))?;
        let rigid_body = self
            .colliders
            .get(handle)
            .and_then(Collider::parent)
            .and_then(|parent| self.rigid_bodies.get(parent));
        let point = hit.witness1;
        let velocity = rigid_body.map_or(Vec3::ZERO, |rigid_body| {
            convert::vec3(rigid_body.velocity_at_point(point))
        });
        Ok(CharacterContactPlane {
            normal: convert::vec3(hit.normal1).normalize_or_zero(),
            velocity,
            point: convert::vec3(point),
            distance: hit.time_of_impact,
            body: metadata.body,
            entity_id: metadata.entity_id,
            surface_index: metadata.surface_index,
        })
    }

    fn apply_character_contact_impulse(
        &mut self,
        character_velocity: Vec3,
        contact: CharacterContactPlane,
    ) -> Result<(), PhysicsError> {
        let native = self.native_body(contact.body)?;
        let Some(rigid_body) = self.rigid_bodies.get_mut(native.rigid_body) else {
            return Err(PhysicsError::BackendInvariant(
                "character contact references a missing rigid body",
            ));
        };
        if !rigid_body.is_dynamic() {
            return Ok(());
        }

        let normal = convert::vector(contact.normal);
        let contact_point = convert::vector(contact.point);
        let relative_velocity = character_velocity - contact.velocity;
        let numerator = (-relative_velocity.dot(contact.normal)).max(0.0);
        if numerator <= f32::EPSILON {
            return Ok(());
        }
        let mass_properties = rigid_body.mass_properties();
        let relative_point = contact_point - mass_properties.world_com;
        let angular_axis = relative_point.cross(normal);
        let linear_inverse_mass = normal.dot(mass_properties.effective_inv_mass * normal);
        let angular_inverse_mass =
            angular_axis.dot(mass_properties.effective_world_inv_inertia * angular_axis);
        let denominator = linear_inverse_mass + angular_inverse_mass;
        if denominator <= f32::EPSILON {
            return Ok(());
        }
        rigid_body.apply_impulse_at_point(-normal * (numerator / denominator), contact_point, true);
        Ok(())
    }

    fn living_support_contact(
        &self,
        body: PhysicsBodyHandle,
        native_body: RigidBodyHandle,
        state: &LivingState,
        pose: &Pose,
        shape: &dyn Shape,
    ) -> Result<Option<CharacterContactPlane>, PhysicsError> {
        let collision_types = state.configuration.dynamics.collision_types();
        let collision_class = state.configuration.collision_class;
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let Some(metadata) = self.collider_metadata.get(&handle) else {
                return false;
            };
            metadata.body != body
                && metadata.blocks_motion()
                && collision_types.intersects(metadata.query_type)
                && collision_class.interacts_with(decode_collision_class(collider.user_data))
        };
        let filter = QueryFilter::default()
            .exclude_rigid_body(native_body)
            .predicate(&predicate);
        let queries = self.query_broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.rigid_bodies,
            &self.colliders,
            filter,
        );
        let dimensions = state.configuration.dimensions;
        let probe_distance =
            living_foot_gap(dimensions) + dimensions.ground_contact_epsilon.max(0.004);
        queries
            .cast_shape(
                pose,
                -Vector::Z,
                shape,
                ShapeCastOptions {
                    max_time_of_impact: probe_distance,
                    target_distance: 0.0,
                    stop_at_penetration: false,
                    compute_impact_geometry_on_penetration: true,
                },
            )
            .map(|(handle, hit)| self.character_contact_plane(handle, hit))
            .transpose()
    }

    fn living_head_obstruction_offset(
        &self,
        body: PhysicsBodyHandle,
        native_body: RigidBodyHandle,
        state: &LivingState,
        primary_pose: &Pose,
    ) -> Option<f32> {
        let dimensions = state.configuration.dimensions;
        if dimensions.head_radius <= 0.0 {
            return None;
        }
        let sweep_distance = dimensions.height_head
            - dimensions.height_collider
            - state.camera_vertical_offset.min(0.0);
        if sweep_distance <= 0.0 {
            return None;
        }

        let collision_class = state.configuration.collision_class;
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let Some(metadata) = self.collider_metadata.get(&handle) else {
                return false;
            };
            let fixed = collider
                .parent()
                .and_then(|parent| self.rigid_bodies.get(parent))
                .is_none_or(RigidBody::is_fixed);
            metadata.body != body
                && metadata.blocks_motion()
                && fixed
                && metadata
                    .query_type
                    .intersects(PhysicalEntityTypes::TERRAIN.union(PhysicalEntityTypes::STATIC))
                && collision_class.interacts_with(decode_collision_class(collider.user_data))
        };
        let filter = QueryFilter::default()
            .exclude_rigid_body(native_body)
            .predicate(&predicate);
        let queries = self.query_broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.rigid_bodies,
            &self.colliders,
            filter,
        );
        let head = Ball::new(dimensions.head_radius);
        let (_, hit) = queries.cast_shape(
            primary_pose,
            Vector::Z,
            &head,
            ShapeCastOptions {
                max_time_of_impact: sweep_distance,
                target_distance: 0.0,
                stop_at_penetration: false,
                compute_impact_geometry_on_penetration: true,
            },
        )?;
        Some(sweep_distance + state.camera_vertical_offset.min(0.0) - hit.time_of_impact)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the ordering is the Cry living-entity integration transaction: integrate, query, solve, then publish status"
    )]
    fn step_living_body(
        &mut self,
        body: PhysicsBodyHandle,
        requested_step: f32,
    ) -> Result<(), PhysicsError> {
        let native = self.native_body(body)?.clone();
        let mut state = self
            .living
            .get(&body)
            .cloned()
            .ok_or(PhysicsError::ActionRequiresLivingBody { action: "step" })?;
        let rigid_pose = *self
            .rigid_bodies
            .get(native.rigid_body)
            .ok_or(PhysicsError::BackendInvariant(
                "living body references a missing Rapier rigid body",
            ))?
            .position();
        let collider =
            self.colliders
                .get(state.primary_collider)
                .ok_or(PhysicsError::BackendInvariant(
                    "living body references a missing primary collider",
                ))?;
        let character_pose = *collider.position();
        let character_shape = collider.shared_shape().clone();

        let time_step = state
            .requested_time_step
            .take()
            .filter(|time_step| *time_step > 0.0)
            .map_or(requested_step, |time_step| time_step.min(requested_step));
        state.time_since_stance_change += time_step;

        if !state.configuration.dynamics.is_active {
            let mut next_pose = rigid_pose;
            next_pose.translation += convert::vector(state.requested_velocity * time_step);
            self.rigid_bodies
                .get_mut(native.rigid_body)
                .ok_or(PhysicsError::BackendInvariant(
                    "living body disappeared while applying inactive movement",
                ))?
                .set_next_kinematic_position(next_pose);
            if state
                .configuration
                .dynamics
                .release_ground_collider_when_not_active
            {
                state.ground_velocity = Vec3::ZERO;
                state.ground_body = None;
                state.ground_surface = None;
            }
            state.force_flight = false;
            state.time_force_inertia = (state.time_force_inertia - time_step).max(0.0);
            self.living.insert(body, state);
            return Ok(());
        }

        let gravity = if state.configuration.dynamics.use_custom_gravity {
            state.configuration.dynamics.gravity
        } else {
            self.gravity
        };
        let previous_ground_velocity = state.ground_velocity;
        let velocity = integrate_living_velocity(&state, gravity, time_step);
        state.unconstrained_velocity = velocity;
        let desired_translation = (velocity + previous_ground_velocity) * time_step
            + if state.flying && !state.configuration.dynamics.is_swimming && !state.force_flight {
                gravity * (0.5 * time_step * time_step)
            } else {
                Vec3::ZERO
            };

        let collision_types = state.configuration.dynamics.collision_types();
        let collision_class = state.configuration.collision_class;
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let Some(metadata) = self.collider_metadata.get(&handle) else {
                return false;
            };
            metadata.blocks_motion()
                && collision_types.intersects(metadata.query_type)
                && collision_class.interacts_with(decode_collision_class(collider.user_data))
        };
        let filter = QueryFilter::default()
            .exclude_rigid_body(native.rigid_body)
            .predicate(&predicate);
        let queries = self.query_broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.rigid_bodies,
            &self.colliders,
            filter,
        );
        let controller = living_controller(&state);
        let mut collisions = Vec::<CharacterCollision>::new();
        let movement = controller.move_shape(
            time_step,
            &queries,
            character_shape.as_ref(),
            &character_pose,
            convert::vector(desired_translation),
            |collision| collisions.push(collision),
        );
        if !collisions.is_empty() {
            let predicate = |handle: ColliderHandle, collider: &Collider| {
                let Some(metadata) = self.collider_metadata.get(&handle) else {
                    return false;
                };
                metadata.blocks_motion()
                    && collision_types.intersects(metadata.query_type)
                    && collision_class.interacts_with(decode_collision_class(collider.user_data))
            };
            let filter = QueryFilter::default()
                .exclude_rigid_body(native.rigid_body)
                .predicate(&predicate);
            let mut queries = self.query_broad_phase.as_query_pipeline_mut(
                self.narrow_phase.query_dispatcher(),
                &mut self.rigid_bodies,
                &mut self.colliders,
                filter,
            );
            controller.solve_character_collision_impulses(
                time_step,
                &mut queries,
                character_shape.as_ref(),
                state.configuration.dynamics.mass,
                &collisions,
            );
        }

        let mut translation = convert::vec3(movement.translation);
        let mut next_pose = rigid_pose;
        next_pose.translation += convert::vector(translation);
        let mut next_character_pose = character_pose;
        next_character_pose.translation += movement.translation;

        let probed_support = self.living_support_contact(
            body,
            native.rigid_body,
            &state,
            &next_character_pose,
            character_shape.as_ref(),
        )?;
        let mut support = probed_support;
        if !state.jump_requested
            && let Some(contact) = probed_support
        {
            let minimum_ground_dot = state
                .configuration
                .dynamics
                .min_fall_angle
                .to_radians()
                .cos();
            if contact.normal.z >= minimum_ground_dot {
                let correction = living_foot_gap(state.configuration.dimensions) - contact.distance;
                translation.z += correction;
                next_pose.translation.z += correction;
                next_character_pose.translation.z += correction;
            }
        }
        if support.is_none() {
            for collision in &collisions {
                let contact = self.character_contact_plane(collision.handle, collision.hit)?;
                if contact.normal.z > 0.087
                    && support.is_none_or(|current| contact.distance < current.distance)
                {
                    support = Some(contact);
                }
            }
        }
        self.rigid_bodies
            .get_mut(native.rigid_body)
            .ok_or(PhysicsError::BackendInvariant(
                "living body disappeared while applying movement",
            ))?
            .set_next_kinematic_position(next_pose);

        let was_flying = state.flying;
        let minimum_ground_dot = state
            .configuration
            .dynamics
            .min_fall_angle
            .to_radians()
            .cos();
        let grounded = support.is_some_and(|contact| contact.normal.z >= minimum_ground_dot)
            && !state.configuration.dynamics.is_swimming
            && gravity.z <= 0.0;
        state.flying = !grounded;
        if state.flying {
            state.time_flying += time_step;
            let mut airborne_velocity = velocity;
            if !was_flying {
                airborne_velocity += previous_ground_velocity;
            }
            airborne_velocity += gravity * time_step;
            if state.configuration.dynamics.is_swimming {
                let inertia = selected_living_inertia(&state);
                airborne_velocity +=
                    (state.requested_velocity - airborne_velocity) * inertia * time_step;
            }
            state.velocity = apply_living_air_resistance(
                airborne_velocity,
                state.configuration.dynamics.air_resistance,
                time_step,
            );
            state.ground_velocity = Vec3::ZERO;
            state.ground_body = None;
            state.ground_surface = None;
        } else {
            let contact = support.ok_or(PhysicsError::BackendInvariant(
                "grounded living body has no support contact",
            ))?;
            state.time_flying = 0.0;
            state.ground_height = convert::vec3(next_pose.translation).z
                - state.configuration.dimensions.height_pivot;
            state.ground_slope = contact.normal;
            state.ground_surface = Some(contact.surface_index);
            state.ground_velocity = clamp_length(
                contact.velocity,
                state.configuration.dynamics.max_ground_velocity,
            );
            let support_native = self.native_body(contact.body)?;
            let support_rigid_body = self.rigid_bodies.get(support_native.rigid_body).ok_or(
                PhysicsError::BackendInvariant(
                    "living support references a missing Rapier rigid body",
                ),
            )?;
            state.ground_body = (!support_rigid_body.is_fixed()).then_some(contact.body);
            state.velocity = translation / time_step - state.ground_velocity;
        }

        update_living_camera_offset(
            &mut state,
            was_flying,
            velocity.z,
            previous_ground_velocity
                .z
                .mul_add(-time_step, translation.z),
            time_step,
        );
        if let Some(obstruction_offset) = self.living_head_obstruction_offset(
            body,
            native.rigid_body,
            &state,
            &next_character_pose,
        ) && (state.camera_vertical_offset < obstruction_offset
            || state.camera_offset_speed.abs() + state.camera_offset_acceleration.abs()
                <= f32::EPSILON)
        {
            state.camera_vertical_offset = obstruction_offset;
        }
        state.force_flight = false;
        state.jump_requested = false;
        state.time_force_inertia = (state.time_force_inertia - time_step).max(0.0);
        state.unconstrained_velocity = state.velocity;
        self.living.insert(body, state);
        Ok(())
    }

    fn clamp_rigid_body_angular_velocities(&mut self, time_step: f32) {
        for native in self.bodies.values() {
            let Some(configuration) = native.rigid_configuration else {
                continue;
            };
            let Some(rigid_body) = self.rigid_bodies.get_mut(native.rigid_body) else {
                continue;
            };
            let angular_velocity = rigid_body.angvel();
            let max_velocity = configuration.max_angular_displacement.map_or(
                configuration.max_angular_velocity,
                |displacement| {
                    configuration
                        .max_angular_velocity
                        .min(displacement / time_step)
                },
            );
            if angular_velocity.length_squared() > max_velocity * max_velocity {
                rigid_body.set_angvel(angular_velocity.normalize() * max_velocity, false);
            }
        }
    }

    /// Integrates force accumulators and native linear-step damping before
    /// Rapier solves contacts. Rapier receives zero gravity and cleared force
    /// accumulators for these bodies, so each producer is consumed exactly
    /// once and the authored damping formula is not replaced by Rapier's
    /// reciprocal/exponential model.
    fn integrate_linear_step_damping(&mut self, time_step: f32) {
        let mut bodies = self
            .bodies
            .iter()
            .filter_map(|(&body, native)| {
                native.rigid_configuration.and_then(|configuration| {
                    matches!(
                        configuration.damping_model,
                        RigidBodyDampingModel::LinearStep { .. }
                    )
                    .then(|| {
                        let damping = self
                            .vehicles
                            .get(&body)
                            .filter(|vehicle| !vehicle.has_chassis_contacts)
                            .map_or(
                                (configuration.linear_damping, configuration.angular_damping),
                                |vehicle| {
                                    (vehicle.configuration.damping, vehicle.configuration.damping)
                                },
                            );
                        (body, native.rigid_body, configuration, damping)
                    })
                })
            })
            .collect::<Vec<_>>();
        bodies.sort_unstable_by_key(|(body, _, _, _)| *body);

        for (_, handle, configuration, (linear_damping, angular_damping)) in bodies {
            let Some(rigid_body) = self.rigid_bodies.get_mut(handle) else {
                continue;
            };
            if !rigid_body.is_dynamic() || !rigid_body.is_enabled() || rigid_body.is_sleeping() {
                continue;
            }

            let mass_properties = rigid_body.mass_properties();
            let gravity = if configuration.gravity_enabled {
                convert::vector(self.gravity)
            } else {
                Vector::ZERO
            };
            let mut linear_velocity = rigid_body.linvel()
                + (gravity + rigid_body.user_force() * mass_properties.effective_inv_mass)
                    * time_step;
            let mut angular_velocity = rigid_body.angvel()
                + mass_properties.effective_world_inv_inertia
                    * rigid_body.user_torque()
                    * time_step;
            rigid_body.reset_forces(false);
            rigid_body.reset_torques(false);

            let RigidBodyDampingModel::LinearStep {
                low_speed_decrement,
            } = configuration.damping_model
            else {
                unreachable!("filtered to linear-step bodies")
            };
            linear_velocity = damp_linear_step(
                linear_velocity,
                linear_damping,
                low_speed_decrement,
                time_step,
            );
            angular_velocity = damp_linear_step(
                angular_velocity,
                angular_damping,
                low_speed_decrement,
                time_step,
            );
            let max_angular_velocity = configuration.max_angular_displacement.map_or(
                configuration.max_angular_velocity,
                |displacement| {
                    configuration
                        .max_angular_velocity
                        .min(displacement / time_step)
                },
            );
            angular_velocity = angular_velocity.clamp_length_max(max_angular_velocity);

            rigid_body.set_linvel(linear_velocity, false);
            rigid_body.set_angvel(angular_velocity, false);
        }
    }

    fn body_aabb_by_native_handle(&self, rigid_body: RigidBodyHandle) -> Option<Aabb3d> {
        let body = self.rigid_bodies.get(rigid_body)?;
        let mut colliders = body
            .colliders()
            .iter()
            .filter_map(|handle| self.colliders.get(*handle))
            .map(Collider::compute_aabb);
        let first = colliders.next()?;
        let bounds = colliders.fold(first, |mut bounds, collider| {
            bounds.merge(&collider);
            bounds
        });
        Some(Aabb3d::from_min_max(
            Vec3::new(bounds.mins.x, bounds.mins.y, bounds.mins.z),
            Vec3::new(bounds.maxs.x, bounds.maxs.y, bounds.maxs.z),
        ))
    }

    /// Direct `CRigidEntity::ApplyBuoyancy` force/damping pass.
    fn apply_buoyancy(&mut self, time_step: f32) {
        let mut areas = self
            .fluid_areas
            .iter()
            .filter_map(|(&handle, &configuration)| {
                let native = self.bodies.get(&handle)?;
                Some((
                    handle,
                    configuration,
                    self.body_aabb_by_native_handle(native.rigid_body)?,
                ))
            })
            .collect::<Vec<_>>();
        areas.sort_by_key(|(handle, _, _)| handle.get());

        let mut candidates = self
            .bodies
            .iter()
            .filter_map(|(&handle, native)| {
                native
                    .rigid_configuration
                    .map(|configuration| (handle, native.clone(), configuration))
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(handle, _, _)| *handle);

        for (handle, native, configuration) in candidates {
            let Some(rigid_body) = self.rigid_bodies.get(native.rigid_body) else {
                continue;
            };
            if !rigid_body.is_dynamic() || !rigid_body.is_enabled() || rigid_body.mass() <= 0.0 {
                self.buoyancy_status
                    .insert(handle, BuoyancyStatus::default());
                continue;
            }
            let Some(body_bounds) = self.body_aabb_by_native_handle(native.rigid_body) else {
                continue;
            };
            let buoyant = BuoyantBody {
                pose: convert::physics_pose(rigid_body.position()),
                bounds: body_bounds,
                center_of_mass: convert::vec3(rigid_body.center_of_mass()),
                linear_velocity: convert::vec3(rigid_body.linvel()),
                angular_velocity: convert::vec3(rigid_body.angvel()),
                mass: rigid_body.mass(),
            };
            let load = self.accumulate_buoyancy(&native, configuration, buoyant, &areas, time_step);

            if let Some(rigid_body) = self.rigid_bodies.get_mut(native.rigid_body) {
                if load.linear_impulse.length_squared() > 0.0 {
                    rigid_body.apply_impulse(convert::vector(load.linear_impulse), true);
                }
                if load.angular_impulse.length_squared() > 0.0 {
                    rigid_body.apply_torque_impulse(convert::vector(load.angular_impulse), true);
                }
                if configuration.sleep_policy == RigidBodySleepPolicy::CryEnergy {
                    let damping = load.extra_damping.mul_add(-time_step, 1.0).max(0.0);
                    rigid_body.set_linvel(rigid_body.linvel() * damping, false);
                    rigid_body.set_angvel(rigid_body.angvel() * damping, false);
                }
            }
            self.buoyancy_status.insert(
                handle,
                BuoyancyStatus {
                    submerged_fraction: load.water_fraction,
                    floating: load.floating,
                },
            );
        }
    }

    /// Applies the energy/support sleep branch in `CRigidEntity::Update`.
    /// Rapier's velocity sleep timer is disabled for these bodies at creation;
    /// collision impulses and explicit actions still wake them normally.
    fn apply_cry_energy_sleep(&mut self) {
        let mut candidates = self
            .bodies
            .iter()
            .filter_map(|(&handle, native)| {
                native.rigid_configuration.and_then(|configuration| {
                    (configuration.sleep_policy == RigidBodySleepPolicy::CryEnergy
                        && configuration.can_sleep)
                        .then_some((handle, native.rigid_body, configuration))
                })
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(handle, _, _)| *handle);

        for (handle, rigid_handle, configuration) in candidates {
            let Some(rigid_body) = self.rigid_bodies.get(rigid_handle) else {
                continue;
            };
            if rigid_body.is_sleeping() || !rigid_body.is_dynamic() || !rigid_body.is_enabled() {
                continue;
            }

            // Cry's E is specific kinetic energy: translational speed squared
            // plus rotational energy divided by mass, all multiplied by 0.5.
            let mass = rigid_body.mass();
            let energy = if mass > f32::EPSILON {
                rigid_body.kinetic_energy() / mass
            } else {
                continue;
            };
            let support = self.cry_contact_support(rigid_body);
            let buoyancy = self
                .buoyancy_status
                .get(&handle)
                .copied()
                .unwrap_or_default();
            let supported = support.pair_count > 0 || buoyancy.floating;
            let gravity_free =
                self.gravity.length_squared() == 0.0 || rigid_body.gravity_scale() == 0.0;

            // The native branch lowers Emin to 10% for a well-supported body
            // or one resting on another awake rigid body at a top-facing
            // contact. Multiple Rapier manifolds represent the native
            // `m_nContacts > 3` case.
            let stable_support = support.support_bodies == 1 && support.contact_count > 3;
            let authored_minimum_energy =
                self.vehicles
                    .get(&handle)
                    .map_or(configuration.sleep_min_energy, |vehicle| {
                        if vehicle.has_chassis_contacts
                            || vehicle.wheels.iter().all(|wheel| wheel.contact.is_none())
                        {
                            configuration.sleep_min_energy
                        } else {
                            vehicle.configuration.minimum_energy
                        }
                    });
            let minimum_energy = authored_minimum_energy
                * if stable_support || support.awake_dynamic_support {
                    0.1
                } else {
                    1.0
                };

            if energy < minimum_energy
                && (supported || gravity_free)
                && let Some(rigid_body) = self.rigid_bodies.get_mut(rigid_handle)
            {
                rigid_body.sleep();
            }
        }
    }

    /// Applies `RockNRoll`'s five sleeping-condition modes and its island-wide
    /// deactivation gate.
    fn apply_rock_n_roll_sleep(&mut self, time_step: f32) -> Result<(), PhysicsError> {
        let mut candidates = self
            .bodies
            .iter()
            .filter_map(|(&handle, native)| {
                let configuration = native.rigid_configuration?;
                let RigidBodySleepPolicy::RockNRoll(mode) = configuration.sleep_policy else {
                    return None;
                };
                Some((handle, native.rigid_body, configuration, mode))
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(handle, _, _, _)| *handle);
        let mut eligible = HashMap::with_capacity(candidates.len());

        for (handle, rigid_handle, configuration, mode) in &candidates {
            let Some(rigid_body) = self.rigid_bodies.get(*rigid_handle) else {
                continue;
            };
            if rigid_body.is_sleeping() {
                eligible.insert(*handle, true);
                continue;
            }
            if !rigid_body.is_dynamic() || !rigid_body.is_enabled() || !configuration.can_sleep {
                eligible.insert(*handle, false);
                continue;
            }
            let linear_speed_squared = rigid_body.linvel().length_squared();
            let angular_speed_squared = rigid_body.angvel().length_squared();
            let energy = rigid_body.kinetic_energy();
            let mass = rigid_body.mass();
            let native = self
                .bodies
                .get_mut(handle)
                .ok_or(PhysicsError::BodyNotFound(*handle))?;

            let condition = rock_n_roll_sleep_condition(
                native,
                *mode,
                *configuration,
                RockNRollMotion {
                    linear_speed_squared,
                    angular_speed_squared,
                    energy,
                    mass,
                },
                time_step,
            );
            if condition {
                native.rock_n_roll_sleep_eligible_time += time_step;
            } else if mode != &RockNRollSleepMode::Disabled {
                native.rock_n_roll_sleep_eligible_time = 0.0;
            }
            eligible.insert(
                *handle,
                condition && native.rock_n_roll_sleep_eligible_time > configuration.sleep_duration,
            );
        }

        self.sleep_eligible_islands(&candidates, &eligible)
    }
    /// Applies one action to a rope body.
    fn apply_rope_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        let mut enabled = None;
        {
            let state =
                self.ropes
                    .get_mut(&body)
                    .ok_or(PhysicsError::OperationRequiresRopeBody {
                        operation: "apply_action",
                    })?;
            match action {
                PhysicsAction::Impulse(action) => {
                    if !action.impulse.is_finite()
                        || action.point.is_some_and(|point| !point.is_finite())
                    {
                        return Err(PhysicsError::InvalidRopeConfiguration { field: "impulse" });
                    }
                    state.apply_impulse(action);
                }
                PhysicsAction::AngularImpulse(impulse) => {
                    if !impulse.is_finite() {
                        return Err(PhysicsError::InvalidRopeConfiguration {
                            field: "angular impulse",
                        });
                    }
                    state.apply_angular_impulse(impulse);
                }
                PhysicsAction::Force(force) => {
                    if !force.is_finite() {
                        return Err(PhysicsError::InvalidRopeConfiguration { field: "force" });
                    }
                    state.add_force(force);
                }
                PhysicsAction::Torque(torque) => {
                    if !torque.is_finite() {
                        return Err(PhysicsError::InvalidRopeConfiguration { field: "torque" });
                    }
                    state.add_torque(torque);
                }
                PhysicsAction::Reset => state.reset(),
                PhysicsAction::Wake(awake) => state.awake = awake,
                PhysicsAction::SetPose(pose) => state.set_pose(pose),
                PhysicsAction::SetVelocity(velocity) => {
                    if !velocity.is_finite() {
                        return Err(PhysicsError::InvalidRopeConfiguration { field: "velocity" });
                    }
                    state.set_velocity(velocity);
                }
                PhysicsAction::SetMass(mass) => {
                    if !mass.is_finite() || mass <= 0.0 {
                        return Err(PhysicsError::InvalidRopeConfiguration { field: "mass" });
                    }
                    state.set_mass(mass);
                }
                PhysicsAction::SetLinearDamping(damping) => {
                    if !damping.is_finite() || damping < 0.0 {
                        return Err(PhysicsError::InvalidRopeConfiguration { field: "damping" });
                    }
                    state.configuration.damping = damping;
                }
                PhysicsAction::SetSleepMinEnergy(energy) => {
                    if !energy.is_finite() || energy < 0.0 {
                        return Err(PhysicsError::InvalidRopeConfiguration {
                            field: "minimum energy",
                        });
                    }
                    state.configuration.minimum_energy = energy;
                }
                PhysicsAction::SetSimulated(simulated) => enabled = Some(simulated),
                PhysicsAction::SetAngularVelocity(angular_velocity) => {
                    if !angular_velocity.is_finite() {
                        return Err(PhysicsError::InvalidRopeConfiguration {
                            field: "angular velocity",
                        });
                    }
                    state.set_angular_velocity(angular_velocity);
                }
                PhysicsAction::SetDensity(_)
                | PhysicsAction::SetAngularDamping(_)
                | PhysicsAction::SetBuoyancy(_)
                | PhysicsAction::Move(_)
                | PhysicsAction::SetLivingDimensions(_)
                | PhysicsAction::SetLivingDynamics(_)
                | PhysicsAction::SyncLiving(_) => {
                    return Err(PhysicsError::UnsupportedRopeAction {
                        action: "generic rigid/living-only action",
                    });
                }
            }
        }
        if let Some(enabled) = enabled {
            self.rigid_body_mut(body)?.set_enabled(enabled);
        }
        Ok(())
    }

    /// Applies one action to a soft body and its optional rigid core.
    fn apply_soft_body_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        let (effect, mut rigid_core_update) = {
            let state =
                self.soft_bodies
                    .get_mut(&body)
                    .ok_or(PhysicsError::OperationRequiresSoftBody {
                        operation: "apply_action",
                    })?;
            let effect = apply_soft_body_state_action(state, action)?;
            (effect, state.take_rigid_core_update())
        };
        if let Some(update) = rigid_core_update.as_mut() {
            update.fit_to_soft_body |= effect.synchronize_core_pose;
        }
        if let Some(update) = rigid_core_update {
            self.apply_soft_rigid_core_update(update)?;
        }
        if let Some(damping) = effect.rigid_core_damping
            && let Some((rigid_core, _)) = self
                .soft_bodies
                .get(&body)
                .and_then(SoftBodyState::rigid_core_handles)
        {
            let core =
                self.rigid_bodies
                    .get_mut(rigid_core)
                    .ok_or(PhysicsError::BackendInvariant(
                        "soft body references a missing rigid-core body",
                    ))?;
            core.set_linear_damping(damping);
            core.set_angular_damping(damping);
        }
        if let Some(enabled) = effect.enabled {
            self.rigid_body_mut(body)?.set_enabled(enabled);
            if let Some((rigid_core, _)) = self
                .soft_bodies
                .get(&body)
                .and_then(SoftBodyState::rigid_core_handles)
            {
                self.rigid_bodies
                    .get_mut(rigid_core)
                    .ok_or(PhysicsError::BackendInvariant(
                        "soft body references a missing rigid-core body",
                    ))?
                    .set_enabled(enabled);
            }
        }
        Ok(())
    }

    /// Applies one action to a linked soft body.
    fn apply_linked_soft_body_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        let mut enabled = None;
        {
            let state = self.linked_soft_bodies.get_mut(&body).ok_or(
                PhysicsError::OperationRequiresLinkedSoftBody {
                    operation: "apply_action",
                },
            )?;
            match action {
                PhysicsAction::Impulse(action) => {
                    if !action.impulse.is_finite()
                        || action.point.is_some_and(|point| !point.is_finite())
                    {
                        return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                            field: "impulse",
                        });
                    }
                    state.apply_impulse(action);
                }
                PhysicsAction::AngularImpulse(impulse) => {
                    if !impulse.is_finite() {
                        return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                            field: "angular impulse",
                        });
                    }
                    state.apply_angular_impulse(impulse);
                }
                PhysicsAction::Force(force) => {
                    if !force.is_finite() {
                        return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                            field: "force",
                        });
                    }
                    state.add_force(force);
                }
                PhysicsAction::Torque(torque) => {
                    if !torque.is_finite() {
                        return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                            field: "torque",
                        });
                    }
                    state.add_torque(torque);
                }
                PhysicsAction::Reset => state.reset(),
                PhysicsAction::Wake(awake) => state.awake = awake,
                PhysicsAction::SetPose(pose) => state.set_pose(pose),
                PhysicsAction::SetVelocity(velocity) => {
                    if !velocity.is_finite() {
                        return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                            field: "velocity",
                        });
                    }
                    state.set_velocity(velocity);
                }
                PhysicsAction::SetAngularVelocity(angular_velocity) => {
                    if !angular_velocity.is_finite() {
                        return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                            field: "angular velocity",
                        });
                    }
                    state.set_angular_velocity(angular_velocity);
                }
                PhysicsAction::SetMass(mass) => {
                    if !mass.is_finite() || mass <= 0.0 {
                        return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                            field: "mass",
                        });
                    }
                    state.set_mass(mass);
                }
                PhysicsAction::SetSleepMinEnergy(energy) => {
                    if !energy.is_finite() || energy < 0.0 {
                        return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                            field: "minimum energy",
                        });
                    }
                    state.configuration.minimum_energy = energy;
                }
                PhysicsAction::SetSimulated(simulated) => enabled = Some(simulated),
                PhysicsAction::SetDensity(_)
                | PhysicsAction::SetLinearDamping(_)
                | PhysicsAction::SetAngularDamping(_)
                | PhysicsAction::SetBuoyancy(_)
                | PhysicsAction::Move(_)
                | PhysicsAction::SetLivingDimensions(_)
                | PhysicsAction::SetLivingDynamics(_)
                | PhysicsAction::SyncLiving(_) => {
                    return Err(PhysicsError::UnsupportedLinkedSoftBodyAction {
                        action: "generic rigid/living-only action",
                    });
                }
            }
        }
        if let Some(enabled) = enabled {
            self.rigid_body_mut(body)?.set_enabled(enabled);
        }
        Ok(())
    }

    /// Applies one action to a character body.
    fn apply_character_body_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        if matches!(
            action,
            PhysicsAction::SetLivingDimensions(_) | PhysicsAction::SetLivingDynamics(_)
        ) {
            return Err(PhysicsError::ActionRequiresLivingBody {
                action: "set_living_configuration",
            });
        }
        if let PhysicsAction::SetMass(mass) = action {
            if !mass.is_finite() || mass <= 0.0 {
                return Err(PhysicsError::InvalidRigidBodyScalar { field: "mass" });
            }
            self.characters
                .get_mut(&body)
                .ok_or(PhysicsError::BackendInvariant(
                    "character body state disappeared during action dispatch",
                ))?
                .configuration
                .mass = mass;
            self.set_body_mass(body, mass)?;
            return Ok(());
        }
        let sync = Self::apply_character_action(
            self.characters
                .get_mut(&body)
                .ok_or(PhysicsError::BackendInvariant(
                    "character body state disappeared during action dispatch",
                ))?,
            action,
        );
        if let Some(sync) = sync {
            self.rigid_body_mut(body)?
                .set_position(convert::pose(sync.pose), true);
        }
        if let PhysicsAction::SetSimulated(simulated) = action {
            self.rigid_body_mut(body)?.set_enabled(simulated);
        }
        Ok(())
    }

    /// Applies one action to a living body.
    fn apply_living_body_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        if let PhysicsAction::SetLivingDimensions(dimensions) = action {
            return self.set_living_dimensions(body, dimensions);
        }
        if let PhysicsAction::SetLivingDynamics(dynamics) = action {
            return self.set_living_dynamics(body, dynamics);
        }
        if let PhysicsAction::SetMass(mass) = action {
            if !mass.is_finite() || mass <= 0.0 {
                return Err(PhysicsError::InvalidRigidBodyScalar { field: "mass" });
            }
            self.living
                .get_mut(&body)
                .ok_or(PhysicsError::BackendInvariant(
                    "living body state disappeared during action dispatch",
                ))?
                .configuration
                .dynamics
                .mass = mass;
            self.set_body_mass(body, mass)?;
            return Ok(());
        }
        let sync = Self::apply_living_action(
            self.living
                .get_mut(&body)
                .ok_or(PhysicsError::BackendInvariant(
                    "living body state disappeared during action dispatch",
                ))?,
            action,
        );
        if let Some(sync) = sync {
            self.rigid_body_mut(body)?
                .set_position(convert::pose(sync.pose), true);
        }
        if let PhysicsAction::Wake(awake) = action {
            if awake {
                self.rigid_body_mut(body)?.wake_up(true);
            } else {
                self.rigid_body_mut(body)?.sleep();
            }
        }
        if let PhysicsAction::SetSimulated(simulated) = action {
            self.rigid_body_mut(body)?.set_enabled(simulated);
        }
        Ok(())
    }

    /// Applies one action to an ordinary rigid, articulated, or static body.
    fn apply_rigid_body_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        match action {
            PhysicsAction::Impulse(action) => {
                let impulse = if action.explosion {
                    action.impulse * 0.3
                } else {
                    action.impulse
                };
                let rigid_body = self.rigid_body_mut(body)?;
                if let Some(point) = action.point {
                    rigid_body.apply_impulse_at_point(
                        convert::vector(impulse),
                        convert::vector(point),
                        true,
                    );
                } else {
                    rigid_body.apply_impulse(convert::vector(impulse), true);
                }
            }
            PhysicsAction::AngularImpulse(impulse) => self
                .rigid_body_mut(body)?
                .apply_torque_impulse(convert::vector(impulse), true),
            PhysicsAction::Force(force) => self
                .rigid_body_mut(body)?
                .add_force(convert::vector(force), true),
            PhysicsAction::Torque(torque) => self
                .rigid_body_mut(body)?
                .add_torque(convert::vector(torque), true),
            PhysicsAction::Reset => {
                let rigid_body = self.rigid_body_mut(body)?;
                rigid_body.set_linvel(Vector::ZERO, true);
                rigid_body.set_angvel(Vector::ZERO, true);
            }
            PhysicsAction::Wake(awake) => {
                if awake {
                    self.rigid_body_mut(body)?.wake_up(true);
                } else {
                    self.rigid_body_mut(body)?.sleep();
                }
            }
            PhysicsAction::SetPose(pose) => self
                .rigid_body_mut(body)?
                .set_position(convert::pose(pose), true),
            PhysicsAction::SetVelocity(velocity) => self
                .rigid_body_mut(body)?
                .set_linvel(convert::vector(velocity), true),
            PhysicsAction::SetAngularVelocity(velocity) => self
                .rigid_body_mut(body)?
                .set_angvel(convert::vector(velocity), true),
            PhysicsAction::SetMass(_)
            | PhysicsAction::SetDensity(_)
            | PhysicsAction::SetLinearDamping(_)
            | PhysicsAction::SetAngularDamping(_)
            | PhysicsAction::SetSleepMinEnergy(_)
            | PhysicsAction::SetBuoyancy(_) => self.apply_rigid_scalar_action(body, action)?,
            PhysicsAction::SetSimulated(simulated) => {
                self.rigid_body_mut(body)?.set_enabled(simulated);
            }
            PhysicsAction::Move(_) => {
                return Err(PhysicsError::ActionRequiresLivingBody { action: "move" });
            }
            PhysicsAction::SyncLiving(_) => {
                return Err(PhysicsError::ActionRequiresLivingBody {
                    action: "sync_living",
                });
            }
            PhysicsAction::SetLivingDimensions(_) | PhysicsAction::SetLivingDynamics(_) => {
                return Err(PhysicsError::ActionRequiresLivingBody {
                    action: "set_living_configuration",
                });
            }
        }
        Ok(())
    }

    /// Builds the Rapier rigid body for one authored descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidParticleConfiguration`] when a particle
    /// names an ignored collider that belongs to a different scene.
    fn body_builder(
        &self,
        descriptor: &BodyDescriptor,
        body: PhysicsBodyHandle,
        rigid_configuration: Option<RigidBodyConfiguration>,
    ) -> Result<RigidBodyBuilder, PhysicsError> {
        let mut builder =
            Self::create_rigid_body_builder(descriptor).user_data(u128::from(body.get()));
        if let Some(configuration) = rigid_configuration {
            let solver_damping = configuration.sleep_policy
                == RigidBodySleepPolicy::SolverVelocityThresholds
                && configuration.damping_model == RigidBodyDampingModel::Solver;
            builder = builder
                .linvel(convert::vector(configuration.initial_linear_velocity))
                .angvel(convert::vector(configuration.initial_angular_velocity))
                .linear_damping(if solver_damping {
                    configuration.linear_damping
                } else {
                    0.0
                })
                .angular_damping(if solver_damping {
                    configuration.angular_damping
                } else {
                    0.0
                })
                .gravity_scale(
                    if configuration.gravity_enabled
                        && configuration.damping_model == RigidBodyDampingModel::Solver
                    {
                        1.0
                    } else {
                        0.0
                    },
                )
                .ccd_enabled(
                    configuration
                        .continuous_collision_mode
                        .uses_ordered_time_of_impact(),
                )
                .soft_ccd_prediction(
                    if configuration
                        .continuous_collision_mode
                        .uses_hit_projection()
                        || configuration
                            .continuous_collision_mode
                            .uses_speculative_normal_constraints()
                    {
                        configuration.continuous_prediction_distance
                    } else {
                        0.0
                    },
                )
                .sleeping(configuration.start_asleep)
                .can_sleep(configuration.can_sleep)
                .enabled(configuration.simulated);
            if let Some(principal_inertia) = configuration.principal_inertia {
                builder = builder.additional_mass_properties(MassProperties::new(
                    convert::vector(configuration.center_of_mass_offset),
                    configuration.mass,
                    convert::vector(principal_inertia),
                ));
            } else if !configuration.compute_mass {
                builder = builder.additional_mass(configuration.mass);
            }
        } else {
            match &descriptor.kind {
                BodyKind::Living(configuration) => {
                    builder = builder
                        .additional_mass(configuration.dynamics.mass)
                        .gravity_scale(0.0)
                        .can_sleep(false);
                }
                BodyKind::Character(configuration) => {
                    builder = builder
                        .additional_mass(configuration.mass)
                        .gravity_scale(0.0)
                        .can_sleep(false);
                }
                BodyKind::Particle(configuration) => {
                    if let Some(ignored) = configuration.ignored_collider
                        && ignored.scene() != self.scene
                    {
                        return Err(PhysicsError::InvalidParticleConfiguration {
                            field: "ignored collider scene",
                        });
                    }
                    builder = builder.gravity_scale(0.0).can_sleep(false);
                }
                BodyKind::Rope(_) | BodyKind::Soft(_) | BodyKind::LinkedSoft(_) => {
                    builder = builder.gravity_scale(0.0).can_sleep(false);
                }
                BodyKind::Static { .. }
                | BodyKind::Query(_)
                | BodyKind::Area
                | BodyKind::FluidArea(_) => {}
                BodyKind::Rigid(_) | BodyKind::WheeledVehicle(_) | BodyKind::Articulated(_) => {
                    unreachable!("rigid configurations were handled above")
                }
            }
        }
        Ok(builder)
    }

    /// Inserts every authored collider of a descriptor, unwinding the whole body
    /// if any one of them cannot be built.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::insert_collider`] returns for the first
    /// collider Rapier rejects.
    fn insert_authored_colliders(
        &mut self,
        body: PhysicsBodyHandle,
        descriptor: &BodyDescriptor,
        owner: ColliderOwner,
        rigid_configuration: Option<RigidBodyConfiguration>,
    ) -> Result<Vec<ColliderHandle>, PhysicsError> {
        let contributes_mass = rigid_configuration.is_some_and(|configuration| {
            configuration.compute_mass || configuration.compute_inertia_tensor
        });
        let force_sensor = matches!(
            &descriptor.kind,
            BodyKind::Query(_)
                | BodyKind::Particle(_)
                | BodyKind::Rope(_)
                | BodyKind::Soft(_)
                | BodyKind::LinkedSoft(_)
                | BodyKind::Area
                | BodyKind::FluidArea(_)
        );
        let mut created = Vec::with_capacity(descriptor.colliders.len());
        for collider in &descriptor.colliders {
            match self.insert_collider(owner, collider, contributes_mass, force_sensor) {
                Ok(collider) => created.push(collider),
                Err(error) => {
                    self.remove_body(body)?;
                    return Err(error);
                }
            }
        }
        Ok(created)
    }

    /// Attaches the per-kind runtime state and query collider a body needs on
    /// top of its authored colliders.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BackendInvariant`] when the body vanishes while
    /// its query shape is attached, or whatever the collider insertion returns
    /// when Rapier rejects that shape.
    fn attach_body_kind_state(
        &mut self,
        body: PhysicsBodyHandle,
        descriptor: &BodyDescriptor,
        owner: ColliderOwner,
        rigid_body: RigidBodyHandle,
        created_colliders: &[ColliderHandle],
    ) -> Result<(), PhysicsError> {
        let pose = descriptor.pose;
        match &descriptor.kind {
            BodyKind::Living(configuration) => self.attach_living_state(body, configuration, owner),
            BodyKind::Character(configuration) => {
                self.attach_character_state(body, *configuration, created_colliders)
            }
            BodyKind::Particle(configuration) => {
                self.attach_particle_state(body, *configuration, owner)
            }
            BodyKind::Rope(configuration) => {
                self.attach_rope_state(body, configuration, pose, owner, rigid_body)
            }
            BodyKind::Soft(configuration) => {
                self.attach_soft_state(body, configuration, pose, owner, rigid_body)
            }
            BodyKind::LinkedSoft(configuration) => {
                self.attach_linked_soft_state(body, configuration, pose, owner, rigid_body)
            }
            BodyKind::Rigid(_)
            | BodyKind::Articulated(_)
            | BodyKind::WheeledVehicle(_)
            | BodyKind::Static { .. }
            | BodyKind::Query(_)
            | BodyKind::Area
            | BodyKind::FluidArea(_) => Ok(()),
        }
    }

    /// Attaches the sensor collider a particle or deformable body uses for
    /// scene queries and records its authored configuration on the body.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BackendInvariant`] with `missing_body` when the
    /// body vanishes mid-attachment, or whatever
    /// [`Self::insert_collider`] returns when Rapier rejects the shape. The
    /// whole body is unwound before either error is returned.
    fn attach_query_collider(
        &mut self,
        body: PhysicsBodyHandle,
        owner: ColliderOwner,
        primary: ColliderConfiguration,
        missing_body: &'static str,
    ) -> Result<ColliderHandle, PhysicsError> {
        let primary_collider = match self.insert_collider(owner, &primary, false, true) {
            Ok(collider) => collider,
            Err(error) => {
                self.remove_body(body)?;
                return Err(error);
            }
        };
        self.bodies
            .get_mut(&body)
            .ok_or(PhysicsError::BackendInvariant(missing_body))?
            .collider_configurations
            .push(primary);
        Ok(primary_collider)
    }

    /// Builds the Cry living-entity capsule or cylinder and its runtime state.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::insert_collider`] returns when Rapier rejects
    /// the authored stance shape.
    fn attach_living_state(
        &mut self,
        body: PhysicsBodyHandle,
        configuration: &LivingBodyConfiguration,
        owner: ColliderOwner,
    ) -> Result<(), PhysicsError> {
        let dimensions = configuration.dimensions;
        let shape = if dimensions.use_capsule {
            ColliderShape::Capsule {
                axis: Axis3::Z,
                half_height: dimensions.collider_half_height,
                radius: dimensions.collider_radius,
            }
        } else {
            ColliderShape::Cylinder {
                axis: Axis3::Z,
                half_height: dimensions.collider_half_height,
                radius: dimensions.collider_radius,
            }
        };
        let primary = ColliderConfiguration {
            shape,
            local_pose: PhysicsPose {
                translation: Vec3::Z * (dimensions.height_collider - dimensions.height_pivot),
                rotation: glam::Quat::IDENTITY,
            },
            collision_class: configuration.collision_class,
            collision_filter: None,
            surface_index: configuration.dynamics.surface_index,
            surface_pierceability: configuration.surface_pierceability,
            friction: configuration.friction,
            restitution: configuration.restitution,
            density: 0.0,
            mass: None,
            sensor: false,
            interacts_with_triggers: true,
            ..ColliderConfiguration::default()
        };
        let primary_collider = match self.insert_collider(owner, &primary, false, false) {
            Ok(collider) => collider,
            Err(error) => {
                self.remove_body(body)?;
                return Err(error);
            }
        };
        self.living.insert(
            body,
            LivingState::new(configuration.clone(), primary_collider),
        );
        Ok(())
    }

    /// Records the character runtime state over the descriptor's own colliders.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BackendInvariant`] when the descriptor authored
    /// no collider for the character to move with.
    fn attach_character_state(
        &mut self,
        body: PhysicsBodyHandle,
        configuration: CharacterBodyConfiguration,
        created_colliders: &[ColliderHandle],
    ) -> Result<(), PhysicsError> {
        let primary_collider = *created_colliders
            .first()
            .ok_or(PhysicsError::BackendInvariant(
                "character body has no primary collider",
            ))?;
        self.characters
            .insert(body, CharacterState::new(configuration, primary_collider));
        Ok(())
    }

    /// Attaches the particle's traceable box sensor and its runtime state.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::attach_query_collider`] returns.
    fn attach_particle_state(
        &mut self,
        body: PhysicsBodyHandle,
        configuration: ParticleBodyConfiguration,
        owner: ColliderOwner,
    ) -> Result<(), PhysicsError> {
        let primary = ColliderConfiguration {
            shape: ColliderShape::Cuboid {
                half_extents: Vec3::new(
                    configuration.size * 0.5,
                    configuration.size * 0.5,
                    configuration.thickness * 0.5,
                ),
            },
            collision_class: configuration.collision_class,
            surface_index: configuration.surface_index,
            surface_pierceability: configuration.pierceability,
            density: 0.0,
            sensor: true,
            simulated: false,
            in_scene_queries: configuration.flags.contains(ParticleFlags::TRACEABLE),
            interacts_with_triggers: false,
            ..ColliderConfiguration::default()
        };
        let primary_collider = self.attach_query_collider(
            body,
            owner,
            primary,
            "particle body disappeared while attaching its query shape",
        )?;
        self.particles.insert(
            body,
            ParticleState::new(configuration, primary_collider, self.gravity),
        );
        Ok(())
    }

    /// Centres the rope's kinematic proxy on its authored points and attaches
    /// the bounding sensor the scene queries see.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::attach_query_collider`] returns.
    fn attach_rope_state(
        &mut self,
        body: PhysicsBodyHandle,
        configuration: &RopeBodyConfiguration,
        pose: PhysicsPose,
        owner: ColliderOwner,
        rigid_body: RigidBodyHandle,
    ) -> Result<(), PhysicsError> {
        let world_points: Vec<_> = configuration
            .points
            .iter()
            .map(|point| pose.transform_point(*point))
            .collect();
        let (center, radius) = deformable_bounds(&world_points);
        self.rigid_bodies[rigid_body]
            .set_position(Pose::from_translation(convert::vector(center)), true);
        let primary = ColliderConfiguration {
            shape: ColliderShape::Sphere {
                radius: (radius + configuration.collision_distance).max(1.0e-4),
            },
            collision_class: configuration.collision_class,
            surface_index: configuration.surface_index,
            density: 0.0,
            sensor: true,
            simulated: false,
            in_scene_queries: configuration.flags.contains(RopeFlags::TRACEABLE),
            interacts_with_triggers: false,
            ..ColliderConfiguration::default()
        };
        let primary_collider = self.attach_query_collider(
            body,
            owner,
            primary,
            "rope body disappeared while attaching its query shape",
        )?;
        self.ropes.insert(
            body,
            RopeState::new(configuration.clone(), pose, primary_collider),
        );
        Ok(())
    }

    /// Centres the soft body's kinematic proxy, attaches its query sensor, and
    /// builds the optional rigid core.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::attach_query_collider`] or
    /// [`Self::create_soft_rigid_core`] returns.
    fn attach_soft_state(
        &mut self,
        body: PhysicsBodyHandle,
        configuration: &SoftBodyConfiguration,
        pose: PhysicsPose,
        owner: ColliderOwner,
        rigid_body: RigidBodyHandle,
    ) -> Result<(), PhysicsError> {
        let world_vertices: Vec<_> = configuration
            .vertices
            .iter()
            .map(|vertex| pose.transform_point(*vertex))
            .collect();
        let (center, radius) = deformable_bounds(&world_vertices);
        self.rigid_bodies[rigid_body]
            .set_position(Pose::from_translation(convert::vector(center)), true);
        let primary = ColliderConfiguration {
            shape: ColliderShape::Sphere {
                radius: (radius + configuration.thickness).max(1.0e-4),
            },
            collision_class: configuration.collision_class,
            surface_index: configuration.surface_index,
            density: 0.0,
            sensor: true,
            simulated: false,
            in_scene_queries: true,
            interacts_with_triggers: false,
            ..ColliderConfiguration::default()
        };
        let primary_collider = self.attach_query_collider(
            body,
            owner,
            primary,
            "soft body disappeared while attaching its query shape",
        )?;
        self.soft_bodies.insert(
            body,
            SoftBodyState::new(configuration.clone(), pose, primary_collider),
        );
        if let Err(error) = self.create_soft_rigid_core(body) {
            self.remove_body(body)?;
            return Err(error);
        }
        Ok(())
    }

    /// Centres the linked soft body's kinematic proxy and attaches its query
    /// sensor.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::attach_query_collider`] returns.
    fn attach_linked_soft_state(
        &mut self,
        body: PhysicsBodyHandle,
        configuration: &LinkedSoftBodyConfiguration,
        pose: PhysicsPose,
        owner: ColliderOwner,
        rigid_body: RigidBodyHandle,
    ) -> Result<(), PhysicsError> {
        let world_vertices = configuration
            .vertices
            .iter()
            .map(|vertex| pose.transform_point(*vertex))
            .collect::<Vec<_>>();
        let (center, radius) = deformable_bounds(&world_vertices);
        self.rigid_bodies[rigid_body]
            .set_position(Pose::from_translation(convert::vector(center)), true);
        let primary = ColliderConfiguration {
            shape: ColliderShape::Sphere {
                radius: (radius + configuration.collision_radius).max(1.0e-4),
            },
            collision_class: configuration.collision_class,
            surface_index: configuration.surface_index,
            density: 0.0,
            sensor: true,
            simulated: false,
            in_scene_queries: true,
            interacts_with_triggers: false,
            ..ColliderConfiguration::default()
        };
        let primary_collider = self.attach_query_collider(
            body,
            owner,
            primary,
            "linked soft body disappeared while attaching its query shape",
        )?;
        self.linked_soft_bodies.insert(
            body,
            LinkedSoftBodyState::new(configuration.clone(), pose, primary_collider),
        );
        Ok(())
    }

    /// Applies a body's authored mass once its complete collider set exists.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::set_body_mass`] returns when the body or one of
    /// its colliders has gone missing.
    fn apply_authored_mass(
        &mut self,
        body: PhysicsBodyHandle,
        descriptor: &BodyDescriptor,
    ) -> Result<(), PhysicsError> {
        // Rapier recomputes a body's mass properties whenever a collider is
        // attached.  Our non-density bodies intentionally attach zero-density
        // colliders, so apply their authored mass after the complete collider
        // set exists instead of relying on the builder's pre-attachment value.
        match &descriptor.kind {
            BodyKind::Rigid(configuration) | BodyKind::Articulated(configuration)
                if !configuration.compute_mass && configuration.principal_inertia.is_none() =>
            {
                self.set_body_mass(body, configuration.mass)?;
            }
            BodyKind::WheeledVehicle(configuration)
                if !configuration.rigid_body.compute_mass
                    && configuration.rigid_body.principal_inertia.is_none() =>
            {
                self.set_body_mass(body, configuration.rigid_body.mass)?;
            }
            BodyKind::Living(configuration) => {
                self.set_body_mass(body, configuration.dynamics.mass)?;
            }
            BodyKind::Character(configuration) => {
                self.set_body_mass(body, configuration.mass)?;
            }
            BodyKind::Particle(_)
            | BodyKind::Rope(_)
            | BodyKind::Soft(_)
            | BodyKind::LinkedSoft(_)
            | BodyKind::Rigid(_)
            | BodyKind::WheeledVehicle(_)
            | BodyKind::Articulated(_)
            | BodyKind::Static { .. }
            | BodyKind::Query(_)
            | BodyKind::Area
            | BodyKind::FluidArea(_) => {}
        }
        Ok(())
    }

    /// [`BodyStatus`] for a body Rapier itself integrates.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::UnsupportedEntityType`] for a physical type with
    /// no rigid simulation class, or whatever [`Self::body_velocities`]
    /// returns.
    fn rigid_body_status(
        &self,
        body: PhysicsBodyHandle,
        native: &NativeBody,
        rigid_body: &RigidBody,
    ) -> Result<BodyStatus, PhysicsError> {
        let awake = !rigid_body.is_sleeping();
        let simulation_class = match native.physical_type {
            PhysicalEntityType::Static => SimulationClass::Static,
            PhysicalEntityType::Living => SimulationClass::Living,
            PhysicalEntityType::Area => SimulationClass::Trigger,
            PhysicalEntityType::Rigid if native.query_type == PhysicalEntityTypes::INDEPENDENT => {
                SimulationClass::Independent
            }
            PhysicalEntityType::Rigid
            | PhysicalEntityType::WheeledVehicle
            | PhysicalEntityType::Articulated => {
                if awake {
                    SimulationClass::ActiveRigid
                } else {
                    SimulationClass::SleepingRigid
                }
            }
            _ => return Err(PhysicsError::UnsupportedEntityType(native.physical_type)),
        };
        let (linear_damping, angular_damping, sleep_min_energy, buoyancy) =
            native.rigid_configuration.map_or_else(
                || (0.0, 0.0, 0.0, RigidBodyBuoyancy::default()),
                |configuration| {
                    (
                        configuration.linear_damping,
                        configuration.angular_damping,
                        configuration.sleep_min_energy,
                        configuration.buoyancy,
                    )
                },
            );
        let (linear_velocity, angular_velocity) = self.body_velocities(body)?;
        let (linear_acceleration, angular_acceleration) = if awake {
            (native.linear_acceleration, native.angular_acceleration)
        } else {
            (Vec3::ZERO, Vec3::ZERO)
        };
        let collider_volume: f32 = rigid_body
            .colliders()
            .iter()
            .filter_map(|handle| self.colliders.get(*handle))
            .map(|collider| collider.shape().mass_properties(1.0).mass())
            .sum();
        let mass = rigid_body.mass();
        Ok(BodyStatus {
            pose: convert::physics_pose(rigid_body.position()),
            linear_velocity,
            angular_velocity,
            linear_acceleration,
            angular_acceleration,
            mass,
            density: if collider_volume > 0.0 {
                mass / collider_volume
            } else {
                0.0
            },
            kinetic_energy: rigid_body.kinetic_energy(),
            linear_damping,
            angular_damping,
            sleep_min_energy,
            buoyancy,
            buoyancy_status: self.buoyancy_status.get(&body).copied().unwrap_or_default(),
            simulation_class,
            awake,
            kinematic: rigid_body.is_kinematic(),
            simulated: rigid_body.is_enabled(),
        })
    }

    /// Records the sensor overlaps Rapier's narrow phase currently reports.
    fn collect_trigger_interactions(
        &self,
        current: &mut HashMap<InteractionKey, PhysicsInteraction>,
    ) {
        for (first, second, intersecting) in self.narrow_phase.intersection_pairs() {
            if !intersecting {
                continue;
            }
            let Some(left) = self.collider_metadata.get(&first).copied() else {
                continue;
            };
            let Some(right) = self.collider_metadata.get(&second).copied() else {
                continue;
            };
            if !left.sensor && !right.sensor {
                continue;
            }
            let (key, swapped) =
                InteractionKey::new(first, second, PhysicsInteractionKind::Trigger);
            let (left, right) = if swapped {
                (right, left)
            } else {
                (left, right)
            };
            current.insert(
                key,
                PhysicsInteraction {
                    phase: PhysicsInteractionPhase::Persisted,
                    kind: PhysicsInteractionKind::Trigger,
                    body_a: left.body,
                    body_b: right.body,
                    entity_a: left.entity_id,
                    entity_b: right.entity_id,
                    surface_a: left.surface_index,
                    surface_b: right.surface_index,
                    tag_a: left.tag,
                    tag_b: right.tag,
                    point: None,
                    normal: None,
                    penetration_depth: 0.0,
                    impulse: Vec3::ZERO,
                },
            );
        }
    }

    /// Records the touching contact pairs Rapier's narrow phase currently
    /// reports, with the manifold point, normal, depth, and impulse of each.
    fn collect_contact_interactions(
        &self,
        current: &mut HashMap<InteractionKey, PhysicsInteraction>,
    ) {
        for pair in self.narrow_phase.contact_pairs() {
            if !pair.has_any_active_contact() {
                continue;
            }
            let Some(left) = self.collider_metadata.get(&pair.collider1).copied() else {
                continue;
            };
            let Some(right) = self.collider_metadata.get(&pair.collider2).copied() else {
                continue;
            };
            let (key, swapped) = InteractionKey::new(
                pair.collider1,
                pair.collider2,
                PhysicsInteractionKind::Contact,
            );
            let manifold = pair
                .manifolds
                .iter()
                .find(|manifold| !manifold.data.solver_contacts.is_empty());
            let point = manifold
                .and_then(|manifold| manifold.data.solver_contacts.first())
                .map(|contact| convert::vec3(contact.point));
            let penetration_depth = manifold.map_or(0.0, |manifold| {
                manifold
                    .data
                    .solver_contacts
                    .iter()
                    .map(|contact| (-contact.dist).max(0.0))
                    .fold(0.0, f32::max)
            });
            let mut normal = manifold.map(|manifold| convert::vec3(manifold.data.normal));
            let mut impulse = convert::vec3(pair.total_impulse());
            let (left, right) = if swapped {
                normal = normal.map(|normal| -normal);
                impulse = -impulse;
                (right, left)
            } else {
                (left, right)
            };
            current.insert(
                key,
                PhysicsInteraction {
                    phase: PhysicsInteractionPhase::Persisted,
                    kind: PhysicsInteractionKind::Contact,
                    body_a: left.body,
                    body_b: right.body,
                    entity_a: left.entity_id,
                    entity_b: right.entity_id,
                    surface_a: left.surface_index,
                    surface_b: right.surface_index,
                    tag_a: left.tag,
                    tag_b: right.tag,
                    point,
                    normal,
                    penetration_depth,
                    impulse,
                },
            );
        }
    }

    /// Rewinds each recorded body to its projected impact pose, damps the
    /// normal component of its velocity, and refreshes the broadphase bounds of
    /// every collider that moved.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BackendInvariant`] when a record names a rigid
    /// body or collider Rapier no longer has.
    fn apply_hit_projection_records(
        &mut self,
        records: HashMap<PhysicsBodyHandle, HitProjectionRecord>,
    ) -> Result<(), PhysicsError> {
        let mut modified_colliders = Vec::new();
        for record in records.into_values() {
            let rigid_body = self.rigid_bodies.get_mut(record.rigid_body).ok_or(
                PhysicsError::BackendInvariant(
                    "hit-projection record references a missing Rapier rigid body",
                ),
            )?;
            let pose = PhysicsPose {
                translation: record
                    .previous_pose
                    .translation
                    .lerp(record.current_pose.translation, record.pose_fraction),
                rotation: record
                    .previous_pose
                    .rotation
                    .lerp(record.current_pose.rotation, record.pose_fraction)
                    .normalize(),
            };
            rigid_body.set_position(convert::pose(pose), true);
            let normal = record.normal.try_normalize().unwrap_or(Vector::ZERO);
            let velocity = rigid_body.linvel();
            let normal_velocity = velocity.dot(normal);
            rigid_body.set_linvel(
                velocity
                    - normal * normal_velocity * (1.0 - record.normal_velocity_retained_fraction),
                true,
            );
            modified_colliders.extend_from_slice(rigid_body.colliders());
        }
        if modified_colliders.is_empty() {
            return Ok(());
        }
        self.rigid_bodies
            .propagate_modified_body_positions_to_colliders(&mut self.colliders);
        modified_colliders.sort_unstable_by_key(|handle| handle.into_raw_parts());
        modified_colliders.dedup();
        for handle in modified_colliders {
            let collider = self
                .colliders
                .get(handle)
                .ok_or(PhysicsError::BackendInvariant(
                    "projected body references a missing Rapier collider",
                ))?;
            self.broad_phase.set_aabb(
                &self.integration_parameters,
                handle,
                collider.compute_broad_phase_aabb(&self.integration_parameters, &self.rigid_bodies),
            );
        }
        Ok(())
    }

    /// Republishes each wheel's rotation, slip, and friction after the substeps,
    /// and counts the distinct dynamic bodies the vehicle is standing on.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BackendInvariant`] when the chassis has gone
    /// missing since the last substep.
    fn publish_vehicle_wheel_state(
        &self,
        body: PhysicsBodyHandle,
        native: RigidBodyHandle,
        state: &mut VehicleState,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        let chassis = self
            .rigid_bodies
            .get(native)
            .ok_or(PhysicsError::BackendInvariant(
                "vehicle chassis disappeared while publishing wheel state",
            ))?;
        let mut active_colliders = HashSet::new();
        for ((configuration, runtime), wheel) in state
            .configuration
            .wheels
            .iter()
            .zip(&mut state.wheels)
            .zip(state.controller.wheels())
        {
            if configuration.ray_cast {
                runtime.angular_velocity = (wheel.rotation - runtime.previous_rotation) / time_step;
            }
            runtime.previous_rotation = wheel.rotation;
            runtime.slipping = false;
            runtime.slip_velocity = Vec3::ZERO;
            runtime.friction = 0.0;
            let Some(contact) = runtime.contact else {
                continue;
            };
            let collider_handle = contact.collider;
            let Some(collider) = self.colliders.get(collider_handle) else {
                continue;
            };
            let contact_point = contact.point;
            let ground_velocity = collider
                .parent()
                .and_then(|parent| self.rigid_bodies.get(parent))
                .map_or(Vector::ZERO, |ground| {
                    ground.velocity_at_point(contact_point)
                });
            let normal = contact.normal;
            let forward = normal.cross(wheel.axle()).normalize_or_zero();
            let relative_velocity = chassis.velocity_at_point(contact_point)
                - ground_velocity
                - forward * (runtime.angular_velocity * configuration.radius);
            let tangent_velocity = relative_velocity - normal * relative_velocity.dot(normal);
            runtime.slip_velocity = convert::vec3(tangent_velocity);
            runtime.slipping = tangent_velocity.length() > state.configuration.slip_threshold;
            runtime.friction = collider.friction().clamp(
                configuration.minimum_friction,
                configuration.maximum_friction,
            ) * if runtime.slipping {
                state.configuration.dynamic_friction
            } else {
                1.0
            };

            if let Some(metadata) = self.collider_metadata.get(&collider_handle)
                && metadata.body != body
                && self.bodies.get(&metadata.body).is_some_and(|ground| {
                    ground.physical_type != PhysicalEntityType::Articulated
                        && self
                            .rigid_bodies
                            .get(ground.rigid_body)
                            .is_some_and(RigidBody::is_dynamic)
                })
            {
                active_colliders.insert(metadata.body);
            }
        }
        state.active_colliders = u32_from_usize(active_colliders.len());
        Ok(())
    }

    /// Feeds the authored suspension and friction of every ray-cast wheel into
    /// Rapier's own vehicle controller, and zeroes them for the wheels this
    /// adapter solves itself.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BackendInvariant`] when the chassis has gone
    /// missing since the last substep.
    fn configure_controller_wheels(
        &self,
        native: RigidBodyHandle,
        state: &mut VehicleState,
    ) -> Result<(), PhysicsError> {
        let chassis_mass = self
            .rigid_bodies
            .get(native)
            .ok_or(PhysicsError::BackendInvariant(
                "vehicle chassis disappeared while configuring its wheels",
            ))?
            .mass();
        for ((configuration, runtime), wheel) in state
            .configuration
            .wheels
            .iter()
            .zip(&state.wheels)
            .zip(state.controller.wheels_mut())
        {
            let controller_driven =
                configuration.axle >= 0 && configuration.ray_cast && runtime.contact.is_some();
            wheel.suspension_stiffness = if controller_driven {
                runtime.spring_stiffness / chassis_mass
            } else {
                0.0
            };
            wheel.damping_compression = if controller_driven {
                runtime.spring_damping / chassis_mass
            } else {
                0.0
            };
            wheel.damping_relaxation = wheel.damping_compression;
            wheel.side_friction_stiffness = if controller_driven {
                configuration.lateral_friction
            } else {
                0.0
            };
            let surface_friction = runtime
                .contact
                .and_then(|contact| self.colliders.get(contact.collider))
                .map_or(0.0, Collider::friction);
            let mut friction = surface_friction.clamp(
                configuration.minimum_friction,
                configuration.maximum_friction,
            );
            if runtime.slipping {
                friction *= state.configuration.dynamic_friction;
            }
            wheel.friction_slip = if controller_driven {
                friction.max(0.0)
            } else {
                0.0
            };
        }
        Ok(())
    }

    /// Builds Rapier's own vehicle controller with one wheel per authored wheel.
    fn build_vehicle_controller(
        native: RigidBodyHandle,
        configuration: &WheeledVehicleConfiguration,
        wheel_springs: &[(f32, f32)],
        mass: f32,
    ) -> DynamicRayCastVehicleController {
        let mut controller = DynamicRayCastVehicleController::new(native);
        controller.index_up_axis = 2;
        controller.index_forward_axis = 1;
        for (wheel, &(stiffness, damping)) in configuration.wheels.iter().zip(wheel_springs) {
            let tuning = WheelTuning {
                suspension_stiffness: stiffness / mass,
                suspension_compression: damping / mass,
                suspension_damping: damping / mass,
                max_suspension_travel: wheel.suspension_max_length,
                side_friction_stiffness: wheel.lateral_friction,
                friction_slip: wheel.maximum_friction,
                max_suspension_force: f32::MAX,
            };
            let controller_wheel = controller.add_wheel(
                convert::vector(wheel.connection),
                convert::vector(wheel.suspension_direction),
                convert::vector(wheel.axle_direction),
                wheel.suspension_max_length,
                wheel.radius,
                &tuning,
            );
            controller_wheel.steering = 0.0;
        }
        controller
    }

    /// Per-wheel suspension stiffness and damping, resolving an authored zero
    /// stiffness and negative damping into Cry's derived values.
    fn vehicle_wheel_springs(
        &self,
        configuration: &WheeledVehicleConfiguration,
        rigid_body: &RigidBody,
        mass: f32,
    ) -> Vec<(f32, f32)> {
        let effective_masses = configuration
            .wheels
            .iter()
            .map(|wheel| {
                let pose = rigid_body.position();
                let point = pose * convert::vector(wheel.connection);
                let direction = pose.rotation * convert::vector(-wheel.suspension_direction);
                let mass_properties = rigid_body.mass_properties();
                let relative_point = point - mass_properties.world_com;
                let angular_axis = relative_point.cross(direction);
                let inverse_mass = direction.dot(mass_properties.effective_inv_mass * direction)
                    + angular_axis.dot(mass_properties.effective_world_inv_inertia * angular_axis);
                if inverse_mass > f32::EPSILON {
                    inverse_mass.recip()
                } else {
                    mass
                }
            })
            .collect::<Vec<_>>();
        let auto_stiffness = cry_vehicle_stiffness(
            configuration,
            &effective_masses,
            rigid_body.local_center_of_mass(),
            self.gravity,
            mass,
        );
        configuration
            .wheels
            .iter()
            .zip(&effective_masses)
            .zip(auto_stiffness)
            .map(|((wheel, effective_mass), auto_stiffness)| {
                let stiffness = if wheel.stiffness == 0.0 {
                    auto_stiffness
                } else {
                    wheel.stiffness
                };
                let damping = if wheel.damping < 0.0 {
                    -wheel.damping * (4.0 * stiffness * effective_mass).max(0.0).sqrt()
                } else {
                    wheel.damping
                };
                (stiffness, damping)
            })
            .collect()
    }

    /// Writes the solved translation back onto the kinematic body and refreshes
    /// the character's support, flight timer, and ground references.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BackendInvariant`] when the character body has
    /// gone missing since its cast.
    fn publish_character_support(
        &mut self,
        native_body: RigidBodyHandle,
        state: &mut CharacterState,
        contacts: &[CharacterContactPlane],
        motion: CharacterMotion,
    ) -> Result<(), PhysicsError> {
        let CharacterMotion {
            rigid_pose,
            translation,
            time_step,
        } = motion;
        state.support = aggregate_character_support(&state.configuration, contacts);
        let mut next_pose = rigid_pose;
        next_pose.translation += convert::vector(translation);
        self.rigid_bodies
            .get_mut(native_body)
            .ok_or(PhysicsError::BackendInvariant(
                "character body disappeared while applying movement",
            ))?
            .set_next_kinematic_position(next_pose);
        state.velocity = translation / time_step;
        state.flying = !state.support.is_on_ground();
        if state.flying {
            state.time_flying += time_step;
            if state.support.state == CharacterSupportState::Unsupported {
                state.ground_body = None;
                state.ground_surface = None;
            }
        } else {
            state.time_flying = 0.0;
            state.ground_body = state.support.body;
            state.ground_surface = state.support.body.and_then(|support_body| {
                contacts
                    .iter()
                    .find(|contact| contact.body == support_body)
                    .map(|contact| contact.surface_index)
            });
        }
        Ok(())
    }

    /// Scans a body's retained contact pairs for the Cry sleep test: how many
    /// solver contacts it has, how many distinct bodies support it, and whether
    /// any awake dynamic body rests on top of it.
    fn cry_contact_support(&self, rigid_body: &RigidBody) -> CryContactSupport {
        let mut contact_pairs = HashSet::new();
        let mut contact_count = 0usize;
        let mut support_bodies = HashSet::new();
        let mut awake_dynamic_support = false;
        let gravity_length = self.gravity.length();
        for &collider in rigid_body.colliders() {
            for pair in self.narrow_phase.contact_pairs_with(collider) {
                if !pair.has_any_active_contact() {
                    continue;
                }
                let pair_key = InteractionKey::new(
                    pair.collider1,
                    pair.collider2,
                    PhysicsInteractionKind::Contact,
                )
                .0;
                if !contact_pairs.insert(pair_key) {
                    continue;
                }
                let own_is_first = pair.collider1 == collider;
                let other_collider = if own_is_first {
                    pair.collider2
                } else {
                    pair.collider1
                };
                let Some(other) = self
                    .collider_metadata
                    .get(&other_collider)
                    .map(|data| data.body)
                else {
                    continue;
                };
                support_bodies.insert(other);
                for manifold in &pair.manifolds {
                    contact_count += manifold.data.solver_contacts.len();
                    let normal =
                        convert::vec3(manifold.data.normal) * if own_is_first { 1.0 } else { -1.0 };
                    let top_facing = gravity_length > 0.0
                        && normal.dot(self.gravity)
                            < -gravity_length * core::f32::consts::FRAC_1_SQRT_2;
                    let other_awake_dynamic = self
                        .bodies
                        .get(&other)
                        .and_then(|native| self.rigid_bodies.get(native.rigid_body))
                        .is_some_and(|body| body.is_dynamic() && !body.is_sleeping());
                    awake_dynamic_support |= top_facing && other_awake_dynamic;
                }
            }
        }
        CryContactSupport {
            pair_count: contact_pairs.len(),
            contact_count,
            support_bodies: support_bodies.len(),
            awake_dynamic_support,
        }
    }

    /// Applies the native island-wide gate: a body only sleeps once every body
    /// it is connected to is itself sleeping, non-dynamic, or eligible.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::connected_bodies`] returns while walking an
    /// island.
    fn sleep_eligible_islands(
        &mut self,
        candidates: &[(
            PhysicsBodyHandle,
            RigidBodyHandle,
            RigidBodyConfiguration,
            RockNRollSleepMode,
        )],
        eligible: &HashMap<PhysicsBodyHandle, bool>,
    ) -> Result<(), PhysicsError> {
        let mut visited = HashSet::new();
        for (root, _, _, _) in candidates {
            if !visited.insert(*root) {
                continue;
            }
            let mut island = Vec::new();
            self.connected_bodies(*root, &mut island)?;
            visited.extend(island.iter().copied());
            let island_eligible = island.iter().all(|body| {
                self.bodies
                    .get(body)
                    .and_then(|native| self.rigid_bodies.get(native.rigid_body))
                    .is_none_or(|rigid_body| {
                        rigid_body.is_sleeping()
                            || !rigid_body.is_dynamic()
                            || eligible.get(body).copied().unwrap_or(false)
                    })
            });
            if island_eligible {
                for body in island {
                    if let Some(rigid_body) = self
                        .bodies
                        .get(&body)
                        .and_then(|native| self.rigid_bodies.get_mut(native.rigid_body))
                        && rigid_body.is_dynamic()
                    {
                        rigid_body.sleep();
                    }
                }
            }
        }
        Ok(())
    }

    /// Sums the buoyancy and medium-resistance impulses one body picks up from
    /// the fluid areas that overlap it, along with the damping and submerged
    /// fraction those areas imply.
    fn accumulate_buoyancy(
        &self,
        native: &NativeBody,
        configuration: RigidBodyConfiguration,
        body: BuoyantBody,
        areas: &[(PhysicsBodyHandle, FluidAreaConfiguration, Aabb3d)],
        time_step: f32,
    ) -> BuoyancyLoad {
        let inverse_mass = 1.0 / body.mass;
        let mut load = BuoyancyLoad {
            extra_damping: configuration.linear_damping,
            ..BuoyancyLoad::default()
        };
        // Cry stores at most four active buoyancy records for a body.
        for (_, area, _) in areas
            .iter()
            .filter(|(_, _, bounds)| aabbs_intersect(body.bounds, *bounds))
            .take(4)
        {
            let resistance = area.resistance * configuration.buoyancy.resistance_scale;
            let density = area.density * configuration.buoyancy.density_scale;
            if resistance + density + area.damping == 0.0 {
                continue;
            }

            let mut submerged_volume = 0.0;
            let mut full_volume = 0.0;
            for collider in native
                .collider_configurations
                .iter()
                .filter(|collider| collider.buoyancy_enabled)
            {
                let pose = body.pose * collider.local_pose;
                let geometry = buoyancy::submerged_geometry(&collider.shape, pose, area.plane);
                full_volume += geometry.full_volume;
                submerged_volume += geometry.volume;

                let relative_velocity = body.linear_velocity - area.flow;
                let resistance_gate = geometry.full_volume.powi(2)
                    * (relative_velocity.length() * resistance * inverse_mass).powi(3)
                    > 0.01f32.powi(3);
                if resistance_gate {
                    let drag = buoyancy::medium_resistance(
                        &collider.shape,
                        pose,
                        area.plane,
                        MediumMotion {
                            linear_velocity: relative_velocity,
                            angular_velocity: body.angular_velocity,
                            center_of_mass: body.center_of_mass,
                        },
                    );
                    load.linear_impulse += drag.linear * (resistance * time_step);
                    load.angular_impulse += drag.angular * (resistance * time_step);
                }

                if geometry.full_volume * density * inverse_mass > 0.01 && geometry.volume > 0.0 {
                    let body_half_extents = Vec3::from((body.bounds.max - body.bounds.min) * 0.5);
                    let body_center = Vec3::from((body.bounds.max + body.bounds.min) * 0.5);
                    let projected_radius = area.plane.normal.abs().dot(body_half_extents);
                    let signed_distance = area.plane.signed_distance(body_center);
                    let buoyancy_impulse = if signed_distance > -projected_radius {
                        area.plane.normal
                            * (self.gravity.length() * density * geometry.volume * time_step)
                    } else {
                        -self.gravity * (density * geometry.volume * time_step)
                    };
                    load.linear_impulse += buoyancy_impulse;
                    load.angular_impulse +=
                        (geometry.center - body.center_of_mass).cross(buoyancy_impulse);
                }
            }

            if full_volume * submerged_volume > 0.0 {
                let submerged_fraction = (submerged_volume / full_volume).min(1.0);
                let interpolated_damping =
                    configuration.buoyancy.damping.max(area.damping).mul_add(
                        submerged_fraction,
                        configuration.linear_damping * (1.0 - submerged_fraction),
                    );
                load.extra_damping = load.extra_damping.max(interpolated_damping);
                if area.medium == az_physics::FluidMedium::Water {
                    load.water_fraction = load.water_fraction.max(submerged_fraction);
                    load.floating |= body.mass
                        < area.density * configuration.buoyancy.density_scale * full_volume;
                }
            }
        }
        load
    }

    /// Applies one of the rigid-body scalar setters, validating the value and
    /// mirroring it onto the body's stored configuration.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidRigidBodyScalar`] for a non-finite or
    /// out-of-range value, [`PhysicsError::BackendInvariant`] when the body has
    /// no Rapier rigid body, or whatever
    /// [`RigidBodyBuoyancy::validate`] returns.
    fn apply_rigid_scalar_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        match action {
            PhysicsAction::SetMass(mass) => {
                if !mass.is_finite() || mass <= 0.0 {
                    return Err(PhysicsError::InvalidRigidBodyScalar { field: "mass" });
                }
                self.set_body_mass(body, mass)?;
                if let Some(configuration) = self
                    .bodies
                    .get_mut(&body)
                    .and_then(|native| native.rigid_configuration.as_mut())
                {
                    configuration.mass = mass;
                }
            }
            PhysicsAction::SetDensity(density) => self.set_body_density(body, density)?,
            PhysicsAction::SetLinearDamping(damping) => {
                if !damping.is_finite() || damping < 0.0 {
                    return Err(PhysicsError::InvalidRigidBodyScalar {
                        field: "linear_damping",
                    });
                }
                self.rigid_body_mut(body)?.set_linear_damping(damping);
                if let Some(configuration) = self
                    .bodies
                    .get_mut(&body)
                    .and_then(|native| native.rigid_configuration.as_mut())
                {
                    configuration.linear_damping = damping;
                }
            }
            PhysicsAction::SetAngularDamping(damping) => {
                if !damping.is_finite() || damping < 0.0 {
                    return Err(PhysicsError::InvalidRigidBodyScalar {
                        field: "angular_damping",
                    });
                }
                self.rigid_body_mut(body)?.set_angular_damping(damping);
                if let Some(configuration) = self
                    .bodies
                    .get_mut(&body)
                    .and_then(|native| native.rigid_configuration.as_mut())
                {
                    configuration.angular_damping = damping;
                }
            }
            PhysicsAction::SetSleepMinEnergy(minimum_energy) => {
                if !minimum_energy.is_finite() || minimum_energy < 0.0 {
                    return Err(PhysicsError::InvalidRigidBodyScalar {
                        field: "sleep_min_energy",
                    });
                }
                if let Some(configuration) = self
                    .bodies
                    .get_mut(&body)
                    .and_then(|native| native.rigid_configuration.as_mut())
                {
                    configuration.sleep_min_energy = minimum_energy;
                }
            }
            PhysicsAction::SetBuoyancy(configuration) => {
                configuration.validate()?;
                if let Some(rigid_configuration) = self
                    .bodies
                    .get_mut(&body)
                    .and_then(|native| native.rigid_configuration.as_mut())
                {
                    rigid_configuration.buoyancy = configuration;
                }
            }
            PhysicsAction::Impulse(_)
            | PhysicsAction::AngularImpulse(_)
            | PhysicsAction::Force(_)
            | PhysicsAction::Torque(_)
            | PhysicsAction::Reset
            | PhysicsAction::Wake(_)
            | PhysicsAction::SetPose(_)
            | PhysicsAction::SetVelocity(_)
            | PhysicsAction::SetAngularVelocity(_)
            | PhysicsAction::SetSimulated(_)
            | PhysicsAction::Move(_)
            | PhysicsAction::SyncLiving(_)
            | PhysicsAction::SetLivingDimensions(_)
            | PhysicsAction::SetLivingDynamics(_) => {
                unreachable!("only the scalar setters reach this dispatch")
            }
        }
        Ok(())
    }
}

/// [`BodyStatus`] for a particle body, whose motion this adapter integrates
/// itself instead of handing it to Rapier.
fn particle_body_status(
    native: &NativeBody,
    rigid_body: &RigidBody,
    state: &ParticleState,
) -> BodyStatus {
    let position = convert::vec3(rigid_body.translation());
    let awake = state.is_awake(position);
    let volume =
        state.configuration.size * state.configuration.size * state.configuration.thickness;
    let angular_inertia = 0.4
        * state.configuration.mass
        * (state.configuration.size * 0.5)
        * (state.configuration.size * 0.5);
    BodyStatus {
        pose: convert::physics_pose(rigid_body.position()),
        linear_velocity: state.velocity,
        angular_velocity: state.angular_velocity,
        linear_acceleration: if awake {
            state.acceleration()
        } else {
            Vec3::ZERO
        },
        angular_acceleration: if awake {
            native.angular_acceleration
        } else {
            Vec3::ZERO
        },
        mass: state.configuration.mass,
        density: if volume > 0.0 {
            state.configuration.mass / volume
        } else {
            0.0
        },
        kinetic_energy: (0.5 * angular_inertia).mul_add(
            state.angular_velocity.length_squared(),
            0.5 * state.configuration.mass * state.velocity.length_squared(),
        ),
        linear_damping: if state.submerged_depth < 0.0 {
            state.configuration.water_resistance
        } else {
            state.configuration.air_resistance
        },
        angular_damping: 0.0,
        sleep_min_energy: 0.5
            * state.configuration.mass
            * state.configuration.minimum_speed
            * state.configuration.minimum_speed,
        buoyancy: RigidBodyBuoyancy::default(),
        buoyancy_status: BuoyancyStatus {
            submerged_fraction: if state.submerged_depth < 0.0 {
                (-state.submerged_depth / (state.configuration.size * 0.5)).min(1.0)
            } else {
                0.0
            },
            floating: state.submerged_depth < 0.0,
        },
        simulation_class: SimulationClass::Independent,
        awake,
        kinematic: true,
        simulated: rigid_body.is_enabled(),
    }
}

/// [`BodyStatus`] for a rope body, whose motion this adapter integrates
/// itself instead of handing it to Rapier.
fn rope_body_status(native: &NativeBody, rigid_body: &RigidBody, state: &RopeState) -> BodyStatus {
    let linear_velocity =
        state.velocities.iter().copied().sum::<Vec3>() / f32_from_usize(state.velocities.len());
    let kinetic_energy = state
        .velocities
        .iter()
        .map(|velocity| velocity.length_squared())
        .sum::<f32>()
        * (0.5 * state.configuration.mass / f32_from_usize(state.velocities.len()));
    let volume = core::f32::consts::PI
        * state.configuration.collision_distance.powi(2)
        * state
            .points
            .windows(2)
            .map(|points| points[0].distance(points[1]))
            .sum::<f32>();
    BodyStatus {
        pose: convert::physics_pose(rigid_body.position()),
        linear_velocity,
        angular_velocity: Vec3::ZERO,
        linear_acceleration: native.linear_acceleration,
        angular_acceleration: Vec3::ZERO,
        mass: state.configuration.mass,
        density: if volume > 0.0 {
            state.configuration.mass / volume
        } else {
            0.0
        },
        kinetic_energy,
        linear_damping: state.configuration.damping,
        angular_damping: 0.0,
        sleep_min_energy: state.configuration.minimum_energy,
        buoyancy: RigidBodyBuoyancy::default(),
        buoyancy_status: BuoyancyStatus::default(),
        simulation_class: SimulationClass::Independent,
        awake: state.awake,
        kinematic: true,
        simulated: rigid_body.is_enabled(),
    }
}

/// [`BodyStatus`] for a soft body, whose motion this adapter integrates
/// itself instead of handing it to Rapier.
fn soft_body_status(
    native: &NativeBody,
    rigid_body: &RigidBody,
    state: &SoftBodyState,
) -> BodyStatus {
    let linear_velocity =
        state.velocities.iter().copied().sum::<Vec3>() / f32_from_usize(state.velocities.len());
    let kinetic_energy = state
        .velocities
        .iter()
        .map(|velocity| velocity.length_squared())
        .sum::<f32>()
        * (0.5 * state.configuration.mass / f32_from_usize(state.velocities.len()));
    BodyStatus {
        pose: convert::physics_pose(rigid_body.position()),
        linear_velocity,
        angular_velocity: Vec3::ZERO,
        linear_acceleration: native.linear_acceleration,
        angular_acceleration: Vec3::ZERO,
        mass: state.configuration.mass,
        density: 0.0,
        kinetic_energy,
        linear_damping: state.configuration.damping,
        angular_damping: 0.0,
        sleep_min_energy: state.configuration.minimum_energy,
        buoyancy: RigidBodyBuoyancy::default(),
        buoyancy_status: BuoyancyStatus::default(),
        simulation_class: SimulationClass::Independent,
        awake: state.awake,
        kinematic: true,
        simulated: rigid_body.is_enabled(),
    }
}

/// [`BodyStatus`] for a linked soft body, whose motion this adapter integrates
/// itself instead of handing it to Rapier.
fn linked_soft_body_status(
    native: &NativeBody,
    rigid_body: &RigidBody,
    state: &LinkedSoftBodyState,
) -> BodyStatus {
    let linear_velocity =
        state.velocities.iter().copied().sum::<Vec3>() / f32_from_usize(state.velocities.len());
    let kinetic_energy = state
        .velocities
        .iter()
        .map(|velocity| velocity.length_squared())
        .sum::<f32>()
        * (0.5 * state.configuration.mass / f32_from_usize(state.velocities.len()));
    BodyStatus {
        pose: convert::physics_pose(rigid_body.position()),
        linear_velocity,
        angular_velocity: Vec3::ZERO,
        linear_acceleration: native.linear_acceleration,
        angular_acceleration: Vec3::ZERO,
        mass: state.configuration.mass,
        density: 0.0,
        kinetic_energy,
        linear_damping: 0.0,
        angular_damping: 0.0,
        sleep_min_energy: state.configuration.minimum_energy,
        buoyancy: RigidBodyBuoyancy::default(),
        buoyancy_status: BuoyancyStatus::default(),
        simulation_class: SimulationClass::Independent,
        awake: state.awake,
        kinematic: true,
        simulated: rigid_body.is_enabled(),
    }
}

/// The working frame of one Cry particle substep, threaded through the sliding,
/// drag, and impact phases.
#[derive(Debug, Clone, Copy)]
struct ParticleStep {
    position: Vec3,
    previous_position: Vec3,
    velocity: Vec3,
    /// Velocity the sweep and the impact response are resolved against, which
    /// carries the half-step gravity a free particle already picked up.
    collision_velocity: Vec3,
    gravity: Vec3,
    resistance: f32,
    orientation: Quat,
    flags: ParticleFlags,
    radius: f32,
    lying_radius: f32,
}

impl ParticleStep {
    /// Opens a substep at the particle's current pose, applying the leading
    /// half-step of gravity that a free particle integrates before its sweep.
    fn begin(state: &ParticleState, current_pose: PhysicsPose, time_step: f32) -> Self {
        let radius = state.configuration.size * 0.5;
        let gravity = if state.submerged_depth < 0.0 {
            state.water_gravity * (-state.submerged_depth / radius).min(1.0)
        } else {
            state.gravity
        };
        let mut collision_velocity = state.velocity;
        if !state.sliding {
            collision_velocity += gravity * (time_step * 0.5);
        }
        Self {
            position: current_pose.translation + collision_velocity * time_step,
            previous_position: current_pose.translation,
            velocity: state.velocity,
            collision_velocity,
            gravity,
            resistance: if state.submerged_depth < 0.0 {
                state.configuration.water_resistance
            } else {
                state.configuration.air_resistance
            },
            orientation: current_pose.rotation,
            flags: state.configuration.flags,
            radius,
            lying_radius: state.configuration.thickness * 0.5,
        }
    }

    /// Adds thrust, medium drag, and path lift for this substep, and republishes
    /// the particle's heading.
    fn integrate_drag(&mut self, state: &mut ParticleState, time_step: f32) {
        self.velocity += (self.gravity
            + state.heading * state.configuration.thrust_acceleration
            + (state.medium_velocity - self.velocity) * self.resistance
            + particle_lift(state.heading, state.gravity)
                * (state.lift_per_speed * self.velocity.length()))
            * time_step;
        let heading = self.velocity.normalize_or_zero();
        if heading != Vec3::ZERO {
            state.heading = heading;
        }
    }

    /// Advances the particle's spin and, unless it is pinned or resting, aligns
    /// it with its flight path.
    fn integrate_orientation(&mut self, state: &mut ParticleState, time_step: f32) {
        if self.flags.contains(ParticleFlags::CONSTANT_ORIENTATION) {
            return;
        }
        if self.flags.contains(ParticleFlags::NO_SPIN) {
            state.angular_velocity = Vec3::ZERO;
        } else {
            state.spin_orientation =
                integrate_orientation(state.spin_orientation, state.angular_velocity, time_step);
        }
        self.orientation =
            if !self.flags.contains(ParticleFlags::NO_PATH_ALIGNMENT) && !state.sliding {
                path_aligned_orientation(state.gravity, state.heading) * state.spin_orientation
            } else {
                state.spin_orientation
            };
    }

    /// Lays the particle flat on the surface it just hit and marks it sliding.
    fn settle_on(&mut self, state: &mut ParticleState, point: Vec3, normal: Vec3) {
        if (self.radius - self.lying_radius).abs() > f32::EPSILON {
            self.position = point + normal * self.lying_radius;
        }
        state.sliding = true;
        state.slide_normal = normal;
    }
}

/// One wheel's contact frame for a vehicle substep: its steered axle, the
/// forward direction along the ground, and the chassis-relative velocity at the
/// contact point.
#[derive(Debug, Clone, Copy)]
struct WheelFrame {
    index: usize,
    configuration: VehicleWheelConfiguration,
    contact: VehicleWheelContact,
    chassis_pose: Pose,
    suspension_up: Vector,
    axle: Vector,
    forward: Vector,
    relative_velocity: Vector,
    ground: Option<RigidBodyHandle>,
}

/// A rigid body's pose and motion as Cry's buoyancy integral sees it.
#[derive(Debug, Clone, Copy)]
struct BuoyantBody {
    pose: PhysicsPose,
    bounds: Aabb3d,
    center_of_mass: Vec3,
    linear_velocity: Vec3,
    angular_velocity: Vec3,
    mass: f32,
}

/// What the fluid areas overlapping one body add up to over a step.
#[derive(Debug, Clone, Copy, Default)]
struct BuoyancyLoad {
    linear_impulse: Vec3,
    angular_impulse: Vec3,
    water_fraction: f32,
    floating: bool,
    extra_damping: f32,
}

/// What a soft-body action leaves for the caller to push onto the body's Rapier
/// rigid core.
#[derive(Debug, Clone, Copy, Default)]
struct SoftBodyActionEffect {
    enabled: Option<bool>,
    synchronize_core_pose: bool,
    rigid_core_damping: Option<f32>,
}

/// Applies one action to a soft body's own state.
///
/// # Errors
///
/// Returns [`PhysicsError::InvalidSoftBodyConfiguration`] for a non-finite or
/// out-of-range scalar, [`PhysicsError::UnsupportedSoftBodyAction`] for a
/// rigid-body-only or living-only action, or whatever
/// [`SoftBodyState::apply_impulse`] returns.
fn apply_soft_body_state_action(
    state: &mut SoftBodyState,
    action: PhysicsAction,
) -> Result<SoftBodyActionEffect, PhysicsError> {
    let mut effect = SoftBodyActionEffect::default();
    match action {
        PhysicsAction::Impulse(action) => state.apply_impulse(SoftBodyImpulse {
            impulse: action.impulse,
            point: action.point,
            triangle: None,
        })?,
        PhysicsAction::AngularImpulse(impulse) => {
            if !impulse.is_finite() {
                return Err(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "angular impulse",
                });
            }
            state.apply_angular_impulse(impulse);
        }
        PhysicsAction::Force(force) => {
            if !force.is_finite() {
                return Err(PhysicsError::InvalidSoftBodyConfiguration { field: "force" });
            }
            state.add_force(force);
        }
        PhysicsAction::Torque(torque) => {
            if !torque.is_finite() {
                return Err(PhysicsError::InvalidSoftBodyConfiguration { field: "torque" });
            }
            state.add_torque(torque);
        }
        PhysicsAction::Reset => {
            state.reset();
            effect.synchronize_core_pose = true;
        }
        PhysicsAction::Wake(awake) => state.awake = awake,
        PhysicsAction::SetPose(pose) => {
            state.set_pose(pose);
            effect.synchronize_core_pose = true;
        }
        PhysicsAction::SetVelocity(velocity) => {
            if !velocity.is_finite() {
                return Err(PhysicsError::InvalidSoftBodyConfiguration { field: "velocity" });
            }
            state.set_velocity(velocity);
        }
        PhysicsAction::SetAngularVelocity(angular_velocity) => {
            if !angular_velocity.is_finite() {
                return Err(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "angular velocity",
                });
            }
            state.set_angular_velocity(angular_velocity);
        }
        PhysicsAction::SetMass(mass) => {
            if !mass.is_finite() || mass <= 0.0 {
                return Err(PhysicsError::InvalidSoftBodyConfiguration { field: "mass" });
            }
            state.set_mass(mass);
        }
        PhysicsAction::SetLinearDamping(damping) => {
            if !damping.is_finite() || damping < 0.0 {
                return Err(PhysicsError::InvalidSoftBodyConfiguration { field: "damping" });
            }
            state.configuration.damping = damping;
            effect.rigid_core_damping = Some(damping);
        }
        PhysicsAction::SetSleepMinEnergy(energy) => {
            if !energy.is_finite() || energy < 0.0 {
                return Err(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "minimum energy",
                });
            }
            state.configuration.minimum_energy = energy;
        }
        PhysicsAction::SetSimulated(simulated) => effect.enabled = Some(simulated),
        PhysicsAction::SetDensity(_)
        | PhysicsAction::SetAngularDamping(_)
        | PhysicsAction::SetBuoyancy(_)
        | PhysicsAction::Move(_)
        | PhysicsAction::SetLivingDimensions(_)
        | PhysicsAction::SetLivingDynamics(_)
        | PhysicsAction::SyncLiving(_) => {
            return Err(PhysicsError::UnsupportedSoftBodyAction {
                action: "generic rigid/living-only action",
            });
        }
    }
    Ok(effect)
}

impl PhysicsConstraintBackend for RapierPhysicsBackend {
    fn create_constraint(
        &mut self,
        descriptor: &ConstraintDescriptor,
    ) -> Result<PhysicsConstraintHandle, PhysicsError> {
        descriptor.validate()?;
        if descriptor.scene() != self.scene {
            return Err(PhysicsError::ConstraintSceneMismatch {
                parent: self.scene,
                child: descriptor.scene(),
            });
        }
        let handle = self.allocate_constraint_handle()?;
        let native = self.create_native_constraint(descriptor)?;
        self.constraints.insert(handle, native);
        Ok(handle)
    }

    fn update_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
        descriptor: &ConstraintDescriptor,
    ) -> Result<(), PhysicsError> {
        descriptor.validate()?;
        if constraint.scene() != self.scene || descriptor.scene() != self.scene {
            return Err(PhysicsError::ConstraintSceneMismatch {
                parent: constraint.scene(),
                child: descriptor.scene(),
            });
        }
        if !self.constraints.contains_key(&constraint) {
            return Err(PhysicsError::ConstraintNotFound(constraint));
        }
        let replacement = self.create_native_constraint(descriptor)?;
        let previous = self
            .constraints
            .insert(constraint, replacement)
            .ok_or(PhysicsError::ConstraintNotFound(constraint))?;
        self.remove_native_constraint(previous);
        Ok(())
    }

    fn remove_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<(), PhysicsError> {
        let native = self
            .constraints
            .remove(&constraint)
            .ok_or(PhysicsError::ConstraintNotFound(constraint))?;
        self.remove_native_constraint(native);
        Ok(())
    }

    fn constraint_status(
        &self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<ConstraintStatus, PhysicsError> {
        let state = self
            .constraints
            .get(&constraint)
            .ok_or(PhysicsError::ConstraintNotFound(constraint))?;
        Ok(ConstraintStatus {
            enabled: state.descriptor.enabled && !state.broken,
            broken: state.broken,
            break_reason: state.break_reason,
            linear_impulse: state.linear_impulse,
            angular_impulse: state.angular_impulse,
        })
    }
}

impl PhysicsParticleBackend for RapierPhysicsBackend {
    fn particle_status(&self, body: PhysicsBodyHandle) -> Result<ParticleStatus, PhysicsError> {
        self.native_body(body)?;
        let state =
            self.particles
                .get(&body)
                .ok_or(PhysicsError::OperationRequiresParticleBody {
                    operation: "particle_status",
                })?;
        Ok(ParticleStatus {
            heading: state.heading,
            acceleration: state.acceleration(),
            sliding: state.sliding,
            slide_normal: state.slide_normal,
            submerged_depth: state.submerged_depth,
            medium_velocity: state.medium_velocity,
            recent_collisions: state.recent_collisions,
            collision_pending: state.collision_pending,
        })
    }

    fn take_particle_collision(&mut self, body: PhysicsBodyHandle) -> Result<bool, PhysicsError> {
        self.native_body(body)?;
        let state =
            self.particles
                .get_mut(&body)
                .ok_or(PhysicsError::OperationRequiresParticleBody {
                    operation: "take_particle_collision",
                })?;
        Ok(core::mem::take(&mut state.collision_pending))
    }
}

impl PhysicsRopeBackend for RapierPhysicsBackend {
    fn set_rope_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        let relative_attachment = {
            let rope = self
                .ropes
                .get(&body)
                .ok_or(PhysicsError::OperationRequiresRopeBody {
                    operation: "set_rope_target_vertices",
                })?;
            if rope
                .configuration
                .flags
                .contains(RopeFlags::TARGET_VERTEX_RELATIVE_TO_END)
            {
                rope.configuration.attachments[1]
            } else if rope
                .configuration
                .flags
                .contains(RopeFlags::TARGET_VERTEX_RELATIVE_TO_START)
            {
                rope.configuration.attachments[0]
            } else {
                None
            }
        };
        let relative_frame = relative_attachment
            .map(|attachment| {
                self.deformable_attachment_frame(
                    attachment.body,
                    attachment.point,
                    attachment.local,
                )
            })
            .transpose()?;
        self.ropes
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresRopeBody {
                operation: "set_rope_target_vertices",
            })?
            .set_target(action, relative_frame)
    }

    fn notify_rope_attachment_moved(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        self.ropes
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresRopeBody {
                operation: "notify_rope_attachment_moved",
            })?
            .notify_attachment_moved();
        Ok(())
    }

    fn apply_rope_volumetric_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: RopeVolumetricPressure,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        self.ropes
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresRopeBody {
                operation: "apply_rope_volumetric_pressure",
            })?
            .apply_volumetric_pressure(pressure)
    }

    fn write_rope_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut RopeStatus,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        self.ropes
            .get(&body)
            .ok_or(PhysicsError::OperationRequiresRopeBody {
                operation: "write_rope_status",
            })?
            .write_status(output);
        Ok(())
    }
}

impl PhysicsSoftBodyBackend for RapierPhysicsBackend {
    fn set_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        action.validate()?;
        let first_attachment = self
            .soft_bodies
            .get(&body)
            .ok_or(PhysicsError::OperationRequiresSoftBody {
                operation: "set_soft_body_target_vertices",
            })?
            .first_attachment();
        let resolved_host = if action.host.is_none() && action.points.is_none() {
            first_attachment
                .map(|attachment| {
                    self.deformable_attachment_frame(
                        attachment.body,
                        attachment.point,
                        attachment.local,
                    )
                })
                .transpose()?
        } else {
            None
        };
        self.soft_bodies
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresSoftBody {
                operation: "set_soft_body_target_vertices",
            })?
            .set_target(action, resolved_host)
    }

    fn update_soft_body_attachments(
        &mut self,
        body: PhysicsBodyHandle,
        update: &SoftBodyAttachmentUpdate,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        if let Some(target) = update.body
            && target.scene() != self.scene
        {
            return Err(PhysicsError::ConstraintSceneMismatch {
                parent: target.scene(),
                child: self.scene,
            });
        }
        self.soft_bodies
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresSoftBody {
                operation: "update_soft_body_attachments",
            })?
            .update_attachments(update)
    }

    fn apply_soft_body_impulse(
        &mut self,
        body: PhysicsBodyHandle,
        impulse: SoftBodyImpulse,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        self.soft_bodies
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresSoftBody {
                operation: "apply_soft_body_impulse",
            })?
            .apply_impulse(impulse)
    }

    fn apply_soft_body_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: SoftBodyPressure,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        self.soft_bodies
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresSoftBody {
                operation: "apply_soft_body_pressure",
            })?
            .apply_volumetric_pressure(pressure)
    }

    fn slice_soft_body(
        &mut self,
        body: PhysicsBodyHandle,
        slice: SoftBodySlice,
    ) -> Result<Option<SoftBodySliceResult>, PhysicsError> {
        self.native_body(body)?;
        let (rigid_core, rigid_core_collider) = self
            .soft_bodies
            .get(&body)
            .and_then(SoftBodyState::rigid_core_handles)
            .unzip();
        let rigid_core_configuration = self
            .soft_bodies
            .get(&body)
            .and_then(|state| state.configuration.rigid_core.as_ref())
            .map(|core| core.collider.clone());
        let result = self
            .soft_bodies
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresSoftBody {
                operation: "slice_soft_body",
            })?
            .slice(slice)?;
        if result.is_some()
            && let (Some(rigid_core), Some(rigid_core_collider)) = (rigid_core, rigid_core_collider)
        {
            self.collider_metadata.remove(&rigid_core_collider);
            self.rigid_bodies.remove(
                rigid_core,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
            if let Some(configuration) = rigid_core_configuration
                && let Some(native) = self.bodies.get_mut(&body)
                && let Some(index) = native
                    .collider_configurations
                    .iter()
                    .rposition(|collider| *collider == configuration)
            {
                native.collider_configurations.remove(index);
            }
        }
        Ok(result)
    }

    fn write_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut SoftBodyStatus,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        self.soft_bodies
            .get(&body)
            .ok_or(PhysicsError::OperationRequiresSoftBody {
                operation: "write_soft_body_status",
            })?
            .write_status(output);
        Ok(())
    }
}

impl PhysicsLinkedSoftBodyBackend for RapierPhysicsBackend {
    fn set_linked_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        target: DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        self.linked_soft_bodies
            .get_mut(&body)
            .ok_or(PhysicsError::OperationRequiresLinkedSoftBody {
                operation: "set_linked_soft_body_target_vertices",
            })?
            .set_target(target)
    }

    fn linked_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
    ) -> Result<LinkedSoftBodyStatusRef<'_>, PhysicsError> {
        self.native_body(body)?;
        let state = self.linked_soft_bodies.get(&body).ok_or(
            PhysicsError::OperationRequiresLinkedSoftBody {
                operation: "linked_soft_body_status",
            },
        )?;
        Ok(state.status())
    }
}

impl PhysicsVehicleBackend for RapierPhysicsBackend {
    fn apply_vehicle_drive(
        &mut self,
        body: PhysicsBodyHandle,
        action: VehicleDriveAction,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        let state =
            self.vehicles
                .get_mut(&body)
                .ok_or(PhysicsError::OperationRequiresVehicleBody {
                    operation: "apply_vehicle_drive",
                })?;
        let previous_hand_brake = state.hand_brake;
        state.apply_drive(action)?;
        let wake = state.pedal != 0.0 || (previous_hand_brake && !state.hand_brake);
        if wake {
            self.rigid_body_mut(body)?.wake_up(true);
        }
        Ok(())
    }

    fn vehicle_status(&self, body: PhysicsBodyHandle) -> Result<VehicleStatus, PhysicsError> {
        let state = self
            .vehicles
            .get(&body)
            .ok_or(PhysicsError::OperationRequiresVehicleBody {
                operation: "vehicle_status",
            })?;
        let rigid_body = self.rigid_body(body)?;
        Ok(VehicleStatus {
            steer: state.steer,
            pedal: state.pedal,
            hand_brake: state.hand_brake,
            foot_brake: if state.pedal * sign_zero(f32_from_i32(state.current_gear - 1)) <= 0.0 {
                state.pedal.abs()
            } else {
                0.0
            },
            velocity: convert::vec3(rigid_body.linvel()),
            wheel_contacts: u32_from_usize(
                state
                    .wheels
                    .iter()
                    .filter(|wheel| wheel.contact.is_some())
                    .count(),
            ),
            current_gear: state.current_gear,
            engine_rpm: angular_to_rpm(state.engine_angular_velocity),
            clutch: state.clutch,
            driving_torque: state.driving_torque,
            active_colliders: state.active_colliders,
        })
    }

    fn vehicle_wheel_status(
        &self,
        body: PhysicsBodyHandle,
        wheel: usize,
    ) -> Result<VehicleWheelStatus, PhysicsError> {
        let state = self
            .vehicles
            .get(&body)
            .ok_or(PhysicsError::OperationRequiresVehicleBody {
                operation: "vehicle_wheel_status",
            })?;
        let configuration = state
            .configuration
            .wheels
            .get(wheel)
            .ok_or(PhysicsError::VehicleWheelNotFound { wheel })?;
        let runtime = state
            .wheels
            .get(wheel)
            .ok_or(PhysicsError::VehicleWheelNotFound { wheel })?;
        let controller = state
            .controller
            .wheels()
            .get(wheel)
            .ok_or(PhysicsError::VehicleWheelNotFound { wheel })?;
        let contact_surface = runtime
            .contact
            .and_then(|contact| self.collider_metadata.get(&contact.collider))
            .map(|metadata| metadata.surface_index);
        let collider = runtime
            .contact
            .and_then(|contact| self.collider_metadata.get(&contact.collider))
            .map(|metadata| metadata.body);
        Ok(VehicleWheelStatus {
            wheel,
            part_id: configuration.part_id,
            contact: runtime.contact.is_some(),
            contact_point: runtime
                .contact
                .map_or(Vec3::ZERO, |contact| convert::vec3(contact.point)),
            contact_normal: runtime
                .contact
                .map_or(Vec3::ZERO, |contact| convert::vec3(contact.normal)),
            angular_velocity: if self.rigid_body(body)?.is_sleeping() {
                0.0
            } else {
                runtime.angular_velocity
            },
            slipping: runtime.contact.is_some() && runtime.slipping,
            slip_velocity: if runtime.contact.is_some() {
                runtime.slip_velocity
            } else {
                Vec3::ZERO
            },
            contact_surface,
            friction: runtime.friction,
            suspension_length: runtime
                .contact
                .map_or(configuration.suspension_max_length, |contact| {
                    contact.suspension_length
                }),
            suspension_full_length: configuration.suspension_max_length,
            suspension_initial_length: configuration.suspension_initial_length,
            radius: configuration.radius,
            torque: runtime.torque,
            steer: controller.steering,
            collider,
        })
    }

    fn vehicle_abilities(
        &self,
        body: PhysicsBodyHandle,
        steer: Option<f32>,
    ) -> Result<VehicleAbilities, PhysicsError> {
        if steer.is_some_and(|steer| !steer.is_finite()) {
            return Err(PhysicsError::InvalidVehicleConfiguration {
                field: "abilities steer",
            });
        }
        let state = self
            .vehicles
            .get(&body)
            .ok_or(PhysicsError::OperationRequiresVehicleBody {
                operation: "vehicle_abilities",
            })?;
        let rigid_body = self.rigid_body(body)?;
        let driving_wheels = state
            .configuration
            .wheels
            .iter()
            .filter(|wheel| wheel.driving)
            .count();
        let maximum_engine_speed = rpm_to_angular(state.configuration.engine_maximum_rpm);
        let maximum_velocity = if driving_wheels == 0 {
            0.0
        } else {
            let radius = state.configuration.wheels[0].radius;
            let mut bounds = [maximum_engine_speed * 0.01, maximum_engine_speed];
            let mut speed = bounds[0];
            for _ in 0..256 {
                speed = (bounds[0] + bounds[1]) * 0.5;
                let balance = ((speed * radius).powi(2) * state.configuration.damping).mul_add(
                    -rigid_body.mass(),
                    state.configuration.axle_friction.mul_add(
                        -speed,
                        (speed * core::f32::consts::PI / maximum_engine_speed).sin()
                            * state.configuration.engine_power,
                    ) * f32_from_usize(driving_wheels),
                );
                if balance < 0.0 {
                    bounds[1] = speed;
                } else {
                    bounds[0] = speed;
                }
                if bounds[1] - bounds[0] <= maximum_engine_speed * 0.005 {
                    break;
                }
            }
            speed * radius
        };

        let rotation_pivot = steer.and_then(|steer| {
            if steer == 0.0 {
                return None;
            }
            let center = convert::vec3(rigid_body.local_center_of_mass());
            let mut points = [None, None];
            for wheel in &state.configuration.wheels {
                if (wheel.connection.x - center.x) * steer > 0.0 {
                    let side = usize::from(center.y - wheel.connection.y < 0.0);
                    points[side] = Some(wheel.connection);
                }
            }
            let (mut rear, front) = (points[0]?, points[1]?);
            rear.x = ((front.y - rear.y) / steer.tan()).mul_add(sign_zero(steer), front.x);
            Some(convert::physics_pose(rigid_body.position()).transform_point(rear))
        });
        Ok(VehicleAbilities {
            rotation_pivot,
            maximum_velocity,
        })
    }
}

impl PhysicsBackend for RapierPhysicsBackend {
    fn gravity(&self) -> Vec3 {
        self.gravity
    }

    fn set_gravity(&mut self, gravity: Vec3) {
        self.gravity = gravity;
    }

    fn create_body(
        &mut self,
        descriptor: &BodyDescriptor,
    ) -> Result<PhysicsBodyHandle, PhysicsError> {
        descriptor.validate()?;
        let body = self.allocate_body_handle()?;
        let physical_type = descriptor.kind.physical_entity_type();
        let query_type = descriptor.kind.query_type();
        let rigid_configuration = match &descriptor.kind {
            BodyKind::Rigid(configuration) | BodyKind::Articulated(configuration) => {
                Some(*configuration)
            }
            BodyKind::WheeledVehicle(configuration) => Some(configuration.rigid_body),
            _ => None,
        };
        let builder = self.body_builder(descriptor, body, rigid_configuration)?;
        let rigid_body = self.rigid_bodies.insert(builder);
        if let Some(configuration) = rigid_configuration {
            configure_sleep_activation(
                self.rigid_bodies[rigid_body].activation_mut(),
                configuration,
            );
        }

        let collider_owner = ColliderOwner {
            body,
            rigid_body,
            entity_id: descriptor.entity_id,
            query_type,
            continuous_collision_mode: rigid_configuration.map_or(
                az_physics::ContinuousCollisionMode::Disabled,
                |configuration| configuration.continuous_collision_mode,
            ),
            continuous_prediction_distance: rigid_configuration.map_or(0.0, |configuration| {
                configuration.continuous_prediction_distance
            }),
        };
        self.bodies.insert(
            body,
            NativeBody {
                rigid_body,
                entity_id: descriptor.entity_id,
                physical_type,
                query_type,
                rigid_configuration,
                collider_configurations: descriptor.colliders.clone(),
                linear_acceleration: Vec3::ZERO,
                angular_acceleration: Vec3::ZERO,
                previous_pose: descriptor.pose,
                rock_n_roll_sleep_eligible_time: 0.0,
                rock_n_roll_smoothed_linear_speed_squared: 0.0,
                rock_n_roll_smoothed_angular_speed_squared: 0.0,
                rock_n_roll_smoothed_energy: 0.0,
            },
        );
        let created_colliders =
            self.insert_authored_colliders(body, descriptor, collider_owner, rigid_configuration)?;

        self.attach_body_kind_state(
            body,
            descriptor,
            collider_owner,
            rigid_body,
            &created_colliders,
        )?;

        self.apply_authored_mass(body, descriptor)?;

        if let BodyKind::FluidArea(configuration) = &descriptor.kind {
            self.fluid_areas.insert(body, *configuration);
        }

        if let BodyKind::WheeledVehicle(configuration) = &descriptor.kind {
            let state = match self.create_vehicle_state(body, configuration) {
                Ok(state) => state,
                Err(error) => {
                    self.remove_body(body)?;
                    return Err(error);
                }
            };
            self.vehicles.insert(body, state);
        }

        Ok(body)
    }

    fn remove_body(&mut self, body: PhysicsBodyHandle) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        let constraints: Vec<_> = self
            .constraints
            .iter()
            .filter_map(|(handle, constraint)| {
                let parent_matches =
                    matches!(constraint.descriptor.parent, ConstraintTarget::Body(parent) if parent == body);
                (constraint.descriptor.child == body || parent_matches).then_some(*handle)
            })
            .collect();
        for constraint in constraints {
            self.remove_constraint(constraint)?;
        }
        let rigid_core = self
            .soft_bodies
            .get(&body)
            .and_then(SoftBodyState::rigid_core_handles);
        let native = self
            .bodies
            .remove(&body)
            .ok_or(PhysicsError::BodyNotFound(body))?;
        let mut colliders = self
            .rigid_bodies
            .get(native.rigid_body)
            .ok_or(PhysicsError::BackendInvariant(
                "engine body references a missing Rapier rigid body",
            ))?
            .colliders()
            .to_vec();
        if let Some((rigid_core_body, rigid_core_collider)) = rigid_core {
            colliders.push(rigid_core_collider);
            self.rigid_bodies.remove(
                rigid_core_body,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
        }
        for collider in &colliders {
            self.collider_metadata.remove(collider);
        }
        self.rigid_bodies.remove(
            native.rigid_body,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
        let mut ignored_events = Vec::new();
        self.query_broad_phase.update(
            &self.integration_parameters,
            &self.colliders,
            &self.rigid_bodies,
            &[],
            &colliders,
            &mut ignored_events,
        );
        self.living.remove(&body);
        self.characters.remove(&body);
        self.particles.remove(&body);
        self.ropes.remove(&body);
        self.soft_bodies.remove(&body);
        self.linked_soft_bodies.remove(&body);
        self.vehicles.remove(&body);
        self.fluid_areas.remove(&body);
        self.buoyancy_status.remove(&body);
        Ok(())
    }

    fn apply_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        if self.ropes.contains_key(&body) {
            return self.apply_rope_action(body, action);
        }
        if self.soft_bodies.contains_key(&body) {
            return self.apply_soft_body_action(body, action);
        }
        if self.linked_soft_bodies.contains_key(&body) {
            return self.apply_linked_soft_body_action(body, action);
        }
        if self.particles.contains_key(&body) {
            return self.apply_particle_action(body, action);
        }
        if self.characters.contains_key(&body) {
            return self.apply_character_body_action(body, action);
        }
        if self.living.contains_key(&body) {
            return self.apply_living_body_action(body, action);
        }
        self.apply_rigid_body_action(body, action)
    }

    fn body_status(&self, body: PhysicsBodyHandle) -> Result<BodyStatus, PhysicsError> {
        let native = self.native_body(body)?;
        let rigid_body =
            self.rigid_bodies
                .get(native.rigid_body)
                .ok_or(PhysicsError::BackendInvariant(
                    "engine body references a missing Rapier rigid body",
                ))?;
        if let Some(state) = self.particles.get(&body) {
            return Ok(particle_body_status(native, rigid_body, state));
        }
        if let Some(state) = self.ropes.get(&body) {
            return Ok(rope_body_status(native, rigid_body, state));
        }
        if let Some(state) = self.soft_bodies.get(&body) {
            return Ok(soft_body_status(native, rigid_body, state));
        }
        if let Some(state) = self.linked_soft_bodies.get(&body) {
            return Ok(linked_soft_body_status(native, rigid_body, state));
        }
        self.rigid_body_status(body, native, rigid_body)
    }

    fn connected_bodies(
        &self,
        body: PhysicsBodyHandle,
        output: &mut Vec<PhysicsBodyHandle>,
    ) -> Result<(), PhysicsError> {
        self.native_body(body)?;
        output.clear();
        output.push(body);
        let mut visited = HashSet::from([body]);
        let engine_handle = |native: RigidBodyHandle| {
            let value = u64::try_from(self.rigid_bodies.get(native)?.user_data).ok()?;
            let handle = PhysicsBodyHandle::in_scene(self.scene, NonZeroU64::new(value)?);
            self.bodies.contains_key(&handle).then_some(handle)
        };

        let mut cursor = 0;
        while let Some(&current) = output.get(cursor) {
            cursor += 1;
            let native = self.native_body(current)?.rigid_body;
            let Some(rigid_body) = self.rigid_bodies.get(native) else {
                return Err(PhysicsError::BackendInvariant(
                    "engine body references a missing Rapier rigid body",
                ));
            };

            let mut enqueue = |candidate: PhysicsBodyHandle| {
                let Some(candidate_native) = self
                    .bodies
                    .get(&candidate)
                    .and_then(|body| self.rigid_bodies.get(body.rigid_body))
                else {
                    return;
                };
                if !candidate_native.is_fixed() && visited.insert(candidate) {
                    output.push(candidate);
                }
            };

            for &collider in rigid_body.colliders() {
                for pair in self.narrow_phase.contact_pairs_with(collider) {
                    if !pair.has_any_active_contact() {
                        continue;
                    }
                    let other = if pair.collider1 == collider {
                        pair.collider2
                    } else {
                        pair.collider1
                    };
                    if let Some(other) = self.collider_metadata.get(&other).map(|data| data.body) {
                        enqueue(other);
                    }
                }
            }
            for (_, joint) in self.impulse_joints.iter() {
                if !joint.data.is_enabled() {
                    continue;
                }
                let other = if joint.body1 == native {
                    Some(joint.body2)
                } else if joint.body2 == native {
                    Some(joint.body1)
                } else {
                    None
                };
                if let Some(other) = other.and_then(engine_handle) {
                    enqueue(other);
                }
            }
            for (_, _, multibody, link) in self.multibody_joints.iter() {
                let Some(parent) = link
                    .parent_id()
                    .and_then(|parent| multibody.link(parent))
                    .map(rapier3d::dynamics::MultibodyLink::rigid_body_handle)
                else {
                    continue;
                };
                let child = link.rigid_body_handle();
                let other = if child == native {
                    Some(parent)
                } else if parent == native {
                    Some(child)
                } else {
                    None
                };
                if let Some(other) = other.and_then(engine_handle) {
                    enqueue(other);
                }
            }
        }
        Ok(())
    }

    fn body_aabb(&self, body: PhysicsBodyHandle) -> Result<Aabb3d, PhysicsError> {
        let native = self.native_body(body)?;
        let rigid_body =
            self.rigid_bodies
                .get(native.rigid_body)
                .ok_or(PhysicsError::BackendInvariant(
                    "engine body references a missing Rapier rigid body",
                ))?;
        let mut colliders = rigid_body
            .colliders()
            .iter()
            .filter_map(|handle| self.colliders.get(*handle))
            .map(Collider::compute_aabb);
        let first = colliders.next().ok_or(PhysicsError::BackendInvariant(
            "engine body has no Rapier collider",
        ))?;
        let bounds = colliders.fold(first, |mut bounds, collider| {
            bounds.merge(&collider);
            bounds
        });
        Ok(Aabb3d::from_min_max(
            Vec3::new(bounds.mins.x, bounds.mins.y, bounds.mins.z),
            Vec3::new(bounds.maxs.x, bounds.maxs.y, bounds.maxs.z),
        ))
    }

    fn living_status(&self, body: PhysicsBodyHandle) -> Result<LivingStatus, PhysicsError> {
        let native = self.native_body(body)?;
        if let Some(state) = self.characters.get(&body) {
            return Ok(LivingStatus {
                flying: state.flying,
                time_flying: state.time_flying,
                camera_offset: Vec3::ZERO,
                velocity: state.velocity,
                unconstrained_velocity: state.velocity,
                requested_velocity: state.requested_velocity,
                ground_velocity: Vec3::ZERO,
                ground_height: 0.0,
                ground_slope: state.configuration.up_direction,
                ground_surface: state.ground_surface,
                ground_body: state.ground_body,
                time_since_stance_change: 0.0,
                stuck: false,
                squashed: false,
            });
        }
        let state = self
            .living
            .get(&body)
            .ok_or(PhysicsError::ActionRequiresLivingBody { action: "status" })?;
        let body_rotation = self
            .rigid_bodies
            .get(native.rigid_body)
            .map(|rigid_body| convert::physics_pose(rigid_body.position()).rotation)
            .ok_or(PhysicsError::BackendInvariant(
                "living body references a missing Rapier rigid body",
            ))?;
        Ok(LivingStatus {
            flying: state.flying && state.configuration.dynamics.is_active,
            time_flying: state.time_flying,
            camera_offset: body_rotation * (-Vec3::Z * state.camera_vertical_offset),
            velocity: state.velocity + state.ground_velocity,
            unconstrained_velocity: state.unconstrained_velocity,
            requested_velocity: state.requested_velocity,
            ground_velocity: state.ground_velocity,
            ground_height: state.ground_height,
            ground_slope: state.ground_slope,
            ground_surface: state.ground_surface,
            ground_body: state.ground_body,
            time_since_stance_change: state.time_since_stance_change,
            stuck: state.stuck,
            squashed: state.squashed,
        })
    }

    fn check_living_stance(
        &self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
    ) -> Result<LivingStanceCheck, PhysicsError> {
        dimensions.validate()?;
        let native = self.native_body(body)?;
        let pose = *self
            .rigid_bodies
            .get(native.rigid_body)
            .ok_or(PhysicsError::BackendInvariant(
                "living body references a missing Rapier rigid body",
            ))?
            .position();
        let mut direction = dimensions.unprojection_direction;
        let unprojection =
            self.living_unprojection_needed(body, dimensions, pose, &mut direction)?;
        Ok(LivingStanceCheck {
            allowed: unprojection.length_squared() <= f32::EPSILON,
            unprojection,
        })
    }

    fn character_support(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<CharacterSupportInfo, PhysicsError> {
        let native = self.native_body(body)?.clone();
        let state = self
            .characters
            .get(&body)
            .ok_or(PhysicsError::OperationRequiresCharacterBody {
                operation: "check_support",
            })?
            .clone();
        let collider =
            self.colliders
                .get(state.primary_collider)
                .ok_or(PhysicsError::BackendInvariant(
                    "character body references a missing primary collider",
                ))?;
        let pose = *collider.position();
        let shape = collider.shared_shape().clone();
        let collision_class = decode_collision_class(collider.user_data);
        let mut contacts = SmallVec::<[CharacterContactPlane; 8]>::new();
        self.collect_character_support_contacts(
            CharacterSweep {
                body,
                native_body: native.rigid_body,
                collision_class,
                pose: &pose,
                shape: shape.as_ref(),
                excluded: &[],
            },
            &state.configuration,
            &mut contacts,
        )?;
        let support = aggregate_character_support(&state.configuration, &contacts);
        self.characters
            .get_mut(&body)
            .expect("character existence was checked")
            .support = support;
        Ok(support)
    }

    fn integrate_character(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        if !time_step.is_finite() || time_step <= 0.0 {
            return Err(PhysicsError::InvalidTimeStep(time_step));
        }
        let asynchronous = self
            .characters
            .get(&body)
            .ok_or_else(|| {
                if self.bodies.contains_key(&body) {
                    PhysicsError::OperationRequiresCharacterBody {
                        operation: "integrate",
                    }
                } else {
                    PhysicsError::BodyNotFound(body)
                }
            })?
            .configuration
            .asynchronous;
        if asynchronous {
            return Ok(());
        }
        self.step_character_body(body, time_step)
    }

    fn ray_cast(&self, query: &RayCastConfiguration) -> Result<RayCastResult, PhysicsError> {
        query.validate()?;
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let Some(metadata) = self.collider_metadata.get(&handle) else {
                return false;
            };
            metadata.in_scene_queries
                && query.physical_entity_types.intersects(metadata.query_type)
                && (query.include_sensors || !metadata.sensor)
                && !query.ignore_bodies.contains(&metadata.body)
                && metadata
                    .entity_id
                    .is_none_or(|entity| !query.ignore_entity_ids.contains(&entity))
                && query.collision_class.is_none_or(|query_class| {
                    query_class.interacts_with(decode_collision_class(collider.user_data))
                })
                && query.collision_filter.is_none_or(|query_filter| {
                    metadata
                        .collision_filter
                        .is_some_and(|candidate| query_filter.interacts_with(candidate))
                })
        };
        let filter = QueryFilter::default().predicate(&predicate);
        let queries = self.query_broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.rigid_bodies,
            &self.colliders,
            filter,
        );
        let ray = Ray::new(
            convert::vector(query.origin),
            convert::vector(query.direction),
        );
        let mut hits: Vec<_> = queries
            .intersect_ray(ray, query.max_distance, true)
            .filter_map(|(collider, _, intersection)| {
                let metadata = self.collider_metadata.get(&collider)?;
                let distance = intersection.time_of_impact;
                Some(RayCastHit {
                    distance,
                    position: query.origin + query.direction * distance,
                    normal: convert::vec3(intersection.normal),
                    entity_id: metadata.entity_id,
                    body: metadata.body,
                    surface_index: metadata.surface_index,
                    surface_pierceability: metadata.surface_pierceability,
                    collider_tag: metadata.tag,
                })
            })
            .collect();
        hits.sort_by(|left, right| left.distance.total_cmp(&right.distance));

        let mut result = RayCastResult::default();
        let pierceability = query
            .pierces_surfaces_greater_than
            .clamp(0, RayCastConfiguration::MAX_SURFACE_PIERCEABILITY);
        for hit in hits.into_iter().take(query.max_hits) {
            if i32::from(hit.surface_pierceability) > pierceability {
                result.add_piercing_hit(hit);
            } else {
                result.set_blocking_hit(hit);
                break;
            }
        }
        Ok(result)
    }

    fn overlap_aabb(&self, query: AabbQuery) -> Result<Vec<OverlapHit>, PhysicsError> {
        query.validate()?;
        let half_extents = (query.max - query.min) * 0.5;
        self.overlap_shape(&ShapeOverlapConfiguration {
            shape: ColliderShape::Cuboid { half_extents },
            pose: PhysicsPose {
                translation: (query.min + query.max) * 0.5,
                rotation: glam::Quat::IDENTITY,
            },
            filter: az_physics::SpatialQueryFilter {
                ignore_entity_ids: Vec::new(),
                ignore_bodies: Vec::new(),
                physical_entity_types: query.physical_entity_types,
                include_sensors: true,
                collision_class: None,
                collision_filter: None,
            },
            max_results: query.max_results,
        })
    }

    fn overlap_shape(
        &self,
        query: &ShapeOverlapConfiguration,
    ) -> Result<Vec<OverlapHit>, PhysicsError> {
        query.validate()?;
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let Some(metadata) = self.collider_metadata.get(&handle) else {
                return false;
            };
            metadata.in_scene_queries
                && query
                    .filter
                    .physical_entity_types
                    .intersects(metadata.query_type)
                && (query.filter.include_sensors || !collider.is_sensor())
                && !query.filter.ignore_bodies.contains(&metadata.body)
                && metadata
                    .entity_id
                    .is_none_or(|entity| !query.filter.ignore_entity_ids.contains(&entity))
                && query.filter.collision_class.is_none_or(|query_class| {
                    query_class.interacts_with(decode_collision_class(collider.user_data))
                })
                && query.filter.collision_filter.is_none_or(|query_filter| {
                    metadata
                        .collision_filter
                        .is_some_and(|candidate| query_filter.interacts_with(candidate))
                })
        };
        let filter = QueryFilter::default().predicate(&predicate);
        let queries = self.query_broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.rigid_bodies,
            &self.colliders,
            filter,
        );
        let (builder, shape_pose) = convert::collider(&query.shape)?;
        let shape = builder.build().shared_shape().clone();
        let pose = convert::pose(query.pose * shape_pose);
        let mut seen = HashSet::new();
        let mut hits = Vec::new();
        for (handle, _) in queries.intersect_shape(pose, shape.as_ref()) {
            let Some(metadata) = self.collider_metadata.get(&handle) else {
                continue;
            };
            if seen.insert(metadata.body) {
                hits.push(OverlapHit {
                    body: metadata.body,
                    entity_id: metadata.entity_id,
                    surface_index: metadata.surface_index,
                    collider_tag: metadata.tag,
                });
                if hits.len() == query.max_results {
                    break;
                }
            }
        }
        hits.sort_by_key(|hit| hit.body.get());
        Ok(hits)
    }

    fn cast_shape_all(
        &self,
        query: &ShapeCastConfiguration,
    ) -> Result<Vec<PhysicsShapeCastHit>, PhysicsError> {
        query.validate()?;
        let (builder, shape_pose) = convert::collider(&query.shape)?;
        let shape = builder.build().shared_shape().clone();
        let pose = convert::pose(query.pose * shape_pose);
        let options = ShapeCastOptions {
            max_time_of_impact: query.max_distance,
            target_distance: query.target_distance,
            stop_at_penetration: query.stop_at_penetration,
            compute_impact_geometry_on_penetration: true,
        };
        let mut excluded_bodies = HashSet::new();
        let mut hits = Vec::with_capacity(query.max_results.min(self.bodies.len()));
        while hits.len() < query.max_results {
            let predicate = |handle: ColliderHandle, collider: &Collider| {
                let Some(metadata) = self.collider_metadata.get(&handle) else {
                    return false;
                };
                metadata.in_scene_queries
                    && query
                        .filter
                        .physical_entity_types
                        .intersects(metadata.query_type)
                    && (query.filter.include_sensors || !metadata.sensor)
                    && !query.filter.ignore_bodies.contains(&metadata.body)
                    && !excluded_bodies.contains(&metadata.body)
                    && metadata
                        .entity_id
                        .is_none_or(|entity| !query.filter.ignore_entity_ids.contains(&entity))
                    && query.filter.collision_class.is_none_or(|query_class| {
                        query_class.interacts_with(decode_collision_class(collider.user_data))
                    })
                    && query.filter.collision_filter.is_none_or(|query_filter| {
                        metadata
                            .collision_filter
                            .is_some_and(|candidate| query_filter.interacts_with(candidate))
                    })
            };
            let filter = QueryFilter::default().predicate(&predicate);
            let queries = self.query_broad_phase.as_query_pipeline(
                self.narrow_phase.query_dispatcher(),
                &self.rigid_bodies,
                &self.colliders,
                filter,
            );
            let Some((handle, impact)) = queries.cast_shape(
                &pose,
                convert::vector(query.direction),
                shape.as_ref(),
                options,
            ) else {
                break;
            };
            let metadata =
                self.collider_metadata
                    .get(&handle)
                    .ok_or(PhysicsError::BackendInvariant(
                        "shape cast returned an untracked collider",
                    ))?;
            excluded_bodies.insert(metadata.body);
            hits.push(PhysicsShapeCastHit {
                distance: impact.time_of_impact,
                position: convert::vec3(impact.witness1),
                normal: convert::vec3(impact.normal1),
                body: metadata.body,
                entity_id: metadata.entity_id,
                surface_index: metadata.surface_index,
                surface_pierceability: metadata.surface_pierceability,
                collider_tag: metadata.tag,
            });
        }
        hits.sort_by(|left, right| left.distance.total_cmp(&right.distance));
        Ok(hits)
    }

    fn drain_interactions(&mut self) -> Vec<PhysicsInteraction> {
        core::mem::take(&mut self.pending_interactions)
    }

    fn step(&mut self, time_step: f32) -> Result<(), PhysicsError> {
        if !time_step.is_finite() || time_step <= 0.0 {
            return Err(PhysicsError::InvalidTimeStep(time_step));
        }
        let previous_velocities = self
            .bodies
            .keys()
            .copied()
            .map(|body| {
                self.body_velocities(body)
                    .map(|(linear, angular)| (body, linear, angular))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.snapshot_body_poses()?;
        let living_bodies = sorted_body_handles(self.living.keys().copied());
        for body in living_bodies {
            self.step_living_body(body, time_step)?;
        }
        let character_bodies = sorted_body_handles(
            self.characters
                .iter()
                .filter_map(|(body, state)| state.configuration.asynchronous.then_some(*body)),
        );
        for body in character_bodies {
            self.step_character_body(body, time_step)?;
        }
        let particle_bodies = sorted_body_handles(self.particles.keys().copied());
        for body in particle_bodies {
            self.step_particle_body(body, time_step)?;
        }
        let rope_bodies = sorted_body_handles(self.ropes.keys().copied());
        for body in rope_bodies {
            self.step_rope_body(body, time_step)?;
        }
        let soft_bodies = sorted_body_handles(self.soft_bodies.keys().copied());
        for body in soft_bodies {
            self.step_soft_body(body, time_step)?;
        }
        let linked_soft_bodies = sorted_body_handles(self.linked_soft_bodies.keys().copied());
        for &body in &linked_soft_bodies {
            self.step_linked_soft_body(body, time_step)?;
        }
        self.solve_linked_soft_body_pairs(time_step)?;
        for body in linked_soft_bodies {
            self.synchronize_linked_soft_body_query(body)?;
        }
        let vehicle_bodies = sorted_body_handles(self.vehicles.keys().copied());
        for body in vehicle_bodies {
            self.step_vehicle_body(body, time_step)?;
        }

        self.apply_buoyancy(time_step);
        self.integrate_linear_step_damping(time_step);
        self.integration_parameters.dt = time_step;
        self.update_contact_prediction_distance();
        let collision_hooks = CollisionHooks {
            metadata: &self.collider_metadata,
        };
        self.pipeline.step(
            convert::vector(self.gravity),
            &self.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            &collision_hooks,
            &(),
        );
        self.evaluate_constraint_breakage(time_step);
        self.apply_hit_projections()?;
        self.apply_reverse_displacement_broadphase_proxies()?;
        self.query_broad_phase = self.broad_phase.clone();
        self.clamp_rigid_body_angular_velocities(time_step);
        self.collect_interactions();
        self.apply_cry_energy_sleep();
        self.apply_rock_n_roll_sleep(time_step)?;
        self.update_accelerations(&previous_velocities, time_step)?;
        self.physics_time += time_step;
        Ok(())
    }
}

/// Maps an authored sleep policy onto Rapier's activation thresholds.
///
/// Every policy other than `SolverVelocityThresholds` is driven by this crate
/// instead, so Rapier's own velocity timer is disabled for it.
const fn configure_sleep_activation(
    activation: &mut RigidBodyActivation,
    configuration: RigidBodyConfiguration,
) {
    match configuration.sleep_policy {
        RigidBodySleepPolicy::SolverVelocityThresholds => {
            activation.normalized_linear_threshold = configuration.sleep_linear_velocity_threshold;
            activation.angular_threshold = configuration.sleep_angular_velocity_threshold;
            activation.time_until_sleep = configuration.sleep_duration;
        }
        RigidBodySleepPolicy::CryEnergy
        | RigidBodySleepPolicy::External
        | RigidBodySleepPolicy::RockNRoll(_) => {
            activation.normalized_linear_threshold = 0.0;
            activation.angular_threshold = 0.0;
            activation.time_until_sleep = f32::MAX;
        }
    }
}

/// Rapier body builder for one authored [`RigidBodyMotion`].
fn rigid_body_motion_builder(motion: RigidBodyMotion) -> RigidBodyBuilder {
    match motion {
        RigidBodyMotion::Dynamic => RigidBodyBuilder::dynamic(),
        RigidBodyMotion::KinematicPosition => RigidBodyBuilder::kinematic_position_based(),
        RigidBodyMotion::KinematicVelocity => RigidBodyBuilder::kinematic_velocity_based(),
    }
}

fn sorted_body_handles(
    handles: impl IntoIterator<Item = PhysicsBodyHandle>,
) -> Vec<PhysicsBodyHandle> {
    let mut handles = handles.into_iter().collect::<Vec<_>>();
    handles.sort_unstable();
    handles
}

fn cry_vehicle_stiffness(
    configuration: &WheeledVehicleConfiguration,
    effective_masses: &[f32],
    local_center_of_mass: Vector,
    gravity: Vec3,
    mass: f32,
) -> Vec<f32> {
    let center = convert::vec3(local_center_of_mass);
    let mut force = [0.0_f32; 2];
    let mut torque = [0.0_f32; 2];
    let mut sides = Vec::with_capacity(configuration.wheels.len());
    for wheel in &configuration.wheels {
        let side = usize::from(wheel.connection.y - center.y >= 0.0);
        sides.push(side);
        if wheel.axle >= 0 {
            let weight = wheel.stiffness_weight.max(0.0);
            force[side] += weight;
            torque[side] = (wheel.connection.y - center.y).mul_add(weight, torque[side]);
        }
    }
    let denominator = torque[0].mul_add(-force[1], torque[1] * force[0]);
    let weight_force = mass * (-gravity.z).max(0.0);
    let mut stiffness = vec![0.0; configuration.wheels.len()];
    if denominator > 1.0e-4 {
        let scales = [
            weight_force * torque[1] / denominator,
            -weight_force * torque[0] / denominator,
        ];
        for ((value, wheel), side) in stiffness.iter_mut().zip(&configuration.wheels).zip(sides) {
            *value = if wheel.stiffness_weight > 0.0 {
                scales[side] * wheel.stiffness_weight
            } else {
                -weight_force * wheel.stiffness_weight / f32_from_usize(configuration.wheels.len())
            };
        }
    } else {
        for (value, effective_mass) in stiffness.iter_mut().zip(effective_masses) {
            *value = effective_mass * (-gravity.z).max(0.0);
        }
    }
    for (value, wheel) in stiffness.iter_mut().zip(&configuration.wheels) {
        if wheel.suspension_max_length > 0.0 {
            *value /= wheel.suspension_max_length - wheel.suspension_initial_length;
        } else {
            *value = 0.0;
        }
    }
    stiffness
}

/// Names a rigid-body-only action for the error a particle, rope, or soft body
/// raises when it is asked to perform one.
const fn rigid_only_action_name(action: PhysicsAction) -> &'static str {
    match action {
        PhysicsAction::Force(_) => "force",
        PhysicsAction::Torque(_) => "torque",
        PhysicsAction::SetDensity(_) => "set_density",
        PhysicsAction::SetLinearDamping(_) => "set_linear_damping",
        PhysicsAction::SetAngularDamping(_) => "set_angular_damping",
        PhysicsAction::SetSleepMinEnergy(_) => "set_sleep_min_energy",
        PhysicsAction::SetBuoyancy(_) => "set_buoyancy",
        _ => "unsupported action",
    }
}

#[inline]
fn rpm_to_angular(rpm: f32) -> f32 {
    rpm * (core::f32::consts::TAU / 60.0)
}

#[inline]
fn angular_to_rpm(speed: f32) -> f32 {
    speed * (60.0 / core::f32::consts::TAU)
}

#[inline]
fn sign_nonzero(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

#[inline]
fn sign_zero(value: f32) -> f32 {
    if value < 0.0 {
        -1.0
    } else if value > 0.0 {
        1.0
    } else {
        0.0
    }
}

#[inline]
fn directional_inverse_mass(body: &RigidBody, point: Vector, direction: Vector) -> f32 {
    if !body.is_dynamic() || direction.length_squared() <= f32::EPSILON {
        return 0.0;
    }
    let mass_properties = body.mass_properties();
    let relative_point = point - mass_properties.world_com;
    let angular_axis = relative_point.cross(direction);
    direction.dot(mass_properties.effective_inv_mass * direction)
        + angular_axis.dot(mass_properties.effective_world_inv_inertia * angular_axis)
}

fn particle_lift(heading: Vec3, gravity: Vec3) -> Vec3 {
    heading.cross(heading.cross(gravity)).normalize_or_zero()
}

fn align_axis(orientation: Quat, local_axis: Vec3, world_axis: Vec3) -> Quat {
    let current = orientation * local_axis;
    if current.length_squared() <= f32::EPSILON || world_axis.length_squared() <= f32::EPSILON {
        orientation
    } else {
        (Quat::from_rotation_arc(current.normalize(), world_axis.normalize()) * orientation)
            .normalize()
    }
}

fn integrate_orientation(orientation: Quat, angular_velocity: Vec3, time_step: f32) -> Quat {
    let speed = angular_velocity.length();
    if speed * time_step <= f32::EPSILON {
        orientation
    } else {
        (Quat::from_axis_angle(angular_velocity / speed, speed * time_step) * orientation)
            .normalize()
    }
}

fn path_aligned_orientation(gravity: Vec3, heading: Vec3) -> Quat {
    if heading.length_squared() <= f32::EPSILON {
        return Quat::IDENTITY;
    }
    let heading = heading.normalize();
    let mut side = gravity.normalize_or_zero().cross(heading);
    if side.length_squared() < 0.01 {
        side = if heading.x.abs() < 0.9 {
            Vec3::X.cross(heading)
        } else {
            Vec3::Z.cross(heading)
        };
    }
    side = side.normalize();
    let up = side.cross(heading).normalize();
    Quat::from_mat3(&Mat3::from_cols(side, heading, up)).normalize()
}

fn reverse_displacement_broadphase_aabb(
    collider: &Collider,
    displacement: Vector,
    margin: f32,
) -> Aabb {
    let mut proxy = collider.compute_aabb();
    let displaced = proxy.translated(displacement);
    proxy.merge(&displaced);
    proxy.loosened(margin)
}

const CHARACTER_SUPPORT_PROBE_TIME: f32 = 1.0 / 60.0;
const CHARACTER_PLANE_SOLVER_EPSILON: f32 = 0.01;
const CHARACTER_PLANE_SOLVER_PASSES: usize = 20;

#[inline]
fn damp_linear_step(
    mut velocity: Vector,
    coefficient: f32,
    low_speed_decrement: f32,
    time_step: f32,
) -> Vector {
    velocity *= coefficient.mul_add(-time_step, 1.0).clamp(0.0, 1.0);
    let speed_squared = velocity.length_squared();
    if speed_squared < coefficient * coefficient {
        if speed_squared <= low_speed_decrement * low_speed_decrement {
            Vector::ZERO
        } else {
            velocity - velocity.normalize() * low_speed_decrement
        }
    } else {
        velocity
    }
}

/// Native `RockNRoll` contact-plane projection.
fn project_character_velocity(
    requested_velocity: Vec3,
    contacts: &[CharacterContactPlane],
) -> Vec3 {
    let mut projected = requested_velocity;
    let mut correction = Vec3::ZERO;
    let mut converged = contacts.is_empty();

    for _ in 0..CHARACTER_PLANE_SOLVER_PASSES {
        let mut maximum_correction = 0.0_f32;
        for contact in contacts {
            let normal = contact.normal;
            let remove = (-projected.dot(normal)).max(0.0);
            let correct =
                (-(projected + correction).dot(normal) + contact.velocity.dot(normal)).max(0.0);
            projected += remove * normal;
            correction += (correct - remove) * normal;
            maximum_correction = maximum_correction.max(correct);
            if projected.dot(requested_velocity) < 0.0 {
                projected = Vec3::ZERO;
            }
        }
        if maximum_correction <= CHARACTER_PLANE_SOLVER_EPSILON {
            converged = true;
            break;
        }
    }

    if !converged {
        projected *= 0.5;
        correction *= 0.5;
    }
    projected + correction
}

fn aggregate_character_support(
    configuration: &CharacterBodyConfiguration,
    contacts: &[CharacterContactPlane],
) -> CharacterSupportInfo {
    let minimum_support_dot = configuration.max_slope.cos();
    let mut normal_sum = Vec3::ZERO;
    let mut velocity_sum = Vec3::ZERO;
    let mut support_count = 0_u32;
    let mut closest: Option<CharacterContactPlane> = None;
    let mut state = CharacterSupportState::Unsupported;

    for contact in contacts {
        let support_dot = contact.normal.dot(configuration.up_direction);
        if support_dot <= 0.0 {
            continue;
        }
        state = if support_dot + f32::EPSILON >= minimum_support_dot {
            CharacterSupportState::Supported
        } else if state != CharacterSupportState::Supported {
            CharacterSupportState::Sliding
        } else {
            state
        };
        normal_sum += contact.normal;
        velocity_sum += contact.velocity;
        support_count += 1;
        if closest.is_none_or(|current| contact.distance < current.distance) {
            closest = Some(*contact);
        }
    }

    let Some(closest) = closest else {
        return CharacterSupportInfo::default();
    };
    CharacterSupportInfo {
        state,
        normal: normal_sum.normalize_or_zero(),
        velocity: velocity_sum / convert::f32_from_u32(support_count),
        distance: closest.distance,
        body: Some(closest.body),
        entity_id: closest.entity_id,
    }
}

fn living_collider_geometry(
    dimensions: LivingDimensions,
) -> Result<(SharedShape, Pose), PhysicsError> {
    let shape = if dimensions.use_capsule {
        ColliderShape::Capsule {
            axis: Axis3::Z,
            half_height: dimensions.collider_half_height,
            radius: dimensions.collider_radius,
        }
    } else {
        ColliderShape::Cylinder {
            axis: Axis3::Z,
            half_height: dimensions.collider_half_height,
            radius: dimensions.collider_radius,
        }
    };
    let (builder, shape_pose) = convert::collider(&shape)?;
    let local_pose = PhysicsPose {
        translation: Vec3::Z * (dimensions.height_collider - dimensions.height_pivot),
        rotation: glam::Quat::IDENTITY,
    } * shape_pose;
    Ok((builder.shape, convert::pose(local_pose)))
}

/// Whether the authored collider geometry differs at all between two stances.
///
/// This is change detection over values the caller stored earlier, not a
/// numeric comparison, so the fields are compared as bit patterns: any edit to
/// an authored dimension, however small, has to rebuild the collider.
const fn living_geometry_changed(current: LivingDimensions, next: LivingDimensions) -> bool {
    current.use_capsule != next.use_capsule
        || current.collider_radius.to_bits() != next.collider_radius.to_bits()
        || current.collider_half_height.to_bits() != next.collider_half_height.to_bits()
        || current.height_collider.to_bits() != next.height_collider.to_bits()
        || current.height_pivot.to_bits() != next.height_pivot.to_bits()
}

fn aabbs_intersect(left: Aabb3d, right: Aabb3d) -> bool {
    left.min.x <= right.max.x
        && left.max.x >= right.min.x
        && left.min.y <= right.max.y
        && left.max.y >= right.min.y
        && left.min.z <= right.max.z
        && left.max.z >= right.min.z
}

fn apply_living_move(state: &mut LivingState, action: LivingMoveAction) {
    match action.mode {
        LivingMoveMode::RequestedVelocity => state.requested_velocity = action.velocity,
        LivingMoveMode::ForceFlight => {
            state.requested_velocity = action.velocity;
            state.force_flight = true;
        }
        LivingMoveMode::AddVelocity => {
            state.velocity += action.velocity;
            state.unconstrained_velocity = state.velocity;
            state.force_flight = false;
            if action.velocity.z > 1.0 {
                state.time_flying = 0.0;
                state.jump_requested = true;
            }
        }
        LivingMoveMode::SetVelocity => {
            state.velocity = action.velocity;
            state.unconstrained_velocity = action.velocity;
            state.force_flight = false;
            state.time_flying = 0.0;
            state.jump_requested = true;
        }
    }
    state.requested_time_step = (action.time_step > 0.0).then_some(action.time_step);
}

fn apply_living_impulse(state: &mut LivingState, action: ImpulseAction) {
    let impulse = if action.explosion {
        action.impulse * 0.3
    } else {
        action.impulse
    };
    let velocity_delta = impulse / state.configuration.dynamics.mass;
    state.velocity += velocity_delta;
    state.unconstrained_velocity = state.velocity;
    if velocity_delta.z > 1.0 {
        state.flying = true;
        state.time_flying = 0.0;
    }
    if state.configuration.dynamics.inertia == 0.0 {
        state.time_force_inertia = state.configuration.dynamics.time_impulse_recover;
    }
}

fn integrate_living_velocity(state: &LivingState, gravity: Vec3, time_step: f32) -> Vec3 {
    let dynamics = state.configuration.dynamics;
    let inertia = selected_living_inertia(state);
    let mut velocity = state.velocity;
    if !state.flying && !state.jump_requested {
        let requested = state.requested_velocity
            - state.ground_slope * state.ground_slope.dot(state.requested_velocity);
        let effective_inertia = inertia.min(1.0 / time_step);
        if inertia == 0.0 {
            velocity = requested;
        }
        let mut acceleration = (requested - velocity) * effective_inertia;
        let slope_dot = state.ground_slope.z;
        if slope_dot < dynamics.min_slide_angle.to_radians().cos() && !dynamics.is_swimming {
            acceleration += gravity - state.ground_slope * gravity.dot(state.ground_slope);
        }
        let next_velocity = velocity + acceleration * time_step;
        velocity = if next_velocity.dot(velocity) < 0.0
            && next_velocity.dot(state.requested_velocity) <= 0.0
        {
            Vec3::ZERO
        } else {
            next_velocity
        };
        if slope_dot < dynamics.max_climb_angle.to_radians().cos()
            && velocity.z > 0.0
            && acceleration.z > 0.0
        {
            velocity.z = 0.0;
        }
    }
    if state.flying && dynamics.air_control > 0.0 {
        if dynamics.inertia > 0.0 {
            let difference = state.requested_velocity - velocity;
            let mut delta =
                state.requested_velocity * (dynamics.inertia * time_step * dynamics.air_control);
            for axis in 0..3 {
                if difference[axis] * delta[axis] < 0.0 {
                    delta[axis] = 0.0;
                }
                delta[axis] = if difference[axis] >= 0.0 {
                    delta[axis].min(difference[axis])
                } else {
                    delta[axis].max(difference[axis])
                };
            }
            if dynamics.air_control >= 1.0 {
                delta.x = difference.x;
                delta.y = difference.y;
            }
            velocity += delta;
        } else if gravity.length_squared() > 0.0 {
            velocity = gravity
                * ((velocity.dot(gravity) - state.requested_velocity.dot(gravity))
                    / gravity.length_squared())
                + state.requested_velocity;
        } else {
            velocity = state.requested_velocity;
        }
    }
    if state.force_flight {
        velocity = state.requested_velocity;
    }
    velocity
}

fn selected_living_inertia(state: &LivingState) -> f32 {
    let dynamics = state.configuration.dynamics;
    if state.time_force_inertia > 0.0001 {
        6.0
    } else if dynamics.inertia_acceleration > 0.0 && state.requested_velocity.length_squared() > 0.1
    {
        dynamics.inertia_acceleration
    } else {
        dynamics.inertia
    }
}

fn apply_living_air_resistance(velocity: Vec3, resistance: f32, time_step: f32) -> Vec3 {
    let drag = -velocity * resistance * time_step;
    if drag.length_squared() < velocity.length_squared() * 4.0 {
        velocity + drag
    } else {
        velocity
    }
}

fn clamp_length(value: Vec3, maximum: f32) -> Vec3 {
    if maximum <= 0.0 {
        Vec3::ZERO
    } else if value.length_squared() > maximum * maximum {
        value.normalize_or_zero() * maximum
    } else {
        value
    }
}

fn update_living_camera_offset(
    state: &mut LivingState,
    was_flying: bool,
    landing_velocity: f32,
    vertical_step: f32,
    time_step: f32,
) {
    let dimensions = state.configuration.dimensions;
    let dynamics = state.configuration.dynamics;
    if was_flying && !state.flying && landing_velocity < -4.0 && dynamics.nod_speed > 0.0 {
        state.camera_offset_speed = landing_velocity;
        state.camera_offset_acceleration = (landing_velocity * landing_velocity
            / (dimensions.collider_half_height * 2.0))
            .max(dynamics.nod_speed);
    }

    if !state.flying && vertical_step > dimensions.collider_half_height * 0.01 {
        state.camera_offset_speed = state
            .camera_offset_speed
            .max(vertical_step / state.stable_height_time.max(f32::EPSILON));
        state.camera_vertical_offset += vertical_step;
        state.stable_height_time = 0.0;
    }
    state.stable_height_time = (state.stable_height_time + time_step).min(0.5);

    state.camera_offset_speed = state
        .camera_offset_acceleration
        .mul_add(time_step, state.camera_offset_speed);
    let crosses_zero = state.camera_vertical_offset
        * state
            .camera_offset_speed
            .mul_add(-time_step, state.camera_vertical_offset)
        < 0.0;
    if (state.camera_offset_acceleration == 0.0
        && state.camera_vertical_offset * state.camera_offset_speed < 0.0)
        || state.camera_vertical_offset * state.camera_offset_acceleration < 0.0
        || crosses_zero
    {
        state.camera_vertical_offset = 0.0;
        state.camera_offset_speed = 0.0;
        state.camera_offset_acceleration = 0.0;
    } else {
        state.camera_vertical_offset = state
            .camera_offset_speed
            .mul_add(-time_step, state.camera_vertical_offset);
    }
}

const fn living_controller(state: &LivingState) -> KinematicCharacterController {
    let configuration = &state.configuration;
    KinematicCharacterController {
        up: Vector::Z,
        offset: CharacterLength::Absolute(
            configuration.dimensions.ground_contact_epsilon.max(1.0e-4),
        ),
        slide: true,
        autostep: None,
        max_slope_climb_angle: configuration.dynamics.max_climb_angle.to_radians(),
        min_slope_slide_angle: configuration.dynamics.min_slide_angle.to_radians(),
        snap_to_ground: None,
        normal_nudge_factor: 1.0e-4,
    }
}

const fn living_foot_gap(dimensions: az_physics::LivingDimensions) -> f32 {
    let capsule_radius = if dimensions.use_capsule {
        dimensions.collider_radius
    } else {
        0.0
    };
    (dimensions.height_collider - dimensions.collider_half_height - capsule_radius).max(0.0)
}

const INTERACTS_WITH_TRIGGERS_BIT: u128 = 1 << 64;

fn encode_collision_filter(configuration: &ColliderConfiguration) -> u128 {
    let collision_class = configuration.collision_class;
    u128::from(collision_class.type_mask)
        | (u128::from(collision_class.ignore_mask) << 32)
        | if configuration.interacts_with_triggers {
            INTERACTS_WITH_TRIGGERS_BIT
        } else {
            0
        }
}

const fn decode_collision_class(value: u128) -> CollisionClass {
    let bytes = value.to_le_bytes();
    CollisionClass::new(
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    )
}

const fn interacts_with_triggers(value: u128) -> bool {
    (value & INTERACTS_WITH_TRIGGERS_BIT) != 0
}

#[inline]
fn speculative_contact_coefficients(
    mode_four_pair: bool,
    separation: f32,
    friction: f32,
    restitution: f32,
) -> (f32, f32) {
    if mode_four_pair && separation > 0.0 {
        (0.0, 0.0)
    } else {
        (friction, restitution)
    }
}

#[inline]
fn contact_generation_distance(left: ColliderMetadata, right: ColliderMetadata) -> f32 {
    left.positive_contact_range() + right.positive_contact_range()
}

#[inline]
fn apply_contact_offsets(
    geometric_separation: f32,
    left: ColliderMetadata,
    right: ColliderMetadata,
) -> Option<f32> {
    (geometric_separation <= contact_generation_distance(left, right))
        .then_some(geometric_separation - left.rest_offset - right.rest_offset)
}

struct CollisionHooks<'a> {
    metadata: &'a HashMap<ColliderHandle, ColliderMetadata>,
}

impl PhysicsHooks for CollisionHooks<'_> {
    fn filter_contact_pair(&self, context: &PairFilterContext<'_>) -> Option<SolverFlags> {
        let left_metadata = self.metadata.get(&context.collider1)?;
        let right_metadata = self.metadata.get(&context.collider2)?;
        if !left_metadata.blocks_motion() || !right_metadata.blocks_motion() {
            return None;
        }
        let left = decode_collision_class(context.colliders[context.collider1].user_data);
        let right = decode_collision_class(context.colliders[context.collider2].user_data);
        let category_pair = match (
            self.metadata
                .get(&context.collider1)
                .and_then(|metadata| metadata.collision_filter),
            self.metadata
                .get(&context.collider2)
                .and_then(|metadata| metadata.collision_filter),
        ) {
            (Some(left), Some(right)) => left.interacts_with(right),
            _ => true,
        };
        (left.interacts_with(right) && category_pair).then_some(SolverFlags::COMPUTE_IMPULSES)
    }

    fn filter_intersection_pair(&self, context: &PairFilterContext<'_>) -> bool {
        let Some(left_metadata) = self.metadata.get(&context.collider1) else {
            return false;
        };
        let Some(right_metadata) = self.metadata.get(&context.collider2) else {
            return false;
        };
        if !left_metadata.participates_in_trigger_pairs()
            || !right_metadata.participates_in_trigger_pairs()
        {
            return false;
        }
        let left_collider = &context.colliders[context.collider1];
        let right_collider = &context.colliders[context.collider2];
        let left_data = left_collider.user_data;
        let right_data = right_collider.user_data;
        if (left_collider.is_sensor() && !interacts_with_triggers(right_data))
            || (right_collider.is_sensor() && !interacts_with_triggers(left_data))
        {
            return false;
        }
        let category_pair = match (
            self.metadata
                .get(&context.collider1)
                .and_then(|metadata| metadata.collision_filter),
            self.metadata
                .get(&context.collider2)
                .and_then(|metadata| metadata.collision_filter),
        ) {
            (Some(left), Some(right)) => left.interacts_with(right),
            _ => true,
        };
        decode_collision_class(left_data).interacts_with(decode_collision_class(right_data))
            && category_pair
    }

    fn modify_solver_contacts(&self, context: &mut ContactModificationContext<'_>) {
        let Some(left) = self.metadata.get(&context.collider1).copied() else {
            return;
        };
        let Some(right) = self.metadata.get(&context.collider2).copied() else {
            return;
        };
        let mode_four_pair = left
            .continuous_collision_mode
            .uses_speculative_normal_constraints()
            || right
                .continuous_collision_mode
                .uses_speculative_normal_constraints();

        context.solver_contacts.retain_mut(|contact| {
            let Some(separation) = apply_contact_offsets(contact.dist, left, right) else {
                return false;
            };
            contact.dist = separation;
            (contact.friction, contact.restitution) = speculative_contact_coefficients(
                mode_four_pair,
                separation,
                contact.friction,
                contact.restitution,
            );
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_physics::{
        CharacterControllerCommands, CollisionCategoryMask, ContinuousCollisionMode, PhysicsScene,
        QueryBodyConfiguration, RopeAttachment, SpatialQueryFilter,
    };

    fn test_metadata(
        rest_offset: f32,
        contact_offset: f32,
        mode: ContinuousCollisionMode,
        continuous_prediction_distance: f32,
    ) -> ColliderMetadata {
        ColliderMetadata {
            body: PhysicsBodyHandle::new(NonZeroU64::new(1).unwrap()),
            entity_id: None,
            query_type: PhysicalEntityTypes::STATIC,
            surface_index: SurfaceIndex::default(),
            surface_pierceability: 0,
            sensor: false,
            simulated: true,
            in_scene_queries: true,
            tag: ColliderTag::NONE,
            rest_offset,
            contact_offset,
            collision_filter: None,
            continuous_collision_mode: mode,
            continuous_prediction_distance,
        }
    }

    fn ground() -> BodyDescriptor {
        BodyDescriptor {
            entity_id: Some(PhysicsEntityId(1)),
            pose: PhysicsPose {
                translation: Vec3::new(0.0, 0.0, -0.5),
                rotation: glam::Quat::IDENTITY,
            },
            kind: BodyKind::Static { terrain: true },
            colliders: vec![ColliderConfiguration {
                shape: ColliderShape::Cuboid {
                    half_extents: Vec3::new(10.0, 10.0, 0.5),
                },
                ..ColliderConfiguration::default()
            }],
        }
    }

    #[test]
    fn dynamic_body_falls_and_raycast_returns_engine_identity() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::default());
        scene.create_body(&ground()).expect("ground");
        let configuration = RigidBodyConfiguration {
            compute_mass: false,
            ..RigidBodyConfiguration::default()
        };
        let falling = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(2)),
                pose: PhysicsPose {
                    translation: Vec3::new(0.0, 0.0, 3.0),
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Rigid(configuration),
                colliders: vec![ColliderConfiguration::default()],
            })
            .expect("dynamic body");

        let before = scene
            .body_status(falling)
            .expect("status")
            .pose
            .translation
            .z;
        for _ in 0..30 {
            scene.step(1.0 / 60.0).expect("step");
        }
        let after = scene
            .body_status(falling)
            .expect("status")
            .pose
            .translation
            .z;
        assert!(after < before);

        let result = scene
            .ray_cast(&RayCastConfiguration {
                origin: Vec3::new(0.0, 0.0, 10.0),
                direction: -Vec3::Z,
                max_distance: 20.0,
                max_hits: 4,
                ..RayCastConfiguration::default()
            })
            .expect("ray cast");
        assert!(result.has_blocking_hit());
        assert!(matches!(
            result.blocking_hit().and_then(|hit| hit.entity_id),
            Some(PhysicsEntityId(1 | 2))
        ));
    }

    #[test]
    fn query_body_is_sensor_only_and_respects_sensor_and_collision_filters() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let candidate_class = CollisionClass::new(1 << 5, 0);
        let body = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(42)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Query(QueryBodyConfiguration::living()),
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Sphere { radius: 1.0 },
                    collision_class: candidate_class,
                    sensor: false,
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("query body");

        let native = scene.backend().native_body(body).expect("native body");
        let collider = scene.backend().rigid_bodies[native.rigid_body].colliders()[0];
        assert!(scene.backend().colliders[collider].is_sensor());

        let ray = |include_sensors, collision_class| RayCastConfiguration {
            origin: -Vec3::X * 5.0,
            direction: Vec3::X,
            max_distance: 10.0,
            include_sensors,
            collision_class,
            ..RayCastConfiguration::default()
        };
        assert_eq!(
            scene
                .ray_cast(&ray(false, None))
                .expect("sensor-excluding ray")
                .hit_count(),
            0
        );
        assert_eq!(
            scene
                .ray_cast(&ray(true, None))
                .expect("sensor ray")
                .blocking_hit()
                .map(|hit| hit.body),
            Some(body)
        );
        assert_eq!(
            scene
                .ray_cast(&ray(true, Some(CollisionClass::new(1 << 1, 1 << 5)),))
                .expect("collision-filtered ray")
                .hit_count(),
            0
        );
    }

    #[test]
    fn rock_n_roll_compiled_filters_gate_queries_and_contacts_with_either_direction_rule() {
        let category = |bit| {
            let mut mask = CollisionCategoryMask::EMPTY;
            assert!(mask.insert(bit));
            mask
        };
        let candidate = CollisionFilter::new(category(7), CollisionCategoryMask::EMPTY, 0);
        let accepts_candidate = CollisionFilter::new(CollisionCategoryMask::EMPTY, category(7), 0);
        let rejects_candidate = CollisionFilter::new(CollisionCategoryMask::EMPTY, category(8), 0);

        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let static_body = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(101)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Static { terrain: false },
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Sphere { radius: 1.0 },
                    collision_filter: Some(candidate),
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("filtered static body");
        let dynamic_body = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(102)),
                pose: PhysicsPose {
                    translation: Vec3::X * 0.5,
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    compute_mass: false,
                    can_sleep: false,
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Sphere { radius: 1.0 },
                    collision_filter: Some(accepts_candidate),
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("filtered dynamic body");
        let rejected_body = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(103)),
                pose: PhysicsPose {
                    translation: -Vec3::X * 0.5,
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    compute_mass: false,
                    can_sleep: false,
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Sphere { radius: 1.0 },
                    collision_filter: Some(rejects_candidate),
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("rejected dynamic body");

        let ray = |collision_filter| RayCastConfiguration {
            origin: -Vec3::X * 5.0,
            direction: Vec3::X,
            max_distance: 5.5,
            collision_filter: Some(collision_filter),
            ..RayCastConfiguration::default()
        };
        assert_eq!(
            scene
                .ray_cast(&ray(accepts_candidate))
                .expect("accepted filtered ray")
                .blocking_hit()
                .map(|hit| hit.body),
            Some(static_body)
        );
        assert_eq!(
            scene
                .ray_cast(&ray(rejects_candidate))
                .expect("rejected filtered ray")
                .hit_count(),
            0
        );

        scene.step(1.0 / 60.0).expect("filtered contact step");
        let interactions = scene.drain_interactions();
        assert!(interactions.iter().any(|interaction| {
            interaction.kind == PhysicsInteractionKind::Contact
                && [interaction.body_a, interaction.body_b]
                    .iter()
                    .all(|body| *body == static_body || *body == dynamic_body)
        }));
        assert!(interactions.iter().all(|interaction| {
            interaction.body_a != rejected_body && interaction.body_b != rejected_body
        }));
    }

    #[test]
    fn shape_cast_all_returns_each_query_body_in_distance_order() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let make_query_body = |entity_id, x| BodyDescriptor {
            entity_id: Some(PhysicsEntityId(entity_id)),
            pose: PhysicsPose {
                translation: Vec3::X * x,
                rotation: glam::Quat::IDENTITY,
            },
            kind: BodyKind::Query(QueryBodyConfiguration::living()),
            colliders: vec![ColliderConfiguration {
                shape: ColliderShape::Sphere { radius: 0.5 },
                ..ColliderConfiguration::default()
            }],
        };
        let near = scene.create_body(&make_query_body(1, 0.0)).unwrap();
        let far = scene.create_body(&make_query_body(2, 4.0)).unwrap();

        let hits = scene
            .cast_shape_all(&ShapeCastConfiguration {
                shape: ColliderShape::Sphere { radius: 0.25 },
                pose: PhysicsPose {
                    translation: -Vec3::X * 5.0,
                    rotation: glam::Quat::IDENTITY,
                },
                direction: Vec3::X,
                max_distance: 12.0,
                target_distance: 0.0,
                stop_at_penetration: true,
                filter: SpatialQueryFilter::default(),
                max_results: 2,
            })
            .expect("shape cast");

        assert_eq!(
            hits.iter().map(|hit| hit.body).collect::<Vec<_>>(),
            [near, far]
        );
        assert!(hits[0].distance < hits[1].distance);
    }

    #[test]
    fn fluid_area_applies_cry_buoyancy_and_publishes_submerged_state() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::default());
        let fluid = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(80)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::FluidArea(FluidAreaConfiguration {
                    density: 10.0,
                    resistance: 0.0,
                    ..FluidAreaConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Cuboid {
                        half_extents: Vec3::splat(10.0),
                    },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("fluid area");
        let body = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(81)),
                pose: PhysicsPose {
                    translation: -Vec3::Z * 0.25,
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    compute_mass: false,
                    mass: 1.0,
                    sleep_policy: RigidBodySleepPolicy::CryEnergy,
                    buoyancy: az_physics::RigidBodyBuoyancy {
                        density_scale: 1.0,
                        resistance_scale: 0.0,
                        damping: 0.0,
                    },
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Sphere { radius: 0.5 },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("floating body");

        let backend = scene.backend();
        assert_eq!(backend.fluid_areas.len(), 1);
        let fluid_native = backend.native_body(fluid).unwrap();
        let body_native = backend.native_body(body).unwrap();
        let rapier_body = backend.rigid_bodies.get(body_native.rigid_body).unwrap();
        assert!(rapier_body.is_dynamic());
        assert!(rapier_body.is_enabled());
        assert!(rapier_body.mass() > 0.0);
        assert_eq!(body_native.collider_configurations.len(), 1);
        assert!(body_native.rigid_configuration.is_some());
        let fluid_bounds = backend
            .body_aabb_by_native_handle(fluid_native.rigid_body)
            .unwrap();
        let body_bounds = backend
            .body_aabb_by_native_handle(body_native.rigid_body)
            .unwrap();
        assert!(
            aabbs_intersect(body_bounds, fluid_bounds),
            "body={body_bounds:?}, fluid={fluid_bounds:?}"
        );
        let geometry = crate::buoyancy::submerged_geometry(
            &body_native.collider_configurations[0].shape,
            PhysicsPose {
                translation: -Vec3::Z * 0.25,
                rotation: glam::Quat::IDENTITY,
            },
            FluidAreaConfiguration::default().plane,
        );
        assert!(geometry.volume > 0.0, "{geometry:?}");

        scene.step(1.0 / 60.0).expect("physics step");
        let status = scene.body_status(body).expect("body status");
        assert!(status.pose.translation.z > -0.25, "{status:?}");
        assert!(status.buoyancy_status.submerged_fraction > 0.5);
        assert!(status.buoyancy_status.floating);
    }

    #[test]
    fn cry_energy_sleep_requires_support_under_gravity() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::default());
        scene.create_body(&ground()).expect("ground");
        let body = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(82)),
                pose: PhysicsPose {
                    translation: Vec3::new(0.0, 0.0, 0.5),
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    compute_mass: false,
                    sleep_policy: RigidBodySleepPolicy::CryEnergy,
                    sleep_min_energy: 0.5,
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration::default()],
            })
            .expect("supported body");

        for _ in 0..10 {
            scene.step(1.0 / 60.0).expect("physics step");
        }
        assert!(!scene.body_status(body).expect("status").awake);
    }

    #[test]
    fn rock_n_roll_sleep_uses_the_native_zero_motion_bonus() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let body = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(83)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    compute_mass: false,
                    sleep_policy: RigidBodySleepPolicy::RockNRoll(
                        RockNRollSleepMode::SmoothedEnergy,
                    ),
                    sleep_duration: 0.05,
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration::default()],
            })
            .expect("RockNRoll body");

        scene.step(1.0 / 60.0).expect("physics step");
        assert!(!scene.body_status(body).expect("status").awake);
    }

    #[test]
    fn living_actions_preserve_cry_move_modes() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::default());
        let living = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(7)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Living(LivingBodyConfiguration::default()),
                colliders: Vec::new(),
            })
            .expect("living body");
        scene
            .apply_action(
                living,
                PhysicsAction::Move(LivingMoveAction {
                    velocity: Vec3::new(1.0, 2.0, 3.0),
                    mode: LivingMoveMode::SetVelocity,
                    time_step: 0.0,
                }),
            )
            .expect("set velocity");
        assert_eq!(
            scene.living_status(living).expect("status").velocity,
            Vec3::new(1.0, 2.0, 3.0)
        );
        scene
            .apply_action(
                living,
                PhysicsAction::Move(LivingMoveAction {
                    velocity: Vec3::Z,
                    mode: LivingMoveMode::AddVelocity,
                    time_step: 0.0,
                }),
            )
            .expect("add velocity");
        assert_eq!(
            scene.living_status(living).expect("status").velocity,
            Vec3::new(1.0, 2.0, 4.0)
        );
    }

    #[test]
    fn living_support_uses_the_cry_virtual_foot_gap() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::default());
        scene.create_body(&ground()).expect("ground");
        let living = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(8)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Living(LivingBodyConfiguration::default()),
                colliders: Vec::new(),
            })
            .expect("living body");

        scene.step(1.0 / 60.0).expect("physics step");

        let status = scene.living_status(living).expect("living status");
        assert!(!status.flying, "{status:?}");
        assert!(status.ground_height.abs() < 1.0e-4, "{status:?}");
        assert_eq!(status.ground_surface, Some(SurfaceIndex::default()));
        assert_eq!(status.ground_body, None, "static ground is not retained");
        let pose = scene.body_status(living).expect("body status").pose;
        assert!(pose.translation.z.abs() < 1.0e-4, "{pose:?}");
    }

    #[test]
    fn living_ground_velocity_is_clamped_and_reported() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::default());
        let platform = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(9)),
                pose: PhysicsPose {
                    translation: Vec3::new(0.0, 0.0, -0.5),
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    motion: RigidBodyMotion::KinematicVelocity,
                    initial_linear_velocity: Vec3::X * 20.0,
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Cuboid {
                        half_extents: Vec3::new(10.0, 10.0, 0.5),
                    },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("platform");
        let living = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(10)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Living(LivingBodyConfiguration::default()),
                colliders: Vec::new(),
            })
            .expect("living body");

        scene.step(1.0 / 60.0).expect("physics step");

        let status = scene.living_status(living).expect("living status");
        assert_eq!(status.ground_body, Some(platform));
        assert!((status.ground_velocity.length() - 10.0).abs() < 1.0e-4);
        assert_eq!(
            status.velocity,
            status.ground_velocity + status.unconstrained_velocity
        );
    }

    #[test]
    fn living_landing_nod_uses_the_native_threshold_and_acceleration() {
        let mut state = LivingState::new(
            LivingBodyConfiguration::default(),
            ColliderHandle::invalid(),
        );
        state.flying = false;

        update_living_camera_offset(&mut state, true, -6.0, 0.0, 1.0 / 60.0);

        assert!(state.camera_vertical_offset > 0.0);
        assert!(state.camera_offset_acceleration >= 60.0);
    }

    #[test]
    fn living_stance_query_and_dimension_change_use_native_unprojection_transaction() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(11)),
                pose: PhysicsPose {
                    translation: Vec3::Z * 2.2,
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Static { terrain: false },
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Cuboid {
                        half_extents: Vec3::new(2.0, 2.0, 0.2),
                    },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("ceiling");
        let living = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(12)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Living(LivingBodyConfiguration::default()),
                colliders: Vec::new(),
            })
            .expect("living body");
        let dimensions = LivingDimensions {
            collider_half_height: 1.0,
            unprojection_direction: Vec3::ZERO,
            max_unprojection: 0.2,
            ..LivingDimensions::default()
        };

        let check = scene
            .check_living_stance(living, dimensions)
            .expect("stance query");
        assert!(!check.allowed);
        assert!(check.unprojection.z < -0.09, "{check:?}");

        scene
            .apply_action(living, PhysicsAction::SetLivingDimensions(dimensions))
            .expect("dimension change");
        let status = scene.body_status(living).expect("body status");
        assert!(status.pose.translation.z < -0.1, "{status:?}");
        assert!(
            scene
                .living_status(living)
                .expect("living status")
                .time_since_stance_change
                .abs()
                <= f32::EPSILON,
            "a stance change resets the timer"
        );
    }

    #[test]
    fn living_dimension_change_rejects_an_unprojection_toward_the_obstruction() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        scene
            .create_body(&BodyDescriptor {
                entity_id: None,
                pose: PhysicsPose {
                    translation: Vec3::Z * 2.2,
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Static { terrain: false },
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Cuboid {
                        half_extents: Vec3::new(2.0, 2.0, 0.2),
                    },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("ceiling");
        let living = scene
            .create_body(&BodyDescriptor {
                entity_id: None,
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Living(LivingBodyConfiguration::default()),
                colliders: Vec::new(),
            })
            .expect("living body");
        let result = scene.apply_action(
            living,
            PhysicsAction::SetLivingDimensions(LivingDimensions {
                collider_half_height: 1.0,
                unprojection_direction: Vec3::Z,
                max_unprojection: 1.0,
                ..LivingDimensions::default()
            }),
        );

        assert!(matches!(
            result,
            Err(PhysicsError::LivingStanceBlocked { .. })
        ));
        assert_eq!(
            scene.body_status(living).expect("body status").pose,
            PhysicsPose::IDENTITY
        );
    }

    #[test]
    fn living_dynamics_change_updates_mass_surface_and_reactivation_state() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let living = scene
            .create_body(&BodyDescriptor {
                entity_id: None,
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Living(LivingBodyConfiguration::default()),
                colliders: Vec::new(),
            })
            .expect("living body");
        let inactive = LivingDynamics {
            mass: 40.0,
            surface_index: SurfaceIndex(7),
            is_active: false,
            ..LivingDynamics::default()
        };
        scene
            .apply_action(living, PhysicsAction::SetLivingDynamics(inactive))
            .expect("deactivate");
        assert!((scene.body_status(living).expect("status").mass - 40.0).abs() < 1.0e-5);
        let backend = scene.backend();
        let state = backend.living.get(&living).expect("living state");
        assert_eq!(
            backend
                .collider_metadata
                .get(&state.primary_collider)
                .expect("metadata")
                .surface_index,
            SurfaceIndex(7)
        );

        scene
            .apply_action(
                living,
                PhysicsAction::SetLivingDynamics(LivingDynamics {
                    is_active: true,
                    ..inactive
                }),
            )
            .expect("reactivate");
        assert!(scene.living_status(living).expect("living status").flying);
    }

    #[test]
    fn cry_collision_class_filter_is_not_rapier_group_semantics() {
        let left = CollisionClass::new(1 << 1, 1 << 5);
        let right = CollisionClass::new(1 << 5, 0);
        let encoded = encode_collision_filter(&ColliderConfiguration {
            collision_class: left,
            ..ColliderConfiguration::default()
        });
        assert!(decode_collision_class(encoded).ignores(right));
        assert!(interacts_with_triggers(encoded));
    }

    #[test]
    fn rock_n_roll_materials_use_multiplicative_combine_rules() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::default());
        scene.create_body(&ground()).expect("ground");

        let collider = scene
            .backend()
            .colliders
            .iter()
            .next()
            .map(|(_, collider)| collider)
            .expect("ground collider");

        assert_eq!(
            collider.friction_combine_rule(),
            CoefficientCombineRule::Multiply
        );
        assert_eq!(
            collider.restitution_combine_rule(),
            CoefficientCombineRule::Multiply
        );
    }

    #[test]
    fn linear_step_damping_consumes_gravity_and_force_once() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::X));
        let body = scene
            .create_body(&BodyDescriptor {
                entity_id: None,
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    initial_linear_velocity: Vec3::X * 0.02,
                    mass: 2.0,
                    linear_damping: 0.05,
                    angular_damping: 0.15,
                    damping_model: RigidBodyDampingModel::LinearStep {
                        low_speed_decrement: 0.005,
                    },
                    compute_mass: false,
                    can_sleep: false,
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration::default()],
            })
            .expect("linear-step body");
        scene
            .apply_action(body, PhysicsAction::Force(Vec3::X * 2.0))
            .expect("force");

        scene.step(0.1).expect("physics step");

        // v = (0.02 + (gravity 1 + force/mass 1) * 0.1) * (1 - 0.05 * 0.1)
        let expected = 0.22 * 0.995;
        let status = scene.body_status(body).expect("status");
        assert!((status.linear_velocity.x - expected).abs() < 1.0e-5);
        let backend = scene.backend();
        let native = backend.native_body(body).unwrap();
        assert_eq!(
            backend.rigid_bodies[native.rigid_body].user_force(),
            Vector::ZERO
        );
        assert!(
            backend.rigid_bodies[native.rigid_body]
                .gravity_scale()
                .abs()
                <= f32::EPSILON,
            "linear-step damping bodies integrate gravity themselves"
        );
    }

    #[test]
    fn continuous_modes_preserve_native_routing_and_event_cap() {
        let mut backend = RapierPhysicsBackend::default();
        assert_eq!(backend.integration_parameters.max_ccd_substeps, 20);

        for (mode, hard_ccd, soft_prediction) in [
            (ContinuousCollisionMode::Disabled, false, 0.0),
            (ContinuousCollisionMode::Mode1, false, 0.05),
            (ContinuousCollisionMode::Mode2, false, 0.05),
            (ContinuousCollisionMode::OrderedTimeOfImpact, true, 0.0),
            (
                ContinuousCollisionMode::ReverseDisplacementSweep,
                false,
                0.05,
            ),
        ] {
            let body = backend
                .create_body(&BodyDescriptor {
                    entity_id: None,
                    pose: PhysicsPose::IDENTITY,
                    kind: BodyKind::Rigid(RigidBodyConfiguration {
                        continuous_collision_mode: mode,
                        compute_mass: false,
                        ..RigidBodyConfiguration::default()
                    }),
                    colliders: vec![ColliderConfiguration::default()],
                })
                .expect("continuous body");
            let native = backend.native_body(body).unwrap();
            let rapier = &backend.rigid_bodies[native.rigid_body];
            assert_eq!(rapier.is_ccd_enabled(), hard_ccd, "mode {mode:?}");
            assert!(
                (rapier.soft_ccd_prediction() - soft_prediction).abs() <= f32::EPSILON,
                "mode {mode:?}"
            );
            assert_eq!(
                native
                    .rigid_configuration
                    .unwrap()
                    .continuous_collision_mode,
                mode
            );
        }
    }

    #[test]
    fn modes_one_and_two_project_a_swept_body_to_the_hit_pose() {
        for mode in [
            ContinuousCollisionMode::Mode1,
            ContinuousCollisionMode::Mode2,
        ] {
            let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
            scene
                .create_body(&BodyDescriptor {
                    entity_id: None,
                    pose: PhysicsPose::IDENTITY,
                    kind: BodyKind::Static { terrain: false },
                    colliders: vec![ColliderConfiguration {
                        shape: ColliderShape::Cuboid {
                            half_extents: Vec3::new(0.1, 2.0, 2.0),
                        },
                        ..ColliderConfiguration::default()
                    }],
                })
                .expect("static wall");
            let body = scene
                .create_body(&BodyDescriptor {
                    entity_id: None,
                    pose: PhysicsPose {
                        translation: Vec3::new(-0.42, 0.0, 0.0),
                        rotation: glam::Quat::IDENTITY,
                    },
                    kind: BodyKind::Rigid(RigidBodyConfiguration {
                        initial_linear_velocity: Vec3::X * 2.2,
                        linear_damping: 0.0,
                        angular_damping: 0.0,
                        gravity_enabled: false,
                        can_sleep: false,
                        continuous_collision_mode: mode,
                        compute_mass: false,
                        ..RigidBodyConfiguration::default()
                    }),
                    colliders: vec![ColliderConfiguration {
                        shape: ColliderShape::Sphere { radius: 0.25 },
                        ..ColliderConfiguration::default()
                    }],
                })
                .expect("continuous body");

            scene.step(0.1).expect("physics step");

            let status = scene.body_status(body).expect("body status");
            assert!(
                status.pose.translation.x < -0.3,
                "mode {mode:?}: {status:?}"
            );
            assert!(status.linear_velocity.x < 0.7, "mode {mode:?}: {status:?}");
        }
    }

    #[test]
    fn reverse_displacement_proxy_extends_forward_and_adds_native_margin() {
        let collider = ColliderBuilder::cuboid(1.0, 1.0, 1.0).build();
        let proxy = reverse_displacement_broadphase_aabb(
            &collider,
            Vector::new(2.0, 0.0, 0.0),
            RapierPhysicsBackend::ROCK_N_ROLL_BROADPHASE_MARGIN,
        );

        assert_eq!(proxy.mins, Vector::new(-1.05, -1.05, -1.05));
        assert_eq!(proxy.maxs, Vector::new(3.05, 1.05, 1.05));
    }

    #[test]
    fn mode_four_proxy_retains_the_next_step_contact_candidate() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let wall = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(96)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Static { terrain: false },
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Cuboid {
                        half_extents: Vec3::new(0.1, 2.0, 2.0),
                    },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("static wall");
        let body = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(97)),
                pose: PhysicsPose {
                    translation: Vec3::new(-1.1, 0.0, 0.0),
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    initial_linear_velocity: Vec3::X * 4.0,
                    linear_damping: 0.0,
                    angular_damping: 0.0,
                    gravity_enabled: false,
                    can_sleep: false,
                    continuous_collision_mode: ContinuousCollisionMode::ReverseDisplacementSweep,
                    compute_mass: false,
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Sphere { radius: 0.25 },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("mode-four body");

        scene.step(0.1).expect("proxy-producing step");
        assert!(scene.drain_interactions().is_empty());
        scene.step(0.1).expect("candidate-consuming step");

        let candidate_is_retained = {
            let backend = scene.backend();
            let body_native = backend.native_body(body).expect("body native handle");
            let wall_native = backend.native_body(wall).expect("wall native handle");
            let body_collider = backend.rigid_bodies[body_native.rigid_body].colliders()[0];
            let wall_collider = backend.rigid_bodies[wall_native.rigid_body].colliders()[0];
            backend
                .narrow_phase
                .contact_pair(body_collider, wall_collider)
                .is_some()
        };
        assert!(candidate_is_retained);

        scene.step(0.1).expect("speculative-contact step");
        let interactions = scene.drain_interactions();
        assert!(
            interactions.iter().any(|interaction| {
                interaction.kind == PhysicsInteractionKind::Contact
                    && [interaction.body_a, interaction.body_b].contains(&body)
                    && [interaction.body_a, interaction.body_b].contains(&wall)
            }),
            "{interactions:?}"
        );
        let status = scene.body_status(body).expect("body status");
        assert!(status.pose.translation.x < -0.2, "{status:?}");
        assert!(status.linear_velocity.x < 4.0, "{status:?}");
    }

    #[test]
    fn positive_mode_four_contacts_are_normal_only() {
        assert_eq!(
            speculative_contact_coefficients(true, 0.05, 0.7, 0.25),
            (0.0, 0.0)
        );
        assert_eq!(
            speculative_contact_coefficients(true, -0.01, 0.7, 0.25),
            (0.7, 0.25)
        );
        assert_eq!(
            speculative_contact_coefficients(false, 0.05, 0.7, 0.25),
            (0.7, 0.25)
        );
    }

    #[test]
    fn per_collider_contact_and_rest_offsets_are_applied_after_pair_pruning() {
        let left = test_metadata(0.01, 0.03, ContinuousCollisionMode::Disabled, 0.0);
        let right = test_metadata(-0.02, 0.04, ContinuousCollisionMode::Disabled, 0.0);

        let adjusted = apply_contact_offsets(0.06, left, right).unwrap();
        assert!((adjusted - 0.07).abs() <= f32::EPSILON);
        assert_eq!(apply_contact_offsets(0.08, left, right), None);
    }

    #[test]
    fn continuous_prediction_range_remains_distinct_from_lmbr_contact_offset() {
        let continuous = test_metadata(
            0.0,
            0.02,
            ContinuousCollisionMode::ReverseDisplacementSweep,
            0.05,
        );
        let ordinary = test_metadata(0.0, 0.02, ContinuousCollisionMode::Disabled, 0.05);

        assert!(
            (contact_generation_distance(continuous, ordinary) - 0.07).abs() <= 1.0e-6,
            "a continuous collider contributes its prediction distance"
        );
        assert!(
            (contact_generation_distance(ordinary, ordinary) - 0.04).abs() <= 1.0e-6,
            "an ordinary pair contributes only its contact offsets"
        );
    }

    #[test]
    fn scene_queries_honor_native_in_scene_queries_flag_and_return_tag() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let hidden = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(77)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Static { terrain: false },
                colliders: vec![ColliderConfiguration {
                    in_scene_queries: false,
                    tag: ColliderTag::from_name("hidden"),
                    ..ColliderConfiguration::default()
                }],
            })
            .unwrap();
        let visible_tag = ColliderTag::from_name("visible");
        scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(78)),
                pose: PhysicsPose {
                    translation: Vec3::X * 3.0,
                    rotation: glam::Quat::IDENTITY,
                },
                kind: BodyKind::Static { terrain: false },
                colliders: vec![ColliderConfiguration {
                    tag: visible_tag,
                    ..ColliderConfiguration::default()
                }],
            })
            .unwrap();

        let result = scene
            .ray_cast(&RayCastConfiguration {
                origin: Vec3::X * -2.0,
                direction: Vec3::X,
                max_distance: 10.0,
                max_hits: 4,
                ..RayCastConfiguration::default()
            })
            .unwrap();
        assert!(result.iter().all(|hit| hit.body != hidden));
        assert_eq!(result.blocking_hit().unwrap().collider_tag, visible_tag);
    }

    #[test]
    fn connected_bodies_includes_constraint_links_without_static_anchors() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let configuration = RigidBodyConfiguration {
            compute_mass: false,
            can_sleep: false,
            ..RigidBodyConfiguration::default()
        };
        let make_body = |entity_id, x| BodyDescriptor {
            entity_id: Some(PhysicsEntityId(entity_id)),
            pose: PhysicsPose {
                translation: Vec3::X * x,
                rotation: glam::Quat::IDENTITY,
            },
            kind: BodyKind::Rigid(configuration),
            colliders: vec![ColliderConfiguration::default()],
        };
        let left = scene.create_body(&make_body(98, 0.0)).unwrap();
        let right = scene.create_body(&make_body(99, 5.0)).unwrap();
        scene
            .create_constraint(&ConstraintDescriptor::fixed(left.into(), right))
            .unwrap();

        let mut connected = Vec::new();
        scene.connected_bodies(left, &mut connected).unwrap();
        assert_eq!(connected.first(), Some(&left));
        assert!(connected.contains(&right));
        assert_eq!(connected.len(), 2);
    }

    #[test]
    fn connected_bodies_excludes_disabled_constraint_links() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let configuration = RigidBodyConfiguration {
            compute_mass: false,
            can_sleep: false,
            ..RigidBodyConfiguration::default()
        };
        let make_body = |entity_id, x| BodyDescriptor {
            entity_id: Some(PhysicsEntityId(entity_id)),
            pose: PhysicsPose {
                translation: Vec3::X * x,
                rotation: glam::Quat::IDENTITY,
            },
            kind: BodyKind::Rigid(configuration),
            colliders: vec![ColliderConfiguration::default()],
        };
        let left = scene.create_body(&make_body(102, 0.0)).unwrap();
        let middle = scene.create_body(&make_body(103, 5.0)).unwrap();
        let right = scene.create_body(&make_body(104, 10.0)).unwrap();
        scene
            .create_constraint(&ConstraintDescriptor::fixed(left.into(), middle))
            .unwrap();
        let mut disabled = ConstraintDescriptor::fixed(middle.into(), right);
        disabled.enabled = false;
        scene.create_constraint(&disabled).unwrap();

        let mut connected = Vec::new();
        scene.connected_bodies(left, &mut connected).unwrap();
        assert_eq!(connected.first(), Some(&left));
        assert!(connected.contains(&middle));
        assert!(!connected.contains(&right));
        assert_eq!(connected.len(), 2);

        scene.connected_bodies(right, &mut connected).unwrap();
        assert_eq!(connected, [right]);
    }

    #[test]
    fn connected_bodies_returns_contact_island_without_static_supports() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let configuration = RigidBodyConfiguration {
            compute_mass: false,
            can_sleep: false,
            ..RigidBodyConfiguration::default()
        };
        let make_body = |entity_id, x| BodyDescriptor {
            entity_id: Some(PhysicsEntityId(entity_id)),
            pose: PhysicsPose {
                translation: Vec3::X * x,
                rotation: glam::Quat::IDENTITY,
            },
            kind: BodyKind::Rigid(configuration),
            colliders: vec![ColliderConfiguration::default()],
        };
        let left = scene.create_body(&make_body(100, 0.0)).unwrap();
        let right = scene.create_body(&make_body(101, 0.75)).unwrap();
        scene.create_body(&ground()).unwrap();
        scene.step(1.0 / 60.0).unwrap();

        let mut connected = Vec::new();
        scene.connected_bodies(left, &mut connected).unwrap();
        assert_eq!(connected.first(), Some(&left));
        assert!(connected.contains(&right));
        assert_eq!(connected.len(), 2);
    }

    #[test]
    fn native_character_plane_projection_preserves_surface_velocity() {
        let contact = CharacterContactPlane {
            normal: Vec3::Y,
            velocity: Vec3::Y * 2.0,
            point: Vec3::ZERO,
            distance: 0.0,
            body: PhysicsBodyHandle::new(NonZeroU64::new(1).unwrap()),
            entity_id: None,
            surface_index: SurfaceIndex::default(),
        };
        assert_eq!(
            project_character_velocity(-Vec3::Y, &[contact]),
            Vec3::Y * 2.0
        );
    }

    #[test]
    fn character_support_distinguishes_ground_from_sliding() {
        let configuration = CharacterBodyConfiguration {
            max_slope: 45.0_f32.to_radians(),
            ..CharacterBodyConfiguration::default()
        };
        let body = PhysicsBodyHandle::new(NonZeroU64::new(1).unwrap());
        let contact = |normal| CharacterContactPlane {
            normal,
            velocity: Vec3::ZERO,
            point: Vec3::ZERO,
            distance: 0.01,
            body,
            entity_id: Some(PhysicsEntityId(42)),
            surface_index: SurfaceIndex(7),
        };
        let supported = aggregate_character_support(&configuration, &[contact(Vec3::Y)]);
        assert_eq!(supported.state, CharacterSupportState::Supported);
        assert!(supported.is_on_ground());

        let steep = glam::Quat::from_rotation_z(60.0_f32.to_radians()) * Vec3::Y;
        let sliding = aggregate_character_support(&configuration, &[contact(steep)]);
        assert_eq!(sliding.state, CharacterSupportState::Sliding);
        assert!(!sliding.is_on_ground());
        assert_eq!(sliding.body, Some(body));

        assert_eq!(
            aggregate_character_support(&configuration, &[]).state,
            CharacterSupportState::Unsupported
        );
    }

    #[test]
    fn synchronous_character_moves_only_through_explicit_integrate() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let character = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(90)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Character(CharacterBodyConfiguration {
                    asynchronous: false,
                    ..CharacterBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Sphere { radius: 0.5 },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("character");
        scene
            .request_velocity(character, Vec3::X)
            .expect("request velocity");
        scene.step(0.25).expect("world step");
        assert_eq!(
            scene
                .body_status(character)
                .expect("status")
                .pose
                .translation,
            Vec3::ZERO
        );

        scene
            .integrate_character(character, 0.25)
            .expect("explicit integrate");
        scene.step(0.25).expect("apply kinematic target");
        assert!(
            scene
                .body_status(character)
                .expect("status")
                .pose
                .translation
                .x
                > 0.2
        );
    }

    #[test]
    fn character_contact_impulse_uses_point_velocity_and_angular_effective_mass() {
        let mut backend = RapierPhysicsBackend::new(Vec3::ZERO);
        let body = backend
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(91)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    compute_mass: false,
                    mass: 2.0,
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Cuboid {
                        half_extents: Vec3::splat(0.5),
                    },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("dynamic body");
        backend
            .apply_character_contact_impulse(
                Vec3::X * 4.0,
                CharacterContactPlane {
                    normal: -Vec3::X,
                    velocity: Vec3::ZERO,
                    point: Vec3::new(-0.5, 0.4, 0.0),
                    distance: 0.0,
                    body,
                    entity_id: Some(PhysicsEntityId(91)),
                    surface_index: SurfaceIndex::default(),
                },
            )
            .expect("contact impulse");
        let status = backend.body_status(body).expect("status");
        assert!(status.linear_velocity.x > 0.0, "{status:?}");
        assert!(status.angular_velocity.z < 0.0, "{status:?}");
    }

    #[test]
    fn deformable_contact_reaction_is_applied_at_the_contact_point() {
        let mut backend = RapierPhysicsBackend::new(Vec3::ZERO);
        let body = backend
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(94)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Rigid(RigidBodyConfiguration {
                    compute_mass: false,
                    mass: 2.0,
                    ..RigidBodyConfiguration::default()
                }),
                colliders: vec![ColliderConfiguration::default()],
            })
            .expect("dynamic body");

        backend
            .apply_deformable_reactions([DeformableReaction {
                body,
                point: Vec3::Y,
                impulse: Vec3::X,
            }])
            .expect("deformable reaction");

        let status = backend.body_status(body).expect("status");
        assert!(status.linear_velocity.x > 0.0, "{status:?}");
        assert!(status.angular_velocity.z < 0.0, "{status:?}");
    }

    #[test]
    fn rope_body_pins_world_attachment_and_reuses_status_buffers() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::new(0.0, 0.0, -9.81)));
        let rope = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(92)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Rope(az_physics::RopeBodyConfiguration {
                    points: vec![Vec3::ZERO, Vec3::X],
                    target_length: 1.0,
                    attachments: [
                        Some(RopeAttachment {
                            body: None,
                            part_id: -1,
                            point: Vec3::ZERO,
                            local: false,
                        }),
                        None,
                    ],
                    ..az_physics::RopeBodyConfiguration::default()
                }),
                colliders: Vec::new(),
            })
            .expect("rope");
        scene.step(1.0 / 60.0).expect("rope step");
        let mut status = RopeStatus::default();
        scene
            .write_rope_status(rope, &mut status)
            .expect("rope status");
        assert_eq!(status.points.len(), 2);
        assert_eq!(status.points[0], Vec3::ZERO);
        assert!(status.points[1].z < 0.0);
        let allocation = status.points.capacity();
        scene
            .write_rope_status(rope, &mut status)
            .expect("rope status reuse");
        assert_eq!(status.points.capacity(), allocation);
    }

    #[test]
    fn rope_subdivision_vertices_are_created_by_contacts() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(95)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Static { terrain: false },
                colliders: vec![ColliderConfiguration {
                    shape: ColliderShape::Cuboid {
                        half_extents: Vec3::new(0.1, 1.0, 1.0),
                    },
                    ..ColliderConfiguration::default()
                }],
            })
            .expect("rope obstacle");
        let rope = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(96)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Rope(az_physics::RopeBodyConfiguration {
                    points: vec![Vec3::NEG_X, Vec3::X],
                    target_length: 2.0,
                    damping: 0.0,
                    collision_distance: 0.05,
                    collision_types: PhysicalEntityTypes::STATIC,
                    maximum_subdivision_vertices: 3,
                    flags: RopeFlags::COLLIDES | RopeFlags::SUBDIVIDE_SEGMENTS,
                    ..az_physics::RopeBodyConfiguration::default()
                }),
                colliders: Vec::new(),
            })
            .expect("subdivided rope");

        scene.step(1.0 / 60.0).expect("rope step");

        let mut status = RopeStatus::default();
        scene
            .write_rope_status(rope, &mut status)
            .expect("rope status");
        assert!(status.subdivided_vertices.len() >= 3, "{status:?}");
        assert_eq!(status.subdivided_vertices.first(), status.points.first());
        assert_eq!(status.subdivided_vertices.last(), status.points.last());
    }

    #[test]
    fn rope_volumetric_pressure_detaches_the_nearest_endpoint() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let attachment = |point| RopeAttachment {
            body: None,
            part_id: -1,
            point,
            local: false,
        };
        let rope = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(97)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Rope(az_physics::RopeBodyConfiguration {
                    points: vec![Vec3::NEG_X, Vec3::X],
                    target_length: 2.0,
                    maximum_force: 0.01,
                    attachments: [Some(attachment(Vec3::NEG_X)), Some(attachment(Vec3::X))],
                    ..az_physics::RopeBodyConfiguration::default()
                }),
                colliders: Vec::new(),
            })
            .expect("pressure-tested rope");

        scene
            .apply_rope_volumetric_pressure(
                rope,
                RopeVolumetricPressure {
                    epicenter: Vec3::new(-0.9, 0.1, 0.0),
                    pressure_scale: 1.0,
                    minimum_radius: 0.01,
                },
            )
            .expect("volumetric pressure");

        let mut status = RopeStatus::default();
        scene
            .write_rope_status(rope, &mut status)
            .expect("rope status");
        assert!(status.torn);
    }

    #[test]
    fn soft_body_preserves_triangle_topology_and_publishes_normals() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let soft = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(93)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Soft(az_physics::SoftBodyConfiguration {
                    vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
                    triangles: vec![[0, 1, 2]],
                    gravity: Vec3::ZERO,
                    stiffness: -1.0,
                    flags: az_physics::SoftBodyFlags::NONE,
                    ..az_physics::SoftBodyConfiguration::default()
                }),
                colliders: Vec::new(),
            })
            .expect("soft body");
        scene.step(1.0 / 60.0).expect("soft-body step");
        let mut status = SoftBodyStatus::default();
        scene
            .write_soft_body_status(soft, &mut status)
            .expect("soft-body status");
        assert_eq!(status.triangles, vec![[0, 1, 2]]);
        assert_eq!(status.normals.len(), 3);
        assert!(status.normals.iter().all(|normal| normal.z > 0.99));
    }

    #[test]
    fn soft_body_rigid_core_is_a_colliding_native_body() {
        let mut scene = PhysicsScene::new(RapierPhysicsBackend::new(Vec3::ZERO));
        let body = scene
            .create_body(&BodyDescriptor {
                entity_id: Some(PhysicsEntityId(94)),
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Soft(az_physics::SoftBodyConfiguration {
                    vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z],
                    triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
                    tetrahedra: vec![[0, 1, 2, 3]],
                    rigid_core: Some(az_physics::SoftBodyRigidCoreConfiguration::new(
                        vec![0, 1, 2, 3],
                        ColliderShape::Cuboid {
                            half_extents: Vec3::splat(0.5),
                        },
                    )),
                    flags: az_physics::SoftBodyFlags::SKIP_LONGEST_EDGES
                        | az_physics::SoftBodyFlags::RIGID_CORE,
                    gravity: Vec3::ZERO,
                    ..az_physics::SoftBodyConfiguration::default()
                }),
                colliders: Vec::new(),
            })
            .expect("soft body with rigid core");

        let backend = scene.backend();
        let state = backend.soft_bodies.get(&body).expect("soft-body state");
        let (core_body, core_collider) = state.rigid_core_handles().expect("native rigid core");
        let native_core = backend
            .rigid_bodies
            .get(core_body)
            .expect("rigid-core body");
        assert!(native_core.is_dynamic());
        assert!((native_core.mass() - 1.0).abs() <= f32::EPSILON);
        let metadata = backend
            .collider_metadata
            .get(&core_collider)
            .expect("rigid-core collider metadata");
        assert!(metadata.simulated);
        assert!(!metadata.sensor);
        assert!(!metadata.in_scene_queries);
    }
}
