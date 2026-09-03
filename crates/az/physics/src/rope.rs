use bevy_reflect::Reflect;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::{
    CollisionClass, DeformableTargetVertices, PhysicalEntityTypes, PhysicsBodyHandle, PhysicsError,
    SurfaceIndex,
};

/// Cry rope behavior bits. Values match `rope_*` in `physinterface.h`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct RopeFlags(u32);

impl RopeFlags {
    pub const NONE: Self = Self(0);
    pub const FINITE_DIFFERENCE_ATTACHED_VELOCITY: Self = Self(0x01);
    pub const NO_VELOCITY_SOLVER: Self = Self(0x02);
    pub const IGNORE_ATTACHMENTS: Self = Self(0x04);
    pub const TARGET_VERTEX_RELATIVE_TO_START: Self = Self(0x08);
    pub const TARGET_VERTEX_RELATIVE_TO_END: Self = Self(0x10);
    pub const COLLIDES_WITH_ATTACHMENT: Self = Self(0x80);
    pub const SUBDIVIDE_SEGMENTS: Self = Self(0x100);
    pub const NO_TEARS: Self = Self(0x200);
    pub const TRACEABLE: Self = Self(0x400);
    pub const COLLIDES: Self = Self(0x20_0000);
    pub const COLLIDES_WITH_TERRAIN: Self = Self(0x40_0000);
    pub const NO_STIFFNESS_WHEN_COLLIDING: Self = Self(0x1000_0000);

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

impl From<u32> for RopeFlags {
    fn from(bits: u32) -> Self {
        Self::from_bits(bits)
    }
}

impl From<RopeFlags> for u32 {
    fn from(flags: RopeFlags) -> Self {
        flags.bits()
    }
}

impl core::ops::BitOr for RopeFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for RopeFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Cry target-pose modes stored by `pe_params_rope::bTargetPoseActive`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum RopeTargetPoseMode {
    #[default]
    Disabled = 0,
    DirectVertexPull = 1,
    JointTorque = 2,
}

/// A rope endpoint tied to the world or to a physics body.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RopeAttachment {
    pub body: Option<PhysicsBodyHandle>,
    pub part_id: i32,
    pub point: Vec3,
    pub local: bool,
}

/// Optional per-segment values used by Cry touch-bending ropes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RopeSegmentConfiguration {
    pub damping: Option<f32>,
    pub stiffness: Option<f32>,
    pub thickness: Option<f32>,
}

/// Complete Cry rope construction state (`pe_params_rope` plus simulation
/// parameters owned by `CRopeEntity`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RopeBodyConfiguration {
    pub points: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub target_length: f32,
    pub mass: f32,
    pub collision_distance: f32,
    pub surface_index: SurfaceIndex,
    pub friction: f32,
    pub pull_friction: f32,
    pub stiffness: f32,
    pub animation_stiffness: f32,
    pub animation_stiffness_decay: f32,
    pub animation_damping: f32,
    pub target_pose_mode: RopeTargetPoseMode,
    pub target_points: Vec<Vec3>,
    pub wind: Vec3,
    pub wind_variance: f32,
    pub air_resistance: f32,
    pub water_resistance: f32,
    pub density: f32,
    pub joint_limit: f32,
    pub joint_limit_decay: f32,
    pub sensor_radius: f32,
    pub maximum_force: f32,
    pub penalty_scale: f32,
    pub attachment_zone: f32,
    pub minimum_segment_length: f32,
    pub unprojection_limit: f32,
    pub no_collision_distance: f32,
    pub maximum_iterations: u32,
    pub collider_flags: u32,
    pub collision_types: PhysicalEntityTypes,
    pub maximum_subdivision_vertices: u32,
    pub collision_bounds: Option<[Vec3; 2]>,
    pub hinge_axis: Option<Vec3>,
    pub attachments: [Option<RopeAttachment>; 2],
    pub segments: Vec<RopeSegmentConfiguration>,
    pub flags: RopeFlags,
    pub gravity: Option<Vec3>,
    pub damping: f32,
    pub minimum_energy: f32,
    pub maximum_time_step: f32,
    pub collision_class: CollisionClass,
}

impl Default for RopeBodyConfiguration {
    fn default() -> Self {
        Self {
            points: vec![Vec3::ZERO, Vec3::NEG_Z],
            velocities: Vec::new(),
            target_length: 0.0,
            mass: 1.0,
            collision_distance: 0.01,
            surface_index: SurfaceIndex(0),
            friction: 0.2,
            pull_friction: 0.0,
            stiffness: 10.0,
            animation_stiffness: 70.0,
            animation_stiffness_decay: 0.75,
            animation_damping: 0.0,
            target_pose_mode: RopeTargetPoseMode::Disabled,
            target_points: Vec::new(),
            wind: Vec3::ZERO,
            wind_variance: 0.0,
            air_resistance: 0.0,
            water_resistance: 5.0,
            density: 500.0,
            joint_limit: 0.0,
            joint_limit_decay: 0.0,
            sensor_radius: 0.05,
            maximum_force: 0.0,
            penalty_scale: 2.0,
            attachment_zone: 0.0,
            minimum_segment_length: 0.0,
            unprojection_limit: 0.5,
            no_collision_distance: 0.5,
            maximum_iterations: 650,
            collider_flags: 0,
            collision_types: PhysicalEntityTypes::NONE,
            maximum_subdivision_vertices: 3,
            collision_bounds: None,
            hinge_axis: None,
            attachments: [None, None],
            segments: Vec::new(),
            flags: RopeFlags::NONE,
            gravity: None,
            damping: 0.2,
            minimum_energy: 0.04 * 0.04,
            maximum_time_step: 0.05,
            collision_class: CollisionClass::new(1 << 5, 0),
        }
    }
}

impl RopeBodyConfiguration {
    /// Validates rope topology and every scalar/vector contract.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidRopeConfiguration`] for invalid input.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        if self.points.len() < 2 || self.points.iter().any(|point| !point.is_finite()) {
            return Err(PhysicsError::InvalidRopeConfiguration { field: "points" });
        }
        let segment_count = self.points.len() - 1;
        if !self.velocities.is_empty()
            && (self.velocities.len() != self.points.len()
                || self.velocities.iter().any(|velocity| !velocity.is_finite()))
        {
            return Err(PhysicsError::InvalidRopeConfiguration {
                field: "velocities",
            });
        }
        if !self.segments.is_empty() && self.segments.len() != segment_count {
            return Err(PhysicsError::InvalidRopeConfiguration {
                field: "segment properties",
            });
        }
        if !self.target_points.is_empty()
            && (self.target_points.len() != self.points.len()
                || self.target_points.iter().any(|point| !point.is_finite()))
        {
            return Err(PhysicsError::InvalidRopeConfiguration {
                field: "target points",
            });
        }
        for (field, value) in [
            ("target length", self.target_length),
            ("mass", self.mass),
            ("collision distance", self.collision_distance),
            ("friction", self.friction),
            ("pull friction", self.pull_friction),
            ("stiffness", self.stiffness),
            ("animation stiffness", self.animation_stiffness),
            ("animation stiffness decay", self.animation_stiffness_decay),
            ("animation damping", self.animation_damping),
            ("wind variance", self.wind_variance),
            ("air resistance", self.air_resistance),
            ("water resistance", self.water_resistance),
            ("density", self.density),
            ("joint limit", self.joint_limit),
            ("sensor radius", self.sensor_radius),
            ("maximum force", self.maximum_force),
            ("penalty scale", self.penalty_scale),
            ("attachment zone", self.attachment_zone),
            ("minimum segment length", self.minimum_segment_length),
            ("unprojection limit", self.unprojection_limit),
            ("no collision distance", self.no_collision_distance),
            ("damping", self.damping),
            ("minimum energy", self.minimum_energy),
            ("maximum time step", self.maximum_time_step),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidRopeConfiguration { field });
            }
        }
        if self.mass == 0.0
            || self.density == 0.0
            || self.maximum_time_step == 0.0
            || self.maximum_iterations == 0
            || !self.wind.is_finite()
            || !self.joint_limit_decay.is_finite()
            || self.animation_stiffness_decay > 1.0
            || self.wind_variance > 1.0
            || self.no_collision_distance > 1.0
            || self.hinge_axis.is_some_and(|axis| {
                !axis.is_finite() || (axis.length_squared() - 1.0).abs() > 1.0e-4
            })
        {
            return Err(PhysicsError::InvalidRopeConfiguration { field: "dynamics" });
        }
        for attachment in self.attachments.into_iter().flatten() {
            if !attachment.point.is_finite() {
                return Err(PhysicsError::InvalidRopeConfiguration {
                    field: "attachment",
                });
            }
        }
        for segment in &self.segments {
            for (field, value) in [
                ("segment damping", segment.damping),
                ("segment stiffness", segment.stiffness),
                ("segment thickness", segment.thickness),
            ] {
                if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
                    return Err(PhysicsError::InvalidRopeConfiguration { field });
                }
            }
        }
        Ok(())
    }
}

/// Owned snapshot of Cry `pe_status_rope`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RopeStatus {
    pub points: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub contact_normals: Vec<Vec3>,
    pub contact_bodies: Vec<Option<PhysicsBodyHandle>>,
    pub static_contacts: u32,
    pub dynamic_contacts: u32,
    pub target_pose_mode: RopeTargetPoseMode,
    pub animation_stiffness: f32,
    pub strained: bool,
    pub subdivided_vertices: Vec<Vec3>,
    pub time_last_active: f32,
    pub host_position: Vec3,
    pub host_rotation: Quat,
    pub torn: bool,
}

/// Cry `CRopeEntity::ApplyVolumetricPressure` input.
///
/// This is the blast-pressure tear test used by the legacy physics world; it
/// is not a continuously applied rope force.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RopeVolumetricPressure {
    pub epicenter: Vec3,
    pub pressure_scale: f32,
    pub minimum_radius: f32,
}

impl RopeVolumetricPressure {
    /// Rejects blast parameters the tear test cannot evaluate.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidRopeConfiguration`] with field
    /// `volumetric pressure` when the epicenter is non-finite or either
    /// `pressure_scale` or `minimum_radius` is non-finite or negative.
    pub fn validate(self) -> Result<(), PhysicsError> {
        if !self.epicenter.is_finite()
            || !self.pressure_scale.is_finite()
            || self.pressure_scale < 0.0
            || !self.minimum_radius.is_finite()
            || self.minimum_radius < 0.0
        {
            return Err(PhysicsError::InvalidRopeConfiguration {
                field: "volumetric pressure",
            });
        }
        Ok(())
    }
}

/// Solver-neutral Cry rope command/status capability.
pub trait PhysicsRopeBackend {
    /// Drives the rope toward an authored target vertex set.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresRopeBody`] when it is not a rope, and
    /// [`PhysicsError::UnsupportedRopeAction`] when the backend's rope does not
    /// implement target vertices.
    fn set_rope_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError>;

    /// Re-reads the host transform after an attachment entity moved.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresRopeBody`] when it is not a rope, and
    /// [`PhysicsError::UnsupportedRopeAction`] when the backend's rope has no
    /// attachment resynchronization path.
    fn notify_rope_attachment_moved(&mut self, body: PhysicsBodyHandle)
    -> Result<(), PhysicsError>;

    /// Runs Cry's one-shot blast-pressure tear test on the rope.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresRopeBody`] when it is not a rope, and
    /// [`PhysicsError::UnsupportedRopeAction`] when the backend's rope does not
    /// implement volumetric pressure.
    fn apply_rope_volumetric_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: RopeVolumetricPressure,
    ) -> Result<(), PhysicsError>;

    /// Writes the rope's `pe_status_rope` snapshot into caller-owned storage.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered and
    /// [`PhysicsError::OperationRequiresRopeBody`] when it is not a rope.
    fn write_rope_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut RopeStatus,
    ) -> Result<(), PhysicsError>;
}

impl<B: PhysicsRopeBackend + ?Sized> PhysicsRopeBackend for Box<B> {
    fn set_rope_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        (**self).set_rope_target_vertices(body, action)
    }

    fn notify_rope_attachment_moved(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<(), PhysicsError> {
        (**self).notify_rope_attachment_moved(body)
    }

    fn apply_rope_volumetric_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: RopeVolumetricPressure,
    ) -> Result<(), PhysicsError> {
        (**self).apply_rope_volumetric_pressure(body, pressure)
    }

    fn write_rope_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut RopeStatus,
    ) -> Result<(), PhysicsError> {
        (**self).write_rope_status(body, output)
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
    fn defaults_match_cry_rope_construction() {
        let rope = RopeBodyConfiguration::default();
        assert_eq!(rope.mass, 1.0);
        assert_eq!(rope.collision_distance, 0.01);
        assert_eq!(rope.friction, 0.2);
        assert_eq!(rope.animation_stiffness, 70.0);
        assert_eq!(rope.water_resistance, 5.0);
        assert_eq!(rope.density, 500.0);
        assert_eq!(rope.maximum_iterations, 650);
        assert_eq!(rope.maximum_subdivision_vertices, 3);
        assert!(rope.validate().is_ok());
    }
}
