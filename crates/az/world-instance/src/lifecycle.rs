use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    Accepted, ApplyWorldInstance, ApplyWorldInstanceTarget, WorldInstanceDefinition,
    WorldInstanceDesiredState, WorldInstanceId, WorldInstanceOperation, WorldInstanceOperationId,
    WorldInstanceSpecRevision,
};

/// Current durable desired-state projection for one logical instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldInstanceSnapshot {
    world_instance: WorldInstanceId,
    definition: WorldInstanceDefinition,
    spec_revision: WorldInstanceSpecRevision,
    desired: WorldInstanceDesiredState,
    last_operation: WorldInstanceOperationId,
}

impl WorldInstanceSnapshot {
    /// Projects one durable desired-state record.
    ///
    /// Only a [`WorldInstanceStore`](crate::WorldInstanceStore) implementation
    /// mints this, because a snapshot is a read of committed durable state.
    #[must_use]
    pub const fn new(
        world_instance: WorldInstanceId,
        definition: WorldInstanceDefinition,
        spec_revision: WorldInstanceSpecRevision,
        desired: WorldInstanceDesiredState,
        last_operation: WorldInstanceOperationId,
    ) -> Self {
        Self {
            world_instance,
            definition,
            spec_revision,
            desired,
            last_operation,
        }
    }

    /// Returns the stable logical identity.
    #[must_use]
    pub const fn world_instance(&self) -> WorldInstanceId {
        self.world_instance
    }

    /// Returns the immutable launch definition.
    #[must_use]
    pub const fn definition(&self) -> &WorldInstanceDefinition {
        &self.definition
    }

    /// Returns the current desired-specification revision.
    #[must_use]
    pub const fn spec_revision(&self) -> WorldInstanceSpecRevision {
        self.spec_revision
    }

    /// Returns the current desired lifecycle state.
    #[must_use]
    pub const fn desired(&self) -> WorldInstanceDesiredState {
        self.desired
    }

    /// Returns the operation that produced this revision.
    #[must_use]
    pub const fn last_operation(&self) -> WorldInstanceOperationId {
        self.last_operation
    }
}

/// Closed failures returned while accepting a lifecycle request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplyWorldInstanceError {
    /// The operation identity was reused with different canonical bytes.
    #[error("world-instance operation {operation} conflicts with its accepted payload")]
    OperationPayloadConflict {
        /// Conflicting operation identity.
        operation: WorldInstanceOperationId,
    },
    /// Creation targeted an existing logical instance.
    #[error("world instance {world_instance} already exists")]
    AlreadyExists {
        /// Existing logical instance.
        world_instance: WorldInstanceId,
    },
    /// An exact update targeted an absent logical instance.
    #[error("world instance {world_instance} was not found")]
    NotFound {
        /// Missing logical instance.
        world_instance: WorldInstanceId,
    },
    /// The request's expected specification revision was stale or future.
    #[error("world instance {world_instance} expected revision {expected:?}, current {current:?}")]
    SpecRevisionConflict {
        /// Affected logical instance.
        world_instance: WorldInstanceId,
        /// Revision named by the request.
        expected: WorldInstanceSpecRevision,
        /// Current durable revision.
        current: WorldInstanceSpecRevision,
    },
    /// A terminal tombstone cannot return to an active state.
    #[error("world instance {world_instance} is terminal")]
    TerminalInstance {
        /// Terminal logical instance.
        world_instance: WorldInstanceId,
    },
    /// The operation was not accepted before its deadline.
    #[error("world-instance operation {operation} acceptance deadline elapsed")]
    AcceptanceDeadlineElapsed {
        /// Expired operation identity.
        operation: WorldInstanceOperationId,
    },
    /// The monotonically increasing specification revision was exhausted.
    #[error("world instance {world_instance} exhausted its specification revision")]
    SpecRevisionExhausted {
        /// Affected logical instance.
        world_instance: WorldInstanceId,
    },
    /// The durable owner could not safely accept the operation.
    #[error("world-instance store is unavailable")]
    StoreUnavailable,
}

/// Closed failures returned while reading a lifecycle snapshot.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GetWorldInstanceError {
    /// No durable instance exists for the identity.
    #[error("world instance {world_instance} was not found")]
    NotFound {
        /// Missing logical instance.
        world_instance: WorldInstanceId,
    },
    /// The durable owner could not safely read the instance.
    #[error("world-instance store is unavailable")]
    StoreUnavailable,
}

#[derive(Debug, Clone, Default)]
struct InMemoryLifecycleState {
    instances: BTreeMap<WorldInstanceId, WorldInstanceSnapshot>,
    operations: BTreeMap<WorldInstanceOperationId, Accepted<WorldInstanceOperation>>,
}

/// Declarative lifecycle application service backed by a reference store.
///
/// The in-memory store is a deterministic conformance adapter. Production
/// durability is supplied by the later `WorldInstanceStore` adapter without
/// changing the public apply/get contract.
#[derive(Debug, Clone, Default)]
pub struct WorldInstanceService {
    state: Arc<Mutex<InMemoryLifecycleState>>,
}

impl WorldInstanceService {
    /// Constructs the in-memory conformance service.
    #[must_use]
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Durably accepts one declarative desired-state request.
    ///
    /// Exact retries return the original accepted record. Reusing the same
    /// operation identity with different canonical bytes is a conflict.
    ///
    /// # Errors
    ///
    /// Returns a closed domain or availability failure without a partial
    /// state or operation record.
    #[allow(
        clippy::unused_async,
        reason = "the production store remains an asynchronous effect behind this public contract"
    )]
    pub async fn apply(
        &self,
        request: ApplyWorldInstance,
    ) -> Result<Accepted<WorldInstanceOperation>, ApplyWorldInstanceError> {
        let accepted_at = unix_millis_now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ApplyWorldInstanceError::StoreUnavailable)?;

        if let Some(accepted) = state.operations.get(&request.operation()) {
            if accepted.value().canonical_digest() == request.canonical_digest() {
                return Ok(accepted.clone());
            }
            return Err(ApplyWorldInstanceError::OperationPayloadConflict {
                operation: request.operation(),
            });
        }

        if request.deadline().elapsed_at(accepted_at) {
            return Err(ApplyWorldInstanceError::AcceptanceDeadlineElapsed {
                operation: request.operation(),
            });
        }

        let mut staged = state.clone();
        let (definition, revision) = match request.target() {
            ApplyWorldInstanceTarget::Create(definition) => {
                if staged.instances.contains_key(&request.world_instance()) {
                    return Err(ApplyWorldInstanceError::AlreadyExists {
                        world_instance: request.world_instance(),
                    });
                }
                (definition.clone(), WorldInstanceSpecRevision::initial())
            }
            ApplyWorldInstanceTarget::Exact(expected) => {
                let snapshot =
                    staged
                        .instances
                        .get(&request.world_instance())
                        .ok_or_else(|| ApplyWorldInstanceError::NotFound {
                            world_instance: request.world_instance(),
                        })?;
                if snapshot.spec_revision != *expected {
                    return Err(ApplyWorldInstanceError::SpecRevisionConflict {
                        world_instance: request.world_instance(),
                        expected: *expected,
                        current: snapshot.spec_revision,
                    });
                }
                if snapshot.desired == WorldInstanceDesiredState::Terminated {
                    return Err(ApplyWorldInstanceError::TerminalInstance {
                        world_instance: request.world_instance(),
                    });
                }
                let revision = snapshot.spec_revision.checked_next().ok_or_else(|| {
                    ApplyWorldInstanceError::SpecRevisionExhausted {
                        world_instance: request.world_instance(),
                    }
                })?;
                (snapshot.definition.clone(), revision)
            }
        };

        let operation = WorldInstanceOperation::new(&request, revision);
        let accepted = Accepted::new(operation, accepted_at);
        let snapshot = WorldInstanceSnapshot {
            world_instance: request.world_instance(),
            definition,
            spec_revision: revision,
            desired: request.desired(),
            last_operation: request.operation(),
        };

        staged.instances.insert(request.world_instance(), snapshot);
        staged
            .operations
            .insert(request.operation(), accepted.clone());
        *state = staged;
        drop(state);
        Ok(accepted)
    }

    /// Reads the current durable desired-state snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`GetWorldInstanceError::NotFound`] for an absent identity or
    /// [`GetWorldInstanceError::StoreUnavailable`] when the owner is poisoned.
    #[allow(
        clippy::unused_async,
        reason = "the production store remains an asynchronous effect behind this public contract"
    )]
    pub async fn get(
        &self,
        world_instance: WorldInstanceId,
    ) -> Result<WorldInstanceSnapshot, GetWorldInstanceError> {
        self.state
            .lock()
            .map_err(|_| GetWorldInstanceError::StoreUnavailable)?
            .instances
            .get(&world_instance)
            .cloned()
            .ok_or(GetWorldInstanceError::NotFound { world_instance })
    }
}

fn unix_millis_now() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}
