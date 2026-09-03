//! End-to-end contract tests exercised through the reference host.

use crate::authority::{Hs256SessionAuthority, ReferenceAuthHost};
use crate::credential::{Audience, CredentialEnvelope, CredentialScheme, MAX_CREDENTIAL_BYTES};
use crate::host::{AuthHost, ExchangeRequest, HostMetadata};
use crate::identity::{ProviderId, ProviderSubject, SubjectId};
use crate::provider::{AuthProvider, ProviderCapabilities};
use crate::session::Scope;
use crate::verification::{Assurance, AuthError, VerifiedExternalIdentity};

const PROVIDER: &str = "azoth.auth.local";
const AUDIENCE: &str = "test-game";
const ISSUER: &str = "test-issuer";
const SIGNING_KEY: &[u8] = b"a-test-signing-key-that-is-long-enough";

/// A minimal provider: payload is `user:password`, accepted iff password is
/// `correct`. Proves the provider surface has no access to signing keys.
struct FakeProvider {
    provider: ProviderId,
}

impl AuthProvider for FakeProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new(
            self.provider.clone(),
            vec![CredentialScheme::new(CredentialScheme::LOCAL_PASSWORD)],
        )
    }

    fn verify_or_exchange(
        &self,
        credential: &CredentialEnvelope,
    ) -> Result<VerifiedExternalIdentity, AuthError> {
        let text = std::str::from_utf8(credential.payload())
            .map_err(|_| AuthError::InvalidCredential("payload is not utf-8".to_owned()))?;
        let (user, password) = text
            .split_once(':')
            .ok_or_else(|| AuthError::InvalidCredential("expected user:password".to_owned()))?;
        if password != "correct" {
            return Err(AuthError::InvalidCredential("bad password".to_owned()));
        }
        Ok(VerifiedExternalIdentity::new(ProviderSubject::new(
            self.provider.clone(),
            SubjectId::new(user),
        ))
        .with_assurance(Assurance::Platform))
    }
}

fn host() -> ReferenceAuthHost {
    let provider = ProviderId::new(PROVIDER);
    let authority = Hs256SessionAuthority::new(SIGNING_KEY, ISSUER, AUDIENCE, 900, 3600);
    let metadata = HostMetadata {
        primary_provider: provider.clone(),
        link_providers: Vec::new(),
        audience: Audience::new(AUDIENCE),
        issuer: ISSUER.to_owned(),
    };
    ReferenceAuthHost::new(
        Box::new(FakeProvider { provider }),
        Box::new(authority),
        metadata,
    )
}

fn password_credential(payload: &str) -> CredentialEnvelope {
    CredentialEnvelope::new(
        CredentialScheme::new(CredentialScheme::LOCAL_PASSWORD),
        Audience::new(AUDIENCE),
        payload.as_bytes().to_vec(),
    )
    .expect("valid credential")
}

fn exchange(payload: &str, scopes: Vec<Scope>) -> Result<crate::session::SessionGrant, AuthError> {
    host().exchange(ExchangeRequest {
        credential: password_credential(payload),
        requested_scopes: scopes,
    })
}

#[test]
fn exchange_issues_a_validatable_signed_session() {
    let host = host();
    let grant = host
        .exchange(ExchangeRequest {
            credential: password_credential("alice:correct"),
            requested_scopes: vec![Scope::new("game:play")],
        })
        .expect("exchange succeeds");

    assert_eq!(grant.claims.iss, ISSUER);
    assert_eq!(grant.claims.aud, AUDIENCE);
    assert_eq!(grant.claims.provider.as_str(), PROVIDER);
    assert_eq!(grant.claims.sub, format!("{PROVIDER}:alice"));
    assert!(grant.claims.has_scope(&Scope::new("game:play")));
    assert!(grant.refresh_token.is_some());

    let validated = host.validate(&grant.access_token).expect("token validates");
    assert_eq!(validated.jti, grant.session_id.to_string());
    assert_eq!(validated.assurance, Assurance::Platform);
}

#[test]
fn bad_password_is_rejected_as_invalid_credential() {
    let error = exchange("alice:wrong", Vec::new()).unwrap_err();
    assert!(matches!(error, AuthError::InvalidCredential(_)));
}

#[test]
fn unsupported_scheme_is_rejected_before_the_provider_runs() {
    let credential = CredentialEnvelope::new(
        CredentialScheme::new(CredentialScheme::STEAM_SESSION_TICKET),
        Audience::new(AUDIENCE),
        b"steam-ticket-bytes".to_vec(),
    )
    .unwrap();
    let error = host()
        .exchange(ExchangeRequest {
            credential,
            requested_scopes: Vec::new(),
        })
        .unwrap_err();
    assert!(matches!(error, AuthError::UnsupportedScheme(_)));
}

#[test]
fn revoked_session_fails_validation() {
    let host = host();
    let grant = host
        .exchange(ExchangeRequest {
            credential: password_credential("bob:correct"),
            requested_scopes: Vec::new(),
        })
        .unwrap();
    host.validate(&grant.access_token)
        .expect("valid before revoke");
    host.revoke(&grant.session_id).expect("revoke succeeds");
    let error = host.validate(&grant.access_token).unwrap_err();
    assert!(matches!(error, AuthError::SessionRevoked));
}

#[test]
fn refresh_mints_a_new_validatable_session() {
    let host = host();
    let grant = host
        .exchange(ExchangeRequest {
            credential: password_credential("carol:correct"),
            requested_scopes: Vec::new(),
        })
        .unwrap();
    let refresh_token = grant.refresh_token.clone().unwrap();
    let refreshed = host.refresh(&refresh_token).expect("refresh succeeds");
    assert_eq!(refreshed.claims.sub, grant.claims.sub);
    host.validate(&refreshed.access_token)
        .expect("refreshed token validates");
}

#[test]
fn a_session_signed_by_a_different_key_is_rejected() {
    let grant = exchange("dave:correct", Vec::new()).unwrap();
    let other = Hs256SessionAuthority::new(
        b"a-completely-different-signing-key",
        ISSUER,
        AUDIENCE,
        900,
        3600,
    );
    let metadata = HostMetadata {
        primary_provider: ProviderId::new(PROVIDER),
        link_providers: Vec::new(),
        audience: Audience::new(AUDIENCE),
        issuer: ISSUER.to_owned(),
    };
    let other_host = ReferenceAuthHost::new(
        Box::new(FakeProvider {
            provider: ProviderId::new(PROVIDER),
        }),
        Box::new(other),
        metadata,
    );
    let error = other_host.validate(&grant.access_token).unwrap_err();
    assert!(matches!(error, AuthError::InvalidSession(_)));
}

#[test]
fn oversized_credential_is_rejected_on_construction() {
    let error = CredentialEnvelope::new(
        CredentialScheme::new(CredentialScheme::LOCAL_PASSWORD),
        Audience::new(AUDIENCE),
        vec![0u8; MAX_CREDENTIAL_BYTES + 1],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        crate::credential::CredentialError::TooLarge { .. }
    ));
}

#[test]
fn credential_debug_redacts_the_payload() {
    let credential = password_credential("secret-user:super-secret");
    let rendered = format!("{credential:?}");
    assert!(rendered.contains("redacted"));
    assert!(!rendered.contains("super-secret"));
}
