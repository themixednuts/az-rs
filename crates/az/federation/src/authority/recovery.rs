use serde::{Deserialize, Serialize};

use crate::{AuthorityEpoch, ContentDigest, FederationId};

/// Durable reference to one exact canonical signed history checkpoint.
///
/// `statement_digest` commits to the checkpoint version, tree size, root,
/// signing role, and signature material. Crypto adapters remain responsible
/// for proving that commitment corresponds to a valid signed checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryCheckpointRef {
    tree_size: u64,
    statement_digest: ContentDigest,
}

impl HistoryCheckpointRef {
    /// Binds one history size to its canonical signed-statement digest.
    #[must_use]
    pub const fn new(tree_size: u64, statement_digest: ContentDigest) -> Self {
        Self {
            tree_size,
            statement_digest,
        }
    }

    /// Returns the exact number of leaves in the referenced history.
    #[must_use]
    pub const fn tree_size(self) -> u64 {
        self.tree_size
    }

    /// Returns the digest of the complete signed checkpoint statement.
    #[must_use]
    pub const fn statement_digest(self) -> ContentDigest {
        self.statement_digest
    }
}

/// Closed construction failures for a root/fork recovery statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryStatementError {
    /// Recovery must move authority to a strictly newer visible epoch.
    #[error("recovery epoch {proposed} does not advance current epoch {current}")]
    NonSuccessorEpoch {
        /// Epoch frozen by the incident.
        current: u64,
        /// Proposed recovery epoch.
        proposed: u64,
    },
    /// Identical checkpoint references do not prove a fork.
    #[error("recovery statement does not name divergent history checkpoints")]
    NoForkDivergence,
}

/// Exact forward-only facts that a root-recovery quorum must authorize.
///
/// This value is deliberately unsigned. The federation policy layer decides
/// the required recovery quorum; `az-federation-crypto` supplies the distinct
/// root-recovery signature role. Storage administration alone cannot construct
/// an authorized recovery from this statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityRecoveryStatement {
    federation: FederationId,
    current_epoch: AuthorityEpoch,
    successor_epoch: AuthorityEpoch,
    trusted_checkpoint: HistoryCheckpointRef,
    divergent_checkpoint: HistoryCheckpointRef,
    selected_checkpoint: HistoryCheckpointRef,
    successor_key_policy: ContentDigest,
    recovery_policy: ContentDigest,
    recovery_manifest: ContentDigest,
}

impl AuthorityRecoveryStatement {
    /// Creates a complete recovery statement after enforcing forward epoch and
    /// explicit fork-lineage invariants.
    ///
    /// # Errors
    ///
    /// Rejects an epoch that does not advance or two identical fork heads.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        federation: FederationId,
        current_epoch: AuthorityEpoch,
        successor_epoch: AuthorityEpoch,
        trusted_checkpoint: HistoryCheckpointRef,
        divergent_checkpoint: HistoryCheckpointRef,
        selected_checkpoint: HistoryCheckpointRef,
        successor_key_policy: ContentDigest,
        recovery_policy: ContentDigest,
        recovery_manifest: ContentDigest,
    ) -> Result<Self, RecoveryStatementError> {
        if successor_epoch <= current_epoch {
            return Err(RecoveryStatementError::NonSuccessorEpoch {
                current: current_epoch.get(),
                proposed: successor_epoch.get(),
            });
        }
        if trusted_checkpoint == divergent_checkpoint {
            return Err(RecoveryStatementError::NoForkDivergence);
        }
        Ok(Self {
            federation,
            current_epoch,
            successor_epoch,
            trusted_checkpoint,
            divergent_checkpoint,
            selected_checkpoint,
            successor_key_policy,
            recovery_policy,
            recovery_manifest,
        })
    }

    /// Returns the federation whose portable authority is frozen.
    #[must_use]
    pub const fn federation(self) -> FederationId {
        self.federation
    }

    /// Returns the frozen authority epoch.
    #[must_use]
    pub const fn current_epoch(self) -> AuthorityEpoch {
        self.current_epoch
    }

    /// Returns the strictly newer epoch proposed by recovery.
    #[must_use]
    pub const fn successor_epoch(self) -> AuthorityEpoch {
        self.successor_epoch
    }

    /// Returns the checkpoint trusted before divergence was observed.
    #[must_use]
    pub const fn trusted_checkpoint(self) -> HistoryCheckpointRef {
        self.trusted_checkpoint
    }

    /// Returns the conflicting checkpoint that proved the fork.
    #[must_use]
    pub const fn divergent_checkpoint(self) -> HistoryCheckpointRef {
        self.divergent_checkpoint
    }

    /// Returns the checkpoint selected as the recovery replay base.
    #[must_use]
    pub const fn selected_checkpoint(self) -> HistoryCheckpointRef {
        self.selected_checkpoint
    }

    /// Returns the immutable successor signing-key policy document digest.
    #[must_use]
    pub const fn successor_key_policy(self) -> ContentDigest {
        self.successor_key_policy
    }

    /// Returns the immutable recovery policy revision digest.
    #[must_use]
    pub const fn recovery_policy(self) -> ContentDigest {
        self.recovery_policy
    }

    /// Returns the canonical manifest digest naming every recovered atom target.
    #[must_use]
    pub const fn recovery_manifest(self) -> ContentDigest {
        self.recovery_manifest
    }
}
