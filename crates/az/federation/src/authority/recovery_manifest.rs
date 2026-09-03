use serde::{Deserialize, Serialize};

use crate::{
    AuthorityAtomId, AuthorityEpoch, ContentDigest, FederationId, HistoryCheckpointRef, OperationId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityRecoveryTarget {
    atom: AuthorityAtomId,
    through_sequence: u64,
    state_digest: ContentDigest,
}

impl AuthorityRecoveryTarget {
    #[must_use]
    pub const fn new(
        atom: AuthorityAtomId,
        through_sequence: u64,
        state_digest: ContentDigest,
    ) -> Self {
        Self {
            atom,
            through_sequence,
            state_digest,
        }
    }

    #[must_use]
    pub const fn atom(self) -> AuthorityAtomId {
        self.atom
    }
    #[must_use]
    pub const fn through_sequence(self) -> u64 {
        self.through_sequence
    }
    #[must_use]
    pub const fn state_digest(self) -> ContentDigest {
        self.state_digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryManifestError {
    #[error("recovery manifest successor epoch does not advance the frozen epoch")]
    NonSuccessorEpoch,
    #[error("recovery manifest must name at least one authority atom")]
    EmptyTargets,
    #[error("recovery manifest targets are not in strict atom order")]
    NonCanonicalTargetOrder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityRecoveryManifest {
    federation: FederationId,
    current_epoch: AuthorityEpoch,
    successor_epoch: AuthorityEpoch,
    selected_checkpoint: HistoryCheckpointRef,
    operation: OperationId,
    impact_inventory: ContentDigest,
    history_amendment: ContentDigest,
    targets: Box<[AuthorityRecoveryTarget]>,
}

impl AuthorityRecoveryManifest {
    /// Constructs the complete, canonical target set for one recovery operation.
    ///
    /// # Errors
    ///
    /// Rejects a non-successor epoch, an empty target set, or targets that are
    /// not in strictly increasing atom order.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        federation: FederationId,
        current_epoch: AuthorityEpoch,
        successor_epoch: AuthorityEpoch,
        selected_checkpoint: HistoryCheckpointRef,
        operation: OperationId,
        impact_inventory: ContentDigest,
        history_amendment: ContentDigest,
        targets: Box<[AuthorityRecoveryTarget]>,
    ) -> Result<Self, RecoveryManifestError> {
        if successor_epoch <= current_epoch {
            return Err(RecoveryManifestError::NonSuccessorEpoch);
        }
        if targets.is_empty() {
            return Err(RecoveryManifestError::EmptyTargets);
        }
        if targets.windows(2).any(|pair| pair[0].atom >= pair[1].atom) {
            return Err(RecoveryManifestError::NonCanonicalTargetOrder);
        }
        Ok(Self {
            federation,
            current_epoch,
            successor_epoch,
            selected_checkpoint,
            operation,
            impact_inventory,
            history_amendment,
            targets,
        })
    }

    #[must_use]
    pub const fn federation(&self) -> FederationId {
        self.federation
    }
    #[must_use]
    pub const fn current_epoch(&self) -> AuthorityEpoch {
        self.current_epoch
    }
    #[must_use]
    pub const fn successor_epoch(&self) -> AuthorityEpoch {
        self.successor_epoch
    }
    #[must_use]
    pub const fn selected_checkpoint(&self) -> HistoryCheckpointRef {
        self.selected_checkpoint
    }
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }
    #[must_use]
    pub const fn impact_inventory(&self) -> ContentDigest {
        self.impact_inventory
    }
    #[must_use]
    pub const fn history_amendment(&self) -> ContentDigest {
        self.history_amendment
    }
    #[must_use]
    pub fn targets(&self) -> &[AuthorityRecoveryTarget] {
        &self.targets
    }
    #[must_use]
    pub fn target(&self, atom: AuthorityAtomId) -> Option<AuthorityRecoveryTarget> {
        self.targets
            .binary_search_by_key(&atom, |target| target.atom)
            .ok()
            .map(|index| self.targets[index])
    }
}
