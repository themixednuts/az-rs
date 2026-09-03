//! Provider-neutral identity vocabulary.

use serde::{Deserialize, Serialize};

/// Stable identifier of an authentication provider, e.g. `azoth.auth.steam`.
///
/// This is the provider gem's contract identity, not a platform SDK enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque, provider-scoped subject identifier.
///
/// Examples: a Steam ID, an EOS Product User ID, a local account id. It is only
/// meaningful paired with its [`ProviderId`]; the engine never parses its
/// internal structure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubjectId(String);

impl SubjectId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SubjectId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A subject as seen through exactly one provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProviderSubject {
    pub provider: ProviderId,
    pub subject: SubjectId,
}

impl ProviderSubject {
    #[must_use]
    pub const fn new(provider: ProviderId, subject: SubjectId) -> Self {
        Self { provider, subject }
    }

    /// A stable, collision-resistant string form for use as a session subject
    /// claim: `<provider>:<subject>`.
    #[must_use]
    pub fn canonical(&self) -> String {
        format!("{}:{}", self.provider, self.subject)
    }
}

/// Lifecycle state of a linked [`Identity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityState {
    /// A fully usable identity.
    #[default]
    Active,
    /// Authentication is refused but the record is retained (ban/suspension).
    Suspended,
    /// The identity was retired; new sessions must not be issued.
    Retired,
}

/// A durable account identity that may link several [`ProviderSubject`]s.
///
/// The primary subject is the provider the account authenticated through; link
/// providers (secondary logins) are added by project identity-link policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub id: SubjectId,
    pub primary: ProviderSubject,
    #[serde(default)]
    pub linked: Vec<ProviderSubject>,
    #[serde(default)]
    pub state: IdentityState,
}

impl Identity {
    #[must_use]
    pub const fn new(id: SubjectId, primary: ProviderSubject) -> Self {
        Self {
            id,
            primary,
            linked: Vec::new(),
            state: IdentityState::Active,
        }
    }

    /// Every provider subject this identity is reachable through.
    pub fn subjects(&self) -> impl Iterator<Item = &ProviderSubject> {
        std::iter::once(&self.primary).chain(self.linked.iter())
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state == IdentityState::Active
    }
}
