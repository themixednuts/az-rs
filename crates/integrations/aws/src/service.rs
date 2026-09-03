//! AWS Service resource for AWS API operations
//!
//! This module provides typed API clients for an Amazon Games Services authentication flow:
//! 1. Token Service - Exchange Steam ticket for JWT
//! 2. Credentials - Get AWS `SigV4` credentials
//! 3. Login Info - Get account characters and worlds
//! 4. Login Queue - Get server connection token

use crate::error::AwsError;
use crate::region_config::{Region, RegionConfig};
use ags_auth_protocol::client::AgsGameClientProfile;
use ags_auth_protocol::login_queue::{
    LoginQueueRefreshRequest, LoginQueueRequest as LoginQueueRequestBody,
    LoginQueueRequestEnvelope, LoginQueueResponse, LoginQueueToken,
};
use ags_auth_protocol::tokens::{AttributionData, PlatformAuth, TokensRequest};
use ags_auth_protocol::types::{AgsAccessToken, PlatformType, SteamId, SteamTicketHex};
use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SigningParams, SigningSettings, sign,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use bevy::prelude::*;
use http::HeaderValue;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::trace;
use zeroize::{Zeroize, ZeroizeOnDrop};

// =============================================================================
// Constants
// =============================================================================

const TOKEN_SERVICE_ROOT: &str = "https://tokenservice.amazongames.com";
const AWS_SERVICE: &str = "execute-api";
const DEFAULT_REGION: Region = Region::UsEast;
const CREDENTIAL_REFRESH_WINDOW: Duration = Duration::from_secs(59);

fn sign_without_tracing<'a>(
    request: SignableRequest<'a>,
    params: &'a SigningParams<'a>,
) -> Result<
    aws_sigv4::SigningOutput<aws_sigv4::http_request::SigningInstructions>,
    aws_sigv4::http_request::SigningError,
> {
    // aws-sigv4 traces Debug values, and LOG_SIGNABLE_BODY can expose request bodies.
    let dispatch = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    tracing::dispatcher::with_default(&dispatch, || sign(request, params))
}

// =============================================================================
// Token Service API: POST /games/{game}/tokens
// =============================================================================

/// Response from token service
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenServiceResponse {
    pub access_token: Option<AgsAccessToken>,
}

fn token_service_request(ticket: SteamTicketHex) -> TokensRequest {
    TokensRequest {
        platform_type: Some(PlatformType::Steam),
        platform_auth: PlatformAuth { ticket },
        fallback_token: AgsAccessToken::default(),
        attribution_data: Some(AttributionData {
            os: "Windows 10".to_string(),
            resolution: "1920x1080".to_string(),
            language: "en-US".to_string(),
            platform: "Windows".to_string(),
            // The wire field is signed. Saturating keeps a clock far in the
            // future from wrapping into a negative timestamp; real values are
            // ~1.8e9 and nowhere near the i64 ceiling.
            created: i64::try_from(
                SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            )
            .unwrap_or(i64::MAX),
        }),
    }
}

// =============================================================================
// Credentials API: GET /prod/credentials/omni
// =============================================================================

/// AWS credentials from the credentials endpoint
#[derive(Clone, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub expiration: u64,
}

impl AwsCredentials {
    fn needs_refresh_at(&self, now: SystemTime) -> bool {
        let now_millis = now
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        u128::from(self.expiration).saturating_sub(now_millis)
            <= CREDENTIAL_REFRESH_WINDOW.as_millis()
    }
}

// =============================================================================
// Login Info API: GET /prod/game/getlogininfo/jwt/omni
// =============================================================================

/// Response wrapper for login info
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetLoginInfoResponse {
    pub login_info_list: LoginInfoList,
}

/// List of characters, worlds, and name reservations
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LoginInfoList {
    pub characters: Vec<Character>,
    #[serde(default)]
    pub worlds: Vec<World>,
}

/// Character information from login info
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Character {
    pub character_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub world_id: String,
    #[serde(default)]
    pub ftue_completed: bool,
    #[serde(default)]
    pub is_trial_owner: bool,
    #[serde(default)]
    pub location_group_id: String,
    #[serde(default)]
    pub location_id: String,
    #[serde(default)]
    pub permanent_app_owner: bool,
}

/// World information from login info
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct World {
    pub world_id: String,
    #[serde(default)]
    pub world_name: String,
    #[serde(default)]
    pub public_name: String,
    #[serde(default)]
    pub world_status: String,
}

// =============================================================================
// Login Queue API: POST /prod/game/login/queue/v2/jwt/omni
// =============================================================================

pub type LoginQueueRequest = LoginQueueRequestEnvelope;
pub type Token = LoginQueueToken;

struct LoginQueuePollArgs<'a> {
    profile: &'a AgsGameClientProfile,
    base_url: &'a str,
    jwt: &'a AgsAccessToken,
    aws_creds: &'a mut AwsCredentials,
    ticket_id: &'a str,
    refresh_secs: u64,
    region: Region,
}

struct LoginQueueRequestArgs<'a> {
    profile: &'a AgsGameClientProfile,
    base_url: &'a str,
    jwt: &'a AgsAccessToken,
    aws_creds: &'a mut AwsCredentials,
    character_id: &'a str,
    world_id: &'a str,
    is_trial_owner: bool,
    steam_ticket: &'a SteamTicketHex,
    region: Region,
}

// =============================================================================
// Login Context - Token + Steam credentials
// =============================================================================

/// Login context containing the server Token and Steam credentials.
/// This is all you need to register with the game server.
#[derive(Clone, Default, Serialize, Deserialize, Resource)]
pub struct LoginContext {
    /// Server connection token from the login queue
    #[serde(flatten)]
    pub token: Token,
    /// Steam auth ticket (hex encoded)
    pub steam_ticket: SteamTicketHex,
    /// Steam user ID
    pub steam_id: SteamId,
    /// AGS access JWT returned by the profile-selected token endpoint.
    #[serde(default)]
    pub eos_access_token: AgsAccessToken,
}

impl LoginContext {
    /// Create a new login context
    #[must_use]
    pub fn new(token: Token, steam_ticket: SteamTicketHex, steam_id: SteamId) -> Self {
        Self {
            token,
            steam_ticket,
            steam_id,
            eos_access_token: AgsAccessToken::default(),
        }
    }
}

impl std::ops::Deref for LoginContext {
    type Target = Token;
    fn deref(&self) -> &Self::Target {
        &self.token
    }
}

// =============================================================================
// AWS Service
// =============================================================================

/// AWS Service resource providing API operations
#[derive(Clone, Resource)]
pub struct AwsService {
    pub(crate) region_config: Arc<std::sync::Mutex<Option<RegionConfig>>>,
    pub(crate) http_service: Arc<http_plugin::HttpService>,
}

impl AwsService {
    /// Create a new AWS service
    #[must_use]
    pub fn new(http_service: Arc<http_plugin::HttpService>) -> Self {
        Self {
            region_config: Arc::new(std::sync::Mutex::new(None)),
            http_service,
        }
    }

    /// Get region config
    #[must_use]
    pub fn get_region_config(&self) -> Option<RegionConfig> {
        self.region_config
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Load region configuration from `CloudFront`
    ///
    /// # Errors
    ///
    /// Returns any error [`RegionConfig::fetch`] returns for the request or the
    /// response body, or [`AwsError::RegionConfigFetchFailed`] if the cached
    /// config mutex is poisoned and the fetched value cannot be stored.
    pub async fn load_region_config(&self, profile: &AgsGameClientProfile) -> Result<(), AwsError> {
        trace!("Loading region configuration...");
        let config = RegionConfig::fetch(&self.http_service, profile.steam_app_id).await?;
        if let Ok(mut guard) = self.region_config.lock() {
            *guard = Some(config);
            trace!("Region configuration loaded successfully");
        } else {
            return Err(AwsError::RegionConfigFetchFailed(
                "Failed to lock region config mutex".to_string(),
            ));
        }
        Ok(())
    }

    /// Get the base URL for a region and service
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::EndpointNotFound`] if the loaded region config has no
    /// entry for `region` and `client_tag`. Falls back to the default
    /// `CloudFront` endpoint, and so cannot fail, when no config is loaded yet
    /// or the cache mutex is poisoned.
    pub fn get_base_url(&self, region: Region, client_tag: &str) -> Result<String, AwsError> {
        // Resolve to an owned value inside the closure so the lock guard is a
        // statement temporary and never spans the fallback path below.
        let endpoint = self.region_config.lock().ok().and_then(|guard| {
            guard
                .as_ref()
                .map(|config| config.get_endpoint(region, client_tag))
        });
        if let Some(endpoint) = endpoint {
            return endpoint.map(|endpoint| format!("https://{endpoint}"));
        }
        trace!("Region config not loaded, using default endpoint");
        Ok("https://d2oeuvxi3kfsrw.cloudfront.net".to_string())
    }

    /// Get the base URL for a region and service (string-based)
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::RegionNotFound`] if `region_key` does not name a
    /// known region, or any error [`Self::get_base_url`] returns.
    pub fn get_base_url_str(&self, region_key: &str, client_tag: &str) -> Result<String, AwsError> {
        let region = Region::from_str(region_key)?;
        self.get_base_url(region, client_tag)
    }

    /// Get the AWS region string for signing
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::EndpointNotFound`] if the loaded region config has no
    /// entry for `region` and `client_tag`. Falls back to `us-east-1`, and so
    /// cannot fail, when no config is loaded yet or the cache mutex is poisoned.
    pub fn get_aws_region(&self, region: Region, client_tag: &str) -> Result<String, AwsError> {
        // Resolve to an owned value inside the closure so the lock guard is a
        // statement temporary and never spans the fallback path below.
        let resolved = self.region_config.lock().ok().and_then(|guard| {
            guard
                .as_ref()
                .map(|config| config.get_aws_region(region, client_tag))
        });
        if let Some(resolved) = resolved {
            return resolved;
        }
        trace!("Region config not loaded, using default region");
        Ok("us-east-1".to_string())
    }

    /// Get the AWS region string for signing (string-based)
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::RegionNotFound`] if `region_key` does not name a
    /// known region, or any error [`Self::get_aws_region`] returns.
    pub fn get_aws_region_str(
        &self,
        region_key: &str,
        client_tag: &str,
    ) -> Result<String, AwsError> {
        let region = Region::from_str(region_key)?;
        self.get_aws_region(region, client_tag)
    }

    /// Get HTTP service reference
    #[must_use]
    pub const fn http_service(&self) -> &Arc<http_plugin::HttpService> {
        &self.http_service
    }

    // =========================================================================
    // API Methods
    // =========================================================================

    /// Fetch JWT token from Amazon token service
    ///
    /// **Network IO** - Call from `IoTaskPool`
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::LoginQueueRequestFailed`] if `steam_ticket_hex` is
    /// not hex, if the token service answers with a non-success status, or if
    /// the response carries no `accessToken`; [`AwsError::HttpService`] if the
    /// request cannot be sent; or [`AwsError::CredentialsParseFailed`] if the
    /// response body cannot be read or parsed.
    pub async fn fetch_jwt_token(
        &self,
        profile: &AgsGameClientProfile,
        steam_ticket_hex: &str,
    ) -> Result<AgsAccessToken, AwsError> {
        let url = format!("{}/games/{}/tokens", TOKEN_SERVICE_ROOT, profile.game);
        let steam_ticket = SteamTicketHex::new(steam_ticket_hex).map_err(|error| {
            AwsError::LoginQueueRequestFailed(format!("Invalid Steam ticket: {error}"))
        })?;
        let request = token_service_request(steam_ticket);

        trace!("=== [1/5] TOKEN SERVICE ===");
        trace!("  POST {}", url);
        trace!("  game: {}", profile.game);
        trace!("  platform_type: {:?}", request.platform_type);
        trace!("  ticket_len: {}", request.platform_auth.ticket.len());

        let response = self
            .http_service
            .post(&url)
            .header("Content-Type", "application/json")
            .header("User-Agent", "OmniSDK/1.6.5/Windows")
            .json(&request)
            .send()
            .await
            .map_err(AwsError::from)?;

        if !response.status().is_success() {
            return Err(AwsError::LoginQueueRequestFailed(format!(
                "Token service error: {}",
                response.status()
            )));
        }

        let bytes = response.bytes().await.map_err(|e| {
            AwsError::CredentialsParseFailed(format!("Failed to read response: {e}"))
        })?;
        let resp: TokenServiceResponse = serde_json::from_slice(&bytes)
            .map_err(|e| AwsError::CredentialsParseFailed(format!("Parse failed: {e}")))?;

        let jwt = resp
            .access_token
            .ok_or_else(|| AwsError::LoginQueueRequestFailed("Missing accessToken".to_string()))?;

        trace!("  jwt_len: {}", jwt.as_str().len());
        trace!("===========================");

        Ok(jwt)
    }

    /// Fetch AWS credentials
    ///
    /// **Network IO** - Call from `IoTaskPool`
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::get_base_url`] returns while resolving the
    /// gateway endpoint, [`AwsError::HttpService`] if the request cannot be
    /// sent, [`AwsError::CredentialsFetchFailed`] on a non-success status, or
    /// [`AwsError::CredentialsParseFailed`] if the body cannot be read or is
    /// not valid credentials JSON.
    pub async fn fetch_credentials(
        &self,
        profile: &AgsGameClientProfile,
        jwt: &AgsAccessToken,
        region: Option<Region>,
    ) -> Result<AwsCredentials, AwsError> {
        let region = region.unwrap_or(DEFAULT_REGION);
        let base_url = self.get_base_url(region, profile.gateway_service_tag.as_str())?;
        let url = format!("{base_url}/prod/credentials/omni");

        trace!("=== [2/5] CREDENTIALS ===");
        trace!("  GET {}", url);

        let response = self
            .http_service
            .get(&url)
            .header("Authorization", format!("Bearer {}", jwt.as_str()))
            .send()
            .await
            .map_err(AwsError::from)?;

        if !response.status().is_success() {
            return Err(AwsError::CredentialsFetchFailed(format!(
                "Error: {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| AwsError::CredentialsParseFailed(format!("Failed to read: {e}")))?;
        let creds: AwsCredentials = serde_json::from_slice(&bytes)
            .map_err(|e| AwsError::CredentialsParseFailed(format!("Parse failed: {e}")))?;

        trace!("  temporary credentials received");
        trace!("=========================");

        Ok(creds)
    }

    /// Fetch login info (characters and worlds)
    ///
    /// **Network IO** - Call from `IoTaskPool`
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::get_base_url`] or [`Self::get_aws_region`]
    /// returns while resolving the gateway, any error the signed GET returns,
    /// or [`AwsError::LoginQueueRequestFailed`] on a non-success status or when
    /// the body cannot be read or parsed as login info.
    pub async fn fetch_login_info(
        &self,
        profile: &AgsGameClientProfile,
        jwt: &AgsAccessToken,
        aws_creds: &AwsCredentials,
        region: Option<Region>,
    ) -> Result<GetLoginInfoResponse, AwsError> {
        let region = region.unwrap_or(DEFAULT_REGION);
        let base_url = self.get_base_url(region, profile.gateway_service_tag.as_str())?;
        let url = format!(
            "{}/prod/game/getlogininfo/jwt/omni?channelId=STEAM_APP_ID.{}&includeNames=true&isTrialOwner=false",
            base_url,
            profile.steam_app_id.get()
        );

        trace!("=== [3/5] LOGIN INFO ===");
        trace!("  GET {}", url);

        let identity = Self::create_identity(aws_creds);
        let aws_region = self.get_aws_region(region, profile.gateway_service_tag.as_str())?;

        let response = self
            .send_signed_get(
                &url,
                jwt,
                profile.auth_header_name.as_str(),
                &identity,
                &aws_region,
            )
            .await?;

        if !response.status().is_success() {
            return Err(AwsError::LoginQueueRequestFailed(format!(
                "Error: {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| AwsError::LoginQueueRequestFailed(format!("Failed to read: {e}")))?;
        let resp: GetLoginInfoResponse = serde_json::from_slice(&bytes)
            .map_err(|e| AwsError::LoginQueueRequestFailed(format!("Parse failed: {e}")))?;

        trace!("  characters: {}", resp.login_info_list.characters.len());
        for (i, c) in resp.login_info_list.characters.iter().enumerate() {
            trace!("    [{}] {} ({})", i, c.name, c.character_id);
        }
        trace!("========================");

        Ok(resp)
    }

    /// Complete login queue flow to get login context
    ///
    /// **Network IO** - Call from `IoTaskPool`
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::LoginQueueRequestFailed`] if `steam_ticket_hex` is
    /// not hex, plus any error raised by the steps this drives: resolving the
    /// gateway endpoint, fetching credentials, the first login-queue request,
    /// and [`Self::login_queue_poll`], which yields
    /// [`AwsError::LoginQueueMissingToken`] if the queue never returns a token.
    pub async fn fetch_login_queue_ticket(
        &self,
        profile: &AgsGameClientProfile,
        jwt: &AgsAccessToken,
        steam_ticket_hex: &str,
        steam_id: u64,
        character_id: Option<&str>,
        region: Option<Region>,
    ) -> Result<LoginContext, AwsError> {
        let region = region.unwrap_or(DEFAULT_REGION);
        let base_url = self.get_base_url(region, profile.gateway_service_tag.as_str())?;
        let steam_ticket = SteamTicketHex::new(steam_ticket_hex).map_err(|error| {
            AwsError::LoginQueueRequestFailed(format!("Invalid Steam ticket: {error}"))
        })?;
        let steam_id = SteamId::new(steam_id);

        // Get AWS credentials
        let mut aws_creds = self.fetch_credentials(profile, jwt, Some(region)).await?;

        // Get login info for character selection
        let login_info = self
            .fetch_login_info(profile, jwt, &aws_creds, Some(region))
            .await?;
        let selected_character = match character_id {
            Some(character_id) => login_info
                .login_info_list
                .characters
                .iter()
                .find(|c| c.character_id == character_id)
                .cloned()
                .ok_or_else(|| {
                    AwsError::LoginQueueRequestFailed(format!(
                        "character override not found in login info: {character_id}"
                    ))
                })?,
            None => login_info
                .login_info_list
                .characters
                .first()
                .cloned()
                .ok_or_else(|| {
                    AwsError::LoginQueueRequestFailed(
                        "login info did not include any characters".to_string(),
                    )
                })?,
        };
        trace!(
            "  selected_character: {} ({}) world_id={} location_group_id={} location_id={} is_trial_owner={}",
            selected_character.name,
            selected_character.character_id,
            selected_character.world_id,
            selected_character.location_group_id,
            selected_character.location_id,
            selected_character.is_trial_owner
        );
        let character_id = selected_character.character_id.clone();
        let world_id = selected_character.world_id.clone();
        let is_trial_owner = selected_character.is_trial_owner;

        // First login request - get TicketId
        let first_response = self
            .login_queue_request(LoginQueueRequestArgs {
                profile,
                base_url: &base_url,
                jwt,
                aws_creds: &mut aws_creds,
                character_id: &character_id,
                world_id: &world_id,
                is_trial_owner,
                steam_ticket: &steam_ticket,
                region,
            })
            .await?;

        let compound_ticket_id = first_response.login_queue_response.ticket_id;
        let refresh_interval = first_response.login_queue_response.refresh_interval;
        // A non-positive interval means unspecified; fall back to 2 seconds
        // rather than wrapping a negative value into an enormous sleep.
        let refresh_secs = match u64::try_from(refresh_interval) {
            Ok(seconds) if seconds > 0 => seconds,
            _ => 2,
        };

        // Second login request - get Token (with retry)
        let token = self
            .login_queue_poll(LoginQueuePollArgs {
                profile,
                base_url: &base_url,
                jwt,
                aws_creds: &mut aws_creds,
                ticket_id: &compound_ticket_id,
                refresh_secs,
                region,
            })
            .await?;

        Ok(LoginContext {
            token,
            steam_ticket,
            steam_id,
            eos_access_token: jwt.clone(),
        })
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    async fn refresh_credentials_if_needed(
        &self,
        profile: &AgsGameClientProfile,
        jwt: &AgsAccessToken,
        region: Region,
        credentials: &mut AwsCredentials,
    ) -> Result<(), AwsError> {
        if credentials.needs_refresh_at(SystemTime::now()) {
            trace!("temporary credentials reached the native 59-second refresh window");
            *credentials = self.fetch_credentials(profile, jwt, Some(region)).await?;
        }
        Ok(())
    }

    fn create_identity(creds: &AwsCredentials) -> Identity {
        Credentials::new(
            creds.access_key_id.clone(),
            creds.secret_access_key.clone(),
            Some(creds.session_token.clone()),
            None,
            "aws-plugin",
        )
        .into()
    }

    async fn send_signed_get(
        &self,
        url: &str,
        jwt: &AgsAccessToken,
        auth_header_name: &str,
        identity: &Identity,
        aws_region: &str,
    ) -> Result<reqwest::Response, AwsError> {
        let signing_settings = SigningSettings::default();
        let signing_params = v4::SigningParams::builder()
            .identity(identity)
            .region(aws_region)
            .name(AWS_SERVICE)
            .time(SystemTime::now())
            .settings(signing_settings)
            .build()
            .map_err(|e| AwsError::SigningFailed(format!("Build params failed: {e}")))?;
        let signing_params: SigningParams = signing_params.into();

        let mut headers = vec![("Accept", "*/*"), ("x-amz-api-version", "2017-09-26")];
        if !jwt.as_str().is_empty() {
            headers.push((auth_header_name, jwt.as_str()));
        }
        let signable = SignableRequest::new(
            "GET",
            url,
            headers.iter().map(|(k, v)| (*k, *v)),
            SignableBody::Bytes(&[]),
        )
        .map_err(|e| AwsError::SigningFailed(format!("Create request failed: {e}")))?;

        let output = sign_without_tracing(signable, &signing_params)
            .map_err(|e| AwsError::SigningFailed(format!("Sign failed: {e}")))?;

        let mut builder = self.http_service.get(url);
        for (key, value) in output.output().headers() {
            let hv = HeaderValue::from_str(value)
                .map_err(|e| AwsError::SigningFailed(format!("Invalid header: {e}")))?;
            builder = builder.header(key, hv);
        }
        builder = builder
            .header("Accept", "*/*")
            .header("x-amz-api-version", "2017-09-26");
        if !jwt.as_str().is_empty() {
            builder = builder.header(auth_header_name, jwt.as_str());
        }

        builder.send().await.map_err(AwsError::from)
    }

    async fn send_signed_post(
        &self,
        url: &str,
        body: &str,
        jwt: &AgsAccessToken,
        auth_header_name: &str,
        identity: &Identity,
        aws_region: &str,
    ) -> Result<reqwest::Response, AwsError> {
        let signing_settings = SigningSettings::default();
        let signing_params = v4::SigningParams::builder()
            .identity(identity)
            .region(aws_region)
            .name(AWS_SERVICE)
            .time(SystemTime::now())
            .settings(signing_settings)
            .build()
            .map_err(|e| AwsError::SigningFailed(format!("Build params failed: {e}")))?;
        let signing_params: SigningParams = signing_params.into();

        let mut headers = vec![
            ("Content-Type", "application/x-amz-json-1.1"),
            ("Accept", "*/*"),
            ("x-amz-api-version", "2017-09-26"),
        ];
        if !jwt.as_str().is_empty() {
            headers.push((auth_header_name, jwt.as_str()));
        }
        let signable = SignableRequest::new(
            "POST",
            url,
            headers.iter().map(|(k, v)| (*k, *v)),
            SignableBody::Bytes(body.as_bytes()),
        )
        .map_err(|e| AwsError::SigningFailed(format!("Create request failed: {e}")))?;

        let output = sign_without_tracing(signable, &signing_params)
            .map_err(|e| AwsError::SigningFailed(format!("Sign failed: {e}")))?;

        let mut builder = self.http_service.post(url).body(body.to_string());
        for (key, value) in output.output().headers() {
            let hv = HeaderValue::from_str(value)
                .map_err(|e| AwsError::SigningFailed(format!("Invalid header: {e}")))?;
            builder = builder.header(key, hv);
        }
        builder = builder
            .header("Content-Type", "application/x-amz-json-1.1")
            .header("Accept", "*/*")
            .header("x-amz-api-version", "2017-09-26");
        if !jwt.as_str().is_empty() {
            builder = builder.header(auth_header_name, jwt.as_str());
        }

        builder.send().await.map_err(AwsError::from)
    }

    async fn login_queue_request(
        &self,
        args: LoginQueueRequestArgs<'_>,
    ) -> Result<LoginQueueResponse, AwsError> {
        let url = format!(
            "{}/prod/game/login/queue/v2/jwt/omni?channelId=STEAM_APP_ID.{}&tokenVersion={}",
            args.base_url,
            args.profile.steam_app_id.get(),
            args.profile.token_version.get()
        );
        let request = LoginQueueRequest {
            login_queue_request: Some(LoginQueueRequestBody {
                character_id: args.character_id.to_string(),
                client_capabilities: args.profile.client_capabilities.clone(),
                is_trial_owner: args.is_trial_owner,
                steam_app_id: args.profile.steam_app_id,
                steam_auth_ticket: SteamTicketHex::new(args.steam_ticket.truncated_game_auth_hex())
                    .map_err(|error| {
                        AwsError::LoginQueueRequestFailed(format!(
                            "Invalid truncated Steam ticket: {error}"
                        ))
                    })?,
                world_id: args.world_id.to_string(),
            }),
            client_application: None,
        };
        let body = serde_json::to_string(&request)
            .map_err(|e| AwsError::LoginQueueRequestFailed(format!("Serialize failed: {e}")))?;
        let request_body = request
            .login_queue_request
            .as_ref()
            .expect("login queue join request includes body");

        trace!("=== [4/5] LOGIN QUEUE (first) ===");
        trace!("  POST {}", url);
        trace!("  character_id: {}", args.character_id);
        trace!("  world_id: {}", request_body.world_id);
        trace!("  is_trial_owner: {}", args.is_trial_owner);
        trace!("  steam_ticket_hex input len: {}", args.steam_ticket.len());
        trace!(
            "  steam_ticket_hex truncated to: {} chars",
            request_body.steam_auth_ticket.len()
        );

        self.refresh_credentials_if_needed(args.profile, args.jwt, args.region, args.aws_creds)
            .await?;
        let identity = Self::create_identity(args.aws_creds);
        let aws_region =
            self.get_aws_region(args.region, args.profile.gateway_service_tag.as_str())?;

        let response = self
            .send_signed_post(
                &url,
                &body,
                args.jwt,
                args.profile.auth_header_name.as_str(),
                &identity,
                &aws_region,
            )
            .await?;

        if !response.status().is_success() {
            return Err(AwsError::LoginQueueRequestFailed(format!(
                "Error: {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| AwsError::LoginQueueResponseParseFailed(format!("Failed to read: {e}")))?;
        let resp: LoginQueueResponse = serde_json::from_slice(&bytes)
            .map_err(|e| AwsError::LoginQueueResponseParseFailed(format!("Parse failed: {e}")))?;

        trace!(
            "  queue ticket received: {} chars",
            resp.login_queue_response.ticket_id.len()
        );
        trace!(
            "  refresh: {:?}",
            resp.login_queue_response.refresh_interval
        );
        trace!("=================================");

        Ok(resp)
    }

    async fn login_queue_poll(&self, args: LoginQueuePollArgs<'_>) -> Result<Token, AwsError> {
        let LoginQueuePollArgs {
            profile,
            base_url,
            jwt,
            aws_creds,
            ticket_id,
            refresh_secs,
            region,
        } = args;
        // Second request URL does NOT include channelId
        let url = format!(
            "{}/prod/game/login/queue/v2/{}/jwt/omni?tokenVersion={}",
            base_url,
            ticket_id,
            profile.token_version.get()
        );
        let request = LoginQueueRefreshRequest {
            client_capabilities: profile.client_capabilities.clone(),
        };
        let body = serde_json::to_string(&request)
            .map_err(|e| AwsError::LoginQueueRequestFailed(format!("Serialize failed: {e}")))?;

        let aws_region = self.get_aws_region(region, profile.gateway_service_tag.as_str())?;

        let mut last_err: Option<AwsError> = None;
        for attempt in 0..5 {
            self.refresh_credentials_if_needed(profile, jwt, region, aws_creds)
                .await?;
            let identity = Self::create_identity(aws_creds);
            trace!("=== [5/5] LOGIN QUEUE (poll #{}) ===", attempt + 1);
            trace!("  POST login queue poll");

            let response = self
                .send_signed_post(
                    &url,
                    &body,
                    jwt,
                    profile.auth_header_name.as_str(),
                    &identity,
                    &aws_region,
                )
                .await?;

            if response.status().as_u16() == 429 {
                trace!("  Rate limited, waiting {}s...", refresh_secs);
                async_io::Timer::after(std::time::Duration::from_secs(refresh_secs)).await;
                last_err = Some(AwsError::LoginQueueRequestFailed("Rate limited".into()));
                continue;
            }

            if !response.status().is_success() {
                return Err(AwsError::LoginQueueRequestFailed(format!(
                    "Error: {}",
                    response.status()
                )));
            }

            let bytes = response.bytes().await.map_err(|e| {
                AwsError::LoginQueueResponseParseFailed(format!("Failed to read: {e}"))
            })?;

            trace!("  response_len: {} bytes", bytes.len());

            let resp: LoginQueueResponse = serde_json::from_slice(&bytes).map_err(|e| {
                AwsError::LoginQueueResponseParseFailed(format!("Parse failed: {e}"))
            })?;

            if let Some(token) = resp.login_queue_response.token {
                trace!("=== LOGIN QUEUE TOKEN RECEIVED ===");
                trace!("  token_version: {}", token.token_version);
                trace!("  signature_len: {} chars", token.signature.len());
                trace!("  jwt_claims_len: {} chars", token.jwt_claims.len());
                trace!("==================================");
                return Ok(token);
            }

            // No token yet, wait and retry
            trace!("  No token yet, waiting {}s...", refresh_secs);
            async_io::Timer::after(std::time::Duration::from_secs(refresh_secs)).await;
            last_err = Some(AwsError::LoginQueueMissingToken);
        }

        Err(last_err.unwrap_or(AwsError::LoginQueueMissingToken))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    struct SharedWriterGuard(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriterGuard {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| io::Error::other("trace writer mutex poisoned"))?
                .write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedWriter {
        type Writer = SharedWriterGuard;

        fn make_writer(&'a self) -> Self::Writer {
            SharedWriterGuard(self.0.clone())
        }
    }

    #[test]
    fn credentials_refresh_at_exactly_fifty_nine_seconds() {
        let expiration = 2_000_000_000_000_u64;
        let credentials = AwsCredentials {
            access_key_id: String::new(),
            secret_access_key: String::new(),
            session_token: String::new(),
            expiration,
        };
        let expiration_time = UNIX_EPOCH + Duration::from_millis(expiration);

        assert!(!credentials.needs_refresh_at(
            expiration_time - CREDENTIAL_REFRESH_WINDOW - Duration::from_millis(1)
        ));
        assert!(credentials.needs_refresh_at(expiration_time - CREDENTIAL_REFRESH_WINDOW));
        assert!(credentials.needs_refresh_at(expiration_time));
    }

    #[test]
    fn signing_helper_suppresses_request_and_credential_trace_values() {
        let writer = SharedWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(writer.clone())
            .finish();
        let dispatch = tracing::Dispatch::new(subscriber);
        let credentials = Credentials::new(
            "ASIAEXAMPLEACCESSKEY",
            "secret-access-key-sentinel",
            Some("session-token-sentinel".to_owned()),
            None,
            "signing-trace-test",
        );
        let identity: Identity = credentials.into();
        let settings = SigningSettings::default();
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region("us-east-1")
            .name(AWS_SERVICE)
            .time(UNIX_EPOCH + Duration::from_secs(2_000_000_000))
            .settings(settings)
            .build()
            .unwrap();
        let params: SigningParams = params.into();
        let signable = SignableRequest::new(
            "POST",
            "https://gateway.example.test/prod/game/login/queue/v2",
            [("x-project-auth", "jwt-header-sentinel")].into_iter(),
            SignableBody::Bytes(b"request-body-sentinel"),
        )
        .unwrap();

        tracing::dispatcher::with_default(&dispatch, || {
            trace!("collector-active");
            sign_without_tracing(signable, &params).unwrap();
        });

        let logged = String::from_utf8(writer.0.lock().unwrap().clone()).unwrap();
        assert!(logged.contains("collector-active"));
        for secret in [
            "secret-access-key-sentinel",
            "session-token-sentinel",
            "jwt-header-sentinel",
            "request-body-sentinel",
        ] {
            assert!(!logged.contains(secret), "trace leaked {secret}");
        }
    }
}
