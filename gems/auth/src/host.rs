//! The engine-owned auth host contract.
//!
//! A generated `auth-host` target serves this surface over its transport. The
//! reference implementation lives in [`crate::authority`] (feature `host`). The
//! host owns session signing, refresh protection, and revocation; providers are
//! composed in but never hold keys.

use serde::{Deserialize, Serialize};

use crate::credential::{Audience, CredentialEnvelope};
use crate::identity::ProviderId;
use crate::session::{Scope, SessionClaims, SessionGrant, SessionId};
use crate::verification::AuthError;

/// Static request-safety limits the host enforces before provider work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLimits {
    /// Maximum credential payload accepted (mirrors
    /// [`crate::credential::MAX_CREDENTIAL_BYTES`]).
    pub max_credential_bytes: usize,
    /// Maximum exchanges per source per window (rate limit input).
    pub max_exchanges_per_minute: u32,
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            max_credential_bytes: crate::credential::MAX_CREDENTIAL_BYTES,
            max_exchanges_per_minute: 60,
        }
    }
}

/// Provider/host metadata a client can query to drive its login UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostMetadata {
    pub primary_provider: ProviderId,
    pub link_providers: Vec<ProviderId>,
    pub audience: Audience,
    pub issuer: String,
}

/// Host liveness/readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostHealth {
    /// Ready to serve exchanges.
    Ready,
    /// Started but not yet ready (e.g. provider SDK still initializing).
    Starting,
    /// A dependency is unavailable; exchanges will fail.
    Degraded,
}

/// A request to exchange an external credential for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeRequest {
    pub credential: CredentialEnvelope,
    /// Scopes the client requests; the host may narrow them by policy.
    #[serde(default)]
    pub requested_scopes: Vec<Scope>,
}

/// The engine auth host surface. A transport layer (the generated binary) maps
/// its wire protocol onto these calls; game/project code depends on this trait,
/// never the concrete host or a provider SDK.
pub trait AuthHost: Send + Sync {
    /// Verify/exchange a credential and mint a signed session.
    ///
    /// # Errors
    /// Returns a stable [`AuthError`] (provider failure, limits, config).
    fn exchange(&self, request: ExchangeRequest) -> Result<SessionGrant, AuthError>;

    /// Validate an access token and return its claims.
    ///
    /// # Errors
    /// Returns [`AuthError::InvalidSession`] / [`AuthError::SessionRevoked`] /
    /// [`AuthError::Expired`] on failure.
    fn validate(&self, access_token: &str) -> Result<SessionClaims, AuthError>;

    /// Exchange a refresh token for a fresh session.
    ///
    /// # Errors
    /// Returns [`AuthError`] if the refresh token is invalid, expired, or
    /// revoked.
    fn refresh(&self, refresh_token: &str) -> Result<SessionGrant, AuthError>;

    /// Revoke a session (and any provider-side session).
    ///
    /// # Errors
    /// Returns [`AuthError`] on a storage/provider failure.
    fn revoke(&self, session: &SessionId) -> Result<(), AuthError>;

    /// Login-UI metadata.
    fn metadata(&self) -> HostMetadata;

    /// Enforced request limits.
    fn limits(&self) -> RequestLimits {
        RequestLimits::default()
    }

    /// Current health.
    fn health(&self) -> HostHealth {
        HostHealth::Ready
    }

    /// Begin graceful shutdown (stop accepting, drain, release providers).
    /// Default is a no-op for stateless hosts.
    fn shutdown(&self) {}
}
