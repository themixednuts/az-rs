//! Client- and host-side provider contracts.
//!
//! These are the *only* surfaces a provider gem implements. Crucially,
//! [`AuthProvider`] receives an external credential and returns a verified
//! identity — it never receives session signing keys, refresh-token encryption
//! keys, or the host key manager. The engine host orchestrates
//! verify → augment → sign; the provider is one non-privileged step.

use crate::credential::{Audience, CredentialEnvelope, CredentialLease, CredentialScheme};
use crate::identity::ProviderId;
use crate::verification::{AuthError, VerifiedExternalIdentity};

/// What a provider can do, declared to the host so it can reject unsupported
/// operations without calling the SDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub provider: ProviderId,
    /// Credential schemes this provider accepts.
    pub schemes: Vec<CredentialScheme>,
    /// Whether the provider can re-verify for refresh (vs. host-only refresh).
    pub supports_refresh: bool,
    /// Whether the provider must be told to release/revoke external sessions.
    pub supports_revoke: bool,
}

impl ProviderCapabilities {
    #[must_use]
    pub const fn new(provider: ProviderId, schemes: Vec<CredentialScheme>) -> Self {
        Self {
            provider,
            schemes,
            supports_refresh: false,
            supports_revoke: false,
        }
    }

    #[must_use]
    pub fn accepts(&self, scheme: &CredentialScheme) -> bool {
        self.schemes.contains(scheme)
    }
}

/// Resolved, non-secret material a provider gem needs to build its provider.
///
/// The gem's `auth_provider` entry point (ADR 0026 convention:
/// `pub fn auth_provider(&AuthProviderContext) -> Result<Box<dyn AuthProvider>,
/// AuthError>`, exported from its crate root) builds its [`AuthProvider`] from
/// this. The generated auth host constructs one from the resolved
/// `[services.auth]` and calls the selected provider gem's entry point.
///
/// SECURITY (ADR 0026 invariant): this context carries **no** signing-key
/// material. Providers verify external credentials and must have no path to
/// session keys; the signing material is delivered only to the authority via
/// [`crate::authority::SessionAuthorityContext`].
#[derive(Debug, Clone)]
pub struct AuthProviderContext {
    audience: Audience,
    provider: ProviderId,
}

impl AuthProviderContext {
    /// Build a provider context for `provider` restricted to `audience`.
    #[must_use]
    pub const fn new(audience: Audience, provider: ProviderId) -> Self {
        Self { audience, provider }
    }

    /// The relying-party audience the provider restricts credentials to.
    #[must_use]
    pub const fn audience(&self) -> &Audience {
        &self.audience
    }

    /// The provider id slot this gem is filling (its declared primary provider).
    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }
}

/// Client-side credential acquisition (game/runtime code). Produces the opaque
/// envelope a client sends to the auth host, plus a lease that releases the
/// underlying SDK handle.
pub trait CredentialSource {
    /// Acquire a credential of `scheme` for `audience`.
    ///
    /// # Errors
    /// Returns [`AuthError`] if the platform is unavailable or the scheme is
    /// unsupported by this source.
    fn acquire(
        &self,
        scheme: &CredentialScheme,
        audience: &Audience,
    ) -> Result<(CredentialEnvelope, CredentialLease), AuthError>;
}

/// Host-side credential verification/exchange (runs inside the generated auth
/// host). Implemented by provider gems; composed by the engine host.
pub trait AuthProvider: Send + Sync {
    /// The provider's declared capabilities.
    fn capabilities(&self) -> ProviderCapabilities;

    /// Verify or exchange an external credential into a provider identity.
    ///
    /// Implementations must reject a scheme they do not accept
    /// ([`AuthError::UnsupportedScheme`]) and an audience mismatch
    /// ([`AuthError::AudienceMismatch`]) before touching the payload.
    ///
    /// # Errors
    /// Returns a stable [`AuthError`] class describing the failure.
    fn verify_or_exchange(
        &self,
        credential: &CredentialEnvelope,
    ) -> Result<VerifiedExternalIdentity, AuthError>;

    /// Release any provider-side session for a revoked identity. Default is a
    /// no-op for providers without external session state.
    ///
    /// # Errors
    /// Returns [`AuthError`] if the provider revoke call fails.
    fn revoke_external(&self, _identity: &VerifiedExternalIdentity) -> Result<(), AuthError> {
        Ok(())
    }
}
