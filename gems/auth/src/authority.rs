//! Reference host authority and host implementation (feature `host`).
//!
//! SECURITY: this is the engine-owned signing/lifecycle core. It is the subject
//! of the focused security review ADR 0026 requires before any production
//! provider ships. Notably, session ids here are unique but not sourced from a
//! CSPRNG, and revocation is an in-memory set — production must back both with
//! the identity store and a real random source. The *architecture* (providers
//! never touch keys) is final; these implementation controls are not.

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::extension::{ClaimsAugmenter, IdentityStore};
use crate::host::{AuthHost, ExchangeRequest, HostHealth, HostMetadata, RequestLimits};
use crate::identity::{Identity, ProviderId, ProviderSubject, SubjectId};
use crate::provider::AuthProvider;
use crate::session::{Scope, SessionClaims, SessionGrant, SessionId};
use crate::verification::{AuthError, VerifiedExternalIdentity};

/// The engine-owned session signer/validator. Providers never receive one.
pub trait SessionAuthority: Send + Sync {
    /// Mint a signed session for a resolved identity.
    ///
    /// # Errors
    /// Returns [`AuthError`] on a signing or clock failure.
    fn issue(
        &self,
        identity: &Identity,
        provider: &ProviderId,
        scopes: Vec<Scope>,
        assurance: crate::verification::Assurance,
    ) -> Result<SessionGrant, AuthError>;

    /// Validate an access token into its claims.
    ///
    /// # Errors
    /// Returns [`AuthError::InvalidSession`] / [`AuthError::Expired`].
    fn validate(&self, access_token: &str) -> Result<SessionClaims, AuthError>;

    /// Validate a refresh token and return the identity subject + provider it
    /// was issued for.
    ///
    /// # Errors
    /// Returns [`AuthError`] if the refresh token is invalid or expired.
    fn validate_refresh(&self, refresh_token: &str) -> Result<RefreshSubject, AuthError>;
}

/// Resolved signing material and session policy for a session-authority gem.
///
/// The gem's `session_authority` entry point (ADR 0026 convention:
/// `pub fn session_authority(&SessionAuthorityContext) -> Box<dyn
/// SessionAuthority>`, exported from its crate root) builds its authority from
/// this.
///
/// SECURITY (ADR 0026 invariant): this is the **only** auth context that
/// carries signing-key material. It is delivered to the authority — never to a
/// provider (see [`crate::provider::AuthProviderContext`]). The key bytes are
/// zeroized on drop.
pub struct SessionAuthorityContext {
    signing_key: zeroize::Zeroizing<Vec<u8>>,
    issuer: String,
    audience: String,
    access_ttl_seconds: u64,
    refresh_ttl_seconds: u64,
}

impl SessionAuthorityContext {
    /// Build an authority context from resolved signing bytes and session
    /// policy. `signing_key` is the material resolved from
    /// `[services.auth.session].signing_key`; it is zeroized on drop.
    #[must_use]
    pub fn new(
        signing_key: Vec<u8>,
        issuer: impl Into<String>,
        audience: impl Into<String>,
        access_ttl_seconds: u64,
        refresh_ttl_seconds: u64,
    ) -> Self {
        Self {
            signing_key: zeroize::Zeroizing::new(signing_key),
            issuer: issuer.into(),
            audience: audience.into(),
            access_ttl_seconds,
            refresh_ttl_seconds,
        }
    }

    /// The resolved session signing key bytes.
    #[must_use]
    pub fn signing_key(&self) -> &[u8] {
        &self.signing_key
    }

    /// The issuer (`iss`) sessions are minted under.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The audience (`aud`) sessions are minted for.
    #[must_use]
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// Access-token lifetime in seconds.
    #[must_use]
    pub const fn access_ttl_seconds(&self) -> u64 {
        self.access_ttl_seconds
    }

    /// Refresh-token lifetime in seconds.
    #[must_use]
    pub const fn refresh_ttl_seconds(&self) -> u64 {
        self.refresh_ttl_seconds
    }
}

/// The engine's default session-authority entry point (ADR 0026 convention).
///
/// The generated auth host calls this on the contract gem (`az_gem_auth`) when
/// no `[services.auth].session_authority` gem is selected, composing the
/// reference [`Hs256SessionAuthority`]. A session-authority gem exports a
/// function of this exact shape from its crate root; the host swaps the call
/// target based on the manifest selection with no other change.
#[must_use]
pub fn session_authority(context: &SessionAuthorityContext) -> Box<dyn SessionAuthority> {
    Box::new(Hs256SessionAuthority::new(
        context.signing_key(),
        context.issuer(),
        context.audience(),
        context.access_ttl_seconds(),
        context.refresh_ttl_seconds(),
    ))
}

/// The subject a refresh token resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshSubject {
    pub identity: SubjectId,
    pub provider: ProviderId,
    pub session: SessionId,
}

#[derive(Debug, Serialize, Deserialize)]
struct RefreshClaims {
    sub: String,
    iss: String,
    aud: String,
    exp: u64,
    iat: u64,
    jti: String,
    provider: ProviderId,
    typ: String,
}

/// HS256 reference authority. Holds the signing key privately; it is never
/// exposed to providers or the host trait surface.
pub struct Hs256SessionAuthority {
    issuer: String,
    audience: String,
    access_ttl: u64,
    refresh_ttl: u64,
    encoding: EncodingKey,
    decoding: DecodingKey,
    validation: Validation,
    refresh_validation: Validation,
}

impl Hs256SessionAuthority {
    /// Build an authority from the resolved signing key bytes.
    #[must_use]
    pub fn new(
        signing_key: &[u8],
        issuer: impl Into<String>,
        audience: impl Into<String>,
        access_ttl: u64,
        refresh_ttl: u64,
    ) -> Self {
        let issuer = issuer.into();
        let audience = audience.into();
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(std::slice::from_ref(&issuer));
        validation.set_audience(std::slice::from_ref(&audience));
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        let mut refresh_validation = validation.clone();
        // Refresh tokens carry the same iss/aud but a `typ` we check by hand.
        refresh_validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        Self {
            issuer,
            audience,
            access_ttl,
            refresh_ttl,
            encoding: EncodingKey::from_secret(signing_key),
            decoding: DecodingKey::from_secret(signing_key),
            validation,
            refresh_validation,
        }
    }
}

impl SessionAuthority for Hs256SessionAuthority {
    fn issue(
        &self,
        identity: &Identity,
        provider: &ProviderId,
        scopes: Vec<Scope>,
        assurance: crate::verification::Assurance,
    ) -> Result<SessionGrant, AuthError> {
        let now = unix_now()?;
        let session_id = new_session_id();
        let exp = now.saturating_add(self.access_ttl);
        let claims = SessionClaims {
            sub: identity.id.to_string(),
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            exp,
            iat: now,
            jti: session_id.to_string(),
            provider: provider.clone(),
            scopes,
            assurance,
        };
        let header = Header::new(Algorithm::HS256);
        let access_token = encode(&header, &claims, &self.encoding).map_err(|error| {
            AuthError::HostConfiguration(format!("session signing failed: {error}"))
        })?;

        let refresh_claims = RefreshClaims {
            sub: identity.id.to_string(),
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            exp: now.saturating_add(self.refresh_ttl),
            iat: now,
            jti: session_id.to_string(),
            provider: provider.clone(),
            typ: "refresh".to_owned(),
        };
        let refresh_token = encode(&header, &refresh_claims, &self.encoding).map_err(|error| {
            AuthError::HostConfiguration(format!("refresh signing failed: {error}"))
        })?;

        Ok(SessionGrant {
            session_id,
            access_token,
            refresh_token: Some(refresh_token),
            expires_at: exp,
            claims,
        })
    }

    fn validate(&self, access_token: &str) -> Result<SessionClaims, AuthError> {
        let data = decode::<SessionClaims>(access_token, &self.decoding, &self.validation)
            .map_err(|error| map_jwt_error(&error))?;
        Ok(data.claims)
    }

    fn validate_refresh(&self, refresh_token: &str) -> Result<RefreshSubject, AuthError> {
        let data = decode::<RefreshClaims>(refresh_token, &self.decoding, &self.refresh_validation)
            .map_err(|error| map_jwt_error(&error))?;
        if data.claims.typ != "refresh" {
            return Err(AuthError::InvalidSession("not a refresh token".to_owned()));
        }
        Ok(RefreshSubject {
            identity: SubjectId::new(data.claims.sub),
            provider: data.claims.provider,
            session: SessionId::new(data.claims.jti),
        })
    }
}

fn map_jwt_error(error: &jsonwebtoken::errors::Error) -> AuthError {
    use jsonwebtoken::errors::ErrorKind;
    match error.kind() {
        ErrorKind::ExpiredSignature => AuthError::Expired,
        kind => AuthError::InvalidSession(format!("{kind:?}")),
    }
}

fn unix_now() -> Result<u64, AuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| {
            AuthError::HostConfiguration(format!("system clock before epoch: {error}"))
        })
}

fn new_session_id() -> SessionId {
    // Unique, but NOT cryptographically random — see the security note above.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    SessionId::new(format!("{nanos:x}-{counter:x}"))
}

/// A minimal identity store that mints a fresh identity per provider subject.
/// Used when a project supplies no persistent store; not for production
/// (identities are not durable across host restarts).
#[derive(Debug, Default)]
pub struct EphemeralIdentityStore;

impl IdentityStore for EphemeralIdentityStore {
    fn resolve(&self, _subject: &ProviderSubject) -> Result<Option<Identity>, AuthError> {
        Ok(None)
    }

    fn get_or_create(&self, verified: &VerifiedExternalIdentity) -> Result<Identity, AuthError> {
        let id = SubjectId::new(verified.subject.canonical());
        Ok(Identity::new(id, verified.subject.clone()))
    }

    fn link(&self, _id: &SubjectId, _subject: ProviderSubject) -> Result<Identity, AuthError> {
        Err(AuthError::HostConfiguration(
            "ephemeral identity store does not support linking".to_owned(),
        ))
    }
}

/// The reference auth host: composes exactly one provider, the engine
/// authority, and optional identity-store/claims contributions. This is what a
/// generated `auth-host` target instantiates.
pub struct ReferenceAuthHost {
    provider: Box<dyn AuthProvider>,
    authority: Box<dyn SessionAuthority>,
    identity_store: Box<dyn IdentityStore>,
    augmenter: Option<Box<dyn ClaimsAugmenter>>,
    metadata: HostMetadata,
    limits: RequestLimits,
    revoked: Mutex<HashSet<String>>,
}

impl ReferenceAuthHost {
    /// Compose a host. The provider is held behind `dyn AuthProvider`; it has
    /// no path to `authority` and therefore no path to signing keys.
    #[must_use]
    pub fn new(
        provider: Box<dyn AuthProvider>,
        authority: Box<dyn SessionAuthority>,
        metadata: HostMetadata,
    ) -> Self {
        Self {
            provider,
            authority,
            identity_store: Box::new(EphemeralIdentityStore),
            augmenter: None,
            metadata,
            limits: RequestLimits::default(),
            revoked: Mutex::new(HashSet::new()),
        }
    }

    #[must_use]
    pub fn with_identity_store(mut self, store: Box<dyn IdentityStore>) -> Self {
        self.identity_store = store;
        self
    }

    #[must_use]
    pub fn with_claims_augmenter(mut self, augmenter: Box<dyn ClaimsAugmenter>) -> Self {
        self.augmenter = Some(augmenter);
        self
    }
}

impl AuthHost for ReferenceAuthHost {
    fn exchange(&self, request: ExchangeRequest) -> Result<SessionGrant, AuthError> {
        if request.credential.len() > self.limits.max_credential_bytes {
            return Err(AuthError::InvalidCredential(
                "credential exceeds host limit".to_owned(),
            ));
        }

        let capabilities = self.provider.capabilities();
        if !capabilities.accepts(&request.credential.scheme) {
            return Err(AuthError::UnsupportedScheme(
                request.credential.scheme.to_string(),
            ));
        }

        let verified = self.provider.verify_or_exchange(&request.credential)?;
        let identity = self.identity_store.get_or_create(&verified)?;
        if !identity.is_active() {
            return Err(AuthError::IdentityInactive);
        }

        let mut scopes = request.requested_scopes;
        if let Some(augmenter) = &self.augmenter {
            let augmented = augmenter.augment(&identity, &verified)?;
            scopes.extend(augmented.scopes);
        }
        scopes.sort();
        scopes.dedup();

        self.authority.issue(
            &identity,
            &verified.subject.provider,
            scopes,
            verified.assurance,
        )
    }

    fn validate(&self, access_token: &str) -> Result<SessionClaims, AuthError> {
        let claims = self.authority.validate(access_token)?;
        let is_revoked = {
            let revoked = self
                .revoked
                .lock()
                .map_err(|_| AuthError::HostConfiguration("revocation lock poisoned".to_owned()))?;
            revoked.contains(&claims.jti)
        };
        if is_revoked {
            return Err(AuthError::SessionRevoked);
        }
        Ok(claims)
    }

    fn refresh(&self, refresh_token: &str) -> Result<SessionGrant, AuthError> {
        let subject = self.authority.validate_refresh(refresh_token)?;
        {
            let revoked = self
                .revoked
                .lock()
                .map_err(|_| AuthError::HostConfiguration("revocation lock poisoned".to_owned()))?;
            if revoked.contains(subject.session.as_str()) {
                return Err(AuthError::SessionRevoked);
            }
        }
        let identity = Identity::new(
            subject.identity.clone(),
            ProviderSubject::new(subject.provider.clone(), subject.identity),
        );
        self.authority.issue(
            &identity,
            &subject.provider,
            Vec::new(),
            crate::verification::Assurance::None,
        )
    }

    fn revoke(&self, session: &SessionId) -> Result<(), AuthError> {
        self.revoked
            .lock()
            .map_err(|_| AuthError::HostConfiguration("revocation lock poisoned".to_owned()))?
            .insert(session.to_string());
        Ok(())
    }

    fn metadata(&self) -> HostMetadata {
        self.metadata.clone()
    }

    fn limits(&self) -> RequestLimits {
        self.limits
    }

    fn health(&self) -> HostHealth {
        HostHealth::Ready
    }
}
