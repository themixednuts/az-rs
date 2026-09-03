//! Error types for EOS plugin operations

use thiserror::Error;

/// Errors that can occur during EOS operations
#[derive(Error, Debug)]
pub enum EosError {
    #[error("EOS initialization failed: {0:?}")]
    InitializationFailed(String),

    #[error("EOS platform creation failed")]
    PlatformCreationFailed,

    #[error("EOS auth interface not available")]
    AuthInterfaceUnavailable,

    #[error("EOS login failed: {0:?}")]
    LoginFailed(String),

    #[error("EOS token copy failed: {0:?}")]
    TokenCopyFailed(String),

    #[error("EOS login timeout after {0} seconds")]
    LoginTimeout(u64),

    #[error("EOS callback channel disconnected")]
    CallbackChannelDisconnected,

    #[error("Invalid Steam ticket: {0}")]
    InvalidSteamTicket(String),

    #[error("Invalid EOS configuration: {0}")]
    InvalidConfiguration(String),
}
