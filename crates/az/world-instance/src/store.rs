//! The durable transaction port the lifecycle owner consumes.
//!
//! The trait is declared here, beside its consumer, and implemented by storage
//! adapters that depend on this crate. It exposes domain transactions, never
//! SQL, cursors, provider handles, or generic CRUD. ADR 0051 supplies the
//! durable subject, fence, and outbox substrate an adapter is built from; those
//! generic types are not re-exported here and this crate does not copy that
//! state machine.

use crate::{
    Accepted, ApplyWorldInstance, ApplyWorldInstanceError, FenceClaim, FencedEffect,
    GetWorldInstanceError, PlacementFence, PortFuture, StalePlacement, WorldInstanceId,
    WorldInstanceOperation, WorldInstanceOperationId, WorldInstanceSnapshot,
    WorldInstanceSpecRevision,
};

/// Availability and invariant failures reported by the durable owner.
///
/// Domain rejections never arrive here; they are the caller-facing error of the
/// operation that was attempted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The durable owner could not safely serve the request.
    #[error("world-instance store is unavailable")]
    Unavailable,
    /// The commit outcome is unknown and must be resolved by operation lookup.
    #[error("world-instance store outcome for operation {operation} is indeterminate")]
    Indeterminate {
        /// Operation whose acceptance must be looked up before any repeat.
        operation: WorldInstanceOperationId,
    },
}

/// One durable lifecycle mutation offered under an exact placement fence.
///
/// Wave 1 widens the committed payload; the fence, expected revision, and
/// operation identity stay required fields rather than optional context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FencedTransition {
    fence: PlacementFence,
    effect: FencedEffect,
    expected_revision: WorldInstanceSpecRevision,
    operation: WorldInstanceOperationId,
}

impl FencedTransition {
    /// Binds a lifecycle mutation to the placement that may perform it.
    #[must_use]
    pub const fn new(
        fence: PlacementFence,
        effect: FencedEffect,
        expected_revision: WorldInstanceSpecRevision,
        operation: WorldInstanceOperationId,
    ) -> Self {
        Self {
            fence,
            effect,
            expected_revision,
            operation,
        }
    }

    /// Returns the placement fence that must still be current at commit.
    #[must_use]
    pub const fn fence(self) -> PlacementFence {
        self.fence
    }

    /// Returns the effect this transition performs.
    #[must_use]
    pub const fn effect(self) -> FencedEffect {
        self.effect
    }

    /// Returns the revision the caller expects to advance from.
    #[must_use]
    pub const fn expected_revision(self) -> WorldInstanceSpecRevision {
        self.expected_revision
    }

    /// Returns the durable idempotency identity of the transition.
    #[must_use]
    pub const fn operation(self) -> WorldInstanceOperationId {
        self.operation
    }

    /// Returns the claim an adapter rechecks against the stored fence.
    #[must_use]
    pub const fn claim(self) -> FenceClaim {
        FenceClaim::new(
            self.fence.world_instance(),
            self.fence.generation(),
            self.effect,
        )
    }
}

/// Closed failures returned while committing a fenced lifecycle transition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommitTransitionError {
    /// The committing placement is no longer the current fence.
    #[error(transparent)]
    StalePlacement(#[from] StalePlacement),
    /// The expected specification revision was stale or in the future.
    #[error("world instance {world_instance} expected revision {expected:?}, current {current:?}")]
    SpecRevisionConflict {
        /// Affected logical instance.
        world_instance: WorldInstanceId,
        /// Revision named by the transition.
        expected: WorldInstanceSpecRevision,
        /// Current durable revision.
        current: WorldInstanceSpecRevision,
    },
    /// The durable owner could not safely apply the transition.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Durable owner of accepted lifecycle operations and fenced transitions.
///
/// Implementations live in storage adapter crates that depend on this one. The
/// lifecycle never learns which adapter it received.
pub trait WorldInstanceStore: Send + Sync {
    /// Durably accepts one declarative desired-state request.
    fn accept_apply(
        &self,
        request: ApplyWorldInstance,
    ) -> PortFuture<'_, Result<Accepted<WorldInstanceOperation>, ApplyWorldInstanceError>>;

    /// Commits one lifecycle mutation under the current placement fence.
    fn commit_transition(
        &self,
        transition: FencedTransition,
    ) -> PortFuture<'_, Result<WorldInstanceSpecRevision, CommitTransitionError>>;

    /// Reads the current durable desired-state snapshot.
    fn snapshot(
        &self,
        world_instance: WorldInstanceId,
    ) -> PortFuture<'_, Result<WorldInstanceSnapshot, GetWorldInstanceError>>;
}
