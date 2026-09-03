//! Host-tool authentication manifest schema.

use serde::{Deserialize, Serialize};

/// Session issuance policy in the host-tool manifest schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    pub issuer: String,
    #[serde(default = "default_access_ttl")]
    pub access_ttl_seconds: u64,
    #[serde(default = "default_refresh_ttl")]
    pub refresh_ttl_seconds: u64,
    pub signing_key: az_secrets::SecretRef,
}

const fn default_access_ttl() -> u64 {
    900
}

const fn default_refresh_ttl() -> u64 {
    2_592_000
}

/// The `[services.auth]` host-tool configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthServiceConfig {
    #[serde(default)]
    pub enabled: bool,
    pub provider: az_gem_auth::ProviderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_authority: Option<String>,
    pub audience: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link_providers: Vec<az_gem_auth::ProviderId>,
    pub session: SessionConfig,
}

impl AuthServiceConfig {
    /// Validates cross-field host configuration invariants.
    ///
    /// # Errors
    ///
    /// Returns a typed error for invalid identity, lifetime, or authority
    /// relationships.
    pub fn validate(&self) -> Result<(), AuthConfigError> {
        if self.audience.trim().is_empty() {
            return Err(AuthConfigError::EmptyAudience);
        }
        if self.session.issuer.trim().is_empty() {
            return Err(AuthConfigError::EmptyIssuer);
        }
        if self.link_providers.contains(&self.provider) {
            return Err(AuthConfigError::ProviderIsOwnLink);
        }
        if self.session.access_ttl_seconds == 0 {
            return Err(AuthConfigError::ZeroAccessTtl);
        }
        if self
            .session_authority
            .as_deref()
            .is_some_and(|authority| authority.trim().is_empty())
        {
            return Err(AuthConfigError::EmptySessionAuthority);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthConfigError {
    #[error("`[services.auth].audience` must not be empty")]
    EmptyAudience,
    #[error("`[services.auth.session].issuer` must not be empty")]
    EmptyIssuer,
    #[error("the primary provider must not also be a link provider")]
    ProviderIsOwnLink,
    #[error("`[services.auth.session].access_ttl_seconds` must be non-zero")]
    ZeroAccessTtl,
    #[error("`[services.auth].session_authority` must name a gem id when present")]
    EmptySessionAuthority,
}
