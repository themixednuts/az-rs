//! `azoth.auth.steam` — the Steam authentication provider (ADR 0026).
//!
//! This gem owns the *mapping* from the neutral [`AuthProvider`] contract onto
//! Steam's ownership model: it validates a `steam-session-ticket` envelope,
//! translates a verified Steam ID into a [`ProviderSubject`], and maps license,
//! ban, replay, cancellation, and expiry results onto stable [`AuthError`]
//! classes.
//!
//! The live SDK call (`BeginAuthSession` / Web API validation, callback pumping,
//! thread affinity) belongs to `az-gem-steam`; this gem takes it as an injected
//! [`SteamTicketVerifier`] so the mapping is unit-testable without the SDK and
//! so game/project code never sees a Steam handle. A `steam-web-api-ticket` is a
//! *distinct* scheme and is never accepted here in place of a session ticket.

use az_gem_auth::credential::{Audience, CredentialEnvelope, CredentialScheme};
use az_gem_auth::identity::{ProviderId, ProviderSubject, SubjectId};
use az_gem_auth::provider::{AuthProvider, ProviderCapabilities};
use az_gem_auth::verification::{Assurance, AuthError, ProviderClaims, VerifiedExternalIdentity};
use az_gem_contract::{Contribution, GemContext, contribution};

/// The stable provider id.
pub const PROVIDER_ID: &str = "azoth.auth.steam";

/// The result of the platform verifying a Steam session ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamVerification {
    pub steam_id: u64,
    /// Whether the subject owns the configured app (license check).
    pub owns_app: bool,
    /// Whether the subject is VAC/publisher banned.
    pub banned: bool,
    pub display_name: Option<String>,
}

/// The SDK-facing boundary.
///
/// `az-gem-steam` (with the runtime feature) supplies a real implementation
/// that begins/ends a Steam auth session; tests supply a fake. The provider
/// never holds a Steam handle directly.
pub trait SteamTicketVerifier: Send + Sync {
    /// Begin and complete verification of a session ticket for `audience`.
    ///
    /// # Errors
    /// Returns [`AuthError`] mapping the SDK result (invalid ticket, expired,
    /// replay, cancelled, unavailable).
    fn verify_session_ticket(
        &self,
        ticket: &[u8],
        audience: &Audience,
    ) -> Result<SteamVerification, AuthError>;
}

/// The Steam auth provider, generic over the platform verifier.
pub struct SteamAuthProvider<V> {
    verifier: V,
    app_id: u32,
}

impl<V> SteamAuthProvider<V> {
    #[must_use]
    pub const fn new(verifier: V, app_id: az_gem_steam::SteamAppId) -> Self {
        Self {
            verifier,
            app_id: app_id.0,
        }
    }

    /// The configured Steam app id (for diagnostics/claims).
    #[must_use]
    pub const fn app_id(&self) -> u32 {
        self.app_id
    }

    fn provider() -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }
}

impl<V: SteamTicketVerifier> AuthProvider for SteamAuthProvider<V> {
    fn capabilities(&self) -> ProviderCapabilities {
        let mut capabilities = ProviderCapabilities::new(
            Self::provider(),
            vec![CredentialScheme::new(
                CredentialScheme::STEAM_SESSION_TICKET,
            )],
        );
        capabilities.supports_revoke = true;
        capabilities
    }

    fn verify_or_exchange(
        &self,
        credential: &CredentialEnvelope,
    ) -> Result<VerifiedExternalIdentity, AuthError> {
        let expected = CredentialScheme::new(CredentialScheme::STEAM_SESSION_TICKET);
        if credential.scheme != expected {
            // A steam-web-api-ticket is a different scheme and flow; never
            // substitute it for a session ticket here.
            return Err(AuthError::UnsupportedScheme(credential.scheme.to_string()));
        }

        let verification = self
            .verifier
            .verify_session_ticket(credential.payload(), &credential.audience)?;

        if verification.banned {
            return Err(AuthError::Banned);
        }
        if !verification.owns_app {
            return Err(AuthError::NotEntitled);
        }

        let subject = ProviderSubject::new(
            Self::provider(),
            SubjectId::new(verification.steam_id.to_string()),
        );
        let claims = ProviderClaims::new().with("steam.app_id", self.app_id.to_string());
        let mut verified = VerifiedExternalIdentity::new(subject)
            .with_assurance(Assurance::Platform)
            .with_claims(claims);
        if let Some(name) = verification.display_name {
            verified = verified.with_display_name(name);
        }
        Ok(verified)
    }
}

/// The ADR 0026 provider entry-point convention.
///
/// Steam cannot be composed from configuration alone: [`SteamAuthProvider`]
/// needs a live [`SteamTicketVerifier`] from `az-gem-steam`'s platform runtime
/// (SDK init, callback pumping, thread affinity), which target generation does
/// not yet inject. Until that wiring lands the generated auth host fails fast
/// at startup with this actionable error rather than accepting credentials it
/// cannot verify.
///
/// # Errors
/// Always returns [`AuthError::HostConfiguration`] describing the missing
/// platform-runtime wiring (ADR 0026 deferred).
pub fn auth_provider(
    _context: &az_gem_auth::AuthProviderContext,
) -> Result<Box<dyn AuthProvider>, AuthError> {
    Err(AuthError::HostConfiguration(
        "azoth.auth.steam needs a live SteamTicketVerifier from az-gem-steam's platform runtime \
         (SDK init, callback pumping, thread affinity); target composition does not yet inject \
         one (ADR 0026 deferred)"
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

    struct FakeVerifier(Result<SteamVerification, AuthError>);

    impl SteamTicketVerifier for FakeVerifier {
        fn verify_session_ticket(
            &self,
            _ticket: &[u8],
            _audience: &Audience,
        ) -> Result<SteamVerification, AuthError> {
            self.0.clone()
        }
    }

    fn ticket() -> CredentialEnvelope {
        CredentialEnvelope::new(
            CredentialScheme::new(CredentialScheme::STEAM_SESSION_TICKET),
            Audience::new("game"),
            b"opaque-steam-ticket".to_vec(),
        )
        .unwrap()
    }

    fn provider(result: Result<SteamVerification, AuthError>) -> SteamAuthProvider<FakeVerifier> {
        SteamAuthProvider::new(FakeVerifier(result), az_gem_steam::SteamAppId(480))
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
    fn valid_ticket_maps_to_provider_subject() {
        let verified = provider(Ok(SteamVerification {
            steam_id: 76_561_197_960_287_930,
            owns_app: true,
            banned: false,
            display_name: Some("Gaben".to_owned()),
        }))
        .verify_or_exchange(&ticket())
        .unwrap();
        assert_eq!(verified.subject.provider.as_str(), PROVIDER_ID);
        assert_eq!(verified.subject.subject.as_str(), "76561197960287930");
        assert_eq!(verified.assurance, Assurance::Platform);
        assert_eq!(verified.claims.get("steam.app_id"), Some("480"));
    }

    #[test]
    fn banned_subject_is_rejected() {
        let error = provider(Ok(SteamVerification {
            steam_id: 1,
            owns_app: true,
            banned: true,
            display_name: None,
        }))
        .verify_or_exchange(&ticket())
        .unwrap_err();
        assert!(matches!(error, AuthError::Banned));
    }

    #[test]
    fn unowned_app_is_not_entitled() {
        let error = provider(Ok(SteamVerification {
            steam_id: 1,
            owns_app: false,
            banned: false,
            display_name: None,
        }))
        .verify_or_exchange(&ticket())
        .unwrap_err();
        assert!(matches!(error, AuthError::NotEntitled));
    }

    #[test]
    fn web_api_ticket_scheme_is_not_accepted_as_session_ticket() {
        let credential = CredentialEnvelope::new(
            CredentialScheme::new(CredentialScheme::STEAM_WEB_API_TICKET),
            Audience::new("game"),
            b"web-api-ticket".to_vec(),
        )
        .unwrap();
        let error = provider(Ok(SteamVerification {
            steam_id: 1,
            owns_app: true,
            banned: false,
            display_name: None,
        }))
        .verify_or_exchange(&credential)
        .unwrap_err();
        assert!(matches!(error, AuthError::UnsupportedScheme(_)));
    }
}
