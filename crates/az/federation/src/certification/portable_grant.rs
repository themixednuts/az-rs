use std::num::NonZeroU32;

use crate::{ActivePolicyRef, CapabilityGrantRef, EvidenceSnapshotId, RevocationCursor};

use super::{CertifiedHostBinding, VerifiedHostChallenge};

/// Structural class of portable mutation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortableMutationClass {
    /// Create portable value under game policy.
    Mint,
    /// Move portable value between subjects or owners.
    Transfer,
    /// Destroy portable value under game policy.
    Destroy,
    /// Convert portable value through a declared Ruleset edge.
    Convert,
    /// Adjust portable state without creating a distinct value transfer.
    Adjust,
}

impl PortableMutationClass {
    const fn bit(self) -> u8 {
        match self {
            Self::Mint => 1 << 0,
            Self::Transfer => 1 << 1,
            Self::Destroy => 1 << 2,
            Self::Convert => 1 << 3,
            Self::Adjust => 1 << 4,
        }
    }
}

/// Fixed, bounded set of portable mutation classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortableMutationClasses(u8);

impl PortableMutationClasses {
    /// Creates a set containing exactly one class.
    #[must_use]
    pub const fn only(class: PortableMutationClass) -> Self {
        Self(class.bit())
    }

    /// Adds one class to the set.
    #[must_use]
    pub const fn with(self, class: PortableMutationClass) -> Self {
        Self(self.0 | class.bit())
    }

    /// Returns the classes present in both bounded sets.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Reports whether the set contains one class.
    #[must_use]
    pub const fn contains(self, class: PortableMutationClass) -> bool {
        self.0 & class.bit() != 0
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Bounded portable mutation authority for one policy window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortableMutationCeiling {
    classes: PortableMutationClasses,
    max_operations_per_window: NonZeroU32,
    max_offline_operations: u32,
}

impl PortableMutationCeiling {
    /// Constructs a nonempty typed ceiling.
    ///
    /// # Errors
    ///
    /// Rejects an empty class set, which cannot authorize useful work.
    pub const fn new(
        classes: PortableMutationClasses,
        max_operations_per_window: NonZeroU32,
        max_offline_operations: u32,
    ) -> Result<Self, PortableMutationGrantError> {
        if classes.is_empty() {
            return Err(PortableMutationGrantError::NoSharedAuthority);
        }
        Ok(Self {
            classes,
            max_operations_per_window,
            max_offline_operations,
        })
    }

    /// Returns the structural classes permitted by this ceiling.
    #[must_use]
    pub const fn classes(self) -> PortableMutationClasses {
        self.classes
    }

    /// Returns the operation limit for the signed policy window.
    #[must_use]
    pub const fn max_operations_per_window(self) -> NonZeroU32 {
        self.max_operations_per_window
    }

    /// Returns the maximum operations permitted without online authority.
    #[must_use]
    pub const fn max_offline_operations(self) -> u32 {
        self.max_offline_operations
    }

    fn intersect(self, other: Self) -> Result<Self, PortableMutationGrantError> {
        let classes = self.classes.intersection(other.classes);
        let per_window = self
            .max_operations_per_window
            .min(other.max_operations_per_window);
        let offline = self
            .max_offline_operations
            .min(other.max_offline_operations);
        Self::new(classes, per_window, offline)
    }
}

/// Behavior-specific request for bounded portable mutation authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableMutationGrantRequest {
    subject: CertifiedHostBinding,
    requested: PortableMutationCeiling,
    evidence_snapshot: EvidenceSnapshotId,
    policy: ActivePolicyRef,
}

impl PortableMutationGrantRequest {
    /// Binds the requested ceiling to exact host, evidence, and policy facts.
    #[must_use]
    pub const fn new(
        subject: CertifiedHostBinding,
        requested: PortableMutationCeiling,
        evidence_snapshot: EvidenceSnapshotId,
        policy: ActivePolicyRef,
    ) -> Self {
        Self {
            subject,
            requested,
            evidence_snapshot,
            policy,
        }
    }

    /// Returns the requested ceiling before evidence and operator narrowing.
    #[must_use]
    pub const fn requested(&self) -> PortableMutationCeiling {
        self.requested
    }

    /// Returns the immutable evidence-snapshot commitment.
    #[must_use]
    pub const fn evidence_snapshot(&self) -> EvidenceSnapshotId {
        self.evidence_snapshot
    }

    /// Returns the exact game and federation policy revision.
    #[must_use]
    pub const fn policy(&self) -> ActivePolicyRef {
        self.policy
    }
}

/// Closed issuance or lifecycle failures for a portable mutation grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PortableMutationGrantError {
    /// Request, evidence, and operator consent share no mutation class.
    #[error("request, evidence, and operator consent share no portable authority")]
    NoSharedAuthority,
    /// Suspension or revocation did not advance the observed revocation view.
    #[error("grant contraction did not advance the revocation cursor")]
    NonForwardRevocation,
}

/// Current prospective standing of this specific portable grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortableMutationGrantStanding {
    /// New work may use the bounded ceiling.
    Active,
    /// New work is temporarily closed for this grant.
    Suspended,
    /// New work is permanently closed for this grant revision.
    Revoked,
}

/// Precise reason a grant cannot authorize new work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GrantUseRefusal {
    /// The caller has not reconciled to the grant's current revocation view.
    #[error("portable grant revocation view is stale")]
    StaleRevocationView,
    /// The exact mutation class is outside the grant.
    #[error("portable mutation class is not granted")]
    CapabilityNotGranted,
    /// The grant is suspended for new work.
    #[error("portable mutation grant is suspended")]
    Suspended,
    /// The grant revision has been revoked.
    #[error("portable mutation grant is revoked")]
    Revoked,
}

/// Issued portable mutation grant after three-way ceiling narrowing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableMutationGrant {
    grant: CapabilityGrantRef,
    request: PortableMutationGrantRequest,
    ceiling: PortableMutationCeiling,
    revocation_view: RevocationCursor,
    standing: PortableMutationGrantStanding,
}

impl PortableMutationGrant {
    /// Issues no more than the request, verified eligibility, or operator consent.
    ///
    /// # Errors
    ///
    /// Refuses issuance when the three ceilings share no class.
    pub fn issue(
        grant: CapabilityGrantRef,
        verified_challenge: VerifiedHostChallenge,
        federation_ceiling: PortableMutationCeiling,
        operator_ceiling: PortableMutationCeiling,
        revocation_view: RevocationCursor,
    ) -> Result<Self, PortableMutationGrantError> {
        let request = verified_challenge.into_request();
        let ceiling = request
            .requested
            .intersect(federation_ceiling)?
            .intersect(operator_ceiling)?;
        Ok(Self {
            grant,
            request,
            ceiling,
            revocation_view,
            standing: PortableMutationGrantStanding::Active,
        })
    }

    /// Returns the immutable grant identity and revision.
    #[must_use]
    pub const fn grant(&self) -> CapabilityGrantRef {
        self.grant
    }

    /// Returns the exact host relation supported by the evidence snapshot.
    #[must_use]
    pub const fn subject(&self) -> &CertifiedHostBinding {
        &self.request.subject
    }

    /// Returns the effective three-way-narrowed ceiling.
    #[must_use]
    pub const fn ceiling(&self) -> PortableMutationCeiling {
        self.ceiling
    }

    /// Returns the current grant standing.
    #[must_use]
    pub const fn standing(&self) -> PortableMutationGrantStanding {
        self.standing
    }

    /// Returns the revocation view callers must have observed.
    #[must_use]
    pub const fn revocation_view(&self) -> RevocationCursor {
        self.revocation_view
    }

    /// Temporarily closes new work at a strictly newer revocation view.
    ///
    /// Resuming authority requires a new attributable grant revision; this
    /// grant has no method that widens it back to active.
    ///
    /// # Errors
    ///
    /// Rejects a cursor that does not advance the recorded view.
    pub const fn suspend(
        &mut self,
        revocation_view: RevocationCursor,
    ) -> Result<(), PortableMutationGrantError> {
        self.contract(revocation_view, PortableMutationGrantStanding::Suspended)
    }

    /// Checks new work against exact class, standing, and revocation freshness.
    ///
    /// # Errors
    ///
    /// Refuses stale, ungranted, suspended, or revoked work.
    pub const fn authorize_new_work(
        &self,
        class: PortableMutationClass,
        observed_revocation: RevocationCursor,
    ) -> Result<(), GrantUseRefusal> {
        if observed_revocation.get() != self.revocation_view.get() {
            return Err(GrantUseRefusal::StaleRevocationView);
        }
        match self.standing {
            PortableMutationGrantStanding::Suspended => return Err(GrantUseRefusal::Suspended),
            PortableMutationGrantStanding::Revoked => return Err(GrantUseRefusal::Revoked),
            PortableMutationGrantStanding::Active => {}
        }
        if !self.ceiling.classes.contains(class) {
            return Err(GrantUseRefusal::CapabilityNotGranted);
        }
        Ok(())
    }

    /// Permanently closes new work at a strictly newer revocation view.
    ///
    /// # Errors
    ///
    /// Rejects a cursor that does not advance the recorded view.
    pub const fn revoke(
        &mut self,
        revocation_view: RevocationCursor,
    ) -> Result<(), PortableMutationGrantError> {
        self.contract(revocation_view, PortableMutationGrantStanding::Revoked)
    }

    const fn contract(
        &mut self,
        revocation_view: RevocationCursor,
        standing: PortableMutationGrantStanding,
    ) -> Result<(), PortableMutationGrantError> {
        if revocation_view.get() <= self.revocation_view.get() {
            return Err(PortableMutationGrantError::NonForwardRevocation);
        }
        self.revocation_view = revocation_view;
        self.standing = standing;
        Ok(())
    }
}
