use glam::Vec3;

use crate::{PhysicalEntityType, PhysicsBodyHandle, PhysicsConstraintHandle, PhysicsSceneId};

/// Errors shared by all physics backends.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PhysicsError {
    #[error("physics scene {0:?} does not exist")]
    SceneNotFound(PhysicsSceneId),

    #[error("physics body {0:?} does not exist")]
    BodyNotFound(PhysicsBodyHandle),

    #[error("physics body handle space is exhausted")]
    BodyHandleExhausted,

    #[error("physics constraint {0:?} does not exist")]
    ConstraintNotFound(PhysicsConstraintHandle),

    #[error("physics constraint handle space is exhausted")]
    ConstraintHandleExhausted,

    #[error("constraint parent scene {parent:?} does not match child scene {child:?}")]
    ConstraintSceneMismatch {
        parent: PhysicsSceneId,
        child: PhysicsSceneId,
    },

    #[error("constraint {field} is invalid")]
    InvalidConstraintConfiguration { field: &'static str },

    #[error("articulation {field} is invalid")]
    InvalidArticulationConfiguration { field: &'static str },

    #[error("articulation link {0} does not exist or has no joint")]
    ArticulationLinkNotFound(usize),

    #[error("particle {field} is invalid")]
    InvalidParticleConfiguration { field: &'static str },

    #[error("wheeled vehicle {field} is invalid")]
    InvalidVehicleConfiguration { field: &'static str },

    #[error("physics operation {operation} requires a wheeled vehicle body")]
    OperationRequiresVehicleBody { operation: &'static str },

    #[error("vehicle wheel {wheel} does not exist")]
    VehicleWheelNotFound { wheel: usize },

    #[error("physics operation {operation} requires a particle body")]
    OperationRequiresParticleBody { operation: &'static str },

    #[error("rope {field} is invalid")]
    InvalidRopeConfiguration { field: &'static str },

    #[error("physics operation {operation} requires a rope body")]
    OperationRequiresRopeBody { operation: &'static str },

    #[error("physics action {action} is not supported by a Cry rope body")]
    UnsupportedRopeAction { action: &'static str },

    #[error("soft body {field} is invalid")]
    InvalidSoftBodyConfiguration { field: &'static str },

    #[error("deformable target vertices or host pose are invalid")]
    InvalidDeformableTarget,

    #[error("physics operation {operation} requires a soft body")]
    OperationRequiresSoftBody { operation: &'static str },

    #[error("physics action {action} is not supported by a Cry soft body")]
    UnsupportedSoftBodyAction { action: &'static str },

    #[error("soft body vertex {vertex} does not exist")]
    SoftBodyVertexNotFound { vertex: u32 },

    #[error("linked soft body {field} is invalid")]
    InvalidLinkedSoftBodyConfiguration { field: &'static str },

    #[error("physics operation {operation} requires a linked soft body")]
    OperationRequiresLinkedSoftBody { operation: &'static str },

    #[error("physics action {action} is not supported by a linked soft body")]
    UnsupportedLinkedSoftBodyAction { action: &'static str },

    #[error("physics action {action} is not supported by a Cry particle body")]
    UnsupportedParticleAction { action: &'static str },

    #[error("time step must be finite and greater than zero, got {0}")]
    InvalidTimeStep(f32),

    #[error("ray direction must be finite and normalized")]
    InvalidRayDirection,

    #[error("ray maximum distance must be finite and non-negative, got {0}")]
    InvalidRayDistance(f32),

    #[error("ray maximum hit count must be greater than zero")]
    InvalidRayHitCount,

    #[error("spatial query maximum result count must be greater than zero")]
    InvalidSpatialQueryResultCount,

    #[error("spatial query bounds must be finite and have positive extent")]
    InvalidSpatialQueryBounds,

    #[error("collider {field} must be finite and greater than zero")]
    InvalidColliderScalar { field: &'static str },

    #[error("collider half extents must be finite and greater than zero on every axis")]
    InvalidColliderHalfExtents,

    #[error("collider topology is invalid: {0}")]
    InvalidColliderTopology(&'static str),

    #[error(
        "collider contact offset {contact_offset} must be finite, non-negative, and at least {minimum_delta} above finite rest offset {rest_offset}"
    )]
    InvalidColliderOffsets {
        rest_offset: f32,
        contact_offset: f32,
        minimum_delta: f32,
    },

    #[error("collider material coefficient {field} must be finite and non-negative")]
    InvalidMaterialCoefficient { field: &'static str },

    #[error("rigid-body {field} must be finite and non-negative")]
    InvalidRigidBodyScalar { field: &'static str },

    #[error("living-body {field} is invalid")]
    InvalidLivingConfiguration { field: &'static str },

    #[error("living-body stance requires unprojection {required:?}, exceeding maximum {maximum}")]
    LivingStanceBlocked { required: Vec3, maximum: f32 },

    #[error("character-controller {field} is invalid")]
    InvalidCharacterConfiguration { field: &'static str },

    #[error("fluid/buoyancy {field} is invalid")]
    InvalidFluidConfiguration { field: &'static str },

    #[error("body type {0:?} is not implemented by this backend")]
    UnsupportedEntityType(PhysicalEntityType),

    #[error("physics action {action} requires a living body")]
    ActionRequiresLivingBody { action: &'static str },

    #[error("physics operation {operation} requires a character-controller body")]
    OperationRequiresCharacterBody { operation: &'static str },

    #[error("static, rigid, and area bodies require at least one collider")]
    MissingCollider,

    #[error("physics backend invariant failed: {0}")]
    BackendInvariant(&'static str),
}
