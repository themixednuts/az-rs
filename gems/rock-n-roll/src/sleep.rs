//! Backend-neutral `RockNRoll` rigid-body sleep evaluation.

use std::{borrow::Borrow, sync::Arc};

use az_physics::{BodyStatus, PhysicsBodyHandle};
use bevy::prelude::{Entity, Reflect, Resource};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{DynamicBodyState, TimeStep};

/// Composable body/world listener contract used at native sleep transitions.
///
/// Returning `false` from [`Self::can_sleep`] vetoes the whole connected
/// island for this evaluation and resets the candidate body's timer.
pub trait SleepListener: Send + Sync + 'static {
    fn can_sleep(&self, _entity: Entity, _body: PhysicsBodyHandle, _status: &BodyStatus) -> bool {
        true
    }

    fn on_sleep(&self, _entity: Entity, _body: PhysicsBodyHandle) {}

    fn on_wake(&self, _entity: Entity, _body: PhysicsBodyHandle) {}
}

/// Ordered `RockNRoll` body/world listener registry.
#[derive(Resource, Default)]
pub struct SleepListeners(Vec<Arc<dyn SleepListener>>);

impl core::fmt::Debug for SleepListeners {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SleepListeners")
            .field("count", &self.0.len())
            .finish()
    }
}

impl SleepListeners {
    pub fn register<L: SleepListener>(&mut self, listener: L) {
        self.0.push(Arc::new(listener));
    }

    pub fn register_shared(&mut self, listener: Arc<dyn SleepListener>) {
        self.0.push(listener);
    }

    pub(crate) fn can_sleep(
        &self,
        entity: Entity,
        body: PhysicsBodyHandle,
        status: &BodyStatus,
    ) -> bool {
        self.0
            .iter()
            .all(|listener| listener.can_sleep(entity, body, status))
    }

    pub(crate) fn notify_sleep(&self, entity: Entity, body: PhysicsBodyHandle) {
        for listener in &self.0 {
            listener.on_sleep(entity, body);
        }
    }

    pub(crate) fn notify_wake(&self, entity: Entity, body: PhysicsBodyHandle) {
        for listener in &self.0 {
            listener.on_wake(entity, body);
        }
    }
}

/// Minimal body state consumed by the native sleep evaluator.
pub trait SleepBodyState {
    fn linear_velocity(&self) -> bevy::math::Vec3;
    fn angular_velocity(&self) -> bevy::math::Vec3;
    fn kinetic_energy(&self) -> f32;
    fn mass(&self) -> f32;
}

impl SleepBodyState for DynamicBodyState {
    fn linear_velocity(&self) -> bevy::math::Vec3 {
        self.velocity.linear
    }

    fn angular_velocity(&self) -> bevy::math::Vec3 {
        self.velocity.angular
    }

    fn kinetic_energy(&self) -> f32 {
        self.kinetic_energy()
    }

    fn mass(&self) -> f32 {
        self.mass.mass()
    }
}

impl SleepBodyState for az_physics::BodyStatus {
    fn linear_velocity(&self) -> bevy::math::Vec3 {
        self.linear_velocity
    }

    fn angular_velocity(&self) -> bevy::math::Vec3 {
        self.angular_velocity
    }

    fn kinetic_energy(&self) -> f32 {
        self.kinetic_energy
    }

    fn mass(&self) -> f32 {
        self.mass
    }
}

const SMOOTH_CURRENT: f32 = 0.1;
const SMOOTH_PREVIOUS: f32 = 0.9;
const STATIONARY_TIME_BOOST: f32 = 3.0;

/// Behavioral names for `RockNRoll`'s recovered numeric sleep modes.
///
/// The original C++ enum labels are not present in the binary. The numeric
/// values and behavior are stable compatibility data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(i32)]
pub enum SleepCondition {
    Disabled = 0,
    InstantaneousVelocity = 1,
    SmoothedVelocity = 2,
    InstantaneousEnergy = 3,
    #[default]
    SmoothedEnergy = 4,
}

impl TryFrom<i32> for SleepCondition {
    type Error = SleepConfigurationError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Disabled),
            1 => Ok(Self::InstantaneousVelocity),
            2 => Ok(Self::SmoothedVelocity),
            3 => Ok(Self::InstantaneousEnergy),
            4 => Ok(Self::SmoothedEnergy),
            value => Err(SleepConfigurationError::UnknownCondition(value)),
        }
    }
}

impl From<SleepCondition> for i32 {
    fn from(value: SleepCondition) -> Self {
        value as Self
    }
}

/// Validated thresholds used by the native sleep evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct SleepConfiguration {
    pub condition: SleepCondition,
    pub linear_velocity_threshold: f32,
    pub angular_velocity_threshold: f32,
    /// Kinetic-energy threshold per unit mass.
    pub energy_threshold: f32,
    pub required_duration: f32,
}

impl SleepConfiguration {
    /// Creates finite, nonnegative sleep thresholds.
    ///
    /// # Errors
    ///
    /// Returns [`SleepConfigurationError`] when a threshold is negative or
    /// non-finite.
    pub fn try_new(
        condition: SleepCondition,
        linear_velocity_threshold: f32,
        angular_velocity_threshold: f32,
        energy_threshold: f32,
        required_duration: f32,
    ) -> Result<Self, SleepConfigurationError> {
        validate_threshold("linear velocity", linear_velocity_threshold)?;
        validate_threshold("angular velocity", angular_velocity_threshold)?;
        validate_threshold("energy", energy_threshold)?;
        validate_threshold("required duration", required_duration)?;

        Ok(Self {
            condition,
            linear_velocity_threshold,
            angular_velocity_threshold,
            energy_threshold,
            required_duration,
        })
    }
}

impl Default for SleepConfiguration {
    fn default() -> Self {
        Self {
            condition: SleepCondition::SmoothedEnergy,
            linear_velocity_threshold: 0.8,
            angular_velocity_threshold: 1.0,
            energy_threshold: 0.5,
            required_duration: 0.5,
        }
    }
}

/// Persistent metrics owned by one dynamic rigid body.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SleepState {
    eligible_duration: f32,
    smoothed_energy: f32,
    smoothed_linear_speed_squared: f32,
    smoothed_angular_speed_squared: f32,
}

impl SleepState {
    #[inline]
    #[must_use]
    pub const fn eligible_duration(&self) -> f32 {
        self.eligible_duration
    }

    #[inline]
    #[must_use]
    pub const fn smoothed_energy(&self) -> f32 {
        self.smoothed_energy
    }

    #[inline]
    #[must_use]
    pub const fn smoothed_linear_speed_squared(&self) -> f32 {
        self.smoothed_linear_speed_squared
    }

    #[inline]
    #[must_use]
    pub const fn smoothed_angular_speed_squared(&self) -> f32 {
        self.smoothed_angular_speed_squared
    }

    /// Advances the native sleep timer and reports whether the body may sleep.
    ///
    /// The returned value uses `RockNRoll`'s strict `timer > required_duration`
    /// comparison. Island-wide agreement and listener vetoes remain world-level
    /// responsibilities.
    pub fn update<B: SleepBodyState + ?Sized>(
        &mut self,
        body: &B,
        configuration: impl Borrow<SleepConfiguration>,
        time_step: TimeStep,
    ) -> bool {
        let configuration = configuration.borrow();
        let dt = time_step.seconds();

        let below_threshold = match configuration.condition {
            SleepCondition::Disabled => false,
            SleepCondition::InstantaneousVelocity => {
                let linear_speed_squared = body.linear_velocity().length_squared();
                let angular_speed_squared = body.angular_velocity().length_squared();
                if body.linear_velocity() == bevy::math::Vec3::ZERO
                    && body.angular_velocity() == bevy::math::Vec3::ZERO
                {
                    self.eligible_duration =
                        STATIONARY_TIME_BOOST.mul_add(dt, self.eligible_duration);
                }
                linear_speed_squared
                    < configuration.linear_velocity_threshold
                        * configuration.linear_velocity_threshold
                    && angular_speed_squared
                        < configuration.angular_velocity_threshold
                            * configuration.angular_velocity_threshold
            }
            SleepCondition::SmoothedVelocity => {
                self.smoothed_linear_speed_squared = smooth(
                    body.linear_velocity().length_squared(),
                    self.smoothed_linear_speed_squared,
                );
                self.smoothed_angular_speed_squared = smooth(
                    body.angular_velocity().length_squared(),
                    self.smoothed_angular_speed_squared,
                );
                if self.smoothed_linear_speed_squared == 0.0
                    && self.smoothed_angular_speed_squared == 0.0
                {
                    self.eligible_duration =
                        STATIONARY_TIME_BOOST.mul_add(dt, self.eligible_duration);
                }
                self.smoothed_linear_speed_squared
                    < configuration.linear_velocity_threshold
                        * configuration.linear_velocity_threshold
                    && self.smoothed_angular_speed_squared
                        < configuration.angular_velocity_threshold
                            * configuration.angular_velocity_threshold
            }
            SleepCondition::InstantaneousEnergy => {
                let energy = body.kinetic_energy();
                if energy == 0.0 {
                    self.eligible_duration =
                        STATIONARY_TIME_BOOST.mul_add(dt, self.eligible_duration);
                }
                energy < configuration.energy_threshold * body.mass()
            }
            SleepCondition::SmoothedEnergy => {
                self.smoothed_energy = smooth(body.kinetic_energy(), self.smoothed_energy);
                if self.smoothed_energy == 0.0 {
                    self.eligible_duration =
                        STATIONARY_TIME_BOOST.mul_add(dt, self.eligible_duration);
                }
                self.smoothed_energy < configuration.energy_threshold * body.mass()
            }
        };

        if below_threshold {
            self.eligible_duration += dt;
        } else {
            self.eligible_duration = 0.0;
        }

        below_threshold && self.eligible_duration > configuration.required_duration
    }

    /// Applies the native listener-veto behavior.
    #[inline]
    pub const fn veto(&mut self) {
        self.eligible_duration = 0.0;
    }
}

#[inline]
fn smooth(current: f32, previous: f32) -> f32 {
    SMOOTH_PREVIOUS.mul_add(previous, SMOOTH_CURRENT * current)
}

fn validate_threshold(field: &'static str, value: f32) -> Result<(), SleepConfigurationError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(SleepConfigurationError::InvalidThreshold { field, value })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum SleepConfigurationError {
    #[error("unknown RockNRoll sleep condition {0}")]
    UnknownCondition(i32),
    #[error("{field} sleep threshold must be finite and nonnegative, got {value}")]
    InvalidThreshold { field: &'static str, value: f32 },
}

#[cfg(test)]
mod tests {
    use bevy::math::{Quat, Vec3};

    use super::*;
    use crate::{DynamicMassProperties, VelocityState};

    fn body(mass: f32, linear: Vec3, angular: Vec3) -> DynamicBodyState {
        DynamicBodyState::try_new(
            Vec3::ZERO,
            Quat::IDENTITY,
            VelocityState {
                linear,
                angular,
                ..Default::default()
            },
            DynamicMassProperties::try_new(mass, Vec3::splat(mass)).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn numeric_modes_round_trip_without_inventing_native_labels() {
        for value in 0..=4 {
            let condition = SleepCondition::try_from(value).unwrap();
            assert_eq!(i32::from(condition), value);
        }
        assert!(SleepCondition::try_from(5).is_err());
    }

    #[test]
    fn default_matches_recovered_rigid_body_sleep_defaults() {
        assert_eq!(
            SleepConfiguration::default(),
            SleepConfiguration::try_new(SleepCondition::SmoothedEnergy, 0.8, 1.0, 0.5, 0.5)
                .unwrap()
        );
    }

    #[test]
    fn exactly_stationary_body_receives_native_three_dt_boost() {
        let mut state = SleepState::default();
        let body = body(1.0, Vec3::ZERO, Vec3::ZERO);
        let step = TimeStep::try_from(0.1).unwrap();

        assert!(!state.update(&body, SleepConfiguration::default(), step));
        assert!((state.eligible_duration() - 0.4).abs() < 1.0e-6);
        assert!(state.update(&body, SleepConfiguration::default(), step));
        assert!((state.eligible_duration() - 0.8).abs() < 1.0e-6);
    }

    #[test]
    fn timer_must_be_strictly_greater_than_required_duration() {
        let mut state = SleepState::default();
        let body = body(1.0, Vec3::X * 0.1, Vec3::ZERO);
        let configuration =
            SleepConfiguration::try_new(SleepCondition::InstantaneousVelocity, 0.8, 1.0, 0.5, 0.5)
                .unwrap();
        let step = TimeStep::try_from(0.1).unwrap();

        for _ in 0..5 {
            assert!(!state.update(&body, configuration, step));
        }
        assert!(state.update(&body, configuration, step));
    }

    #[test]
    fn smoothed_velocity_uses_point_one_current_and_point_nine_previous() {
        let mut state = SleepState::default();
        let body = body(1.0, Vec3::X * 2.0, Vec3::Y * 3.0);
        let configuration =
            SleepConfiguration::try_new(SleepCondition::SmoothedVelocity, 10.0, 10.0, 0.5, 10.0)
                .unwrap();

        state.update(&body, configuration, TimeStep::try_from(0.1).unwrap());

        assert!((state.smoothed_linear_speed_squared() - 0.4).abs() < 1.0e-6);
        assert!((state.smoothed_angular_speed_squared() - 0.9).abs() < 1.0e-6);
    }

    #[test]
    fn energy_threshold_is_scaled_by_mass() {
        let configuration =
            SleepConfiguration::try_new(SleepCondition::InstantaneousEnergy, 0.8, 1.0, 0.5, 0.0)
                .unwrap();
        let step = TimeStep::try_from(0.1).unwrap();
        let mut at_threshold = SleepState::default();
        let mut below_threshold = SleepState::default();

        assert!(!at_threshold.update(&body(2.0, Vec3::X, Vec3::ZERO), configuration, step));
        assert!(below_threshold.update(&body(2.0, Vec3::X * 0.9, Vec3::ZERO), configuration, step));
    }

    // The property under test is that the timer is *cleared*, i.e. reset to the
    // exact `0.0` the constructor assigns; a tolerance would pass on a timer
    // that was merely shrunk.
    #[allow(clippy::float_cmp)]
    #[test]
    fn failed_threshold_and_listener_veto_clear_the_timer() {
        let mut state = SleepState::default();
        let step = TimeStep::try_from(0.1).unwrap();
        let configuration = SleepConfiguration::default();

        state.update(&body(1.0, Vec3::ZERO, Vec3::ZERO), configuration, step);
        state.veto();
        assert_eq!(state.eligible_duration(), 0.0);

        state.update(&body(1.0, Vec3::ZERO, Vec3::ZERO), configuration, step);
        assert!(!state.update(&body(1.0, Vec3::X * 100.0, Vec3::ZERO), configuration, step));
        assert_eq!(state.eligible_duration(), 0.0);
    }

    #[test]
    fn invalid_thresholds_are_rejected() {
        assert!(
            SleepConfiguration::try_new(SleepCondition::Disabled, -1.0, 0.0, 0.0, 0.0).is_err()
        );
        assert!(
            SleepConfiguration::try_new(SleepCondition::Disabled, 0.0, f32::NAN, 0.0, 0.0).is_err()
        );
    }
}
