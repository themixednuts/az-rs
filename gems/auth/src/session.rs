//! Sessions: signed claims minted by the engine host after verification.

use serde::{Deserialize, Serialize};

use crate::identity::ProviderId;
use crate::verification::Assurance;

/// A capability scope granted to a session, e.g. `game:play` or `profile:read`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scope(String);

impl Scope {
    #[must_use]
    pub fn new(scope: impl Into<String>) -> Self {
        Self(scope.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Unique session identifier (the JWT `jti`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The signed claim body of an access token.
///
/// Field names are the registered JWT claims so this serializes directly as a
/// JWT payload. `sub` is the durable identity id; `provider` records which
/// provider authenticated it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionClaims {
    /// Subject — the durable identity id (not the raw provider subject).
    pub sub: String,
    /// Issuer — the configured `[services.auth.session].issuer`.
    pub iss: String,
    /// Audience — the configured relying-party audience.
    pub aud: String,
    /// Expiry, unix seconds.
    pub exp: u64,
    /// Issued-at, unix seconds.
    pub iat: u64,
    /// JWT id — the [`SessionId`].
    pub jti: String,
    /// The provider that authenticated this session.
    pub provider: ProviderId,
    /// Granted capability scopes.
    #[serde(default)]
    pub scopes: Vec<Scope>,
    /// The provider assurance level carried into the session.
    #[serde(default)]
    pub assurance: Assurance,
}

impl SessionClaims {
    #[must_use]
    pub fn session_id(&self) -> SessionId {
        SessionId::new(self.jti.clone())
    }

    #[must_use]
    pub fn has_scope(&self, scope: &Scope) -> bool {
        self.scopes.contains(scope)
    }
}

/// A minted session: the access token, an optional refresh token, and the
/// decoded claims for immediate use without re-parsing.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionGrant {
    pub session_id: SessionId,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub expires_at: u64,
    pub claims: SessionClaims,
}

impl std::fmt::Debug for SessionGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionGrant")
            .field("session_id", &self.session_id)
            .field("access_token", &format_args!("<redacted>"))
            .field(
                "refresh_token",
                &format_args!(
                    "{}",
                    if self.refresh_token.is_some() {
                        "<redacted>"
                    } else {
                        "none"
                    }
                ),
            )
            .field("expires_at", &self.expires_at)
            .field("claims", &self.claims)
            .finish()
    }
}
