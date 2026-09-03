//! Backend-neutral `RockNRoll` continuous-motion contracts.
//!
//! The source enum labels are absent from the shipped binary. This module
//! therefore preserves its numeric values and proven routing behavior without
//! assigning speculative legacy names.

use std::num::NonZeroU32;

use bevy::math::{Vec3, Vec3A, bounding::Aabb3d};
use bevy::prelude::Reflect;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Margin added to every `RockNRoll` broadphase AABB lane.
pub const BROADPHASE_AABB_MARGIN: f32 = 0.05;

/// Hard cap enforced by the native ordered CCD event loop each substep.
pub const MAX_CCD_EVENTS_PER_SUBSTEP: u32 = 20;

const CCD_ADVANCEMENT_RATE: f32 = 60.0;

/// Fixed configuration copied into each native continuous-collision manager.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContinuousCollisionConfiguration {
    event_distance_threshold: f32,
    advancement_distance: f32,
    retry_limit: NonZeroU32,
}

impl ContinuousCollisionConfiguration {
    /// Creates a finite continuous-collision configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ContinuousCollisionConfigurationError`] when a distance is
    /// invalid or `retry_limit` is zero.
    pub fn try_new(
        event_distance_threshold: f32,
        advancement_distance: f32,
        retry_limit: u32,
    ) -> Result<Self, ContinuousCollisionConfigurationError> {
        if !event_distance_threshold.is_finite() || event_distance_threshold < 0.0 {
            return Err(
                ContinuousCollisionConfigurationError::InvalidEventDistanceThreshold(
                    event_distance_threshold,
                ),
            );
        }
        if !advancement_distance.is_finite() || advancement_distance <= 0.0 {
            return Err(
                ContinuousCollisionConfigurationError::InvalidAdvancementDistance(
                    advancement_distance,
                ),
            );
        }
        let retry_limit = NonZeroU32::new(retry_limit)
            .ok_or(ContinuousCollisionConfigurationError::ZeroRetryLimit)?;

        Ok(Self {
            event_distance_threshold,
            advancement_distance,
            retry_limit,
        })
    }

    #[inline]
    #[must_use]
    pub const fn event_distance_threshold(self) -> f32 {
        self.event_distance_threshold
    }

    #[inline]
    #[must_use]
    pub const fn advancement_distance(self) -> f32 {
        self.advancement_distance
    }

    #[inline]
    #[must_use]
    pub const fn retry_limit(self) -> NonZeroU32 {
        self.retry_limit
    }

    /// Computes the native conservative fraction increment for one pair.
    ///
    /// The shape terms are half-extents of each current shape AABB. `RockNRoll`
    /// uses their half-diagonal lengths as angular sweep radii.
    #[must_use]
    pub fn conservative_fraction_increment(
        self,
        linear_velocity_a: Vec3,
        angular_velocity_a: Vec3,
        half_extents_a: Vec3,
        linear_velocity_b: Vec3,
        angular_velocity_b: Vec3,
        half_extents_b: Vec3,
    ) -> f32 {
        let speed_bound = angular_velocity_b.length().mul_add(
            half_extents_b.length(),
            angular_velocity_a.length().mul_add(
                half_extents_a.length(),
                (linear_velocity_a - linear_velocity_b).length(),
            ),
        );

        // A retry limit is a small per-step iteration count (native default 5),
        // so `u16` holds it exactly and `f32::from` is lossless.
        let retry_limit = f32::from(u16::try_from(self.retry_limit.get()).unwrap_or(u16::MAX));
        ((self.advancement_distance / speed_bound) * CCD_ADVANCEMENT_RATE) / retry_limit
    }
}

impl Default for ContinuousCollisionConfiguration {
    fn default() -> Self {
        Self {
            event_distance_threshold: 0.05,
            advancement_distance: 0.05,
            retry_limit: NonZeroU32::new(5).expect("five is nonzero"),
        }
    }
}

/// Recovered numeric values of the reflected `Continuous physics` field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(i32)]
pub enum ContinuousPhysicsMode {
    #[default]
    Disabled = 0,
    /// Installs the shared hit/projection callback; distinction from mode 2 is
    /// not yet proven.
    Mode1 = 1,
    /// Installs the shared hit/projection callback; distinction from mode 1 is
    /// not yet proven.
    Mode2 = 2,
    /// Installs the ordered time-of-impact event callback.
    OrderedTimeOfImpact = 3,
    /// Reverses the displacement used by swept broadphase bounds.
    ReverseDisplacementSweep = 4,
}

impl ContinuousPhysicsMode {
    /// Returns the native pair-callback family selected during pair setup.
    #[inline]
    #[must_use]
    pub const fn callback_kind(self) -> ContinuousCallbackKind {
        match self {
            Self::Mode1 | Self::Mode2 => ContinuousCallbackKind::HitProjection,
            Self::OrderedTimeOfImpact => ContinuousCallbackKind::OrderedTimeOfImpact,
            Self::Disabled | Self::ReverseDisplacementSweep => ContinuousCallbackKind::None,
        }
    }

    /// Returns whether positive-separation contacts use the native mode-4
    /// normal-only speculative constraint path.
    #[inline]
    #[must_use]
    pub const fn uses_speculative_normal_constraints(self) -> bool {
        matches!(self, Self::ReverseDisplacementSweep)
    }

    /// Returns the displacement used to extend a flagged broadphase AABB.
    #[inline]
    #[must_use]
    pub fn sweep_displacement(self, previous: Vec3, current: Vec3) -> Vec3 {
        if self == Self::ReverseDisplacementSweep {
            current - previous
        } else {
            previous - current
        }
    }

    /// Reproduces the native swept and margin-expanded broadphase AABB update.
    ///
    /// `sweep` corresponds to the collidable flag tested by the native update.
    /// When it is false, only the fixed margin is applied.
    #[must_use]
    pub fn broadphase_aabb(
        self,
        current_bounds: &Aabb3d,
        previous_translation: Vec3,
        current_translation: Vec3,
        sweep: bool,
    ) -> Aabb3d {
        let mut min = current_bounds.min;
        let mut max = current_bounds.max;

        if sweep {
            let displacement =
                Vec3A::from(self.sweep_displacement(previous_translation, current_translation));
            min = min.min(current_bounds.min + displacement);
            max = max.max(current_bounds.max + displacement);
        }

        let margin = Vec3A::splat(BROADPHASE_AABB_MARGIN);
        Aabb3d::from_min_max(min - margin, max + margin)
    }
}

/// A positive-separation contact consumed by `RockNRoll`'s mode-4 solver path.
///
/// The native constraint builder keeps the original witness on one body,
/// projects the other witness by `normal * separation`, and stores
/// `-separation / time_step` as the normal velocity bias. It does not emit the
/// common friction or restitution constraints for this branch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeculativeNormalContact {
    point: Vec3,
    normal: Vec3,
    separation: f32,
}

impl SpeculativeNormalContact {
    /// Constructs a finite, positive-separation mode-4 contact without
    /// changing or normalizing the native contact data.
    ///
    /// # Errors
    ///
    /// Returns [`SpeculativeNormalContactError::NonFiniteContact`] if `point`
    /// or `normal` has a non-finite component, or
    /// [`SpeculativeNormalContactError::InvalidSeparation`] if `separation` is
    /// not finite and strictly positive.
    pub fn try_new(
        point: Vec3,
        normal: Vec3,
        separation: f32,
    ) -> Result<Self, SpeculativeNormalContactError> {
        if !point.is_finite() || !normal.is_finite() {
            return Err(SpeculativeNormalContactError::NonFiniteContact);
        }
        if !separation.is_finite() || separation <= 0.0 {
            return Err(SpeculativeNormalContactError::InvalidSeparation(separation));
        }

        Ok(Self {
            point,
            normal,
            separation,
        })
    }

    #[inline]
    #[must_use]
    pub const fn point(self) -> Vec3 {
        self.point
    }

    #[inline]
    #[must_use]
    pub const fn normal(self) -> Vec3 {
        self.normal
    }

    #[inline]
    #[must_use]
    pub const fn separation(self) -> f32 {
        self.separation
    }

    /// Returns the witness projected onto the opposite contact plane.
    #[inline]
    #[must_use]
    pub fn projected_witness(self) -> Vec3 {
        self.point + self.normal * self.separation
    }

    /// Returns the exact normal velocity bias stored by the native builder.
    ///
    /// # Errors
    ///
    /// Returns [`SpeculativeNormalContactError::InvalidTimeStep`] if
    /// `time_step` is not finite and strictly positive.
    pub fn velocity_bias(self, time_step: f32) -> Result<f32, SpeculativeNormalContactError> {
        if !time_step.is_finite() || time_step <= 0.0 {
            return Err(SpeculativeNormalContactError::InvalidTimeStep(time_step));
        }
        Ok(-self.separation / time_step)
    }
}

impl TryFrom<i32> for ContinuousPhysicsMode {
    type Error = ContinuousPhysicsModeError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Disabled),
            1 => Ok(Self::Mode1),
            2 => Ok(Self::Mode2),
            3 => Ok(Self::OrderedTimeOfImpact),
            4 => Ok(Self::ReverseDisplacementSweep),
            value => Err(ContinuousPhysicsModeError(value)),
        }
    }
}

impl From<ContinuousPhysicsMode> for i32 {
    #[inline]
    fn from(value: ContinuousPhysicsMode) -> Self {
        value as Self
    }
}

/// Native callback families selected from [`ContinuousPhysicsMode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuousCallbackKind {
    None,
    HitProjection,
    OrderedTimeOfImpact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("unknown RockNRoll continuous-physics mode {0}")]
pub struct ContinuousPhysicsModeError(pub i32);

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum SpeculativeNormalContactError {
    #[error("speculative contact point and normal must be finite")]
    NonFiniteContact,
    #[error("speculative contact separation must be finite and positive, got {0}")]
    InvalidSeparation(f32),
    #[error("speculative contact time step must be finite and positive, got {0}")]
    InvalidTimeStep(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ContinuousCollisionConfigurationError {
    #[error("CCD event distance threshold must be finite and nonnegative, got {0}")]
    InvalidEventDistanceThreshold(f32),
    #[error("CCD advancement distance must be finite and positive, got {0}")]
    InvalidAdvancementDistance(f32),
    #[error("CCD retry limit must be nonzero")]
    ZeroRetryLimit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_modes_round_trip_without_speculative_source_labels() {
        for value in 0..=4 {
            let mode = ContinuousPhysicsMode::try_from(value).unwrap();
            assert_eq!(i32::from(mode), value);
        }
        assert!(ContinuousPhysicsMode::try_from(5).is_err());
    }

    // Parity test: each value is compared against the exact literal the native
    // world constructor assigns, so an epsilon would stop it noticing a changed
    // default.
    #[allow(clippy::float_cmp)]
    #[test]
    fn manager_defaults_match_world_construction() {
        let configuration = ContinuousCollisionConfiguration::default();
        assert_eq!(configuration.event_distance_threshold(), 0.05);
        assert_eq!(configuration.advancement_distance(), 0.05);
        assert_eq!(configuration.retry_limit().get(), 5);
        assert_eq!(MAX_CCD_EVENTS_PER_SUBSTEP, 20);
    }

    #[test]
    fn conservative_advance_uses_relative_linear_and_angular_sweep_speed() {
        let configuration = ContinuousCollisionConfiguration::default();
        let increment = configuration.conservative_fraction_increment(
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(3.0, 4.0, 0.0),
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 4.0),
            Vec3::new(0.0, 0.0, 2.0),
        );

        // (0.05 / (3 + 2*5 + 4*2)) * 60 / 5
        let expected = (0.05 / 21.0) * 60.0 / 5.0;
        assert!((increment - expected).abs() < 1.0e-7);
    }

    #[test]
    fn invalid_manager_configuration_is_rejected() {
        assert!(ContinuousCollisionConfiguration::try_new(-0.05, 0.05, 5).is_err());
        assert!(ContinuousCollisionConfiguration::try_new(0.05, 0.0, 5).is_err());
        assert!(ContinuousCollisionConfiguration::try_new(0.05, 0.05, 0).is_err());
    }

    #[test]
    fn callback_routing_matches_pair_setup() {
        assert_eq!(
            ContinuousPhysicsMode::Disabled.callback_kind(),
            ContinuousCallbackKind::None
        );
        assert_eq!(
            ContinuousPhysicsMode::Mode1.callback_kind(),
            ContinuousCallbackKind::HitProjection
        );
        assert_eq!(
            ContinuousPhysicsMode::Mode2.callback_kind(),
            ContinuousCallbackKind::HitProjection
        );
        assert_eq!(
            ContinuousPhysicsMode::OrderedTimeOfImpact.callback_kind(),
            ContinuousCallbackKind::OrderedTimeOfImpact
        );
        assert_eq!(
            ContinuousPhysicsMode::ReverseDisplacementSweep.callback_kind(),
            ContinuousCallbackKind::None
        );
        assert!(
            ContinuousPhysicsMode::ReverseDisplacementSweep.uses_speculative_normal_constraints()
        );
        assert!(!ContinuousPhysicsMode::Mode2.uses_speculative_normal_constraints());
    }

    #[test]
    fn mode_four_contact_projects_one_witness_and_uses_native_bias() {
        let contact =
            SpeculativeNormalContact::try_new(Vec3::new(2.0, 3.0, 4.0), Vec3::Y, 0.125).unwrap();

        assert_eq!(contact.projected_witness(), Vec3::new(2.0, 3.125, 4.0));
        assert!((contact.velocity_bias(0.025).unwrap() + 5.0).abs() < 1.0e-6);
    }

    #[test]
    fn speculative_contact_rejects_nonpositive_separation_and_time_step() {
        assert!(SpeculativeNormalContact::try_new(Vec3::ZERO, Vec3::Y, 0.0).is_err());
        let contact = SpeculativeNormalContact::try_new(Vec3::ZERO, Vec3::Y, 0.1).unwrap();
        assert!(contact.velocity_bias(0.0).is_err());
    }

    #[test]
    fn ordinary_sweep_unions_current_bounds_toward_previous_position() {
        let bounds = Aabb3d::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));
        let swept = ContinuousPhysicsMode::OrderedTimeOfImpact.broadphase_aabb(
            &bounds,
            Vec3::new(-3.0, 0.0, 0.0),
            Vec3::ZERO,
            true,
        );

        assert_eq!(swept.min, Vec3A::new(-4.05, -1.05, -1.05));
        assert_eq!(swept.max, Vec3A::splat(1.05));
    }

    #[test]
    fn mode_four_reverses_the_sweep_displacement() {
        let bounds = Aabb3d::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));
        let swept = ContinuousPhysicsMode::ReverseDisplacementSweep.broadphase_aabb(
            &bounds,
            Vec3::new(-3.0, 0.0, 0.0),
            Vec3::ZERO,
            true,
        );

        assert_eq!(swept.min, Vec3A::splat(-1.05));
        assert_eq!(swept.max, Vec3A::new(4.05, 1.05, 1.05));
    }

    #[test]
    fn unswept_bounds_still_receive_the_native_margin() {
        let bounds = Aabb3d::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));
        let expanded =
            ContinuousPhysicsMode::Disabled.broadphase_aabb(&bounds, Vec3::ZERO, Vec3::ZERO, false);

        assert_eq!(expanded.min, Vec3A::splat(-1.05));
        assert_eq!(expanded.max, Vec3A::splat(1.05));
    }
}
