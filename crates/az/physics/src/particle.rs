use bevy_reflect::Reflect;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::{CollisionClass, PhysicalEntityTypes, PhysicsBodyHandle, PhysicsError, SurfaceIndex};

/// Cry particle-entity behavior bits. Values match `particle_*` in
/// `CryCommon/physinterface.h`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct ParticleFlags(u32);

impl ParticleFlags {
    pub const NONE: Self = Self(0);
    pub const SINGLE_CONTACT: Self = Self(0x01);
    pub const CONSTANT_ORIENTATION: Self = Self(0x02);
    pub const NO_ROLL: Self = Self(0x04);
    pub const NO_PATH_ALIGNMENT: Self = Self(0x08);
    pub const NO_SPIN: Self = Self(0x10);
    pub const NO_SELF_COLLISIONS: Self = Self(0x100);
    pub const NO_IMPULSE: Self = Self(0x200);
    pub const TRACEABLE: Self = Self(0x400);

    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, flags: Self) -> bool {
        self.0 & flags.0 == flags.0
    }
}

impl From<u32> for ParticleFlags {
    fn from(bits: u32) -> Self {
        Self::from_bits(bits)
    }
}

impl From<ParticleFlags> for u32 {
    fn from(flags: ParticleFlags) -> Self {
        flags.bits()
    }
}

impl core::ops::BitOr for ParticleFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for ParticleFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Complete Cry `pe_params_particle` construction state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ParticleBodyConfiguration {
    pub flags: ParticleFlags,
    pub mass: f32,
    /// Full pseudo-diameter. Cry stores half of this value internally.
    pub size: f32,
    /// Full collision thickness while resting on a surface.
    pub thickness: f32,
    pub heading: Vec3,
    pub speed: f32,
    pub air_resistance: f32,
    pub water_resistance: f32,
    pub thrust_acceleration: f32,
    /// Lift acceleration at the authored initial speed.
    pub lift_acceleration: f32,
    pub surface_index: SurfaceIndex,
    pub angular_velocity: Vec3,
    /// `None` inherits the scene gravity at creation.
    pub gravity: Option<Vec3>,
    /// `None` uses 80% of the resolved air gravity, matching Cry construction.
    pub water_gravity: Option<Vec3>,
    pub alignment_normal: Vec3,
    pub roll_axis: Vec3,
    pub initial_orientation: Quat,
    pub minimum_bounce_speed: f32,
    pub minimum_speed: f32,
    pub ignored_collider: Option<PhysicsBodyHandle>,
    pub pierceability: u8,
    pub collision_types: PhysicalEntityTypes,
    pub collision_class: CollisionClass,
    pub area_check_period: u8,
    pub dont_play_hit_effect: bool,
}

impl Default for ParticleBodyConfiguration {
    fn default() -> Self {
        Self {
            flags: ParticleFlags::NONE,
            mass: 0.2,
            size: 0.1,
            thickness: 0.1,
            heading: Vec3::X,
            speed: 0.0,
            air_resistance: 0.0,
            water_resistance: 0.5,
            thrust_acceleration: 0.0,
            lift_acceleration: 0.0,
            surface_index: SurfaceIndex(0),
            angular_velocity: Vec3::ZERO,
            gravity: None,
            water_gravity: None,
            alignment_normal: Vec3::Z,
            roll_axis: Vec3::Z,
            initial_orientation: Quat::IDENTITY,
            minimum_bounce_speed: 1.5,
            minimum_speed: 0.02,
            ignored_collider: None,
            pierceability: 15,
            collision_types: PhysicalEntityTypes::ALL,
            collision_class: CollisionClass::new(1 << 6, 0),
            area_check_period: 6,
            dont_play_hit_effect: false,
        }
    }
}

impl ParticleBodyConfiguration {
    /// Validates all direct Cry particle parameters.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidParticleConfiguration`] naming the first
    /// offending field: a scalar that is non-finite or negative, a zero `size`,
    /// a non-finite direction vector, or `motion` when the heading or initial
    /// orientation is not unit-length or a motion scalar or gravity override is
    /// non-finite.
    pub fn validate(self) -> Result<(), PhysicsError> {
        for (field, value) in [
            ("mass", self.mass),
            ("size", self.size),
            ("thickness", self.thickness),
            ("air resistance", self.air_resistance),
            ("water resistance", self.water_resistance),
            ("minimum bounce speed", self.minimum_bounce_speed),
            ("minimum speed", self.minimum_speed),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidParticleConfiguration { field });
            }
        }
        if self.size == 0.0 {
            return Err(PhysicsError::InvalidParticleConfiguration { field: "size" });
        }
        for (field, value) in [
            ("heading", self.heading),
            ("angular velocity", self.angular_velocity),
            ("alignment normal", self.alignment_normal),
            ("roll axis", self.roll_axis),
        ] {
            if !value.is_finite() {
                return Err(PhysicsError::InvalidParticleConfiguration { field });
            }
        }
        if (self.heading.length_squared() - 1.0).abs() > 1.0e-4
            || !self.speed.is_finite()
            || !self.thrust_acceleration.is_finite()
            || !self.lift_acceleration.is_finite()
            || !self.initial_orientation.is_finite()
            || (self.initial_orientation.length_squared() - 1.0).abs() > 1.0e-4
            || self.gravity.is_some_and(|gravity| !gravity.is_finite())
            || self
                .water_gravity
                .is_some_and(|gravity| !gravity.is_finite())
        {
            return Err(PhysicsError::InvalidParticleConfiguration { field: "motion" });
        }
        Ok(())
    }
}

/// Cry particle-specific runtime status.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ParticleStatus {
    pub heading: Vec3,
    pub acceleration: Vec3,
    pub sliding: bool,
    pub slide_normal: Vec3,
    pub submerged_depth: f32,
    pub medium_velocity: Vec3,
    pub recent_collisions: u8,
    pub collision_pending: bool,
}

/// Backend capability for the state that is specific to Cry particle bodies.
pub trait PhysicsParticleBackend {
    /// Reads the particle-specific state produced by the last step.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered and
    /// [`PhysicsError::OperationRequiresParticleBody`] when it is not a Cry
    /// particle body.
    fn particle_status(&self, body: PhysicsBodyHandle) -> Result<ParticleStatus, PhysicsError>;

    /// Returns and clears Cry's one-bit `pe_status_collisions` latch.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered and
    /// [`PhysicsError::OperationRequiresParticleBody`] when it is not a Cry
    /// particle body.
    fn take_particle_collision(&mut self, body: PhysicsBodyHandle) -> Result<bool, PhysicsError>;
}

impl<B: PhysicsParticleBackend + ?Sized> PhysicsParticleBackend for Box<B> {
    fn particle_status(&self, body: PhysicsBodyHandle) -> Result<ParticleStatus, PhysicsError> {
        (**self).particle_status(body)
    }

    fn take_particle_collision(&mut self, body: PhysicsBodyHandle) -> Result<bool, PhysicsError> {
        (**self).take_particle_collision(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "these assert the exact default constants the native engine constructs, so an epsilon comparison would let a wrong constant pass"
    )]
    fn defaults_match_cry_particle_construction() {
        let particle = ParticleBodyConfiguration::default();
        assert_eq!(particle.mass, 0.2);
        assert_eq!(particle.size, 0.1);
        assert_eq!(particle.water_resistance, 0.5);
        assert_eq!(particle.minimum_bounce_speed, 1.5);
        assert_eq!(particle.area_check_period, 6);
        assert_eq!(particle.collision_class.type_mask, 1 << 6);
        assert!(particle.validate().is_ok());
    }
}
