//! CryAnimation-compatible motion extraction, smoothing, and root-motion output.
//!
//! The math is renderer and physics-backend independent. Consumers implement
//! [`MotionParameterSink`] and apply [`RootMotionCommand`] to their animation
//! graph and character controller respectively.

use std::{
    borrow::Borrow,
    ops::{Add, Mul, Sub},
};

use bevy_math::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::mannequin::AnimationLane;

/// Motion-parameter ids consumed by character and blend-space animation.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MotionParameterId {
    TravelSpeed = 0,
    TurnSpeed = 1,
    TravelAngle = 2,
    TravelSlope = 3,
    TurnAngle = 4,
    TravelDistance = 5,
    StopLeg = 6,
    BlendWeight = 7,
    BlendWeight2 = 8,
    BlendWeight3 = 9,
    BlendWeight4 = 10,
    AimHorizontalNavigationSpeed = 11,
    AimHorizontalNavigationAngle = 12,
    DesiredFacing = 13,
    VelocityX = 14,
    VelocityY = 15,
    SlopeYaw = 16,
    SlopePitch = 17,
}

impl MotionParameterId {
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub const fn is_cyclic(self) -> bool {
        matches!(
            self,
            Self::TravelAngle | Self::TurnAngle | Self::DesiredFacing
        )
    }

    /// Resolves a blend-space dimension name to its runtime parameter id.
    ///
    /// Names and ids follow the runtime motion-parameter table. The first
    /// entries retain their `CryEngine` spellings for legacy blend-space input.
    #[must_use]
    pub fn from_cry_name(name: &str) -> Option<Self> {
        Some(match name {
            "MoveSpeed" => Self::TravelSpeed,
            "TurnSpeed" => Self::TurnSpeed,
            "TravelSlope" => Self::TravelSlope,
            "TravelAngle" => Self::TravelAngle,
            "TravelDist" => Self::TravelDistance,
            "TurnAngle" => Self::TurnAngle,
            "BlendWeight" => Self::BlendWeight,
            "VelocityX" => Self::VelocityX,
            "VelocityY" => Self::VelocityY,
            "SlopeYaw" => Self::SlopeYaw,
            "SlopePitch" => Self::SlopePitch,
            "BlendWeight2" => Self::BlendWeight2,
            "BlendWeight3" => Self::BlendWeight3,
            "BlendWeight4" => Self::BlendWeight4,
            "AimHorzNavSpeed" => Self::AimHorizontalNavigationSpeed,
            "AimHorzNavAngle" => Self::AimHorizontalNavigationAngle,
            "DesiredFacing" => Self::DesiredFacing,
            "StopLeg" => Self::StopLeg,
            _ => return None,
        })
    }
}

impl TryFrom<u8> for MotionParameterId {
    type Error = UnknownMotionParameterId;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::TravelSpeed),
            1 => Ok(Self::TurnSpeed),
            2 => Ok(Self::TravelAngle),
            3 => Ok(Self::TravelSlope),
            4 => Ok(Self::TurnAngle),
            5 => Ok(Self::TravelDistance),
            6 => Ok(Self::StopLeg),
            7 => Ok(Self::BlendWeight),
            8 => Ok(Self::BlendWeight2),
            9 => Ok(Self::BlendWeight3),
            10 => Ok(Self::BlendWeight4),
            11 => Ok(Self::AimHorizontalNavigationSpeed),
            12 => Ok(Self::AimHorizontalNavigationAngle),
            13 => Ok(Self::DesiredFacing),
            14 => Ok(Self::VelocityX),
            15 => Ok(Self::VelocityY),
            16 => Ok(Self::SlopeYaw),
            17 => Ok(Self::SlopePitch),
            value => Err(UnknownMotionParameterId(value)),
        }
    }
}

impl From<MotionParameterId> for u8 {
    fn from(value: MotionParameterId) -> Self {
        value as Self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unknown motion parameter id {0}")]
pub struct UnknownMotionParameterId(pub u8);

/// Character-relative translation and rotation produced by an animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RootMotionDelta {
    pub rotation: Quat,
    pub translation: Vec3,
}

impl Default for RootMotionDelta {
    fn default() -> Self {
        Self {
            rotation: Quat::IDENTITY,
            translation: Vec3::ZERO,
        }
    }
}

impl From<(Quat, Vec3)> for RootMotionDelta {
    fn from((rotation, translation): (Quat, Vec3)) -> Self {
        Self {
            rotation,
            translation,
        }
    }
}

/// Values extracted from one frame of character motion for blend-space input.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MotionParameters {
    pub signed_ground_angle: f32,
    pub relative_travel_delta: Vec2,
    pub travel_distance: f32,
    pub travel_speed: f32,
    pub turn_angle: f32,
    pub turn_speed: f32,
}

impl MotionParameters {
    #[must_use]
    pub fn travel_angle(self) -> f32 {
        if is_zero_vec2(self.relative_travel_delta, 0.0) {
            0.0
        } else {
            (-self.relative_travel_delta.x).atan2(self.relative_travel_delta.y)
        }
    }
}

/// Persistent velocity state used by Cry's critically damped smoothing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MotionParameterSmoothingState {
    pub velocity: MotionParameters,
}

/// Settings needed by the motion extractor.
pub trait MotionExtractionSettings {
    fn movement_speed_epsilon(&self) -> f32;
}

/// Settings needed by the motion-parameter smoother and final output stage.
pub trait MotionSmoothingSettings {
    fn ground_angle_converge_time(&self) -> f32;
    fn travel_angle_converge_time(&self) -> f32;
    fn travel_distance_converge_time(&self) -> f32;
    fn travel_speed_converge_time(&self) -> f32;
    fn turn_angle_converge_time(&self) -> f32;
    fn turn_speed_converge_time(&self) -> f32;
    fn turn_speed_scale(&self) -> f32;
}

/// Receives final desired parameters from the character animation manager.
pub trait MotionParameterSink {
    fn set_desired_motion_parameter(
        &mut self,
        parameter: MotionParameterId,
        value: f32,
        delta_time: f32,
    );
}

/// One-frame request issued by Mannequin's `AnimationDrivenMotion` clip.
///
/// Cry's procedural context defaults this request when it is not refreshed, so
/// the clip deliberately sends the same value on enter and every active update
/// and has no exit call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationDrivenMotionRequest {
    pub translation_multiplier: f32,
    pub rotation_multiplier: f32,
}

impl Default for AnimationDrivenMotionRequest {
    fn default() -> Self {
        Self {
            translation_multiplier: 1.0,
            rotation_multiplier: 1.0,
        }
    }
}

/// Narrow compatibility surface implemented by an animated-character backend.
pub trait AnimationDrivenMotionSink {
    fn request_animation_driven_motion(
        &mut self,
        lane: AnimationLane,
        request: AnimationDrivenMotionRequest,
    );
}

/// Installed state for the Cry `AnimationDrivenMotion` procedural clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationDrivenMotionState {
    lane: AnimationLane,
    request: AnimationDrivenMotionRequest,
}

impl AnimationDrivenMotionState {
    #[must_use]
    pub fn enter(
        sink: &mut impl AnimationDrivenMotionSink,
        lane: AnimationLane,
        request: impl Into<AnimationDrivenMotionRequest>,
    ) -> Self {
        let request = request.into();
        sink.request_animation_driven_motion(lane, request);
        Self { lane, request }
    }

    pub fn update(&self, sink: &mut impl AnimationDrivenMotionSink) {
        sink.request_animation_driven_motion(self.lane, self.request);
    }

    #[must_use]
    pub const fn lane(&self) -> AnimationLane {
        self.lane
    }

    #[must_use]
    pub const fn request(&self) -> AnimationDrivenMotionRequest {
        self.request
    }
}

/// Sampling frequency used by Cry's authored exponential motion decay.
pub const EXPONENTIAL_MOTION_STEPS_PER_SECOND: f32 = 30.0;

/// A total translation distributed over animation time by an authored decay
/// factor. The value is stateless so rollback and prediction can sample any two
/// times without advancing hidden mutable state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExponentialDisplacement {
    total: Vec3,
    decay_per_step: f32,
}

impl ExponentialDisplacement {
    #[must_use]
    pub fn new(total: impl Into<Vec3>, decay_per_step: f32) -> Self {
        Self {
            total: total.into(),
            decay_per_step,
        }
    }

    #[must_use]
    pub const fn total(self) -> Vec3 {
        self.total
    }

    #[must_use]
    pub const fn decay_per_step(self) -> f32 {
        self.decay_per_step
    }

    /// Returns cumulative translation at `time_seconds`.
    ///
    /// A zero-duration animation and decay factors at zero or one use the
    /// complete translation immediately. Those are explicit branches in the
    /// source runtime rather than limiting cases of the exponential curve.
    #[must_use]
    pub fn cumulative(self, time_seconds: f32, duration_seconds: f32) -> Vec3 {
        if self.total.length_squared() <= f32::EPSILON {
            return Vec3::ZERO;
        }
        if duration_seconds == 0.0
            || self.decay_per_step <= f32::EPSILON
            || (self.decay_per_step - 1.0).abs() <= f32::EPSILON
        {
            return if time_seconds > 0.0 {
                self.total
            } else {
                Vec3::ZERO
            };
        }

        let time = time_seconds.clamp(0.0, duration_seconds.max(0.0));
        self.total
            * (1.0
                - self
                    .decay_per_step
                    .powf(time * EXPONENTIAL_MOTION_STEPS_PER_SECOND))
    }

    /// Translation emitted between two animation times.
    #[must_use]
    pub fn delta(
        self,
        previous_time_seconds: f32,
        next_time_seconds: f32,
        duration_seconds: f32,
    ) -> Vec3 {
        self.cumulative(next_time_seconds, duration_seconds)
            - self.cumulative(previous_time_seconds, duration_seconds)
    }
}

/// Converts a local `(right, forward, up)` translation into world axes using
/// a planar forward direction.
///
/// A degenerate direction selects world +Y, making the transform the identity
/// for the standard Z-up basis.
#[must_use]
pub fn planar_basis_translation(
    local_translation: impl Into<Vec3>,
    planar_forward: impl Into<Vec3>,
) -> Vec3 {
    let local_translation = local_translation.into();
    let mut forward = planar_forward.into();
    forward.z = 0.0;
    forward = forward.try_normalize().unwrap_or(Vec3::Y);
    let right = Vec3::new(forward.y, -forward.x, 0.0);
    right * local_translation.x + forward * local_translation.y + Vec3::Z * local_translation.z
}

/// Extract the motion values consumed by parametric animations.
///
/// `entity_rotation` is the current world rotation, `relative_motion` is the
/// character-local frame delta, and `world_ground_normal` is the physics ground
/// contact normal. Movement at or below the configured speed epsilon is
/// suppressed independently of frame duration.
#[must_use]
pub fn extract_motion_parameters(
    delta_time: f32,
    entity_rotation: Quat,
    relative_motion: impl Borrow<RootMotionDelta>,
    world_ground_normal: Vec3,
    settings: &(impl MotionExtractionSettings + ?Sized),
) -> MotionParameters {
    let relative_motion = relative_motion.borrow();
    let movement_epsilon = settings.movement_speed_epsilon().max(0.0);

    let mut relative_translation = relative_motion.translation;
    relative_translation.z = 0.0;

    let world_translation = entity_rotation * relative_translation;
    let movement_direction = if is_zero_vec3(world_translation, movement_epsilon) {
        entity_rotation * Vec3::Y
    } else {
        world_translation
    };
    let movement_heading = (-movement_direction.x).atan2(movement_direction.y);
    let movement_local_ground_normal =
        Quat::from_rotation_z(-movement_heading) * world_ground_normal;
    let ground_angle = std::f32::consts::FRAC_PI_2 - movement_local_ground_normal.y.abs().acos();

    if !is_zero_vec3(relative_translation, 0.0) {
        let slope_scale = ground_angle.cos();
        if slope_scale.abs() > f32::EPSILON {
            relative_translation /= slope_scale;
        }
    }

    let frame_movement_epsilon = movement_epsilon * delta_time.max(0.0);
    if is_zero_vec3(relative_translation, frame_movement_epsilon) {
        relative_translation = Vec3::ZERO;
    }

    let signed_ground_angle = if movement_local_ground_normal.y > 0.0 {
        -ground_angle
    } else {
        ground_angle
    };
    let travel_distance = relative_translation.length();
    let turn_direction = relative_motion.rotation * Vec3::Y;
    let turn_angle = (-turn_direction.x).atan2(turn_direction.y);

    MotionParameters {
        signed_ground_angle,
        relative_travel_delta: Vec2::new(relative_translation.x, relative_translation.y),
        travel_distance,
        travel_speed: if delta_time > 0.0 {
            travel_distance / delta_time
        } else {
            0.0
        },
        turn_angle,
        turn_speed: if delta_time > 0.0 {
            turn_angle / delta_time
        } else {
            0.0
        },
    }
}

/// Apply Cry's `SmoothCD` critically damped spring to a scalar or vector.
#[expect(
    clippy::suboptimal_flops,
    reason = "bit-exact port of CryCommon SmoothCD; the decay polynomial must be evaluated \
              in the shipped operation order, and mul_add would change the rounding"
)]
#[expect(
    clippy::eq_op,
    reason = "`*velocity - *velocity` is Cry's zeroing idiom and the only way to produce a \
              zero of the generic `T`, which has no Default or ZERO bound"
)]
pub fn smooth_critically_damped<T>(
    value: &mut T,
    velocity: &mut T,
    delta_time: f32,
    target: T,
    smooth_time: f32,
) where
    T: Copy + Add<Output = T> + Sub<Output = T> + Mul<f32, Output = T>,
{
    if smooth_time > 0.0 {
        let omega = 2.0 / smooth_time;
        let x = omega * delta_time;
        let decay = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
        let change = *value - target;
        let temporary = (*velocity + change * omega) * delta_time;
        *velocity = (*velocity - temporary * omega) * decay;
        *value = target + (change + temporary) * decay;
    } else if delta_time > 0.0 {
        *velocity = (target - *value) * (1.0 / delta_time);
        *value = target;
    } else {
        *value = target;
        *velocity = *velocity - *velocity;
    }
}

/// Smooth all motion parameters using the six independently reflected times.
pub fn smooth_motion_parameters(
    smoothed: &mut MotionParameters,
    target: MotionParameters,
    state: &mut MotionParameterSmoothingState,
    settings: &(impl MotionSmoothingSettings + ?Sized),
    delta_time: f32,
) {
    smooth_critically_damped(
        &mut smoothed.signed_ground_angle,
        &mut state.velocity.signed_ground_angle,
        delta_time,
        target.signed_ground_angle,
        settings.ground_angle_converge_time(),
    );
    smooth_critically_damped(
        &mut smoothed.relative_travel_delta,
        &mut state.velocity.relative_travel_delta,
        delta_time,
        target.relative_travel_delta,
        settings.travel_angle_converge_time(),
    );
    smooth_critically_damped(
        &mut smoothed.travel_distance,
        &mut state.velocity.travel_distance,
        delta_time,
        target.travel_distance,
        settings.travel_distance_converge_time(),
    );
    smooth_critically_damped(
        &mut smoothed.travel_speed,
        &mut state.velocity.travel_speed,
        delta_time,
        target.travel_speed,
        settings.travel_speed_converge_time(),
    );
    smooth_critically_damped(
        &mut smoothed.turn_angle,
        &mut state.velocity.turn_angle,
        delta_time,
        target.turn_angle,
        settings.turn_angle_converge_time(),
    );
    smooth_critically_damped(
        &mut smoothed.turn_speed,
        &mut state.velocity.turn_speed,
        delta_time,
        target.turn_speed,
        settings.turn_speed_converge_time(),
    );
}

/// Write the six values in runtime character-manager order.
pub fn drive_motion_parameters(
    sink: &mut impl MotionParameterSink,
    smoothed: MotionParameters,
    settings: &(impl MotionSmoothingSettings + ?Sized),
    delta_time: f32,
) {
    sink.set_desired_motion_parameter(
        MotionParameterId::TravelAngle,
        smoothed.travel_angle(),
        delta_time,
    );
    sink.set_desired_motion_parameter(
        MotionParameterId::TravelSlope,
        smoothed.signed_ground_angle,
        delta_time,
    );
    sink.set_desired_motion_parameter(
        MotionParameterId::TravelDistance,
        smoothed.travel_distance,
        delta_time,
    );
    sink.set_desired_motion_parameter(
        MotionParameterId::TravelSpeed,
        smoothed.travel_speed,
        delta_time,
    );
    sink.set_desired_motion_parameter(
        MotionParameterId::TurnAngle,
        smoothed.turn_angle,
        delta_time,
    );
    sink.set_desired_motion_parameter(
        MotionParameterId::TurnSpeed,
        smoothed.turn_speed * settings.turn_speed_scale(),
        delta_time,
    );
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RootMotionCommand {
    Apply {
        linear_velocity: Vec3,
        rotation_delta: Quat,
    },
    Stop,
    None,
}

/// Tracks the edge on which animation-driven motion stops.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RootMotionState {
    applied_last_frame: bool,
}

impl RootMotionState {
    #[must_use]
    pub const fn was_applied(&self) -> bool {
        self.applied_last_frame
    }

    /// Convert an animation delta into a backend-neutral character command.
    pub fn update(
        &mut self,
        enabled: bool,
        entity_rotation: Quat,
        relative_motion: impl Borrow<RootMotionDelta>,
        delta_time: f32,
    ) -> RootMotionCommand {
        if enabled {
            let relative_motion = relative_motion.borrow();
            let inverse_delta_time = if delta_time > 0.0 {
                1.0 / delta_time
            } else {
                0.0
            };
            self.applied_last_frame = true;
            RootMotionCommand::Apply {
                linear_velocity: entity_rotation
                    * (relative_motion.translation * inverse_delta_time),
                rotation_delta: relative_motion.rotation,
            }
        } else if std::mem::take(&mut self.applied_last_frame) {
            RootMotionCommand::Stop
        } else {
            RootMotionCommand::None
        }
    }
}

fn is_zero_vec2(value: Vec2, epsilon: f32) -> bool {
    value.x.abs() <= epsilon && value.y.abs() <= epsilon
}

fn is_zero_vec3(value: Vec3, epsilon: f32) -> bool {
    value.x.abs() <= epsilon && value.y.abs() <= epsilon && value.z.abs() <= epsilon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy)]
    struct Settings;

    impl MotionExtractionSettings for Settings {
        fn movement_speed_epsilon(&self) -> f32 {
            0.02
        }
    }

    impl MotionSmoothingSettings for Settings {
        fn ground_angle_converge_time(&self) -> f32 {
            0.1
        }
        fn travel_angle_converge_time(&self) -> f32 {
            0.1
        }
        fn travel_distance_converge_time(&self) -> f32 {
            0.1
        }
        fn travel_speed_converge_time(&self) -> f32 {
            0.1
        }
        fn turn_angle_converge_time(&self) -> f32 {
            0.75
        }
        fn turn_speed_converge_time(&self) -> f32 {
            0.75
        }
        fn turn_speed_scale(&self) -> f32 {
            2.0
        }
    }

    #[test]
    fn extracts_lumberyard_reference_motion() {
        let slope = -0.2;
        let ground_normal = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
            * Quat::from_rotation_x(-slope)
            * Vec3::Z;
        let parameters = extract_motion_parameters(
            0.016,
            Quat::from_rotation_z(std::f32::consts::PI),
            RootMotionDelta {
                rotation: Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2),
                translation: Vec3::new(-1.0, 0.0, 0.0),
            },
            ground_normal,
            &Settings,
        );

        assert!((parameters.signed_ground_angle - slope).abs() < 0.01);
        assert!((parameters.travel_angle() - std::f32::consts::FRAC_PI_2).abs() < 0.01);
        assert!((parameters.travel_distance - 1.0 / slope.cos()).abs() < 0.01);
        assert!((parameters.turn_angle + std::f32::consts::FRAC_PI_2).abs() < 0.01);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "sub-epsilon movement must be suppressed to exactly zero"
    )]
    fn suppresses_motion_below_speed_epsilon() {
        let parameters = extract_motion_parameters(
            0.5,
            Quat::IDENTITY,
            RootMotionDelta {
                rotation: Quat::IDENTITY,
                translation: Vec3::new(0.009, 0.0, 0.0),
            },
            Vec3::Z,
            &Settings,
        );

        assert_eq!(parameters.relative_travel_delta, Vec2::ZERO);
        assert_eq!(parameters.travel_speed, 0.0);
    }

    #[test]
    fn root_motion_emits_one_stop_edge() {
        let mut state = RootMotionState::default();
        let delta = RootMotionDelta {
            rotation: Quat::from_rotation_z(0.2),
            translation: Vec3::Y,
        };

        assert!(matches!(
            state.update(true, Quat::IDENTITY, delta, 0.5),
            RootMotionCommand::Apply { linear_velocity, .. } if linear_velocity == Vec3::new(0.0, 2.0, 0.0)
        ));
        assert_eq!(
            state.update(false, Quat::IDENTITY, delta, 0.5),
            RootMotionCommand::Stop
        );
        assert_eq!(
            state.update(false, Quat::IDENTITY, delta, 0.5),
            RootMotionCommand::None
        );
    }

    #[test]
    fn exponential_displacement_uses_thirty_hertz_authored_decay() {
        let motion = ExponentialDisplacement::new(Vec3::X * 8.0, 0.5);

        assert_eq!(motion.delta(0.0, 1.0 / 30.0, 1.0), Vec3::X * 4.0);
        assert_eq!(motion.delta(1.0 / 30.0, 2.0 / 30.0, 1.0), Vec3::X * 2.0);
    }

    #[test]
    fn degenerate_decay_and_zero_duration_emit_the_complete_motion() {
        let total = Vec3::new(1.0, 2.0, 3.0);

        assert_eq!(
            ExponentialDisplacement::new(total, 1.0).delta(0.0, 1.0 / 60.0, 1.0),
            total
        );
        assert_eq!(
            ExponentialDisplacement::new(total, 0.5).delta(0.0, 1.0 / 60.0, 0.0),
            total
        );
    }

    #[test]
    fn planar_basis_translation_uses_right_forward_up_coordinates() {
        assert_eq!(
            planar_basis_translation(Vec3::new(2.0, 3.0, 4.0), Vec3::Y),
            Vec3::new(2.0, 3.0, 4.0)
        );
        assert_eq!(planar_basis_translation(Vec3::Y, Vec3::X), Vec3::X);
    }
}
