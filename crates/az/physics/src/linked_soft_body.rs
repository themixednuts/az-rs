use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{
    CollisionClass, DeformableTargetVertices, PhysicalEntityTypes, PhysicsBodyHandle, PhysicsError,
    SurfaceIndex,
};

/// Aerodynamic force model used by `RockNRoll` linked soft bodies.
///
/// These discriminants and one/two-sided branches are part of the `RockNRoll`
/// contract, including their values and branch order.
#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum LinkedSoftBodyAerodynamics {
    #[default]
    None = 0,
    VertexPoint = 1,
    VertexTwoSidedLiftDrag = 2,
    VertexOneSidedLiftDrag = 3,
    FaceTwoSidedLiftDrag = 4,
    FaceOneSidedLiftDrag = 5,
}

impl TryFrom<u32> for LinkedSoftBodyAerodynamics {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::VertexPoint),
            2 => Ok(Self::VertexTwoSidedLiftDrag),
            3 => Ok(Self::VertexOneSidedLiftDrag),
            4 => Ok(Self::FaceTwoSidedLiftDrag),
            5 => Ok(Self::FaceOneSidedLiftDrag),
            _ => Err(value),
        }
    }
}

impl From<LinkedSoftBodyAerodynamics> for u32 {
    fn from(value: LinkedSoftBodyAerodynamics) -> Self {
        value as Self
    }
}

/// Feature representation selected for `RockNRoll` linked-soft-body contacts.
///
/// The solver selects one of three independently maintained AABB trees and
/// encodes the selected feature in the high two bits of each contact record.
/// The numeric values are therefore part of the native contract.
#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum LinkedSoftBodyCollisionFeature {
    #[default]
    Cluster = 0,
    Vertex = 1,
    Face = 2,
}

impl TryFrom<u32> for LinkedSoftBodyCollisionFeature {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Cluster),
            1 => Ok(Self::Vertex),
            2 => Ok(Self::Face),
            _ => Err(value),
        }
    }
}

impl From<LinkedSoftBodyCollisionFeature> for u32 {
    fn from(value: LinkedSoftBodyCollisionFeature) -> Self {
        value as Self
    }
}

/// How the optional ambient-medium velocity driver chooses its next value.
///
/// The native mode selects these exact branches. The names describe behavior
/// without guessing an authoring term.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum LinkedSoftBodyMediumVelocityMode {
    /// Interpolate to a uniformly randomized velocity inside the configured
    /// component-wise range.
    #[default]
    InterpolatedRandom = 0,
    /// Alternate immediately between the two configured endpoint velocities.
    AlternatingStep = 1,
}

impl From<bool> for LinkedSoftBodyMediumVelocityMode {
    fn from(alternating_step: bool) -> Self {
        if alternating_step {
            Self::AlternatingStep
        } else {
            Self::InterpolatedRandom
        }
    }
}

impl From<LinkedSoftBodyMediumVelocityMode> for bool {
    fn from(mode: LinkedSoftBodyMediumVelocityMode) -> Self {
        mode == LinkedSoftBodyMediumVelocityMode::AlternatingStep
    }
}

/// Time-varying ambient-medium velocity used by `RockNRoll` aerodynamics.
///
/// The implementation uses a 48-bit LCG, independently randomizes all three
/// velocity components, and randomizes every interval duration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LinkedSoftBodyMediumVelocityAnimation {
    pub mode: LinkedSoftBodyMediumVelocityMode,
    pub minimum_velocity: Vec3,
    pub maximum_velocity: Vec3,
    pub minimum_duration: f32,
    pub maximum_duration: f32,
}

impl Default for LinkedSoftBodyMediumVelocityAnimation {
    fn default() -> Self {
        Self {
            mode: LinkedSoftBodyMediumVelocityMode::InterpolatedRandom,
            minimum_velocity: Vec3::ZERO,
            maximum_velocity: Vec3::ZERO,
            minimum_duration: 1.0,
            maximum_duration: 1.0,
        }
    }
}

/// Per-vertex target envelope for a linked soft body.
///
/// The solver keeps a vertex inside a sphere of `maximum_distance` around the
/// target, then outside a second sphere of `minimum_distance` displaced along
/// the target normal. `normal_offset` is signed. These names describe the
/// proven geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LinkedSoftBodyVertexEnvelope {
    pub maximum_distance: f32,
    pub normal_offset: f32,
    pub minimum_distance: f32,
}

/// Aggregate collision feature used by `RockNRoll`'s linked soft body.
///
/// The first scalar multiplies a contact impulse before the solver accumulates
/// the aggregate linear correction. The member list is a bounded `u16` vector.
/// The Rust name describes the observed behavior.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LinkedSoftBodyClusterConfiguration {
    pub linear_impulse_scale: f32,
    pub vertices: Vec<u16>,
}

/// Solver-neutral product for `RockNRoll`'s linked triangular soft body.
///
/// Faces and both link families are independent inputs. The solver projects
/// both link families with the same equation but independent stiffness
/// coefficients. A builder must not infer either family from triangle edges
/// or merge them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LinkedSoftBodyConfiguration {
    pub vertices: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub faces: Vec<[u32; 3]>,
    pub links: Vec<[u32; 2]>,
    /// Second native link family. This name remains neutral instead of
    /// guessing that it represents bending links.
    pub secondary_links: Vec<[u32; 2]>,
    pub clusters: Vec<LinkedSoftBodyClusterConfiguration>,
    pub mass: f32,
    pub friction: f32,
    pub restitution: f32,
    pub link_stiffness_coefficient: f32,
    /// Stiffness for [`Self::secondary_links`].
    pub secondary_link_stiffness_coefficient: f32,
    /// Lower squared-length bound as a fraction of each link's rest squared
    /// length.
    ///
    /// A value of zero prevents stretch without resisting compression, while
    /// one constrains links to their rest length in both directions.
    pub minimum_link_length_squared_ratio: f32,
    pub maximum_link_solve_iterations: u32,
    pub vertex_envelopes: Vec<LinkedSoftBodyVertexEnvelope>,
    pub maximum_target_solve_iterations: u32,
    /// Deformable-to-deformable contact iteration cap.
    pub maximum_deformable_contact_solve_iterations: u32,
    /// Deformable-to-rigid contact iteration cap.
    pub maximum_rigid_contact_solve_iterations: u32,
    pub aerodynamics: LinkedSoftBodyAerodynamics,
    pub air_friction_lift: f32,
    pub air_friction_drag: f32,
    /// Rotates the aerodynamic reference velocity through the current pose
    /// frame before evaluating lift and drag.
    ///
    /// The name describes the behavior directly.
    pub aerodynamics_in_pose_space: bool,
    pub initial_medium_velocity: Vec3,
    pub medium_density: f32,
    pub medium_velocity_animation: Option<LinkedSoftBodyMediumVelocityAnimation>,
    /// Attraction toward target vertices supplied through
    /// [`LinkedSoftBodyApi::set_linked_soft_body_target_vertices`].
    ///
    /// The solver consumes this value only while a target buffer is active.
    pub target_position_coefficient: f32,
    /// Damping applied only to target-position attraction.
    ///
    /// This is deliberately not a generic velocity damping control. The
    /// solver reads it solely inside the target-attraction branch.
    pub target_position_damping_coefficient: f32,
    /// Fraction of the pose-frame delta applied per step. The solver
    /// interpolates both the frame quaternion and translation by this value.
    pub pose_matching_coefficient: f32,
    pub pressure_coefficient: f32,
    pub volume_maintenance_factor: f32,
    pub desired_volume: f32,
    /// Per-second projection rate toward the velocity-advanced rest pose.
    ///
    /// The solver clamps `rate * dt` to one before blending each predicted
    /// vertex.
    pub rest_pose_projection_rate: f32,
    /// Feature representation used for linked-soft-body contacts against
    /// rigid or scene geometry.
    pub rigid_collision_feature: LinkedSoftBodyCollisionFeature,
    /// Feature representation used for linked-soft-body pairs.
    pub deformable_collision_feature: LinkedSoftBodyCollisionFeature,
    /// Applies all cluster position corrections from an iteration
    /// simultaneously instead of mutating vertices cluster-by-cluster.
    pub accumulate_cluster_corrections: bool,
    /// Multiplier applied to world gravity by the external-force pass.
    pub gravity_factor: f32,
    pub minimum_energy: f32,
    /// Radius used by solver adapters for vertex/scene contacts.
    pub collision_radius: f32,
    pub collision_types: PhysicalEntityTypes,
    pub collision_class: CollisionClass,
    pub surface_index: SurfaceIndex,
}

impl Default for LinkedSoftBodyConfiguration {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            velocities: Vec::new(),
            faces: Vec::new(),
            links: Vec::new(),
            secondary_links: Vec::new(),
            clusters: Vec::new(),
            mass: 1.0,
            friction: 0.5,
            restitution: 0.0,
            link_stiffness_coefficient: 1.0,
            secondary_link_stiffness_coefficient: 1.0,
            minimum_link_length_squared_ratio: 0.0,
            maximum_link_solve_iterations: 10,
            vertex_envelopes: Vec::new(),
            maximum_target_solve_iterations: 10,
            maximum_deformable_contact_solve_iterations: 1,
            maximum_rigid_contact_solve_iterations: 1,
            aerodynamics: LinkedSoftBodyAerodynamics::None,
            air_friction_lift: 0.0,
            air_friction_drag: 0.0,
            aerodynamics_in_pose_space: false,
            initial_medium_velocity: Vec3::ZERO,
            medium_density: 1.2,
            medium_velocity_animation: None,
            target_position_coefficient: 0.0,
            target_position_damping_coefficient: 0.0,
            pose_matching_coefficient: 0.0,
            pressure_coefficient: 0.0,
            volume_maintenance_factor: 0.0,
            desired_volume: 0.0,
            rest_pose_projection_rate: 0.0,
            rigid_collision_feature: LinkedSoftBodyCollisionFeature::Vertex,
            deformable_collision_feature: LinkedSoftBodyCollisionFeature::Vertex,
            accumulate_cluster_corrections: false,
            gravity_factor: 1.0,
            minimum_energy: 0.0,
            collision_radius: 0.01,
            collision_types: PhysicalEntityTypes::STATIC
                | PhysicalEntityTypes::DYNAMIC
                | PhysicalEntityTypes::LIVING,
            collision_class: CollisionClass::new(1 << 4, 0),
            surface_index: SurfaceIndex(0),
        }
    }
}

impl LinkedSoftBodyConfiguration {
    /// Validates topology and every scalar consumed by the linked solver.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidLinkedSoftBodyConfiguration`] when the
    /// product cannot be materialized without guessing.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        self.validate_topology()?;
        self.validate_clusters()?;
        self.validate_faces_and_link_lengths()?;
        self.validate_vertex_envelopes()?;
        self.validate_dynamics()?;
        self.validate_medium_velocity_animation()?;
        self.validate_feature_requirements()
    }

    /// Checks vertex, velocity, and link arrays for finite values and in-range,
    /// non-self-referential indices.
    fn validate_topology(&self) -> Result<(), PhysicsError> {
        if self.vertices.len() < 2 || self.vertices.iter().any(|vertex| !vertex.is_finite()) {
            return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration { field: "vertices" });
        }
        if !self.velocities.is_empty()
            && (self.velocities.len() != self.vertices.len()
                || self.velocities.iter().any(|velocity| !velocity.is_finite()))
        {
            return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                field: "velocities",
            });
        }
        if self.links.is_empty()
            || self.links.iter().any(|link| {
                link[0] == link[1]
                    || link
                        .iter()
                        .any(|&index| index as usize >= self.vertices.len())
            })
        {
            return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration { field: "links" });
        }
        if self.secondary_links.iter().any(|link| {
            link[0] == link[1]
                || link
                    .iter()
                    .any(|&index| index as usize >= self.vertices.len())
        }) {
            return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                field: "secondary links",
            });
        }
        Ok(())
    }

    /// Checks that every cluster is non-empty, has a usable impulse scale, and
    /// lists each vertex at most once.
    fn validate_clusters(&self) -> Result<(), PhysicsError> {
        for cluster in &self.clusters {
            if cluster.vertices.is_empty()
                || !cluster.linear_impulse_scale.is_finite()
                || cluster.linear_impulse_scale < 0.0
            {
                return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration { field: "clusters" });
            }
            let mut seen = vec![false; self.vertices.len()];
            for &vertex in &cluster.vertices {
                let vertex = usize::from(vertex);
                if vertex >= self.vertices.len() || core::mem::replace(&mut seen[vertex], true) {
                    return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                        field: "cluster vertices",
                    });
                }
            }
        }
        Ok(())
    }

    /// Checks that faces are in range and non-degenerate and that no link joins
    /// two coincident vertices.
    fn validate_faces_and_link_lengths(&self) -> Result<(), PhysicsError> {
        for face in &self.faces {
            if face
                .iter()
                .any(|&index| index as usize >= self.vertices.len())
            {
                return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration { field: "faces" });
            }
            let [a, b, c] = face.map(|index| self.vertices[index as usize]);
            if (b - a).cross(c - a).length_squared() <= f32::EPSILON {
                return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                    field: "degenerate face",
                });
            }
        }
        for link in &self.links {
            if self.vertices[link[0] as usize].distance_squared(self.vertices[link[1] as usize])
                <= f32::EPSILON
            {
                return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                    field: "zero-length link",
                });
            }
        }
        for link in &self.secondary_links {
            if self.vertices[link[0] as usize].distance_squared(self.vertices[link[1] as usize])
                <= f32::EPSILON
            {
                return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                    field: "zero-length secondary link",
                });
            }
        }
        Ok(())
    }

    /// Checks that the optional per-vertex envelopes match the vertex count and
    /// hold finite, non-negative distances.
    fn validate_vertex_envelopes(&self) -> Result<(), PhysicsError> {
        if !self.vertex_envelopes.is_empty()
            && (self.vertex_envelopes.len() != self.vertices.len()
                || self.vertex_envelopes.iter().any(|envelope| {
                    !envelope.maximum_distance.is_finite()
                        || envelope.maximum_distance < 0.0
                        || !envelope.normal_offset.is_finite()
                        || !envelope.minimum_distance.is_finite()
                        || envelope.minimum_distance < 0.0
                }))
        {
            return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                field: "vertex envelopes",
            });
        }
        Ok(())
    }

    /// Checks every solver scalar and the coefficients that must stay within
    /// zero through one.
    fn validate_dynamics(&self) -> Result<(), PhysicsError> {
        for (field, value) in [
            ("mass", self.mass),
            ("friction", self.friction),
            ("restitution", self.restitution),
            (
                "link stiffness coefficient",
                self.link_stiffness_coefficient,
            ),
            (
                "secondary link stiffness coefficient",
                self.secondary_link_stiffness_coefficient,
            ),
            (
                "minimum link length squared ratio",
                self.minimum_link_length_squared_ratio,
            ),
            ("air friction lift", self.air_friction_lift),
            ("air friction drag", self.air_friction_drag),
            ("medium density", self.medium_density),
            (
                "target position coefficient",
                self.target_position_coefficient,
            ),
            (
                "target position damping coefficient",
                self.target_position_damping_coefficient,
            ),
            ("pose matching coefficient", self.pose_matching_coefficient),
            ("pressure coefficient", self.pressure_coefficient),
            ("volume maintenance factor", self.volume_maintenance_factor),
            ("desired volume", self.desired_volume),
            ("rest pose projection rate", self.rest_pose_projection_rate),
            ("minimum energy", self.minimum_energy),
            ("collision radius", self.collision_radius),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration { field });
            }
        }
        if self.mass == 0.0
            || !self.initial_medium_velocity.is_finite()
            || !self.gravity_factor.is_finite()
            || self.link_stiffness_coefficient > 1.0
            || self.secondary_link_stiffness_coefficient > 1.0
            || self.minimum_link_length_squared_ratio > 1.0
            || self.pose_matching_coefficient > 1.0
            || self.friction > 1.0
            || self.restitution > 1.0
        {
            return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration { field: "dynamics" });
        }
        Ok(())
    }

    /// Checks the optional medium-velocity animation's finite bounds and its
    /// positive, correctly ordered durations.
    fn validate_medium_velocity_animation(&self) -> Result<(), PhysicsError> {
        if let Some(animation) = self.medium_velocity_animation
            && (!animation.minimum_velocity.is_finite()
                || !animation.maximum_velocity.is_finite()
                || !animation.minimum_duration.is_finite()
                || !animation.maximum_duration.is_finite()
                || animation.minimum_duration <= 0.0
                || animation.maximum_duration < animation.minimum_duration)
        {
            return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                field: "medium velocity animation",
            });
        }
        Ok(())
    }

    /// Checks that the features selected by pressure, aerodynamics, and
    /// collision are actually present in the authored product.
    fn validate_feature_requirements(&self) -> Result<(), PhysicsError> {
        let requires_faces = self.pressure_coefficient > 0.0
            || self.volume_maintenance_factor > 0.0
            || !self.vertex_envelopes.is_empty()
            || matches!(
                self.aerodynamics,
                LinkedSoftBodyAerodynamics::FaceTwoSidedLiftDrag
                    | LinkedSoftBodyAerodynamics::FaceOneSidedLiftDrag
            );
        if requires_faces && self.faces.is_empty() {
            return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                field: "faces required by pressure or aerodynamics",
            });
        }
        if self.collision_types != PhysicalEntityTypes::NONE {
            for (field, feature) in [
                ("rigid collision feature", self.rigid_collision_feature),
                (
                    "deformable collision feature",
                    self.deformable_collision_feature,
                ),
            ] {
                let available = match feature {
                    LinkedSoftBodyCollisionFeature::Cluster => !self.clusters.is_empty(),
                    LinkedSoftBodyCollisionFeature::Vertex => !self.vertices.is_empty(),
                    LinkedSoftBodyCollisionFeature::Face => !self.faces.is_empty(),
                };
                if !available {
                    return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration { field });
                }
            }
        }
        Ok(())
    }
}

/// Owned linked-soft-body state for diagnostics and deformation consumers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LinkedSoftBodyStatus {
    pub vertices: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub faces: Vec<[u32; 3]>,
    pub awake: bool,
}

/// Borrowed linked-soft-body state for zero-copy inspection and rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkedSoftBodyStatusRef<'a> {
    pub vertices: &'a [Vec3],
    pub velocities: &'a [Vec3],
    pub normals: &'a [Vec3],
    pub faces: &'a [[u32; 3]],
    pub awake: bool,
}

impl From<LinkedSoftBodyStatusRef<'_>> for LinkedSoftBodyStatus {
    fn from(status: LinkedSoftBodyStatusRef<'_>) -> Self {
        Self {
            vertices: status.vertices.to_vec(),
            velocities: status.velocities.to_vec(),
            normals: status.normals.to_vec(),
            faces: status.faces.to_vec(),
            awake: status.awake,
        }
    }
}

/// Solver-neutral linked-soft-body status capability. The returned view
/// borrows the backend's existing buffers and performs no allocation or copy.
pub trait PhysicsLinkedSoftBodyBackend {
    /// Drives the linked soft body toward an authored target vertex set.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered,
    /// [`PhysicsError::OperationRequiresLinkedSoftBody`] when it is not a linked
    /// soft body, and [`PhysicsError::UnsupportedLinkedSoftBodyAction`] when the
    /// backend does not implement target vertices.
    fn set_linked_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        target: DeformableTargetVertices,
    ) -> Result<(), PhysicsError>;

    /// Borrows the backend's live vertex, velocity, normal, and face buffers.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered and
    /// [`PhysicsError::OperationRequiresLinkedSoftBody`] when it is not a linked
    /// soft body.
    fn linked_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
    ) -> Result<LinkedSoftBodyStatusRef<'_>, PhysicsError>;
}

impl<B: PhysicsLinkedSoftBodyBackend + ?Sized> PhysicsLinkedSoftBodyBackend for Box<B> {
    fn set_linked_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        target: DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        (**self).set_linked_soft_body_target_vertices(body, target)
    }

    fn linked_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
    ) -> Result<LinkedSoftBodyStatusRef<'_>, PhysicsError> {
        (**self).linked_soft_body_status(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aerodynamic_discriminants_are_stable() {
        for value in 0..=5 {
            let model = LinkedSoftBodyAerodynamics::try_from(value).unwrap();
            assert_eq!(u32::from(model), value);
        }
        assert!(LinkedSoftBodyAerodynamics::try_from(6).is_err());
    }

    #[test]
    fn collision_feature_discriminants_are_stable() {
        for value in 0..=2 {
            let feature = LinkedSoftBodyCollisionFeature::try_from(value).unwrap();
            assert_eq!(u32::from(feature), value);
        }
        assert!(LinkedSoftBodyCollisionFeature::try_from(3).is_err());
    }

    #[test]
    fn medium_velocity_animation_rejects_reversed_duration_range() {
        let configuration = LinkedSoftBodyConfiguration {
            vertices: vec![Vec3::ZERO, Vec3::X],
            links: vec![[0, 1]],
            medium_velocity_animation: Some(LinkedSoftBodyMediumVelocityAnimation {
                minimum_duration: 2.0,
                maximum_duration: 1.0,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(configuration.validate().is_err());
    }

    #[test]
    fn faces_and_links_remain_independent() {
        let configuration = LinkedSoftBodyConfiguration {
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)],
            faces: vec![[0, 1, 2]],
            links: vec![[0, 3]],
            ..Default::default()
        };
        assert!(configuration.validate().is_ok());
    }

    #[test]
    fn target_envelopes_require_matching_faces_and_vertices() {
        let configuration = LinkedSoftBodyConfiguration {
            vertices: vec![Vec3::ZERO, Vec3::X],
            links: vec![[0, 1]],
            vertex_envelopes: vec![LinkedSoftBodyVertexEnvelope::default(); 2],
            ..Default::default()
        };
        assert!(configuration.validate().is_err());
    }

    #[test]
    fn secondary_link_family_is_validated_independently() {
        let valid = LinkedSoftBodyConfiguration {
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            links: vec![[0, 1]],
            secondary_links: vec![[1, 2]],
            secondary_link_stiffness_coefficient: 0.25,
            ..Default::default()
        };
        assert!(valid.validate().is_ok());

        let invalid = LinkedSoftBodyConfiguration {
            secondary_links: vec![[2, 2]],
            ..valid
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn native_cluster_members_are_bounded_and_unique() {
        let valid = LinkedSoftBodyConfiguration {
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            links: vec![[0, 1]],
            clusters: vec![LinkedSoftBodyClusterConfiguration {
                linear_impulse_scale: 0.5,
                vertices: vec![0, 1, 2],
            }],
            ..Default::default()
        };
        assert!(valid.validate().is_ok());

        let invalid = LinkedSoftBodyConfiguration {
            clusters: vec![LinkedSoftBodyClusterConfiguration {
                linear_impulse_scale: 0.5,
                vertices: vec![1, 1],
            }],
            ..valid
        };
        assert!(invalid.validate().is_err());
    }
}
