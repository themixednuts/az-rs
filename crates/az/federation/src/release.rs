use serde::{Deserialize, Serialize};

use crate::{
    ActivePolicyRef, ContentDigest, PublisherReleaseAuthorizationId, ReleaseEligibilityDecisionId,
    RuntimeReleaseBinding,
};

/// Independently governed release behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReleaseBehavior {
    /// Starting a new workload placement.
    Placement,
    /// Admitting a player to a workload.
    Admission,
    /// Accepting a new portable-state mutation.
    PortableMutation,
    /// Accepting a new outcome claim.
    OutcomeAcceptance,
    /// Finishing already accepted work under its pinned authority.
    Closeout,
    /// Selecting this exact release as a forward rollback target.
    Rollback,
    /// Replaying evidence produced by this release.
    Replay,
}

impl ReleaseBehavior {
    /// Every behavior controlled by release eligibility.
    pub const ALL: [Self; 7] = [
        Self::Placement,
        Self::Admission,
        Self::PortableMutation,
        Self::OutcomeAcceptance,
        Self::Closeout,
        Self::Rollback,
        Self::Replay,
    ];
}

/// Current eligibility of one behavior for an exact release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BehaviorEligibility {
    /// New work is eligible.
    Allowed,
    /// Only the governed grace behavior remains eligible.
    Grace,
    /// This behavior is not eligible.
    Denied,
}

/// Behavior-specific federation eligibility without a universal compliance bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReleaseEligibilitySnapshot {
    placement: BehaviorEligibility,
    admission: BehaviorEligibility,
    portable_mutation: BehaviorEligibility,
    outcome_acceptance: BehaviorEligibility,
    closeout: BehaviorEligibility,
    rollback: BehaviorEligibility,
    replay: BehaviorEligibility,
}

impl ReleaseEligibilitySnapshot {
    /// Creates the complete behavior eligibility snapshot.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        placement: BehaviorEligibility,
        admission: BehaviorEligibility,
        portable_mutation: BehaviorEligibility,
        outcome_acceptance: BehaviorEligibility,
        closeout: BehaviorEligibility,
        rollback: BehaviorEligibility,
        replay: BehaviorEligibility,
    ) -> Self {
        Self {
            placement,
            admission,
            portable_mutation,
            outcome_acceptance,
            closeout,
            rollback,
            replay,
        }
    }

    /// Creates a snapshot with the same explicit state for every behavior.
    #[must_use]
    pub const fn uniform(eligibility: BehaviorEligibility) -> Self {
        Self::new(
            eligibility,
            eligibility,
            eligibility,
            eligibility,
            eligibility,
            eligibility,
            eligibility,
        )
    }

    /// Returns eligibility for exactly one requested behavior.
    #[must_use]
    pub const fn for_behavior(self, behavior: ReleaseBehavior) -> BehaviorEligibility {
        match behavior {
            ReleaseBehavior::Placement => self.placement,
            ReleaseBehavior::Admission => self.admission,
            ReleaseBehavior::PortableMutation => self.portable_mutation,
            ReleaseBehavior::OutcomeAcceptance => self.outcome_acceptance,
            ReleaseBehavior::Closeout => self.closeout,
            ReleaseBehavior::Rollback => self.rollback,
            ReleaseBehavior::Replay => self.replay,
        }
    }
}

/// Immutable publisher authorization for one exact runtime release.
///
/// The referenced record is signed and verified outside this pure domain type;
/// its identity commits to the full publisher metadata and signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublisherReleaseAuthorization {
    id: PublisherReleaseAuthorizationId,
    release: RuntimeReleaseBinding,
    provenance: ContentDigest,
}

impl PublisherReleaseAuthorization {
    /// Binds the immutable authorization record to its release and provenance.
    #[must_use]
    pub const fn new(
        id: PublisherReleaseAuthorizationId,
        release: RuntimeReleaseBinding,
        provenance: ContentDigest,
    ) -> Self {
        Self {
            id,
            release,
            provenance,
        }
    }

    /// Returns the authorization-record identity.
    #[must_use]
    pub const fn id(self) -> PublisherReleaseAuthorizationId {
        self.id
    }

    /// Returns the publisher-authorized release.
    #[must_use]
    pub const fn release(self) -> RuntimeReleaseBinding {
        self.release
    }

    /// Returns the publisher provenance-statement commitment.
    #[must_use]
    pub const fn provenance(self) -> ContentDigest {
        self.provenance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum EligibilityDisposition {
    Active(ReleaseEligibilitySnapshot),
    Retired,
}

/// Governed federation eligibility for one publisher-authorized release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReleaseEligibilityDecision {
    id: ReleaseEligibilityDecisionId,
    release: RuntimeReleaseBinding,
    publisher: PublisherReleaseAuthorizationId,
    governing_policy: ActivePolicyRef,
    disposition: EligibilityDisposition,
}

impl ReleaseEligibilityDecision {
    /// Creates an active behavior-specific eligibility decision.
    #[must_use]
    pub const fn active(
        id: ReleaseEligibilityDecisionId,
        release: RuntimeReleaseBinding,
        publisher: PublisherReleaseAuthorizationId,
        governing_policy: ActivePolicyRef,
        snapshot: ReleaseEligibilitySnapshot,
    ) -> Self {
        Self {
            id,
            release,
            publisher,
            governing_policy,
            disposition: EligibilityDisposition::Active(snapshot),
        }
    }

    /// Creates a terminal retirement decision that authorizes no behavior.
    #[must_use]
    pub const fn retired(
        id: ReleaseEligibilityDecisionId,
        release: RuntimeReleaseBinding,
        publisher: PublisherReleaseAuthorizationId,
        governing_policy: ActivePolicyRef,
    ) -> Self {
        Self {
            id,
            release,
            publisher,
            governing_policy,
            disposition: EligibilityDisposition::Retired,
        }
    }
}

/// Precise refusal to compose or use a Certified Release proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReleaseGateRefusal {
    /// The publisher and federation records name different releases.
    #[error("publisher authorization and federation eligibility name different releases")]
    ReleaseMismatch,
    /// Eligibility refers to a different publisher authorization record.
    #[error("federation eligibility names a different publisher authorization")]
    PublisherAuthorizationMismatch,
    /// The governed eligibility snapshot denies this behavior.
    #[error("release behavior is denied")]
    BehaviorDenied {
        /// Behavior the caller attempted to authorize.
        behavior: ReleaseBehavior,
    },
    /// A retired release grants no official behavior.
    #[error("release is retired")]
    ReleaseRetired,
}

/// Exact composition of independent publisher and federation release records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CertifiedReleaseProof {
    publisher: PublisherReleaseAuthorization,
    federation: ReleaseEligibilityDecision,
}

impl CertifiedReleaseProof {
    /// Composes independent records only when both name the same authorization.
    ///
    /// # Errors
    ///
    /// Rejects release or publisher-authorization substitution.
    pub fn compose(
        publisher: PublisherReleaseAuthorization,
        federation: ReleaseEligibilityDecision,
    ) -> Result<Self, ReleaseGateRefusal> {
        if publisher.release != federation.release {
            return Err(ReleaseGateRefusal::ReleaseMismatch);
        }
        if publisher.id != federation.publisher {
            return Err(ReleaseGateRefusal::PublisherAuthorizationMismatch);
        }
        Ok(Self {
            publisher,
            federation,
        })
    }

    /// Narrows this proof to one explicitly eligible behavior.
    ///
    /// # Errors
    ///
    /// Rejects a denied behavior or any use of a retired release.
    pub const fn authorize(
        self,
        behavior: ReleaseBehavior,
    ) -> Result<CertifiedBehaviorRelease, ReleaseGateRefusal> {
        let EligibilityDisposition::Active(snapshot) = self.federation.disposition else {
            return Err(ReleaseGateRefusal::ReleaseRetired);
        };
        let eligibility = snapshot.for_behavior(behavior);
        if matches!(eligibility, BehaviorEligibility::Denied) {
            return Err(ReleaseGateRefusal::BehaviorDenied { behavior });
        }
        Ok(CertifiedBehaviorRelease {
            proof: self,
            behavior,
            eligibility,
        })
    }
}

/// Certified Release proof narrowed to one allowed or grace behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CertifiedBehaviorRelease {
    proof: CertifiedReleaseProof,
    behavior: ReleaseBehavior,
    eligibility: BehaviorEligibility,
}

impl CertifiedBehaviorRelease {
    /// Returns the exact runtime release authorized for this behavior.
    #[must_use]
    pub const fn release(self) -> RuntimeReleaseBinding {
        self.proof.publisher.release
    }

    /// Returns the narrowed behavior.
    #[must_use]
    pub const fn behavior(self) -> ReleaseBehavior {
        self.behavior
    }

    /// Returns whether the behavior is fully allowed or in governed grace.
    #[must_use]
    pub const fn eligibility(self) -> BehaviorEligibility {
        self.eligibility
    }
}
