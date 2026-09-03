use az_federation_crypto::MessageDigest;

use super::AuthorityRecoveryManifest;
use crate::{
    AuthorityAtomId, AuthorityEpoch, ContentDigest, FederationId, HistoryCheckpointRef,
    RootRecoveryQuorum, VerifiedAuthorityReplay,
};

/// Mismatch between a root quorum and independently verified replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryReplayAuthorizationError {
    #[error("recovery manifest digest was not approved by the root quorum")]
    WrongManifestDigest,
    #[error("recovery manifest names a different successor lineage")]
    WrongManifestLineage,
    #[error("recovery manifest does not name the replayed atom")]
    AtomNotInManifest,
    #[error("recovery replay does not reach its manifest target")]
    WrongManifestTarget,
    /// Replay belongs to another federation.
    #[error("recovery replay belongs to another federation")]
    WrongFederation,
    /// Replay reconstructed a different authority epoch.
    #[error("recovery replay belongs to another authority epoch")]
    WrongCurrentEpoch,
    /// Replay target was not derived from the quorum-selected checkpoint.
    #[error("recovery replay belongs to another history checkpoint")]
    WrongSelectedCheckpoint,
}

/// Binds a verified atom replay to the exact root-approved recovery manifest.
///
/// # Errors
///
/// Refuses a substituted manifest digest or lineage, an atom absent from the
/// manifest, or a replay that does not reach its committed terminal target.
pub fn authorize_manifest_recovery_replay<S>(
    quorum: RootRecoveryQuorum,
    manifest: &AuthorityRecoveryManifest,
    manifest_digest: ContentDigest,
    replay: VerifiedAuthorityReplay<S>,
) -> Result<AuthorizedRecoveryReplay<S>, RecoveryReplayAuthorizationError> {
    let statement = quorum.statement();
    if statement.recovery_manifest() != manifest_digest {
        return Err(RecoveryReplayAuthorizationError::WrongManifestDigest);
    }
    if manifest.federation() != statement.federation()
        || manifest.current_epoch() != statement.current_epoch()
        || manifest.successor_epoch() != statement.successor_epoch()
        || manifest.selected_checkpoint() != statement.selected_checkpoint()
    {
        return Err(RecoveryReplayAuthorizationError::WrongManifestLineage);
    }
    let target = manifest
        .target(replay.atom())
        .ok_or(RecoveryReplayAuthorizationError::AtomNotInManifest)?;
    if target.through_sequence() != replay.through_sequence()
        || target.state_digest() != replay.state_digest()
    {
        return Err(RecoveryReplayAuthorizationError::WrongManifestTarget);
    }
    if replay.federation() != statement.federation() {
        return Err(RecoveryReplayAuthorizationError::WrongFederation);
    }
    if replay.epoch() != statement.current_epoch() {
        return Err(RecoveryReplayAuthorizationError::WrongCurrentEpoch);
    }
    if replay.checkpoint() != statement.selected_checkpoint() {
        return Err(RecoveryReplayAuthorizationError::WrongSelectedCheckpoint);
    }
    Ok(AuthorizedRecoveryReplay { quorum, replay })
}

/// One atom replay bound to an authorized forward recovery statement.
///
/// Only [`authorize_manifest_recovery_replay`] constructs this value. Durable
/// recovery adapters can require it instead of repeating quorum, manifest, and
/// replay checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedRecoveryReplay<S> {
    quorum: RootRecoveryQuorum,
    replay: VerifiedAuthorityReplay<S>,
}

impl<S> AuthorizedRecoveryReplay<S> {
    /// Returns the federation entering a successor epoch.
    #[must_use]
    pub const fn federation(&self) -> FederationId {
        self.replay.federation()
    }

    /// Returns the recovered serial authority atom.
    #[must_use]
    pub const fn atom(&self) -> AuthorityAtomId {
        self.replay.atom()
    }

    /// Returns the frozen epoch reconstructed by replay.
    #[must_use]
    pub const fn current_epoch(&self) -> AuthorityEpoch {
        self.replay.epoch()
    }

    /// Returns the strictly newer epoch authorized by the roots.
    #[must_use]
    pub const fn successor_epoch(&self) -> AuthorityEpoch {
        self.quorum.statement().successor_epoch()
    }

    /// Returns the selected history checkpoint shared by quorum and replay.
    #[must_use]
    pub const fn checkpoint(&self) -> HistoryCheckpointRef {
        self.replay.checkpoint()
    }

    /// Returns the canonical recovery message approved by the roots.
    #[must_use]
    pub const fn recovery_message(&self) -> MessageDigest {
        self.quorum.message_digest()
    }

    /// Returns the immutable root-recovery key policy.
    #[must_use]
    pub const fn recovery_policy(&self) -> ContentDigest {
        self.quorum.policy_digest()
    }

    /// Returns the verified reconstructed state.
    #[must_use]
    pub const fn state(&self) -> &S {
        self.replay.state()
    }

    /// Returns the verified reconstructed state digest.
    #[must_use]
    pub const fn state_digest(&self) -> ContentDigest {
        self.replay.state_digest()
    }

    /// Returns the last authority sequence included in replay.
    #[must_use]
    pub const fn through_sequence(&self) -> u64 {
        self.replay.through_sequence()
    }

    /// Transfers the verified state to a durable recovery adapter.
    #[must_use]
    pub fn into_state(self) -> S {
        self.replay.into_state()
    }
}
