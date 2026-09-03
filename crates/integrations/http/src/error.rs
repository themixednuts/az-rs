//! Error types for HTTP plugin

use thiserror::Error;

/// Errors that can occur when using `HttpService`
#[derive(Debug, Error)]
pub enum HttpError {
    /// Error from reqwest (HTTP client error)
    #[error("HTTP request error: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// Error from Tokio runtime (task join error, runtime shutdown, etc.)
    #[error("Tokio runtime error: {0}")]
    Tokio(#[from] tokio::task::JoinError),

    /// Error from `std::io` (network/IO errors)
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Other HTTP-related errors
    #[error("HTTP error: {0}")]
    Other(String),
}

impl From<&str> for HttpError {
    fn from(msg: &str) -> Self {
        Self::Other(msg.to_string())
    }
}

/// Convenience type alias for Result<T, `HttpError`>
pub type Result<T> = std::result::Result<T, HttpError>;
