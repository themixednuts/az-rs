//! Provider-neutral Certified Federation domain and application kernel.

mod binding;
mod canonical;
mod certification;
mod identity;
mod release;

pub mod authority;

pub use authority::{
    AuthorityAtomCommit, AuthorityAtomCommitResult, AuthorityAtomPort, AuthorityAtomSnapshot,
    AuthorityOperationLookupFuture, AuthorityOperationLookupPort, AuthorityReceipt,
    AuthorityRecoveryManifest, AuthorityRecoveryStatement, AuthorityRecoveryTarget,
    AuthorityReplayEvent, AuthorityReplayPolicy, AuthorityReplayRefusal, AuthorityReplaySnapshot,
    AuthorityReplayTarget, AuthorizedRecoveryReplay, CommitRefusal, HistoryCheckpointRef,
    InMemoryAuthorityAtom, InMemoryCommitFault, MAX_ROOT_RECOVERY_KEYS, PlacementAuthorityPort,
    PlacementAuthorityUnavailable, PlacementCurrency, PlacementFenceFuture, PublicationOutcome,
    PublicationRefusal, PublicationStatus, PublicationStep, RecoveryManifestError,
    RecoveryReplayAuthorizationError, RecoveryStatementError, RootRecoveryKeyPolicy,
    RootRecoveryPolicyError, RootRecoveryQuorum, RootRecoveryRefusal, StorageFailure,
    TransparencyLoadFuture, TransparencyOutboxFuture, TransparencyOutboxPort,
    TransparencyPublication, UncertainCommit, VerifiedAuthorityReplay,
    authorize_manifest_recovery_replay, authorize_root_recovery, reconcile_uncertain_commit,
    verify_authority_replay, verify_placement_currency,
};
pub use binding::{
    ActivePolicyRef, CapabilityGrantRef, CertifiedExecutionBinding, MAX_RULESET_NAMESPACE_BYTES,
    RulesetBinding, RulesetNamespace, RulesetNamespaceError, RuntimeReleaseBinding,
};
pub use canonical::CanonicalRequestDigest;
pub use certification::{
    CertifiedHostBinding, ChallengeVerificationFailure, ChallengeVerificationFuture,
    ChallengeVerificationPort, EvidenceVerificationFailure, GrantUseRefusal, HostChallengeNonce,
    HostChallengePlan, HostChallengePlanError, HostChallengeRefusal, HostChallengeSubmission,
    PortableMutationCeiling, PortableMutationClass, PortableMutationClasses, PortableMutationGrant,
    PortableMutationGrantError, PortableMutationGrantRequest, PortableMutationGrantStanding,
    VerifiedHostChallenge, verify_host_challenge,
};
pub use identity::{
    AuthorityAtomId, AuthorityEpoch, AuthoritySequence, AuthorityTime, ContentDigest,
    EvidenceSnapshotId, FederationId, FederationPrincipalId, GrantRevision, HostChallengeId,
    InstanceHostId, InvalidMonotonicValue, InvalidOperationId, OperationId, OperatorId,
    PolicyRevision, PublisherReleaseAuthorizationId, ReleaseEligibilityDecisionId,
    RevocationCursor,
};
pub use release::{
    BehaviorEligibility, CertifiedBehaviorRelease, CertifiedReleaseProof,
    PublisherReleaseAuthorization, ReleaseBehavior, ReleaseEligibilityDecision,
    ReleaseEligibilitySnapshot, ReleaseGateRefusal,
};

#[cfg(test)]
mod architecture_tests;
