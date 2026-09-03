use serde::{Deserialize, Serialize};

use crate::{
    WorldInstanceDefinition, WorldInstanceDesiredState, WorldInstanceId, WorldInstanceOperationId,
    WorldInstanceSpecRevision,
};

/// Digest of the complete canonical apply request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalApplyDigest([u8; 32]);

impl CanonicalApplyDigest {
    /// Constructs a digest from canonical protocol bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Provider-neutral identity of the actor requesting a lifecycle change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationActorId([u8; 32]);

impl OperationActorId {
    /// Constructs an actor identity from the composition root's canonical digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the actor identity bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Absolute deadline for durably accepting a new operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AcceptanceDeadline(u64);

impl AcceptanceDeadline {
    /// A deadline suitable for work with no caller-imposed acceptance limit.
    pub const MAX: Self = Self(u64::MAX);

    /// Constructs a deadline from Unix-epoch milliseconds.
    #[must_use]
    pub const fn from_unix_millis(value: u64) -> Self {
        Self(value)
    }

    /// Returns the Unix-epoch millisecond representation.
    #[must_use]
    pub const fn as_unix_millis(self) -> u64 {
        self.0
    }

    pub(crate) const fn elapsed_at(self, unix_millis: u64) -> bool {
        unix_millis > self.0
    }
}

/// Creation or compare-and-set target for an apply request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyWorldInstanceTarget {
    /// Create a new logical instance with one immutable definition.
    Create(WorldInstanceDefinition),
    /// Change desired state only when the current revision matches.
    Exact(WorldInstanceSpecRevision),
}

/// One canonical declarative lifecycle request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyWorldInstance {
    operation: WorldInstanceOperationId,
    world_instance: WorldInstanceId,
    target: ApplyWorldInstanceTarget,
    desired: WorldInstanceDesiredState,
    actor: OperationActorId,
    canonical_digest: CanonicalApplyDigest,
    deadline: AcceptanceDeadline,
}

impl ApplyWorldInstance {
    /// Constructs a complete lifecycle apply request.
    #[must_use]
    pub const fn new(
        operation: WorldInstanceOperationId,
        world_instance: WorldInstanceId,
        target: ApplyWorldInstanceTarget,
        desired: WorldInstanceDesiredState,
        actor: OperationActorId,
        canonical_digest: CanonicalApplyDigest,
        deadline: AcceptanceDeadline,
    ) -> Self {
        Self {
            operation,
            world_instance,
            target,
            desired,
            actor,
            canonical_digest,
            deadline,
        }
    }

    /// Returns this request with a different acceptance deadline.
    #[must_use]
    pub const fn with_deadline(mut self, deadline: AcceptanceDeadline) -> Self {
        self.deadline = deadline;
        self
    }

    /// Returns the durable operation identity.
    #[must_use]
    pub const fn operation(&self) -> WorldInstanceOperationId {
        self.operation
    }

    /// Returns the affected logical instance.
    #[must_use]
    pub const fn world_instance(&self) -> WorldInstanceId {
        self.world_instance
    }

    /// Returns the creation or compare-and-set target.
    #[must_use]
    pub const fn target(&self) -> &ApplyWorldInstanceTarget {
        &self.target
    }

    /// Returns the desired lifecycle state.
    #[must_use]
    pub const fn desired(&self) -> WorldInstanceDesiredState {
        self.desired
    }

    /// Returns the attributed actor.
    #[must_use]
    pub const fn actor(&self) -> OperationActorId {
        self.actor
    }

    /// Returns the canonical request digest used for idempotency.
    #[must_use]
    pub const fn canonical_digest(&self) -> CanonicalApplyDigest {
        self.canonical_digest
    }

    /// Returns the acceptance deadline.
    #[must_use]
    pub const fn deadline(&self) -> AcceptanceDeadline {
        self.deadline
    }
}

/// A value whose acceptance has been durably recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted<T> {
    value: T,
    accepted_unix_millis: u64,
}

impl<T> Accepted<T> {
    /// Records that the durable owner accepted `value` at that Unix millisecond.
    ///
    /// Only a [`WorldInstanceStore`](crate::WorldInstanceStore) implementation
    /// mints this, because acceptance is exactly what a durable transaction
    /// proves.
    #[must_use]
    pub const fn new(value: T, accepted_unix_millis: u64) -> Self {
        Self {
            value,
            accepted_unix_millis,
        }
    }

    /// Returns the accepted value.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Returns when the durable owner accepted the value.
    #[must_use]
    pub const fn accepted_unix_millis(&self) -> u64 {
        self.accepted_unix_millis
    }
}

/// Durable record of one accepted lifecycle operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldInstanceOperation {
    id: WorldInstanceOperationId,
    world_instance: WorldInstanceId,
    spec_revision: WorldInstanceSpecRevision,
    actor: OperationActorId,
    canonical_digest: CanonicalApplyDigest,
}

impl WorldInstanceOperation {
    /// Records the operation a durable owner accepted, at the revision it
    /// produced.
    #[must_use]
    pub const fn new(
        request: &ApplyWorldInstance,
        spec_revision: WorldInstanceSpecRevision,
    ) -> Self {
        Self {
            id: request.operation,
            world_instance: request.world_instance,
            spec_revision,
            actor: request.actor,
            canonical_digest: request.canonical_digest,
        }
    }

    /// Returns the operation identity.
    #[must_use]
    pub const fn id(&self) -> WorldInstanceOperationId {
        self.id
    }

    /// Returns the affected logical instance.
    #[must_use]
    pub const fn world_instance(&self) -> WorldInstanceId {
        self.world_instance
    }

    /// Returns the revision produced by this operation.
    #[must_use]
    pub const fn spec_revision(&self) -> WorldInstanceSpecRevision {
        self.spec_revision
    }

    /// Returns the attributed actor.
    #[must_use]
    pub const fn actor(&self) -> OperationActorId {
        self.actor
    }

    /// Returns the canonical request digest.
    #[must_use]
    pub const fn canonical_digest(&self) -> CanonicalApplyDigest {
        self.canonical_digest
    }
}
