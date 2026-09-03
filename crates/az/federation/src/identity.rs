use std::{fmt, num::NonZeroU64};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Digest of immutable canonical content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentDigest([u8; 32]);

impl ContentDigest {
    /// Restores a digest from canonical bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the canonical digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

macro_rules! digest_identity {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(ContentDigest);

        impl $name {
            /// Constructs the domain identity from its canonical digest.
            #[must_use]
            pub const fn from_digest(digest: ContentDigest) -> Self {
                Self(digest)
            }

            /// Returns the canonical identity digest.
            #[must_use]
            pub const fn digest(self) -> ContentDigest {
                self.0
            }
        }
    };
}

digest_identity!(FederationId, "Identity of one Certified Federation.");
digest_identity!(OperatorId, "Identity of one accountable server operator.");
digest_identity!(InstanceHostId, "Identity of one certified instance host.");
digest_identity!(
    FederationPrincipalId,
    "Federation-scoped identity of an authenticated principal."
);
digest_identity!(
    AuthorityAtomId,
    "Identity of one game-defined serial authority atom."
);
digest_identity!(
    PublisherReleaseAuthorizationId,
    "Identity of one immutable publisher release authorization."
);
digest_identity!(
    ReleaseEligibilityDecisionId,
    "Identity of one governed federation release-eligibility decision."
);
digest_identity!(
    HostChallengeId,
    "Identity of one attributed host challenge."
);
digest_identity!(
    EvidenceSnapshotId,
    "Identity of one immutable host evidence snapshot."
);

/// Canonical UTC microseconds observed by federation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthorityTime(i64);

impl AuthorityTime {
    /// Restores an authority-observed UTC timestamp.
    #[must_use]
    pub const fn from_unix_micros(value: i64) -> Self {
        Self(value)
    }

    /// Returns canonical UTC microseconds.
    #[must_use]
    pub const fn as_unix_micros(self) -> i64 {
        self.0
    }
}

/// Identity of one caller intent for idempotent mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationId(Uuid);

/// An operation identity must not be nil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("federation operation identity must not be nil")]
pub struct InvalidOperationId;

impl OperationId {
    /// Allocates a time-ordered operation identity.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Restores a protocol or persistence identity after rejecting nil.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidOperationId`] for the nil UUID.
    pub const fn try_from_uuid(value: Uuid) -> Result<Self, InvalidOperationId> {
        if value.is_nil() {
            Err(InvalidOperationId)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the UUID representation.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A monotonic federation value must be nonzero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("monotonic federation value must be nonzero")]
pub struct InvalidMonotonicValue;

macro_rules! monotonic_identity {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(NonZeroU64);

        impl $name {
            /// Returns the integer representation.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }

        impl TryFrom<u64> for $name {
            type Error = InvalidMonotonicValue;

            fn try_from(value: u64) -> Result<Self, Self::Error> {
                NonZeroU64::new(value)
                    .map(Self)
                    .ok_or(InvalidMonotonicValue)
            }
        }
    };
}

monotonic_identity!(
    AuthorityEpoch,
    "Visible recovery epoch of federation authority."
);
monotonic_identity!(
    AuthoritySequence,
    "Ordered receipt sequence within an authority atom."
);
monotonic_identity!(GrantRevision, "Revision of one bounded capability grant.");
monotonic_identity!(PolicyRevision, "Revision of one active federation policy.");

/// Ordered revocation view observed by a host-produced request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RevocationCursor(u64);

impl RevocationCursor {
    /// Constructs a cursor. Zero identifies the genesis view.
    #[must_use]
    pub const fn new(sequence: u64) -> Self {
        Self(sequence)
    }

    /// Returns the ordered revocation sequence.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}
