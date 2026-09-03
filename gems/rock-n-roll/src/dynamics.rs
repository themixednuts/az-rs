//! Backend-neutral `RockNRoll` rigid-body integration primitives.
//!
//! These types preserve the recovered `RockNRoll` force, damping, and pose
//! integration contract. Solver adapters may use their own storage, but can
//! use this module as the parity oracle without exposing solver-native types.

use std::f32::consts::FRAC_PI_2;

use bevy::math::{Mat3, Quat, Vec3};
use bevy::prelude::Reflect;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// `RockNRoll`'s fixed low-speed damping decrement, in world units per second.
pub const LOW_SPEED_DECREMENT: f32 = 0.005;

/// A finite, positive physics substep.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct TimeStep(f32);

impl TimeStep {
    /// Returns the substep duration in seconds.
    #[inline]
    #[must_use]
    pub const fn seconds(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for TimeStep {
    type Error = DynamicsError;

    fn try_from(seconds: f32) -> Result<Self, Self::Error> {
        if seconds.is_finite() && seconds > 0.0 {
            Ok(Self(seconds))
        } else {
            Err(DynamicsError::InvalidTimeStep(seconds))
        }
    }
}

impl From<TimeStep> for f32 {
    #[inline]
    fn from(value: TimeStep) -> Self {
        value.seconds()
    }
}

/// Validated dynamic-body mass and principal inertia.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicMassProperties {
    mass: f32,
    inverse_mass: f32,
    principal_inertia: Vec3,
    inverse_principal_inertia: Vec3,
}

impl DynamicMassProperties {
    /// Builds mass properties from mass and the body-space principal inertia.
    ///
    /// # Errors
    ///
    /// Returns [`DynamicsError`] unless every value is finite and positive.
    pub fn try_new(mass: f32, principal_inertia: Vec3) -> Result<Self, DynamicsError> {
        if !mass.is_finite() || mass <= 0.0 {
            return Err(DynamicsError::InvalidMass(mass));
        }
        if !principal_inertia.is_finite() || !principal_inertia.cmpgt(Vec3::ZERO).all() {
            return Err(DynamicsError::InvalidPrincipalInertia(principal_inertia));
        }

        Ok(Self {
            mass,
            inverse_mass: mass.recip(),
            principal_inertia,
            inverse_principal_inertia: principal_inertia.recip(),
        })
    }

    #[inline]
    #[must_use]
    pub const fn mass(self) -> f32 {
        self.mass
    }

    #[inline]
    #[must_use]
    pub const fn inverse_mass(self) -> f32 {
        self.inverse_mass
    }

    #[inline]
    #[must_use]
    pub const fn principal_inertia(self) -> Vec3 {
        self.principal_inertia
    }

    #[inline]
    #[must_use]
    pub const fn inverse_principal_inertia(self) -> Vec3 {
        self.inverse_principal_inertia
    }
}

/// Validated `RockNRoll` linear and angular damping coefficients.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct Damping {
    pub linear: f32,
    pub angular: f32,
}

impl Damping {
    /// Creates nonnegative finite damping coefficients.
    ///
    /// # Errors
    ///
    /// Returns [`DynamicsError`] for a negative or non-finite coefficient.
    pub fn try_new(linear: f32, angular: f32) -> Result<Self, DynamicsError> {
        if !linear.is_finite() || linear < 0.0 {
            return Err(DynamicsError::InvalidDamping {
                field: "linear",
                value: linear,
            });
        }
        if !angular.is_finite() || angular < 0.0 {
            return Err(DynamicsError::InvalidDamping {
                field: "angular",
                value: angular,
            });
        }
        Ok(Self { linear, angular })
    }

    /// Applies the native linear branch including the fixed low-speed
    /// decrement used after coefficient damping.
    #[must_use]
    pub fn apply_linear(self, velocity: Vec3, time_step: TimeStep) -> Vec3 {
        damp(velocity, self.linear, time_step.seconds())
    }

    /// Applies the native angular branch including the fixed low-speed
    /// decrement used after coefficient damping.
    #[must_use]
    pub fn apply_angular(self, velocity: Vec3, time_step: TimeStep) -> Vec3 {
        damp(velocity, self.angular, time_step.seconds())
    }
}

/// Continuous force and torque consumed by the next force-integration pass.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ForceAccumulator {
    pub force: Vec3,
    pub torque: Vec3,
}

impl ForceAccumulator {
    #[inline]
    pub fn add_force(&mut self, force: Vec3) {
        self.force += force;
    }

    #[inline]
    pub fn add_torque(&mut self, torque: Vec3) {
        self.torque += torque;
    }

    /// Adds the native persistent gravity producer: `force += mass * gravity`.
    #[inline]
    pub fn add_gravity(&mut self, gravity: Vec3, mass: DynamicMassProperties) {
        self.force += mass.mass() * gravity;
    }

    #[inline]
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Actual and solver-correction velocities used by `RockNRoll` pose integration.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VelocityState {
    pub linear: Vec3,
    pub angular: Vec3,
    pub solver_linear_correction: Vec3,
    pub solver_angular_correction: Vec3,
}

/// Dynamic rigid-body state required by the recovered integration slice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicBodyState {
    pub translation: Vec3,
    pub rotation: Quat,
    pub velocity: VelocityState,
    pub forces: ForceAccumulator,
    pub mass: DynamicMassProperties,
    pub world_inverse_inertia: Mat3,
}

impl DynamicBodyState {
    /// Creates a dynamic body with a finite pose and normalized rotation.
    ///
    /// # Errors
    ///
    /// Returns [`DynamicsError`] when the translation or rotation cannot
    /// represent a finite rigid transform.
    pub fn try_new(
        translation: Vec3,
        rotation: Quat,
        velocity: VelocityState,
        mass: DynamicMassProperties,
    ) -> Result<Self, DynamicsError> {
        if !translation.is_finite() {
            return Err(DynamicsError::InvalidTranslation(translation));
        }

        let rotation_length_squared = rotation.length_squared();
        if !rotation.is_finite()
            || !rotation_length_squared.is_finite()
            || rotation_length_squared <= f32::MIN_POSITIVE
        {
            return Err(DynamicsError::InvalidRotation(rotation));
        }
        let rotation = rotation * rotation_length_squared.sqrt().recip();

        Ok(Self {
            translation,
            rotation,
            velocity,
            forces: ForceAccumulator::default(),
            mass,
            world_inverse_inertia: world_inverse_inertia(rotation, mass),
        })
    }

    /// Applies gravity, accumulated force, and accumulated torque to velocity.
    ///
    /// The continuous accumulators are consumed even when they are zero. This
    /// matches the native pass and keeps one-shot solver corrections separate.
    pub fn integrate_forces(&mut self, gravity: Vec3, time_step: TimeStep) {
        self.forces.add_gravity(gravity, self.mass);

        let dt = time_step.seconds();
        self.velocity.linear += self.mass.inverse_mass() * self.forces.force * dt;
        self.velocity.angular += self.world_inverse_inertia * self.forces.torque * dt;
        self.forces.clear();

        let maximum_angular_speed = FRAC_PI_2 / dt;
        self.velocity.angular = self
            .velocity
            .angular
            .clamp_length_max(maximum_angular_speed);
    }

    /// Applies native damping and advances pose from actual plus solver velocity.
    pub fn integrate_pose(&mut self, damping: Damping, time_step: TimeStep) {
        let dt = time_step.seconds();
        self.velocity.linear = damp(self.velocity.linear, damping.linear, dt);
        self.velocity.angular = damp(self.velocity.angular, damping.angular, dt);

        self.translation += (self.velocity.linear + self.velocity.solver_linear_correction) * dt;

        let angular = self.velocity.angular + self.velocity.solver_angular_correction;
        if angular != Vec3::ZERO {
            let omega = Quat::from_xyzw(angular.x, angular.y, angular.z, 0.0);
            let derivative = omega * self.rotation;
            self.rotation = Quat::from_xyzw(
                (0.5 * dt).mul_add(derivative.x, self.rotation.x),
                (0.5 * dt).mul_add(derivative.y, self.rotation.y),
                (0.5 * dt).mul_add(derivative.z, self.rotation.z),
                (0.5 * dt).mul_add(derivative.w, self.rotation.w),
            )
            .normalize();
        }

        self.velocity.solver_linear_correction = Vec3::ZERO;
        self.velocity.solver_angular_correction = Vec3::ZERO;
        self.world_inverse_inertia = world_inverse_inertia(self.rotation, self.mass);
    }

    /// Returns `0.5 * (mass * |v|^2 + omega dot I_world * omega)`.
    #[inline]
    #[must_use]
    pub fn kinetic_energy(&self) -> f32 {
        let linear = self.mass.mass() * self.velocity.linear.length_squared();
        let world_inertia = self.world_inverse_inertia.inverse();
        let angular = self
            .velocity
            .angular
            .dot(world_inertia * self.velocity.angular);
        0.5 * (linear + angular)
    }
}

#[inline]
fn damp(mut velocity: Vec3, coefficient: f32, time_step: f32) -> Vec3 {
    velocity *= coefficient.mul_add(-time_step, 1.0).clamp(0.0, 1.0);

    let speed_squared = velocity.length_squared();
    if speed_squared < coefficient * coefficient {
        if speed_squared <= LOW_SPEED_DECREMENT * LOW_SPEED_DECREMENT {
            Vec3::ZERO
        } else {
            velocity - velocity.normalize() * LOW_SPEED_DECREMENT
        }
    } else {
        velocity
    }
}

#[inline]
fn world_inverse_inertia(rotation: Quat, mass: DynamicMassProperties) -> Mat3 {
    let rotation = Mat3::from_quat(rotation);
    rotation * Mat3::from_diagonal(mass.inverse_principal_inertia()) * rotation.transpose()
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum DynamicsError {
    #[error("physics time step must be finite and positive, got {0}")]
    InvalidTimeStep(f32),
    #[error("dynamic mass must be finite and positive, got {0}")]
    InvalidMass(f32),
    #[error("principal inertia must be finite and positive, got {0:?}")]
    InvalidPrincipalInertia(Vec3),
    #[error("rigid-body translation must be finite, got {0:?}")]
    InvalidTranslation(Vec3),
    #[error("rigid-body rotation must be finite and nonzero, got {0:?}")]
    InvalidRotation(Quat),
    #[error("{field} damping must be finite and nonnegative, got {value}")]
    InvalidDamping { field: &'static str, value: f32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1.0e-5;

    fn mass(mass: f32, inertia: Vec3) -> DynamicMassProperties {
        DynamicMassProperties::try_new(mass, inertia).expect("valid test mass properties")
    }

    fn state(mass: DynamicMassProperties) -> DynamicBodyState {
        DynamicBodyState::try_new(Vec3::ZERO, Quat::IDENTITY, VelocityState::default(), mass)
            .expect("valid test body state")
    }

    #[test]
    fn gravity_accumulates_as_force_but_accelerates_independently_of_mass() {
        let gravity = Vec3::new(0.0, 0.0, -9.81);
        let step = TimeStep::try_from(0.25).unwrap();
        let mut light = state(mass(2.0, Vec3::ONE));
        let mut heavy = state(mass(10.0, Vec3::ONE));

        light.integrate_forces(gravity, step);
        heavy.integrate_forces(gravity, step);

        let expected = gravity * step.seconds();
        assert!(light.velocity.linear.abs_diff_eq(expected, EPSILON));
        assert!(heavy.velocity.linear.abs_diff_eq(expected, EPSILON));
        assert_eq!(light.forces, ForceAccumulator::default());
        assert_eq!(heavy.forces, ForceAccumulator::default());
    }

    #[test]
    fn force_and_torque_use_inverse_mass_and_world_inverse_inertia() {
        let mut body = state(mass(2.0, Vec3::new(2.0, 4.0, 8.0)));
        body.forces.add_force(Vec3::new(8.0, 0.0, 0.0));
        body.forces.add_torque(Vec3::new(2.0, 4.0, 8.0));

        body.integrate_forces(Vec3::ZERO, TimeStep::try_from(0.5).unwrap());

        assert!(
            body.velocity
                .linear
                .abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), EPSILON)
        );
        assert!(body.velocity.angular.abs_diff_eq(Vec3::splat(0.5), EPSILON));
    }

    #[test]
    fn angular_speed_is_limited_to_ninety_degrees_per_substep() {
        let step = TimeStep::try_from(0.5).unwrap();
        let mut body = state(mass(1.0, Vec3::ONE));
        body.velocity.angular = Vec3::X * 100.0;

        body.integrate_forces(Vec3::ZERO, step);

        assert!((body.velocity.angular.length() - FRAC_PI_2 / step.seconds()).abs() < EPSILON);
    }

    #[test]
    fn damping_preserves_native_low_speed_decrement() {
        let damping = Damping::try_new(0.05, 0.15).unwrap();
        let step = TimeStep::try_from(0.1).unwrap();
        let mass = mass(1.0, Vec3::ONE);
        let mut body = DynamicBodyState::try_new(
            Vec3::ZERO,
            Quat::IDENTITY,
            VelocityState {
                linear: Vec3::X * 0.02,
                angular: Vec3::Y * 0.004,
                ..Default::default()
            },
            mass,
        )
        .unwrap();

        body.integrate_pose(damping, step);

        let expected_linear = 0.02f32.mul_add(0.05f32.mul_add(-0.1, 1.0), -LOW_SPEED_DECREMENT);
        assert!((body.velocity.linear.x - expected_linear).abs() < EPSILON);
        assert_eq!(body.velocity.angular, Vec3::ZERO);
    }

    #[test]
    fn pose_consumes_solver_corrections_and_refreshes_world_inertia() {
        let mass = mass(1.0, Vec3::new(2.0, 4.0, 8.0));
        let mut body = DynamicBodyState::try_new(
            Vec3::ZERO,
            Quat::IDENTITY,
            VelocityState {
                linear: Vec3::X,
                angular: Vec3::Z,
                solver_linear_correction: Vec3::Y,
                solver_angular_correction: Vec3::ZERO,
            },
            mass,
        )
        .unwrap();

        body.integrate_pose(
            Damping::try_new(0.0, 0.0).unwrap(),
            TimeStep::try_from(0.5).unwrap(),
        );

        assert!(
            body.translation
                .abs_diff_eq(Vec3::new(0.5, 0.5, 0.0), EPSILON)
        );
        assert_eq!(body.velocity.solver_linear_correction, Vec3::ZERO);
        assert_eq!(body.velocity.solver_angular_correction, Vec3::ZERO);

        let expected = world_inverse_inertia(body.rotation, mass);
        assert!(body.world_inverse_inertia.abs_diff_eq(expected, EPSILON));
    }

    #[test]
    fn invalid_dynamic_inputs_are_rejected_before_integration() {
        assert!(TimeStep::try_from(0.0).is_err());
        assert!(DynamicMassProperties::try_new(0.0, Vec3::ONE).is_err());
        assert!(DynamicMassProperties::try_new(1.0, Vec3::ZERO).is_err());
        assert!(Damping::try_new(-1.0, 0.0).is_err());
        assert!(
            DynamicBodyState::try_new(
                Vec3::NAN,
                Quat::IDENTITY,
                VelocityState::default(),
                mass(1.0, Vec3::ONE),
            )
            .is_err()
        );
        assert!(
            DynamicBodyState::try_new(
                Vec3::ZERO,
                Quat::from_xyzw(0.0, 0.0, 0.0, 0.0),
                VelocityState::default(),
                mass(1.0, Vec3::ONE),
            )
            .is_err()
        );
    }
}
