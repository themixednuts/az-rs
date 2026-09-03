use thiserror::Error;

/// Errors that can occur during AWS operations
#[derive(Error, Debug)]
pub enum AwsError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("Failed to fetch region configuration: {0}")]
    RegionConfigFetchFailed(String),

    #[error("Failed to parse region configuration: {0}")]
    RegionConfigParseFailed(String),

    #[error("Region not found: {0}")]
    RegionNotFound(String),

    #[error("Endpoint not found for region {0} and service {1}")]
    EndpointNotFound(String, String),

    #[error("Credentials fetch failed: {0}")]
    CredentialsFetchFailed(String),

    #[error("Credentials parse failed: {0}")]
    CredentialsParseFailed(String),

    #[error("Signing failed: {0}")]
    SigningFailed(String),

    #[error("Login queue request failed: {0}")]
    LoginQueueRequestFailed(String),

    #[error("Login queue response parse failed: {0}")]
    LoginQueueResponseParseFailed(String),

    #[error("Login queue response missing Token")]
    LoginQueueMissingToken,

    #[error("HTTP service error: {0}")]
    HttpService(#[from] http_plugin::HttpError),
}
