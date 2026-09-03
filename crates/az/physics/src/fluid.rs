use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::PhysicsError;

/// Cry area medium discriminator used by `pe_params_buoyancy::iMedium`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum FluidMedium {
    #[default]
    Water = 0,
    Air = 1,
}

/// Oriented world-space surface plane. The submerged half-space is the side
/// for which `(point - origin).dot(normal) <= 0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct FluidPlane {
    pub normal: Vec3,
    pub origin: Vec3,
}

impl FluidPlane {
    /// Rejects planes whose normal is not a finite unit vector.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidFluidConfiguration`] with field `plane`
    /// when the normal is non-finite, its length deviates from one by more than
    /// `1.0e-4`, or the origin is non-finite.
    pub fn validate(self) -> Result<(), PhysicsError> {
        if !self.normal.is_finite()
            || (self.normal.length_squared() - 1.0).abs() > 1.0e-4
            || !self.origin.is_finite()
        {
            return Err(PhysicsError::InvalidFluidConfiguration { field: "plane" });
        }
        Ok(())
    }

    #[inline]
    #[must_use]
    pub fn signed_distance(self, point: Vec3) -> f32 {
        (point - self.origin).dot(self.normal)
    }
}

impl Default for FluidPlane {
    fn default() -> Self {
        Self {
            normal: Vec3::Z,
            origin: Vec3::ZERO,
        }
    }
}

/// One Cry buoyancy/medium area. The body's AABB must overlap the area's
/// sensor collider before this plane and medium are considered.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct FluidAreaConfiguration {
    pub medium: FluidMedium,
    pub plane: FluidPlane,
    pub flow: Vec3,
    pub density: f32,
    pub resistance: f32,
    pub damping: f32,
}

impl FluidAreaConfiguration {
    /// Rejects area parameters the solver cannot integrate.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`FluidPlane::validate`], then returns
    /// [`PhysicsError::InvalidFluidConfiguration`] with field `flow` for a
    /// non-finite flow vector, or with field `density`, `resistance`, or
    /// `damping` for a coefficient that is non-finite or negative.
    pub fn validate(self) -> Result<(), PhysicsError> {
        self.plane.validate()?;
        if !self.flow.is_finite() {
            return Err(PhysicsError::InvalidFluidConfiguration { field: "flow" });
        }
        for (field, value) in [
            ("density", self.density),
            ("resistance", self.resistance),
            ("damping", self.damping),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidFluidConfiguration { field });
            }
        }
        Ok(())
    }
}

impl Default for FluidAreaConfiguration {
    fn default() -> Self {
        Self {
            medium: FluidMedium::Water,
            plane: FluidPlane::default(),
            flow: Vec3::ZERO,
            density: 1_000.0,
            resistance: 1_000.0,
            damping: 0.0,
        }
    }
}

/// Per-rigid-body multipliers set by `LmbrCentral`'s buoyancy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RigidBodyBuoyancy {
    pub density_scale: f32,
    pub resistance_scale: f32,
    pub damping: f32,
}

impl RigidBodyBuoyancy {
    /// Rejects per-body buoyancy multipliers the solver cannot integrate.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidFluidConfiguration`] with field
    /// `buoyancy_density`, `buoyancy_resistance`, or `buoyancy_damping` when the
    /// matching multiplier is non-finite or negative.
    pub fn validate(self) -> Result<(), PhysicsError> {
        for (field, value) in [
            ("buoyancy_density", self.density_scale),
            ("buoyancy_resistance", self.resistance_scale),
            ("buoyancy_damping", self.damping),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidFluidConfiguration { field });
            }
        }
        Ok(())
    }
}

impl Default for RigidBodyBuoyancy {
    fn default() -> Self {
        Self {
            density_scale: 1.0,
            resistance_scale: 1.0,
            damping: 0.0,
        }
    }
}

/// Runtime result of the last medium pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct BuoyancyStatus {
    pub submerged_fraction: f32,
    pub floating: bool,
}
