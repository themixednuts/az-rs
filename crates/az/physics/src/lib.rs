//! Backend-neutral runtime physics contracts.
//!
//! The public surface models `CryPhysics` actions and statuses together with the
//! scene/body/query split used by O3DE. Concrete solvers live in adapter crates;
//! solver handles and math types never leak through this API.

mod articulated;
mod body;
mod character;
mod collision;
mod constraint;
mod deformable;
mod error;
mod fluid;
mod linked_soft_body;
mod living;
mod material;
mod particle;
mod query;
mod rigid;
mod rope;
mod scene;
mod soft_body;
mod time;
mod vehicle;

pub use articulated::{
    ArticulatedBodyConfiguration, ArticulatedJointConfiguration, ArticulatedJointStatus,
    ArticulatedJointTarget, ArticulatedLyingMode, ArticulatedSimulationMode, Articulation,
    ArticulationDescriptor, ArticulationGrounding, ArticulationLinkDescriptor,
};
pub use body::{
    Axis3, BodyDescriptor, BodyKind, BodyStatus, CONTINUOUS_HIT_PROJECTION_EPSILON,
    CharacterBodyConfiguration, ColliderConfiguration, ColliderShape, ColliderTag,
    ContinuousCollisionMode, ContinuousHitProjection, PhysicalEntityType, PhysicalEntityTypes,
    PhysicsBodyError, PhysicsBodyHandle, PhysicsColliderSet, PhysicsEntityId, PhysicsMeshGeometry,
    PhysicsSceneId, PhysicsShapeInstance, PhysicsShapeSet, QueryBodyConfiguration,
    RigidBodyConfiguration, RigidBodyDampingModel, RigidBodyMotion, RigidBodySleepPolicy,
    RockNRollSleepMode, SimulationClass, SurfaceIndex, continuous_hit_projection,
};
pub use character::{CharacterControllerCommands, CharacterSupportInfo, CharacterSupportState};
pub use collision::{
    COLLISION_CATEGORY_WORDS, CollisionCategoryMask, CollisionClass, CollisionFilter,
    PhysicsInteraction, PhysicsInteractionKind, PhysicsInteractionPhase,
};
pub use constraint::{
    ConstraintAxes, ConstraintAxis, ConstraintAxisConfiguration, ConstraintAxisMask,
    ConstraintBreakReason, ConstraintCoupling, ConstraintDescriptor, ConstraintDrive,
    ConstraintLimit, ConstraintMotion, ConstraintSolverModel, ConstraintStatus, ConstraintTarget,
    DiagnosticOnlyRockNRollConstraintFamily, PhysicsConstraintBackend, PhysicsConstraintHandle,
};
pub use deformable::DeformableTargetVertices;
pub use error::PhysicsError;
pub use fluid::{
    BuoyancyStatus, FluidAreaConfiguration, FluidMedium, FluidPlane, RigidBodyBuoyancy,
};
pub use linked_soft_body::{
    LinkedSoftBodyAerodynamics, LinkedSoftBodyClusterConfiguration, LinkedSoftBodyCollisionFeature,
    LinkedSoftBodyConfiguration, LinkedSoftBodyMediumVelocityAnimation,
    LinkedSoftBodyMediumVelocityMode, LinkedSoftBodyStatus, LinkedSoftBodyStatusRef,
    LinkedSoftBodyVertexEnvelope, PhysicsLinkedSoftBodyBackend,
};
pub use living::{
    ImpulseAction, LivingBodyCommands, LivingBodyConfiguration, LivingBodyQueries,
    LivingDimensions, LivingDynamics, LivingMoveAction, LivingMoveMode, LivingStanceCheck,
    LivingStatus, PhysicsAction, SyncLivingAction,
};
pub use material::{CollisionFilterRegistry, PhysicsMaterial, PhysicsMaterialRegistry};
pub use particle::{
    ParticleBodyConfiguration, ParticleFlags, ParticleStatus, PhysicsParticleBackend,
};
pub use query::{
    AabbQuery, OverlapHit, RayCastConfiguration, RayCastHit, RayCastResult, ShapeCastConfiguration,
    ShapeCastHit, ShapeOverlapConfiguration, SpatialQueryFilter,
};
pub use rigid::{RigidBodyCommands, RigidBodyQueries};
pub use rope::{
    PhysicsRopeBackend, RopeAttachment, RopeBodyConfiguration, RopeFlags, RopeSegmentConfiguration,
    RopeStatus, RopeTargetPoseMode, RopeVolumetricPressure,
};
pub use scene::{PhysicsBackend, PhysicsBackendFactory, PhysicsScene, PhysicsWorld};
pub use soft_body::{
    PhysicsSoftBodyBackend, SoftBodyAttachment, SoftBodyAttachmentUpdate, SoftBodyConfiguration,
    SoftBodyFlags, SoftBodyImpulse, SoftBodyPressure, SoftBodyRigidCoreConfiguration,
    SoftBodySlice, SoftBodySliceResult, SoftBodyStatus,
};
pub use time::{
    PhysicsStepConfiguration, PhysicsStepReport, PhysicsSubsteps, PhysicsTimeStepMode,
    PhysicsTimeStepSelector,
};
pub use vehicle::{
    PhysicsVehicleBackend, VehicleAbilities, VehicleDriveAction, VehicleStatus,
    VehicleWheelConfiguration, VehicleWheelStatus, WheeledVehicleConfiguration,
};

/// Cross-gem ordering for authored components, backend bodies, and simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, bevy_ecs::schedule::SystemSet)]
pub enum PhysicsSet {
    Authoring,
    Materialization,
    /// Synchronize authored static/kinematic poses before gameplay forces.
    PoseSync,
    /// Deterministic pre-solve forces and impulses (buoyancy, force volumes,
    /// gameplay forces) after bodies exist and before the solver advances.
    Forces,
    Step,
    Interactions,
    Writeback,
}

/// World-space rigid transform used at the solver boundary.
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, bevy_reflect::Reflect,
)]
pub struct PhysicsPose {
    pub translation: glam::Vec3,
    pub rotation: glam::Quat,
}

impl PhysicsPose {
    pub const IDENTITY: Self = Self {
        translation: glam::Vec3::ZERO,
        rotation: glam::Quat::IDENTITY,
    };

    #[inline]
    #[must_use]
    pub fn transform_point(self, point: glam::Vec3) -> glam::Vec3 {
        self.rotation * point + self.translation
    }

    #[inline]
    #[must_use]
    pub fn inverse(self) -> Self {
        let rotation = self.rotation.inverse();
        Self {
            translation: rotation * -self.translation,
            rotation,
        }
    }
}

impl Default for PhysicsPose {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl std::ops::Mul for PhysicsPose {
    type Output = Self;

    #[inline]
    fn mul(self, child: Self) -> Self::Output {
        Self {
            translation: self.transform_point(child.translation),
            rotation: (self.rotation * child.rotation).normalize(),
        }
    }
}
