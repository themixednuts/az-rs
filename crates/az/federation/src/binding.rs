use arrayvec::ArrayString;
use az_world_instance::PlacementFence;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{
    AuthorityEpoch, ContentDigest, FederationId, GrantRevision, InstanceHostId, OperatorId,
    PolicyRevision, RevocationCursor,
};

/// Maximum canonical byte length of a Ruleset namespace.
pub const MAX_RULESET_NAMESPACE_BYTES: usize = 96;

/// A Ruleset namespace is empty, too long, or contains a noncanonical byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RulesetNamespaceError {
    /// A namespace must contain at least one byte.
    #[error("Ruleset namespace must not be empty")]
    Empty,
    /// A namespace exceeds the protocol bound.
    #[error("Ruleset namespace exceeds {MAX_RULESET_NAMESPACE_BYTES} bytes")]
    TooLong,
    /// A namespace contains a byte outside its canonical ASCII alphabet.
    #[error("Ruleset namespace contains a noncanonical byte")]
    InvalidByte,
}

/// Game-owned namespace that prevents Ruleset identity collisions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RulesetNamespace(ArrayString<MAX_RULESET_NAMESPACE_BYTES>);

impl<'de> Deserialize<'de> for RulesetNamespace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = ArrayString::<MAX_RULESET_NAMESPACE_BYTES>::deserialize(deserializer)?;
        Self::parse(value.as_str()).map_err(D::Error::custom)
    }
}

impl RulesetNamespace {
    /// Parses the canonical lowercase ASCII namespace alphabet.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, uppercase, whitespace, and punctuation outside
    /// `-`, `_`, `.`, and `/`.
    pub fn parse(value: &str) -> Result<Self, RulesetNamespaceError> {
        if value.is_empty() {
            return Err(RulesetNamespaceError::Empty);
        }
        if value.len() > MAX_RULESET_NAMESPACE_BYTES {
            return Err(RulesetNamespaceError::TooLong);
        }
        if !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_./".contains(&byte)
        }) {
            return Err(RulesetNamespaceError::InvalidByte);
        }
        let mut canonical = ArrayString::new();
        canonical
            .try_push_str(value)
            .map_err(|_| RulesetNamespaceError::TooLong)?;
        Ok(Self(canonical))
    }

    /// Returns the canonical namespace text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Exact bounded grant and revision used by one certified effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityGrantRef {
    id: ContentDigest,
    revision: GrantRevision,
}

impl CapabilityGrantRef {
    /// Binds a grant identity to its exact revision.
    #[must_use]
    pub const fn new(id: ContentDigest, revision: GrantRevision) -> Self {
        Self { id, revision }
    }

    /// Returns the grant identity.
    #[must_use]
    pub const fn id(self) -> ContentDigest {
        self.id
    }

    /// Returns the exact grant revision.
    #[must_use]
    pub const fn revision(self) -> GrantRevision {
        self.revision
    }
}

/// Immutable publisher release meaning pinned to one certified effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeReleaseBinding(ContentDigest);

impl RuntimeReleaseBinding {
    /// Constructs a release binding from its canonical release digest.
    #[must_use]
    pub const fn new(digest: ContentDigest) -> Self {
        Self(digest)
    }

    /// Returns the canonical release digest.
    #[must_use]
    pub const fn digest(self) -> ContentDigest {
        self.0
    }
}

/// Game-owned Ruleset namespace and immutable meaning.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RulesetBinding {
    namespace: RulesetNamespace,
    digest: ContentDigest,
}

impl RulesetBinding {
    /// Binds a namespaced Ruleset to its canonical meaning.
    #[must_use]
    pub const fn new(namespace: RulesetNamespace, digest: ContentDigest) -> Self {
        Self { namespace, digest }
    }

    /// Returns the game-owned namespace.
    #[must_use]
    pub const fn namespace(&self) -> &RulesetNamespace {
        &self.namespace
    }

    /// Returns the canonical Ruleset digest.
    #[must_use]
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }
}

/// Active policy identity and exact revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActivePolicyRef {
    id: ContentDigest,
    revision: PolicyRevision,
}

impl ActivePolicyRef {
    /// Binds an active policy identity to its exact revision.
    #[must_use]
    pub const fn new(id: ContentDigest, revision: PolicyRevision) -> Self {
        Self { id, revision }
    }

    /// Returns the policy identity.
    #[must_use]
    pub const fn id(self) -> ContentDigest {
        self.id
    }

    /// Returns the active policy revision.
    #[must_use]
    pub const fn revision(self) -> PolicyRevision {
        self.revision
    }
}

/// Complete authority context required for every certified placement effect.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CertifiedExecutionBinding {
    federation: FederationId,
    authority_epoch: AuthorityEpoch,
    operator: OperatorId,
    host: InstanceHostId,
    placement: PlacementFence,
    grant: CapabilityGrantRef,
    runtime_release: RuntimeReleaseBinding,
    ruleset: RulesetBinding,
    active_policy: ActivePolicyRef,
    revocation_view: RevocationCursor,
}

impl CertifiedExecutionBinding {
    /// Creates one complete placement-produced authority binding.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        federation: FederationId,
        authority_epoch: AuthorityEpoch,
        operator: OperatorId,
        host: InstanceHostId,
        placement: PlacementFence,
        grant: CapabilityGrantRef,
        runtime_release: RuntimeReleaseBinding,
        ruleset: RulesetBinding,
        active_policy: ActivePolicyRef,
        revocation_view: RevocationCursor,
    ) -> Self {
        Self {
            federation,
            authority_epoch,
            operator,
            host,
            placement,
            grant,
            runtime_release,
            ruleset,
            active_policy,
            revocation_view,
        }
    }

    /// Returns the federation identity.
    #[must_use]
    pub const fn federation(&self) -> FederationId {
        self.federation
    }
    /// Returns the visible authority epoch.
    #[must_use]
    pub const fn authority_epoch(&self) -> AuthorityEpoch {
        self.authority_epoch
    }
    /// Returns the accountable operator.
    #[must_use]
    pub const fn operator(&self) -> OperatorId {
        self.operator
    }
    /// Returns the instance host.
    #[must_use]
    pub const fn host(&self) -> InstanceHostId {
        self.host
    }
    /// Returns the lifecycle-owned placement fence.
    #[must_use]
    pub const fn placement(&self) -> PlacementFence {
        self.placement
    }
    /// Returns the bounded grant reference.
    #[must_use]
    pub const fn grant(&self) -> CapabilityGrantRef {
        self.grant
    }
    /// Returns the immutable release binding.
    #[must_use]
    pub const fn runtime_release(&self) -> RuntimeReleaseBinding {
        self.runtime_release
    }
    /// Returns the game-owned Ruleset binding.
    #[must_use]
    pub const fn ruleset(&self) -> &RulesetBinding {
        &self.ruleset
    }
    /// Returns the active policy reference.
    #[must_use]
    pub const fn active_policy(&self) -> ActivePolicyRef {
        self.active_policy
    }
    /// Returns the observed revocation cursor.
    #[must_use]
    pub const fn revocation_view(&self) -> RevocationCursor {
        self.revocation_view
    }
}
