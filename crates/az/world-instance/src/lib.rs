//! Provider-neutral lifecycle contracts for authoritative world instances.
//!
//! This crate is the lifecycle core described by ADR 0052. It owns stable
//! logical identity, replaceable placement identity, the placement fence, and
//! the consumer-owned effect ports that storage, provider, ingress, and
//! distribution adapters implement. It depends on no game, federation,
//! provider, transport, or database implementation; those crates depend on it.
//!
//! Stable logical identity and replaceable placement identity stay distinct:
//!
//! ```compile_fail
//! use az_world_instance::{WorldInstanceId, WorldInstancePlacementId};
//! use uuid::Uuid;
//!
//! fn load_instance(_: WorldInstanceId) {}
//!
//! let placement = WorldInstancePlacementId::try_from_uuid(Uuid::now_v7()).unwrap();
//! load_instance(placement);
//! ```

#![forbid(unsafe_code)]

mod identity;
mod ingress;
mod lifecycle;
mod operation;
mod port;
mod provider;
mod release;
mod spec;
mod store;

pub use identity::{
    AdmissionTicketId, CheckpointId, FenceClaim, FencedEffect, IngressRouteId,
    InvalidPlacementGeneration, InvalidWorldIdentity, PlacementFence, PlacementGeneration,
    PlayerPresenceId, ServerProcessId, StalePlacement, WorldInstanceId, WorldInstanceIdentityKind,
    WorldInstanceOperationId, WorldInstancePlacementId, WorldTransferId,
};
pub use ingress::{
    CurrentRoute, IngressControl, IngressError, IngressRouteRequest, OpaqueRoute, StagedRoute,
};
pub use lifecycle::{
    ApplyWorldInstanceError, GetWorldInstanceError, WorldInstanceService, WorldInstanceSnapshot,
};
pub use operation::{
    AcceptanceDeadline, Accepted, ApplyWorldInstance, ApplyWorldInstanceTarget,
    CanonicalApplyDigest, OperationActorId, WorldInstanceOperation,
};
pub use port::PortFuture;
pub use provider::{
    EnsurePlacement, PlacementObservation, PlacementProvider, PlacementProviderError,
    PlacementRejection, PlacementRequest,
};
pub use release::{
    ArtifactDigest, DistributionError, ReleaseDistribution, ReleaseManifestDigest, VerifiedRelease,
};
pub use spec::{
    InvalidWorldInstanceSpecRevision, WorldInstanceDefinition, WorldInstanceDesiredState,
    WorldInstanceSpecRevision,
};
pub use store::{CommitTransitionError, FencedTransition, StoreError, WorldInstanceStore};
