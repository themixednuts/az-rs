//! `azoth.auth.eos` — the Epic Online Services authentication provider (ADR
//! 0026).
//!
//! This gem maps the neutral [`AuthProvider`] contract onto **EOS Connect**: it
//! takes a declared external credential, drives Connect login, maps the
//! resulting Product User ID onto a [`ProviderSubject`], and handles the
//! `EOS_InvalidUser`/continuance-token/CreateUser branch as opaque provider
//! state via [`AuthError::ContinuationRequired`].
//!
//! The live SDK call (init, callback pumping, main-thread affinity, Connect
//! calls) belongs to `az-gem-eos`; this gem takes it as an injected
//! [`EosConnectVerifier`]. Epic Account Services (`EOS_Auth`) is a *separately
//! declared* capability and scheme, not an implicit expansion of Connect.

use az_gem_auth::credential::{Audience, CredentialEnvelope, CredentialScheme};
use az_gem_auth::identity::{ProviderId, ProviderSubject, SubjectId};
use az_gem_auth::provider::{AuthProvider, ProviderCapabilities};
use az_gem_auth::verification::{Assurance, AuthError, ProviderClaims, VerifiedExternalIdentity};
use az_gem_contract::{Contribution, GemContext, contribution};

/// The stable provider id.
pub const PROVIDER_ID: &str = "azoth.auth.eos";

/// The outcome of an EOS Connect login attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EosConnectOutcome {
    /// Login succeeded; the subject has a Product User ID.
    LoggedIn {
        product_user_id: String,
        display_name: Option<String>,
    },
    /// `EOS_InvalidUser`: the external account has no product user yet. The
    /// opaque continuance token is echoed back by the client to a `CreateUser`
    /// step; the engine surfaces it as a continuation.
    ContinuanceRequired { continuance_token: String },
}

/// The SDK-facing boundary, supplied by `az-gem-eos` at runtime; tests supply a
/// fake.
pub trait EosConnectVerifier: Send + Sync {
    /// Drive EOS Connect login for a declared external credential.
    ///
    /// # Errors
    /// Returns [`AuthError`] mapping the Connect result (invalid token,
    /// expired, unavailable).
    fn connect_login(
        &self,
        credential: &[u8],
        audience: &Audience,
    ) -> Result<EosConnectOutcome, AuthError>;
}

/// The EOS auth provider, generic over the platform verifier.
pub struct EosAuthProvider<V> {
    verifier: V,
    product_id: Option<String>,
}

impl<V> EosAuthProvider<V> {
    #[must_use]
    pub fn new(verifier: V, settings: &az_gem_eos::EosPlatformSettings) -> Self {
        Self {
            verifier,
            product_id: Some(settings.product_id.clone()),
        }
    }

    fn provider() -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }
}

impl<V: EosConnectVerifier> AuthProvider for EosAuthProvider<V> {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new(
            Self::provider(),
            vec![CredentialScheme::new(CredentialScheme::EOS_CONNECT)],
        )
    }

    fn verify_or_exchange(
        &self,
        credential: &CredentialEnvelope,
    ) -> Result<VerifiedExternalIdentity, AuthError> {
        let expected = CredentialScheme::new(CredentialScheme::EOS_CONNECT);
        if credential.scheme != expected {
            return Err(AuthError::UnsupportedScheme(credential.scheme.to_string()));
        }

        match self
            .verifier
            .connect_login(credential.payload(), &credential.audience)?
        {
            EosConnectOutcome::LoggedIn {
                product_user_id,
                display_name,
            } => {
                let subject =
                    ProviderSubject::new(Self::provider(), SubjectId::new(product_user_id));
                let mut claims = ProviderClaims::new();
                if let Some(product_id) = &self.product_id {
                    claims = claims.with("eos.product_id", product_id.clone());
                }
                let mut verified = VerifiedExternalIdentity::new(subject)
                    .with_assurance(Assurance::Platform)
                    .with_claims(claims);
                if let Some(name) = display_name {
                    verified = verified.with_display_name(name);
                }
                Ok(verified)
            }
            EosConnectOutcome::ContinuanceRequired { continuance_token } => {
                Err(AuthError::ContinuationRequired {
                    token: continuance_token,
                })
            }
        }
    }
}

/// The ADR 0026 provider entry-point convention.
///
/// EOS cannot be composed from configuration alone: [`EosAuthProvider`] needs a
/// live [`EosConnectVerifier`] from `az-gem-eos`'s platform runtime (SDK init,
/// callback pumping, main-thread affinity, Connect calls), which target
/// generation does not yet inject. Until that wiring lands the generated auth
/// host fails fast at startup with this actionable error rather than accepting
/// credentials it cannot verify.
///
/// # Errors
/// Always returns [`AuthError::HostConfiguration`] describing the missing
/// platform-runtime wiring (ADR 0026 deferred).
pub fn auth_provider(
    _context: &az_gem_auth::AuthProviderContext,
) -> Result<Box<dyn AuthProvider>, AuthError> {
    Err(AuthError::HostConfiguration(
        "azoth.auth.eos needs a live EosConnectVerifier from az-gem-eos's platform runtime (SDK \
         init, callback pumping, main-thread affinity, Connect calls); target composition does \
         not yet inject one (ADR 0026 deferred)"
            .to_owned(),
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

    struct FakeVerifier(Result<EosConnectOutcome, AuthError>);

    impl EosConnectVerifier for FakeVerifier {
        fn connect_login(
            &self,
            _credential: &[u8],
            _audience: &Audience,
        ) -> Result<EosConnectOutcome, AuthError> {
            self.0.clone()
        }
    }

    fn credential() -> CredentialEnvelope {
        CredentialEnvelope::new(
            CredentialScheme::new(CredentialScheme::EOS_CONNECT),
            Audience::new("game"),
            b"external-connect-token".to_vec(),
        )
        .unwrap()
    }

    fn provider(result: Result<EosConnectOutcome, AuthError>) -> EosAuthProvider<FakeVerifier> {
        let settings = az_gem_eos::EosPlatformSettings::new("prod", "sandbox", "deployment");
        EosAuthProvider::new(FakeVerifier(result), &settings)
    }

    #[test]
    fn auth_provider_entry_point_fails_closed_without_a_live_verifier() {
        let context = az_gem_auth::AuthProviderContext::new(
            Audience::new("game"),
            ProviderId::new(PROVIDER_ID),
        );
        assert!(matches!(
            auth_provider(&context),
            Err(AuthError::HostConfiguration(_))
        ));
    }

    #[test]
    fn logged_in_maps_product_user_id_to_subject() {
        let verified = provider(Ok(EosConnectOutcome::LoggedIn {
            product_user_id: "0002aaaa".to_owned(),
            display_name: Some("Player".to_owned()),
        }))
        .verify_or_exchange(&credential())
        .unwrap();
        assert_eq!(verified.subject.provider.as_str(), PROVIDER_ID);
        assert_eq!(verified.subject.subject.as_str(), "0002aaaa");
        assert_eq!(verified.claims.get("eos.product_id"), Some("prod"));
    }

    #[test]
    fn invalid_user_surfaces_as_continuation_required() {
        let error = provider(Ok(EosConnectOutcome::ContinuanceRequired {
            continuance_token: "cont-123".to_owned(),
        }))
        .verify_or_exchange(&credential())
        .unwrap_err();
        assert!(matches!(
            error,
            AuthError::ContinuationRequired { token } if token == "cont-123"
        ));
    }
}
