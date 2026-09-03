//! Verified external identities and stable error classes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::identity::ProviderSubject;

/// How strongly the provider vouches for the verified subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Assurance {
    /// Unauthenticated or dev-only (local provider default).
    #[default]
    None,
    /// Ownership of a platform account was proven (Steam ticket, EOS Connect).
    Platform,
    /// A stronger factor was presented (e.g. Epic Account Services).
    Strong,
}

/// Neutral provider claims carried alongside a verified identity. Values are
/// provider-declared and title-augmentable, never SDK enums.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderClaims(pub BTreeMap<String, String>);

impl ProviderClaims {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

/// The result of a provider verifying or exchanging an external credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedExternalIdentity {
    pub subject: ProviderSubject,
    #[serde(default)]
    pub assurance: Assurance,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub claims: ProviderClaims,
}

impl VerifiedExternalIdentity {
    #[must_use]
    pub fn new(subject: ProviderSubject) -> Self {
        Self {
            subject,
            assurance: Assurance::default(),
            display_name: None,
            claims: ProviderClaims::default(),
        }
    }

    #[must_use]
    pub const fn with_assurance(mut self, assurance: Assurance) -> Self {
        self.assurance = assurance;
        self
    }

    #[must_use]
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    #[must_use]
    pub fn with_claims(mut self, claims: ProviderClaims) -> Self {
        self.claims = claims;
        self
    }
}

/// Stable, provider-neutral authentication error classes. Providers map their
/// SDK-specific failures onto these; game and host code branch on them without
/// knowing which provider produced them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// The credential scheme is not one this provider accepts.
    #[error("unsupported credential scheme `{0}`")]
    UnsupportedScheme(String),
    /// The credential was addressed to a different audience.
    #[error("credential audience `{got}` does not match expected `{expected}`")]
    AudienceMismatch { expected: String, got: String },
    /// The credential is malformed or failed provider validation.
    #[error("invalid credential: {0}")]
    InvalidCredential(String),
    /// The credential was valid but has expired.
    #[error("credential expired")]
    Expired,
    /// The subject does not own the required license/entitlement.
    #[error("subject is not entitled")]
    NotEntitled,
    /// The subject is banned or suspended by the provider.
    #[error("subject is banned")]
    Banned,
    /// A replayed credential was detected.
    #[error("credential replay detected")]
    Replay,
    /// The identity exists but is suspended/retired in the identity store.
    #[error("identity is not active")]
    IdentityInactive,
    /// The provider needs an additional step (e.g. EOS `CreateUser` continuance).
    /// The opaque token is provider state the client echoes back.
    #[error("continuation required")]
    ContinuationRequired { token: String },
    /// The session token failed signature, issuer, audience, or expiry checks.
    #[error("invalid session: {0}")]
    InvalidSession(String),
    /// The session was explicitly revoked.
    #[error("session revoked")]
    SessionRevoked,
    /// A transient provider or infrastructure failure; retry may succeed.
    #[error("provider unavailable: {0}")]
    ProviderUnavailable(String),
    /// A configuration or key-resolution failure in the host.
    #[error("host configuration error: {0}")]
    HostConfiguration(String),
}
