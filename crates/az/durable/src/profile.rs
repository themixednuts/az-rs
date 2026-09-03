use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ContentDigest;

/// Invalid nil profile identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("durable profile identity cannot be nil")]
pub struct InvalidDurableProfileId;

/// Exact proved adapter-profile identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DurableProfileId(Uuid);

impl DurableProfileId {
    /// Validates a caller-owned profile UUID.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDurableProfileId`] for nil.
    pub const fn try_from_uuid(value: Uuid) -> Result<Self, InvalidDurableProfileId> {
        if value.is_nil() {
            Err(InvalidDurableProfileId)
        } else {
            Ok(Self(value))
        }
    }
}

/// Content-addressed conformance evidence for one exact profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConformanceEvidenceRef(ContentDigest);

impl ConformanceEvidenceRef {
    /// Wraps an already verified evidence digest.
    #[must_use]
    pub const fn new(digest: ContentDigest) -> Self {
        Self(digest)
    }
}

/// An adapter profile paired with the evidence that qualified it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenProfileRef {
    profile: DurableProfileId,
    evidence: ConformanceEvidenceRef,
}

impl ProvenProfileRef {
    /// Binds an exact profile to its conformance evidence.
    #[must_use]
    pub const fn new(profile: DurableProfileId, evidence: ConformanceEvidenceRef) -> Self {
        Self { profile, evidence }
    }

    /// Returns the qualified profile identity.
    #[must_use]
    pub const fn profile(self) -> DurableProfileId {
        self.profile
    }

    /// Returns its evidence reference.
    #[must_use]
    pub const fn evidence(self) -> ConformanceEvidenceRef {
        self.evidence
    }
}

/// Minimum profile requirement for an operation.
///
/// The first reference slice requires one exact qualified profile. Capability
/// envelopes extend this closed value without introducing provider facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableProfileRequirement {
    exact: ProvenProfileRef,
}

impl DurableProfileRequirement {
    /// Requires one exact qualified profile.
    #[must_use]
    pub const fn exact(profile: ProvenProfileRef) -> Self {
        Self { exact: profile }
    }

    /// Checks a proved adapter profile.
    ///
    /// # Errors
    ///
    /// Returns a typed mismatch without exposing provider details.
    pub fn check(self, actual: ProvenProfileRef) -> Result<(), ProfileMismatch> {
        if self.exact == actual {
            Ok(())
        } else {
            Err(ProfileMismatch {
                required: self,
                actual,
            })
        }
    }
}

/// A proved adapter profile does not satisfy an operation's requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMismatch {
    /// Required profile contract.
    pub required: DurableProfileRequirement,
    /// Actual qualified profile.
    pub actual: ProvenProfileRef,
}
