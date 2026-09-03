//! `azoth.auth.local` — a local/dev authentication provider.
//!
//! This provider needs no external SDK: it verifies a `user:password`
//! credential against an in-process account table. It exists so a project can
//! run the full auth pipeline (and its tests) without Steam/EOS, and as the
//! reference implementation of the [`AuthProvider`] contract.
//!
//! It is a development provider. Passwords are salted-SHA256 here; a production
//! deployment would use a memory-hard KDF and a real account store — the ADR
//! 0026 security review covers that. Its architecture is final: it only ever
//! returns a [`VerifiedExternalIdentity`], never touching session keys.

use std::collections::HashMap;

use az_gem_auth::credential::{Audience, CredentialEnvelope, CredentialLease, CredentialScheme};
use az_gem_auth::identity::{ProviderId, ProviderSubject, SubjectId};
use az_gem_auth::provider::{AuthProvider, CredentialSource, ProviderCapabilities};
use az_gem_auth::verification::{Assurance, AuthError, VerifiedExternalIdentity};
use az_gem_contract::{Contribution, GemContext, contribution};
use sha2::{Digest, Sha256};

/// The stable provider id.
pub const PROVIDER_ID: &str = "azoth.auth.local";

/// A locally-stored account: a salted password hash and an optional display
/// name.
#[derive(Debug, Clone)]
pub struct LocalAccount {
    salt: String,
    password_hash: [u8; 32],
    display_name: Option<String>,
}

impl LocalAccount {
    /// Create an account from a plaintext password (hashed immediately).
    #[must_use]
    pub fn new(username: &str, password: &str) -> Self {
        let salt = format!("azoth-local::{username}");
        Self {
            password_hash: hash_password(&salt, password),
            salt,
            display_name: None,
        }
    }

    #[must_use]
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    fn verify(&self, password: &str) -> bool {
        // Constant-time-ish: compare full 32-byte hashes.
        hash_password(&self.salt, password) == self.password_hash
    }
}

fn hash_password(salt: &str, password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b"::");
    hasher.update(password.as_bytes());
    hasher.finalize().into()
}

/// The host-side local provider: verifies `user:password` credentials against
/// its account table.
#[derive(Debug, Default)]
pub struct LocalAuthProvider {
    accounts: HashMap<String, LocalAccount>,
    audience: Option<Audience>,
}

impl LocalAuthProvider {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict accepted credentials to a single audience.
    #[must_use]
    pub fn with_audience(mut self, audience: Audience) -> Self {
        self.audience = Some(audience);
        self
    }

    /// Register an account.
    #[must_use]
    pub fn with_account(mut self, username: impl Into<String>, account: LocalAccount) -> Self {
        self.accounts.insert(username.into(), account);
        self
    }

    fn provider() -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }
}

impl AuthProvider for LocalAuthProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new(
            Self::provider(),
            vec![CredentialScheme::new(CredentialScheme::LOCAL_PASSWORD)],
        )
    }

    fn verify_or_exchange(
        &self,
        credential: &CredentialEnvelope,
    ) -> Result<VerifiedExternalIdentity, AuthError> {
        let expected_scheme = CredentialScheme::new(CredentialScheme::LOCAL_PASSWORD);
        if credential.scheme != expected_scheme {
            return Err(AuthError::UnsupportedScheme(credential.scheme.to_string()));
        }
        if let Some(audience) = &self.audience
            && credential.audience != *audience
        {
            return Err(AuthError::AudienceMismatch {
                expected: audience.to_string(),
                got: credential.audience.to_string(),
            });
        }

        let text = std::str::from_utf8(credential.payload())
            .map_err(|_| AuthError::InvalidCredential("payload is not utf-8".to_owned()))?;
        let (username, password) = text
            .split_once(':')
            .ok_or_else(|| AuthError::InvalidCredential("expected user:password".to_owned()))?;

        let account = self
            .accounts
            .get(username)
            .ok_or_else(|| AuthError::InvalidCredential("unknown account".to_owned()))?;
        if !account.verify(password) {
            return Err(AuthError::InvalidCredential("bad password".to_owned()));
        }

        let mut verified = VerifiedExternalIdentity::new(ProviderSubject::new(
            Self::provider(),
            SubjectId::new(username),
        ))
        .with_assurance(Assurance::None);
        if let Some(name) = &account.display_name {
            verified = verified.with_display_name(name.clone());
        }
        Ok(verified)
    }
}

/// The client-side local credential source: packages a `user:password` string
/// into a credential envelope.
#[derive(Debug, Clone)]
pub struct LocalCredentialSource {
    username: String,
    password: String,
}

impl LocalCredentialSource {
    #[must_use]
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

impl CredentialSource for LocalCredentialSource {
    fn acquire(
        &self,
        scheme: &CredentialScheme,
        audience: &Audience,
    ) -> Result<(CredentialEnvelope, CredentialLease), AuthError> {
        if scheme.as_str() != CredentialScheme::LOCAL_PASSWORD {
            return Err(AuthError::UnsupportedScheme(scheme.to_string()));
        }
        let payload = format!("{}:{}", self.username, self.password);
        let envelope =
            CredentialEnvelope::new(scheme.clone(), audience.clone(), payload.into_bytes())
                .map_err(|error| AuthError::InvalidCredential(error.to_string()))?;
        Ok((envelope, CredentialLease::noop()))
    }
}

/// The ADR 0026 provider entry-point convention.
///
/// Constructs the host-side [`AuthProvider`] from resolved, non-secret context.
/// The generated auth host calls `az_gem_auth_local::auth_provider(&ctx)` when
/// a project selects this gem as its `[services.auth].provider`.
///
/// The local provider is fully wireable from configuration alone (no SDK), so
/// this succeeds. It starts with zero registered accounts — account
/// provisioning is a separate identity-store selection point (ADR 0026).
///
/// # Errors
/// Infallible today, but returns the shared provider-entry-point `Result` so
/// every provider gem composes through one signature.
pub fn auth_provider(
    context: &az_gem_auth::AuthProviderContext,
) -> Result<Box<dyn AuthProvider>, AuthError> {
    Ok(Box::new(
        LocalAuthProvider::new().with_audience(context.audience().clone()),
    ))
}

/// The gem's `provider` contribution.
///
/// [`auth_provider`] above is ADR 0026's *service* convention — the generated
/// auth host calls it by name to build the provider — and it is untouched by
/// registration. This contribution is the other half: identity and linkage for
/// the composing host, with no registry entry of its own.
///
/// Sealing is privacy: the generated `provider_contribution` is the only way in.
struct Provider;

#[contribution]
impl Contribution for Provider {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> LocalAuthProvider {
        LocalAuthProvider::new()
            .with_audience(Audience::new("game"))
            .with_account(
                "alice",
                LocalAccount::new("alice", "hunter2").with_display_name("Alice"),
            )
    }

    #[test]
    fn client_source_and_host_provider_round_trip() {
        let source = LocalCredentialSource::new("alice", "hunter2");
        let (credential, _lease) = source
            .acquire(
                &CredentialScheme::new(CredentialScheme::LOCAL_PASSWORD),
                &Audience::new("game"),
            )
            .unwrap();
        let verified = provider().verify_or_exchange(&credential).unwrap();
        assert_eq!(verified.subject.subject.as_str(), "alice");
        assert_eq!(verified.display_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn wrong_password_is_rejected() {
        let source = LocalCredentialSource::new("alice", "wrong");
        let (credential, _lease) = source
            .acquire(
                &CredentialScheme::new(CredentialScheme::LOCAL_PASSWORD),
                &Audience::new("game"),
            )
            .unwrap();
        assert!(matches!(
            provider().verify_or_exchange(&credential),
            Err(AuthError::InvalidCredential(_))
        ));
    }

    #[test]
    fn auth_provider_entry_point_builds_an_audience_restricted_provider() {
        let context = az_gem_auth::AuthProviderContext::new(
            Audience::new("game"),
            ProviderId::new(PROVIDER_ID),
        );
        let provider = auth_provider(&context).unwrap();
        assert_eq!(provider.capabilities().provider.as_str(), PROVIDER_ID);
        assert!(
            provider
                .capabilities()
                .accepts(&CredentialScheme::new(CredentialScheme::LOCAL_PASSWORD))
        );
    }

    #[test]
    fn audience_mismatch_is_rejected() {
        let source = LocalCredentialSource::new("alice", "hunter2");
        let (credential, _lease) = source
            .acquire(
                &CredentialScheme::new(CredentialScheme::LOCAL_PASSWORD),
                &Audience::new("other-game"),
            )
            .unwrap();
        assert!(matches!(
            provider().verify_or_exchange(&credential),
            Err(AuthError::AudienceMismatch { .. })
        ));
    }
}
