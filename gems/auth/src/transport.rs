//! Minimal JSON/HTTP transport that serves the [`AuthHost`] trait (ADR 0026).
//!
//! This is the "transport is follow-up" piece: the generated `auth-host`
//! binary calls [`serve`] with a composed [`AuthHost`]. It is a deliberately
//! small axum surface — one route per host operation, a `/v1/health` probe,
//! and an explicit `/v1/shutdown` (chosen over an OS signal so the generated
//! binary shuts down identically on every platform it runs on).
//!
//! Errors are mapped to a stable `{ "error": <class>, "message": <text> }`
//! body and an HTTP status per [`classify`]; the classification is part of
//! this module's contract, not an implementation detail.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use crate::host::{AuthHost, ExchangeRequest, HostHealth};
use crate::session::SessionId;
use crate::verification::AuthError;

#[derive(Clone)]
struct AppState {
    host: Arc<dyn AuthHost>,
    shutdown: Arc<Notify>,
}

/// Bind `listener` and serve `host`'s [`AuthHost`] surface until
/// `POST /v1/shutdown` is called, then run [`AuthHost::shutdown`] and return.
///
/// # Errors
/// Returns an I/O error if the accept loop fails.
pub async fn serve(host: Arc<dyn AuthHost>, listener: TcpListener) -> std::io::Result<()> {
    let shutdown = Arc::new(Notify::new());
    let state = AppState {
        host: host.clone(),
        shutdown: shutdown.clone(),
    };
    let result = axum::serve(listener, router(state))
        .with_graceful_shutdown(async move { shutdown.notified().await })
        .await;
    host.shutdown();
    result
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/metadata", get(metadata))
        .route("/v1/exchange", post(exchange))
        .route("/v1/validate", post(validate))
        .route("/v1/refresh", post(refresh))
        .route("/v1/revoke", post(revoke))
        .route("/v1/shutdown", post(shutdown_now))
        .with_state(state)
}

#[derive(Serialize)]
struct HealthResponse {
    status: HostHealth,
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: state.host.health(),
    })
}

async fn metadata(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.host.metadata())
}

async fn exchange(State(state): State<AppState>, Json(request): Json<ExchangeRequest>) -> Response {
    match state.host.exchange(request) {
        Ok(grant) => (StatusCode::OK, Json(grant)).into_response(),
        Err(error) => error_response(&error),
    }
}

#[derive(Deserialize)]
struct ValidateRequest {
    access_token: String,
}

async fn validate(State(state): State<AppState>, Json(request): Json<ValidateRequest>) -> Response {
    match state.host.validate(&request.access_token) {
        Ok(claims) => (StatusCode::OK, Json(claims)).into_response(),
        Err(error) => error_response(&error),
    }
}

#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

async fn refresh(State(state): State<AppState>, Json(request): Json<RefreshRequest>) -> Response {
    match state.host.refresh(&request.refresh_token) {
        Ok(grant) => (StatusCode::OK, Json(grant)).into_response(),
        Err(error) => error_response(&error),
    }
}

#[derive(Deserialize)]
struct RevokeRequest {
    session_id: String,
}

async fn revoke(State(state): State<AppState>, Json(request): Json<RevokeRequest>) -> Response {
    match state.host.revoke(&SessionId::new(request.session_id)) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error_response(&error),
    }
}

async fn shutdown_now(State(state): State<AppState>) -> impl IntoResponse {
    state.shutdown.notify_one();
    StatusCode::ACCEPTED
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}

fn error_response(error: &AuthError) -> Response {
    let (status, class) = classify(error);
    (
        status,
        Json(ErrorBody {
            error: class,
            message: error.to_string(),
        }),
    )
        .into_response()
}

/// The stable HTTP-status/error-class mapping for [`AuthError`].
const fn classify(error: &AuthError) -> (StatusCode, &'static str) {
    match error {
        AuthError::UnsupportedScheme(_)
        | AuthError::InvalidCredential(_)
        | AuthError::AudienceMismatch { .. } => (StatusCode::BAD_REQUEST, "invalid-request"),
        AuthError::Expired | AuthError::InvalidSession(_) => {
            (StatusCode::UNAUTHORIZED, "invalid-session")
        }
        AuthError::NotEntitled
        | AuthError::Banned
        | AuthError::IdentityInactive
        | AuthError::SessionRevoked => (StatusCode::FORBIDDEN, "forbidden"),
        AuthError::Replay | AuthError::ContinuationRequired { .. } => {
            (StatusCode::CONFLICT, "conflict")
        }
        AuthError::ProviderUnavailable(_) => {
            (StatusCode::SERVICE_UNAVAILABLE, "provider-unavailable")
        }
        AuthError::HostConfiguration(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "host-configuration")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::{Hs256SessionAuthority, ReferenceAuthHost};
    use crate::credential::{Audience, CredentialEnvelope, CredentialScheme};
    use crate::host::HostMetadata;
    use crate::identity::{ProviderId, ProviderSubject, SubjectId};
    use crate::provider::{AuthProvider, ProviderCapabilities};
    use crate::verification::{Assurance, VerifiedExternalIdentity};
    use base64::Engine as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const PROVIDER: &str = "azoth.auth.local";
    const AUDIENCE: &str = "test-game";
    const ISSUER: &str = "test-issuer";
    const SIGNING_KEY: &[u8] = b"a-test-signing-key-that-is-long-enough";

    /// A minimal in-crate fake provider (mirrors `crate::tests::FakeProvider`):
    /// payload is `user:password`, accepted iff password is `correct`. This
    /// module cannot depend on `az-gem-auth-local` (that crate depends on this
    /// one), so it proves the transport is correct against a local fake
    /// instead — the real local provider's own contract behavior is covered by
    /// `gems/auth-local`'s tests.
    struct FakeProvider;

    impl AuthProvider for FakeProvider {
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::new(
                ProviderId::new(PROVIDER),
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
                ProviderId::new(PROVIDER),
                SubjectId::new(user),
            ))
            .with_assurance(Assurance::Platform))
        }
    }

    fn host() -> Arc<dyn AuthHost> {
        let authority = Hs256SessionAuthority::new(SIGNING_KEY, ISSUER, AUDIENCE, 900, 3600);
        let metadata = HostMetadata {
            primary_provider: ProviderId::new(PROVIDER),
            link_providers: Vec::new(),
            audience: Audience::new(AUDIENCE),
            issuer: ISSUER.to_owned(),
        };
        Arc::new(ReferenceAuthHost::new(
            Box::new(FakeProvider),
            Box::new(authority),
            metadata,
        ))
    }

    /// A dependency-free raw HTTP/1.1 client: writes the request, then reads
    /// until the server closes the connection (`Connection: close`), and
    /// splits the response into `(status, body)`.
    async fn request(
        addr: std::net::SocketAddr,
        method: &str,
        path: &str,
        body: &str,
    ) -> (u16, String) {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let request = if body.is_empty() {
            format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        } else {
            format!(
                "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        let (status_line, rest) = response.split_once("\r\n").unwrap();
        let status = status_line
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let body = rest
            .split_once("\r\n\r\n")
            .map_or(String::new(), |(_, body)| body.to_string());
        (status, body)
    }

    #[tokio::test]
    async fn full_local_exchange_round_trips_through_the_transport() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let serving = tokio::spawn(serve(host(), listener));

        let (status, body) = request(addr, "GET", "/v1/health", "").await;
        assert_eq!(status, 200, "health body: {body}");
        assert!(body.contains("\"status\":\"ready\""), "health body: {body}");

        let payload = base64::engine::general_purpose::STANDARD.encode(b"alice:correct");
        let exchange_body = format!(
            r#"{{"credential":{{"scheme":"local-password","audience":{AUDIENCE:?},"payload":"{payload}"}},"requested_scopes":["game:play"]}}"#
        );
        let (status, body) = request(addr, "POST", "/v1/exchange", &exchange_body).await;
        assert_eq!(status, 200, "exchange body: {body}");
        let grant: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(grant["claims"]["sub"], format!("{PROVIDER}:alice"));
        let access_token = grant["access_token"].as_str().unwrap().to_string();

        let validate_body = format!(r#"{{"access_token":{access_token:?}}}"#);
        let (status, body) = request(addr, "POST", "/v1/validate", &validate_body).await;
        assert_eq!(status, 200, "validate body: {body}");

        let bad_payload = base64::engine::general_purpose::STANDARD.encode(b"alice:wrong");
        let bad_exchange_body = format!(
            r#"{{"credential":{{"scheme":"local-password","audience":{AUDIENCE:?},"payload":"{bad_payload}"}},"requested_scopes":[]}}"#
        );
        let (status, body) = request(addr, "POST", "/v1/exchange", &bad_exchange_body).await;
        assert_eq!(status, 400, "rejected exchange body: {body}");
        assert!(body.contains("\"error\":\"invalid-request\""), "{body}");

        let (status, _) = request(addr, "POST", "/v1/shutdown", "").await;
        assert_eq!(status, 202);

        serving
            .await
            .expect("serve task does not panic")
            .expect("serve task shuts down cleanly");
    }
}
