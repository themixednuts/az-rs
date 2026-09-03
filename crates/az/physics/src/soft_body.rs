use bevy_reflect::Reflect;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::{
    ColliderConfiguration, ColliderShape, CollisionClass, DeformableTargetVertices,
    PhysicalEntityTypes, PhysicsBodyHandle, PhysicsError, SurfaceIndex,
};

/// Cry soft-body behavior bits. Values match `se_*` in `physinterface.h`.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct SoftBodyFlags(u32);

impl SoftBodyFlags {
    pub const NONE: Self = Self(0);
    pub const SKIP_LONGEST_EDGES: Self = Self(0x01);
    pub const RIGID_CORE: Self = Self(0x02);

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

impl From<u32> for SoftBodyFlags {
    fn from(bits: u32) -> Self {
        Self::from_bits(bits)
    }
}

impl From<SoftBodyFlags> for u32 {
    fn from(flags: SoftBodyFlags) -> Self {
        flags.bits()
    }
}

impl core::ops::BitOr for SoftBodyFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// One vertex attachment used by Cry `pe_action_attach_points`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct SoftBodyAttachment {
    pub vertex: u32,
    pub body: Option<PhysicsBodyHandle>,
    pub part_id: i32,
    pub point: Vec3,
    pub local: bool,
}

/// Authored geometry for Cry's optional colliding rigid core.
///
/// `contained_vertices` is the stable result of Cry's point-in-geometry test
/// against the tetrahedral lattice. The collider is the original geometry
/// attached to the native `PE_RIGID`; it is never inferred from those points.
/// Its mass must remain unset and its density zero because Cry derives core
/// mass from the contained fraction of the soft body's total mass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct SoftBodyRigidCoreConfiguration {
    pub contained_vertices: Vec<u32>,
    pub collider: ColliderConfiguration,
}

impl SoftBodyRigidCoreConfiguration {
    #[must_use]
    pub fn new(contained_vertices: impl Into<Vec<u32>>, shape: ColliderShape) -> Self {
        Self {
            contained_vertices: contained_vertices.into(),
            collider: ColliderConfiguration {
                shape,
                density: 0.0,
                mass: None,
                in_scene_queries: false,
                ..ColliderConfiguration::default()
            },
        }
    }
}

/// Complete Cry soft-body construction state (`pe_params_softbody` plus
/// simulation values owned by `CSoftEntity`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct SoftBodyConfiguration {
    pub vertices: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    /// Optional Cry tetrahedral lattice. When non-empty the body follows the
    /// volumetric `CSoftEntity` path; `triangles` remains the skinned/status
    /// surface while lattice edges drive simulation.
    pub tetrahedra: Vec<[u32; 4]>,
    /// Optional Cry rigid core. Offline builders perform Cry's point-in-core
    /// test and store both its stable lattice membership and source geometry.
    pub rigid_core: Option<SoftBodyRigidCoreConfiguration>,
    pub mass: f32,
    /// Geometry density supplied with Cry `pe_geomparams`. It is required for
    /// per-vertex displaced volume and therefore soft-body buoyancy.
    pub density: f32,
    pub thickness: f32,
    /// Maximum fractional edge stretch accepted during one integration step.
    /// This is Cry `pe_params_softbody::maxSafeStep`; it is dimensionless and
    /// does not cap the simulation time step.
    pub maximum_safe_step: f32,
    pub stiffness: f32,
    pub stretch_damping_ratio: f32,
    pub friction: f32,
    pub water_resistance: f32,
    pub air_resistance: f32,
    pub wind: Vec3,
    pub wind_variance: f32,
    pub maximum_iterations: u32,
    pub accuracy: f32,
    pub impulse_scale: f32,
    pub explosion_scale: f32,
    pub collision_impulse_scale: f32,
    pub maximum_collision_impulse: f32,
    pub collision_types: PhysicalEntityTypes,
    pub mass_decay: f32,
    pub normal_shape_stiffness: f32,
    pub tangential_shape_stiffness: f32,
    pub animation_stiffness: f32,
    pub animation_stiffness_decay: f32,
    pub animation_damping: f32,
    pub maximum_animation_distance: f32,
    pub host_space_simulation: f32,
    pub target_vertices: Vec<Vec3>,
    pub attachments: Vec<SoftBodyAttachment>,
    pub gravity: Vec3,
    pub damping: f32,
    pub minimum_energy: f32,
    pub maximum_time_step: f32,
    pub flags: SoftBodyFlags,
    pub collision_class: CollisionClass,
    pub surface_index: SurfaceIndex,
}

impl Default for SoftBodyConfiguration {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            velocities: Vec::new(),
            triangles: Vec::new(),
            tetrahedra: Vec::new(),
            rigid_core: None,
            mass: 1.0,
            density: 1_000.0,
            thickness: 0.04,
            maximum_safe_step: 0.2,
            stiffness: 10.0,
            stretch_damping_ratio: 0.0,
            friction: 0.0,
            water_resistance: 0.0,
            air_resistance: 0.0,
            wind: Vec3::ZERO,
            wind_variance: 0.2,
            maximum_iterations: 20,
            accuracy: 0.01,
            impulse_scale: 0.05,
            explosion_scale: 0.001,
            collision_impulse_scale: 1.0,
            maximum_collision_impulse: 3_000.0,
            collision_types: PhysicalEntityTypes::STATIC
                | PhysicalEntityTypes::DYNAMIC
                | PhysicalEntityTypes::LIVING,
            mass_decay: 0.0,
            normal_shape_stiffness: 0.0,
            tangential_shape_stiffness: 0.0,
            animation_stiffness: 0.0,
            animation_stiffness_decay: 0.0,
            animation_damping: 0.0,
            maximum_animation_distance: 0.0,
            host_space_simulation: 0.0,
            target_vertices: Vec::new(),
            attachments: Vec::new(),
            gravity: Vec3::new(0.0, 0.0, -9.8),
            damping: 0.0,
            minimum_energy: 0.01 * 0.01,
            maximum_time_step: 0.1,
            flags: SoftBodyFlags::SKIP_LONGEST_EDGES,
            collision_class: CollisionClass::new(1 << 4, 0),
            surface_index: SurfaceIndex(0),
        }
    }
}

impl SoftBodyConfiguration {
    /// Validates soft-body topology and every scalar/vector contract.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidSoftBodyConfiguration`] for invalid input.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        self.validate_mesh()?;
        self.validate_rigid_core()?;
        self.validate_per_vertex_arrays()?;
        self.validate_dynamics()?;
        for attachment in &self.attachments {
            if attachment.vertex as usize >= self.vertices.len() || !attachment.point.is_finite() {
                return Err(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "attachment",
                });
            }
        }
        Ok(())
    }

    /// Checks the surface mesh and optional tetrahedral lattice for finite,
    /// in-range, non-degenerate elements.
    fn validate_mesh(&self) -> Result<(), PhysicsError> {
        if self.vertices.len() < 3 || self.vertices.iter().any(|vertex| !vertex.is_finite()) {
            return Err(PhysicsError::InvalidSoftBodyConfiguration { field: "vertices" });
        }
        if self.triangles.is_empty()
            || self
                .triangles
                .iter()
                .flatten()
                .any(|&index| index as usize >= self.vertices.len())
        {
            return Err(PhysicsError::InvalidSoftBodyConfiguration { field: "triangles" });
        }
        for triangle in &self.triangles {
            let [a, b, c] = triangle.map(|index| self.vertices[index as usize]);
            if (b - a).cross(c - a).length_squared() <= f32::EPSILON {
                return Err(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "degenerate triangle",
                });
            }
        }
        for tetrahedron in &self.tetrahedra {
            if tetrahedron
                .iter()
                .any(|&index| index as usize >= self.vertices.len())
            {
                return Err(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "tetrahedral lattice",
                });
            }
            let vertices = tetrahedron.map(|index| self.vertices[index as usize]);
            let volume = (vertices[1] - vertices[0])
                .cross(vertices[2] - vertices[0])
                .dot(vertices[3] - vertices[0]);
            if volume.abs() <= f32::EPSILON {
                return Err(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "tetrahedral lattice",
                });
            }
        }
        Ok(())
    }

    /// Checks that the optional rigid core agrees with
    /// [`SoftBodyFlags::RIGID_CORE`] and carries a massless simulated collider.
    fn validate_rigid_core(&self) -> Result<(), PhysicsError> {
        match (
            self.rigid_core.as_ref(),
            self.flags.contains(SoftBodyFlags::RIGID_CORE),
        ) {
            (Some(core), true) => {
                if self.tetrahedra.is_empty()
                    || core.contained_vertices.is_empty()
                    || core
                        .contained_vertices
                        .iter()
                        .any(|&index| index as usize >= self.vertices.len())
                {
                    return Err(PhysicsError::InvalidSoftBodyConfiguration {
                        field: "rigid core vertices",
                    });
                }
                core.collider.validate()?;
                if core.collider.mass.is_some() || core.collider.density != 0.0 {
                    return Err(PhysicsError::InvalidSoftBodyConfiguration {
                        field: "rigid core mass",
                    });
                }
                if core.collider.sensor
                    || !core.collider.simulated
                    || core.collider.in_scene_queries
                {
                    return Err(PhysicsError::InvalidSoftBodyConfiguration {
                        field: "rigid core collider",
                    });
                }
            }
            (None, false) => {}
            _ => {
                return Err(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "rigid core flag",
                });
            }
        }
        Ok(())
    }

    /// Checks that the optional per-vertex velocity and target arrays match the
    /// vertex count and hold finite values.
    fn validate_per_vertex_arrays(&self) -> Result<(), PhysicsError> {
        if !self.velocities.is_empty()
            && (self.velocities.len() != self.vertices.len()
                || self.velocities.iter().any(|velocity| !velocity.is_finite()))
        {
            return Err(PhysicsError::InvalidSoftBodyConfiguration {
                field: "velocities",
            });
        }
        if !self.target_vertices.is_empty()
            && (self.target_vertices.len() != self.vertices.len()
                || self
                    .target_vertices
                    .iter()
                    .any(|vertex| !vertex.is_finite()))
        {
            return Err(PhysicsError::InvalidSoftBodyConfiguration {
                field: "target vertices",
            });
        }
        Ok(())
    }

    /// Checks every solver scalar, ratio, and medium vector.
    fn validate_dynamics(&self) -> Result<(), PhysicsError> {
        for (field, value) in [
            ("mass", self.mass),
            ("density", self.density),
            ("thickness", self.thickness),
            ("maximum safe step", self.maximum_safe_step),
            ("stretch damping ratio", self.stretch_damping_ratio),
            ("friction", self.friction),
            ("water resistance", self.water_resistance),
            ("air resistance", self.air_resistance),
            ("wind variance", self.wind_variance),
            ("accuracy", self.accuracy),
            ("impulse scale", self.impulse_scale),
            ("explosion scale", self.explosion_scale),
            ("collision impulse scale", self.collision_impulse_scale),
            ("maximum collision impulse", self.maximum_collision_impulse),
            ("normal shape stiffness", self.normal_shape_stiffness),
            (
                "tangential shape stiffness",
                self.tangential_shape_stiffness,
            ),
            ("animation stiffness", self.animation_stiffness),
            ("animation stiffness decay", self.animation_stiffness_decay),
            ("animation damping", self.animation_damping),
            (
                "maximum animation distance",
                self.maximum_animation_distance,
            ),
            ("damping", self.damping),
            ("minimum energy", self.minimum_energy),
            ("maximum time step", self.maximum_time_step),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidSoftBodyConfiguration { field });
            }
        }
        if self.mass == 0.0
            || self.density == 0.0
            || self.thickness == 0.0
            || self.maximum_time_step == 0.0
            || self.maximum_iterations == 0
            || self.accuracy == 0.0
            || !self.stiffness.is_finite()
            || !self.mass_decay.is_finite()
            || !self.host_space_simulation.is_finite()
            || !(0.0..=1.0).contains(&self.wind_variance)
            || !(0.0..=1.0).contains(&self.animation_stiffness_decay)
            || !(0.0..=1.0).contains(&self.host_space_simulation)
            || !self.gravity.is_finite()
            || !self.wind.is_finite()
        {
            return Err(PhysicsError::InvalidSoftBodyConfiguration { field: "dynamics" });
        }
        Ok(())
    }
}

/// Owned Cry `pe_action_attach_points` update. `body == None` means world;
/// an empty `attachments` slice is never inferred as a detach-all command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct SoftBodyAttachmentUpdate {
    pub body: Option<PhysicsBodyHandle>,
    pub part_id: i32,
    pub vertices: Vec<u32>,
    pub points: Vec<Vec3>,
    pub local: bool,
    pub attached: bool,
}

impl SoftBodyAttachmentUpdate {
    /// Validates vertex/point cardinality and finite attachment points.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidSoftBodyConfiguration`] for invalid input.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        if self.vertices.is_empty()
            || (!self.points.is_empty() && self.points.len() != self.vertices.len())
            || self.points.iter().any(|point| !point.is_finite())
        {
            return Err(PhysicsError::InvalidSoftBodyConfiguration {
                field: "attachment update",
            });
        }
        Ok(())
    }
}

/// Cry soft-body impulse.
///
/// `triangle` is `pe_action_impulse::ipart`; when it is absent the solver finds
/// the closest surface triangle to `point` (or to the current bounds center
/// when `point` is absent) and distributes the impulse with barycentric
/// weights.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct SoftBodyImpulse {
    pub impulse: Vec3,
    pub point: Option<Vec3>,
    pub triangle: Option<u32>,
}

impl SoftBodyImpulse {
    /// Validates finite impulse data.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidSoftBodyConfiguration`] for invalid input.
    pub fn validate(self) -> Result<(), PhysicsError> {
        if !self.impulse.is_finite() || self.point.is_some_and(|point| !point.is_finite()) {
            return Err(PhysicsError::InvalidSoftBodyConfiguration { field: "impulse" });
        }
        Ok(())
    }
}

/// Cry `CSoftEntity::ApplyVolumetricPressure` input. This is the soft-body
/// explosion path; it is deliberately separate from `pe_action_impulse`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct SoftBodyPressure {
    pub epicenter: Vec3,
    pub strength: f32,
    pub minimum_radius: f32,
}

/// Cry `pe_action_slice` cutter.
///
/// Cry accepts exactly three world-space points and passes the resulting
/// triangle to `CTriMesh::Slice` with a 0.01-unit vertex snap distance and a 5%
/// disconnected-island threshold.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct SoftBodySlice {
    pub triangle: [Vec3; 3],
}

impl SoftBodySlice {
    pub const MINIMUM_EDGE_LENGTH: f32 = 0.01;
    pub const MINIMUM_ISLAND_AREA_FRACTION: f32 = 0.05;

    /// Validates the finite, non-degenerate cutter triangle.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidSoftBodyConfiguration`] for an invalid
    /// cutter, matching Cry's refusal to run a non-triangular slice action.
    pub fn validate(self) -> Result<(), PhysicsError> {
        let [a, b, c] = self.triangle;
        if self.triangle.iter().any(|point| !point.is_finite())
            || (b - a).cross(c - a).length_squared() <= f32::EPSILON
        {
            return Err(PhysicsError::InvalidSoftBodyConfiguration {
                field: "slice triangle",
            });
        }
        Ok(())
    }
}

/// Topology change produced by one successful Cry soft-body slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub struct SoftBodySliceResult {
    pub added_vertices: u32,
    pub removed_triangles: u32,
    pub added_triangles: u32,
    pub removed_islands: u32,
}

impl SoftBodyPressure {
    /// Validates finite pressure input.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidSoftBodyConfiguration`] for invalid input.
    pub fn validate(self) -> Result<(), PhysicsError> {
        if !self.epicenter.is_finite()
            || !self.strength.is_finite()
            || !self.minimum_radius.is_finite()
            || self.minimum_radius < 0.0
        {
            return Err(PhysicsError::InvalidSoftBodyConfiguration {
                field: "volumetric pressure",
            });
        }
        Ok(())
    }
}

/// Owned snapshot of Cry `pe_status_softvtx`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct SoftBodyStatus {
    pub vertices: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub vertex_map: Vec<u32>,
    pub triangles: Vec<[u32; 3]>,
    pub host_position: Vec3,
    pub host_rotation: Quat,
    pub position: Vec3,
    pub rotation: Quat,
    pub awake: bool,
}

/// Solver-neutral Cry soft-body command/status capability.
pub trait PhysicsSoftBodyBackend {
    /// Drives the soft body toward an authored target vertex set.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresSoftBody`] when it is not a soft body,
    /// and [`PhysicsError::UnsupportedSoftBodyAction`] when the backend's soft
    /// body does not implement target vertices.
    fn set_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError>;

    /// Attaches or detaches the listed vertices.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` or the update's
    /// attachment body is not registered,
    /// [`PhysicsError::OperationRequiresSoftBody`] when `body` is not a soft
    /// body, and [`PhysicsError::SoftBodyVertexNotFound`] when a listed vertex
    /// is out of range.
    fn update_soft_body_attachments(
        &mut self,
        body: PhysicsBodyHandle,
        update: &SoftBodyAttachmentUpdate,
    ) -> Result<(), PhysicsError>;

    /// Applies one impulse to the soft-body surface.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresSoftBody`] when it is not a soft body,
    /// and [`PhysicsError::SoftBodyVertexNotFound`] when the impulse names a
    /// triangle the mesh does not contain.
    fn apply_soft_body_impulse(
        &mut self,
        body: PhysicsBodyHandle,
        impulse: SoftBodyImpulse,
    ) -> Result<(), PhysicsError>;

    /// Runs Cry's soft-body explosion (volumetric pressure) path.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresSoftBody`] when it is not a soft body,
    /// and [`PhysicsError::UnsupportedSoftBodyAction`] when the backend's soft
    /// body does not implement volumetric pressure.
    fn apply_soft_body_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: SoftBodyPressure,
    ) -> Result<(), PhysicsError>;

    /// Cuts the soft-body mesh with a world-space cutter triangle, reporting the
    /// resulting topology change when the cut connects.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresSoftBody`] when it is not a soft body,
    /// and [`PhysicsError::UnsupportedSoftBodyAction`] when the backend's soft
    /// body does not implement slicing.
    fn slice_soft_body(
        &mut self,
        body: PhysicsBodyHandle,
        slice: SoftBodySlice,
    ) -> Result<Option<SoftBodySliceResult>, PhysicsError>;

    /// Writes the soft body's `pe_status_softvtx` snapshot into caller-owned
    /// storage.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered and
    /// [`PhysicsError::OperationRequiresSoftBody`] when it is not a soft body.
    fn write_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut SoftBodyStatus,
    ) -> Result<(), PhysicsError>;
}

impl<B: PhysicsSoftBodyBackend + ?Sized> PhysicsSoftBodyBackend for Box<B> {
    fn set_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        (**self).set_soft_body_target_vertices(body, action)
    }

    fn update_soft_body_attachments(
        &mut self,
        body: PhysicsBodyHandle,
        update: &SoftBodyAttachmentUpdate,
    ) -> Result<(), PhysicsError> {
        (**self).update_soft_body_attachments(body, update)
    }

    fn apply_soft_body_impulse(
        &mut self,
        body: PhysicsBodyHandle,
        impulse: SoftBodyImpulse,
    ) -> Result<(), PhysicsError> {
        (**self).apply_soft_body_impulse(body, impulse)
    }

    fn apply_soft_body_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: SoftBodyPressure,
    ) -> Result<(), PhysicsError> {
        (**self).apply_soft_body_pressure(body, pressure)
    }

    fn slice_soft_body(
        &mut self,
        body: PhysicsBodyHandle,
        slice: SoftBodySlice,
    ) -> Result<Option<SoftBodySliceResult>, PhysicsError> {
        (**self).slice_soft_body(body, slice)
    }

    fn write_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut SoftBodyStatus,
    ) -> Result<(), PhysicsError> {
        (**self).write_soft_body_status(body, output)
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
    fn defaults_match_cry_soft_entity_construction() {
        let soft = SoftBodyConfiguration::default();
        assert_eq!(soft.maximum_time_step, 0.1);
        assert_eq!(soft.stiffness, 10.0);
        assert_eq!(soft.thickness, 0.04);
        assert_eq!(soft.maximum_safe_step, 0.2);
        assert_eq!(soft.maximum_iterations, 20);
        assert_eq!(soft.accuracy, 0.01);
        assert_eq!(soft.impulse_scale, 0.05);
        assert_eq!(soft.explosion_scale, 0.001);
        assert_eq!(soft.maximum_collision_impulse, 3_000.0);
        assert!(soft.flags.contains(SoftBodyFlags::SKIP_LONGEST_EDGES));
    }
}
