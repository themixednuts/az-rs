//! Bevy plugin for Epic Online Services (EOS) SDK.
//!
//! Drives EOS Connect from a host-provided external credential and starts an
//! EOS Anti-Cheat `ClientServer` session. See `service.rs` and `anticheat.rs`
//! for the lifecycle details.

pub mod error;
pub mod service;
pub mod settings;

#[cfg(feature = "anti-cheat-client")]
pub mod anticheat;
#[cfg(feature = "client-runtime")]
pub mod client;
#[cfg(feature = "anti-cheat-client")]
pub mod plugin;

#[cfg(feature = "anti-cheat-client")]
pub use anticheat::{AntiCheatClientState, AntiCheatService};
#[cfg(feature = "client-runtime")]
pub use client::EosClientPlugin;
#[cfg(feature = "anti-cheat-client")]
pub use plugin::{EosAntiCheatReady, EosConnectCredential, EosPlugin};
pub use service::EosService;
pub use settings::EosSettings;
