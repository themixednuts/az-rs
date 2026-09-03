//! Amazon Games Services region configuration.
//!
//! Fetches and parses the region configuration JSON to determine `CloudFront` endpoints
//! for different AWS regions (US East, EU Central, SA East, AP Southeast).

use crate::error::AwsError;
use ags_auth_protocol::types::SteamAppId;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

const REGION_CONFIG_ROOT: &str = "https://d2c74t4zimux3r.cloudfront.net";

/// Public API endpoint configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicApi {
    /// AWS region (e.g., "us-east-1", "eu-central-1")
    pub aws_region: String,
    /// API version
    pub version: String,
    /// API endpoint (domain name)
    pub api_endpoint: String,
    /// Client tag identifying the service type
    pub client_tag: String,
    /// Optional stage (e.g., "prod")
    #[serde(default)]
    pub stage: Option<String>,
}

/// Region configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionInfo {
    /// Display name (e.g., "US East")
    pub display_name: String,
    /// Localized name key (e.g., "@`mm_us_east`")
    pub localized_name: String,
    /// Cognito pool ID
    pub pool_id: String,
    /// List of public API endpoints
    pub public_apis: Vec<PublicApi>,
}

/// Complete region configuration response
#[derive(Debug, Clone, Deserialize)]
pub struct RegionConfig {
    /// Channel name (e.g., "Retail")
    #[serde(rename = "channelName")]
    pub channel_name: String,

    /// Platform metadata (ignored - not needed for region config)
    #[serde(rename = "platformMetadata", default)]
    _platform_metadata: Option<serde_json::Value>,

    /// Region data keyed by region ID (e.g., "iad-prod", "fra-prod")
    /// Using flatten to deserialize remaining region keys as `HashMap` entries
    #[serde(flatten)]
    pub regions: HashMap<String, RegionInfo>,
}

impl RegionConfig {
    /// Fetch region configuration from `CloudFront`
    ///
    /// **Network IO Operation** - MUST be called from `IoTaskPool`
    ///
    /// This performs an HTTP request to fetch the region configuration JSON.
    /// Must be called from within a task spawned on Bevy's `IoTaskPool`.
    ///
    /// # Arguments
    /// * `http_service` - HTTP service (automatically uses Tokio runtime)
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::HttpService`] if the request cannot be sent,
    /// [`AwsError::RegionConfigFetchFailed`] if `CloudFront` answers with a
    /// non-success status, or [`AwsError::RegionConfigParseFailed`] if the
    /// response body cannot be read or is not valid region-config JSON.
    pub async fn fetch(
        http_service: &http_plugin::HttpService,
        steam_app_id: SteamAppId,
    ) -> Result<Self, AwsError> {
        let url = format!("{REGION_CONFIG_ROOT}/STEAM_APP_ID.{steam_app_id}.json");
        tracing::trace!("Fetching region config from: {}", url);
        let response = http_service
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                tracing::trace!("HTTP request failed: {}", e);
                AwsError::from(e)
            })?;

        if !response.status().is_success() {
            let status = response.status();
            tracing::trace!("Region config endpoint returned error: {}", status);
            return Err(AwsError::RegionConfigFetchFailed(format!(
                "Endpoint returned error: {status}"
            )));
        }

        tracing::trace!("Region config HTTP response received, parsing JSON...");

        // Get response body as text first for debugging
        let status = response.status();
        let headers = response.headers().clone();
        let text = response.text().await.map_err(|e| {
            tracing::trace!("Failed to read response body as text: {}", e);
            AwsError::RegionConfigParseFailed(format!("Failed to read response: {e}"))
        })?;

        tracing::trace!("Response status: {}", status);
        tracing::trace!("Response headers: {:?}", headers);
        tracing::trace!(
            "Response body (first 500 chars): {}",
            text.chars().take(500).collect::<String>()
        );

        // Parse JSON from text
        let config: Self = serde_json::from_str(&text).map_err(|e| {
            tracing::trace!("Failed to parse region config JSON: {}", e);
            tracing::trace!("Response body length: {} bytes", text.len());
            tracing::trace!(
                "Response body preview: {}",
                text.chars().take(1000).collect::<String>()
            );
            AwsError::RegionConfigParseFailed(format!(
                "Parse failed: {} (response length: {} bytes)",
                e,
                text.len()
            ))
        })?;

        tracing::trace!(
            "Region config parsed successfully, found {} regions",
            config.regions.len()
        );
        Ok(config)
    }

    /// Get the `CloudFront` endpoint for a specific region and service
    ///
    /// # Arguments
    /// * `region` - Region identifier
    /// * `client_tag` - Service tag (e.g., "JavelinGatewayService-CF", "authStack")
    ///
    /// # Returns
    /// The API endpoint domain (without https://)
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::RegionNotFound`] if the config carries no entry for
    /// that region key, or [`AwsError::EndpointNotFound`] if the region is
    /// present but publishes no API with `client_tag`.
    pub fn get_endpoint(&self, region: Region, client_tag: &str) -> Result<String, AwsError> {
        let region_key = region.as_str();
        let region_info = self
            .regions
            .get(region_key)
            .ok_or_else(|| AwsError::RegionNotFound(region_key.to_string()))?;
        let api = region_info
            .public_apis
            .iter()
            .find(|api| api.client_tag == client_tag)
            .ok_or_else(|| {
                AwsError::EndpointNotFound(region_key.to_string(), client_tag.to_string())
            })?;

        Ok(api.api_endpoint.clone())
    }

    /// Get the `CloudFront` endpoint for a specific region key string and service
    ///
    /// # Arguments
    /// * `region_key` - Region key string (e.g., "iad-prod", "fra-prod", "gru-prod")
    /// * `client_tag` - Service tag (e.g., "JavelinGatewayService-CF", "authStack")
    ///
    /// # Returns
    /// The API endpoint domain (without https://)
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::RegionNotFound`] if the config carries no entry for
    /// that region key, or [`AwsError::EndpointNotFound`] if the region is
    /// present but publishes no API with `client_tag`.
    pub fn get_endpoint_str(&self, region_key: &str, client_tag: &str) -> Result<String, AwsError> {
        let region_info = self
            .regions
            .get(region_key)
            .ok_or_else(|| AwsError::RegionNotFound(region_key.to_string()))?;
        let api = region_info
            .public_apis
            .iter()
            .find(|api| api.client_tag == client_tag)
            .ok_or_else(|| {
                AwsError::EndpointNotFound(region_key.to_string(), client_tag.to_string())
            })?;

        Ok(api.api_endpoint.clone())
    }

    /// Get the AWS region for a specific endpoint
    ///
    /// # Arguments
    /// * `region` - Region identifier
    /// * `client_tag` - Service tag (e.g., "JavelinGatewayService-CF", "authStack")
    ///
    /// # Returns
    /// The AWS region string (e.g., "us-east-1")
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::RegionNotFound`] if the config carries no entry for
    /// that region key, or [`AwsError::EndpointNotFound`] if the region is
    /// present but publishes no API with `client_tag`.
    pub fn get_aws_region(&self, region: Region, client_tag: &str) -> Result<String, AwsError> {
        let region_key = region.as_str();
        let region_info = self
            .regions
            .get(region_key)
            .ok_or_else(|| AwsError::RegionNotFound(region_key.to_string()))?;
        let api = region_info
            .public_apis
            .iter()
            .find(|api| api.client_tag == client_tag)
            .ok_or_else(|| {
                AwsError::EndpointNotFound(region_key.to_string(), client_tag.to_string())
            })?;

        Ok(api.aws_region.clone())
    }

    /// Get the AWS region for a specific endpoint (string-based)
    ///
    /// # Arguments
    /// * `region_key` - Region key string (e.g., "iad-prod", "fra-prod", "gru-prod")
    /// * `client_tag` - Service tag (e.g., "JavelinGatewayService-CF", "authStack")
    ///
    /// # Returns
    /// The AWS region string (e.g., "us-east-1")
    ///
    /// # Errors
    ///
    /// Returns [`AwsError::RegionNotFound`] if the config carries no entry for
    /// that region key, or [`AwsError::EndpointNotFound`] if the region is
    /// present but publishes no API with `client_tag`.
    pub fn get_aws_region_str(
        &self,
        region_key: &str,
        client_tag: &str,
    ) -> Result<String, AwsError> {
        let region_info = self
            .regions
            .get(region_key)
            .ok_or_else(|| AwsError::RegionNotFound(region_key.to_string()))?;
        let api = region_info
            .public_apis
            .iter()
            .find(|api| api.client_tag == client_tag)
            .ok_or_else(|| {
                AwsError::EndpointNotFound(region_key.to_string(), client_tag.to_string())
            })?;

        Ok(api.aws_region.clone())
    }

    /// Get the auth stack endpoint for a region
    ///
    /// # Arguments
    /// * `region` - Region identifier
    ///
    /// # Returns
    /// Full HTTPS URL for the auth stack endpoint
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::get_endpoint`] returns when looking up the
    /// `authStack` tag for `region`.
    pub fn get_auth_stack_endpoint(&self, region: Region) -> Result<String, AwsError> {
        self.get_endpoint(region, "authStack")
            .map(|endpoint| format!("https://{endpoint}"))
    }

    /// Get the auth stack endpoint for a region (string-based)
    ///
    /// # Arguments
    /// * `region_key` - Region key string (e.g., "iad-prod" for US East)
    ///
    /// # Returns
    /// Full HTTPS URL for the auth stack endpoint
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::get_endpoint_str`] returns when looking up the
    /// `authStack` tag for `region_key`.
    pub fn get_auth_stack_endpoint_str(&self, region_key: &str) -> Result<String, AwsError> {
        self.get_endpoint_str(region_key, "authStack")
            .map(|endpoint| format!("https://{endpoint}"))
    }

    /// List all available regions
    #[must_use]
    pub fn available_regions(&self) -> Vec<String> {
        self.regions.keys().cloned().collect()
    }

    /// Get region display name
    #[must_use]
    pub fn get_region_display_name(&self, region_key: &str) -> Option<&str> {
        self.regions
            .get(region_key)
            .map(|r| r.display_name.as_str())
    }

    /// Get region display name from Region enum
    #[must_use]
    pub fn get_region_display_name_from_enum(&self, region: Region) -> Option<&str> {
        self.get_region_display_name(region.as_str())
    }
}

/// Region identifier for game servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Region {
    /// US East (Virginia)
    #[serde(rename = "iad-prod")]
    UsEast,
    /// EU Central (Frankfurt)
    #[serde(rename = "fra-prod")]
    EuCentral,
    /// SA East (Sao Paulo)
    #[serde(rename = "gru-prod")]
    SaEast,
    /// AP Southeast (Sydney)
    #[serde(rename = "syd-prod")]
    ApSoutheast,
    /// US West (Oregon)
    #[serde(rename = "pdx-prod")]
    UsWest,
}

impl Region {
    /// Get the region key string (e.g., "iad-prod")
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::UsEast => "iad-prod",
            Self::EuCentral => "fra-prod",
            Self::SaEast => "gru-prod",
            Self::ApSoutheast => "syd-prod",
            Self::UsWest => "pdx-prod",
        }
    }

    /// Get the display name for the region
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::UsEast => "US East",
            Self::EuCentral => "EU Central",
            Self::SaEast => "SA East",
            Self::ApSoutheast => "AP Southeast",
            Self::UsWest => "US West",
        }
    }

    /// Get all available regions
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::UsEast,
            Self::EuCentral,
            Self::SaEast,
            Self::ApSoutheast,
            Self::UsWest,
        ]
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl FromStr for Region {
    type Err = AwsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "iad-prod" => Ok(Self::UsEast),
            "fra-prod" => Ok(Self::EuCentral),
            "gru-prod" => Ok(Self::SaEast),
            "syd-prod" => Ok(Self::ApSoutheast),
            "pdx-prod" => Ok(Self::UsWest),
            _ => Err(AwsError::RegionNotFound(s.to_string())),
        }
    }
}

impl From<Region> for String {
    fn from(region: Region) -> Self {
        region.as_str().to_string()
    }
}
