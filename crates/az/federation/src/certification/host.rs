use crate::{ContentDigest, InstanceHostId, OperatorId, RulesetBinding, RuntimeReleaseBinding};

/// Exact host relation to which evidence and a capability grant apply.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CertifiedHostBinding {
    operator: OperatorId,
    host: InstanceHostId,
    release: RuntimeReleaseBinding,
    ruleset: RulesetBinding,
    topology: ContentDigest,
    jurisdiction: ContentDigest,
}

impl CertifiedHostBinding {
    /// Creates the complete host evidence and grant subject.
    #[must_use]
    pub const fn new(
        operator: OperatorId,
        host: InstanceHostId,
        release: RuntimeReleaseBinding,
        ruleset: RulesetBinding,
        topology: ContentDigest,
        jurisdiction: ContentDigest,
    ) -> Self {
        Self {
            operator,
            host,
            release,
            ruleset,
            topology,
            jurisdiction,
        }
    }

    /// Returns the accountable operator.
    #[must_use]
    pub const fn operator(&self) -> OperatorId {
        self.operator
    }

    /// Returns the exact host identity.
    #[must_use]
    pub const fn host(&self) -> InstanceHostId {
        self.host
    }

    /// Returns the exact release observed on the host.
    #[must_use]
    pub const fn release(&self) -> RuntimeReleaseBinding {
        self.release
    }

    /// Returns the game-owned Ruleset binding.
    #[must_use]
    pub const fn ruleset(&self) -> &RulesetBinding {
        &self.ruleset
    }

    /// Returns the measured deployment-topology commitment.
    #[must_use]
    pub const fn topology(&self) -> ContentDigest {
        self.topology
    }

    /// Returns the jurisdiction-policy commitment.
    #[must_use]
    pub const fn jurisdiction(&self) -> ContentDigest {
        self.jurisdiction
    }
}
