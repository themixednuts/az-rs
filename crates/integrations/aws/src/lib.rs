//! AWS Plugin for Bevy
//!
//! Provides AWS API operations for an AGS authentication flow.
//! This plugin handles:
//! - Region configuration management
//! - AWS credentials fetching
//! - SigV4-signed requests to AWS API Gateway endpoints
//! - Login queue operations

#![warn(clippy::await_holding_lock)]
#![allow(clippy::upper_case_acronyms)]

pub mod error;
pub mod plugin;
pub mod region_config;
pub mod service;

// Re-export types used by other crates
pub use ags_auth_protocol::login_queue::{
    LoginQueueRequestEnvelope as LoginQueueRequest, LoginQueueResponse, LoginQueueToken as Token,
};
pub use ags_auth_protocol::types::{
    AgsAccessToken, ClientCapabilities, GAME_AUTH_STEAM_TICKET_HEX_CHARS, JwtClaims, SteamId,
    SteamTicketHex, TokenClientCapabilities,
};
pub use error::AwsError;
pub use plugin::{AwsAuthProfile, AwsPlugin, LoginRequest};
pub use region_config::{Region, RegionConfig};
pub use service::{
    AwsCredentials, AwsService, Character, GetLoginInfoResponse, LoginContext, LoginInfoList, World,
};
