// Thread messages for communication between main thread and carrier thread
// Following GridMate Carrier.cpp ThreadMessage class

use super::message::MessageData;
use super::types::DisconnectReason;

/// Commands sent from the application's owning task into the
/// per-connection carrier driver task (`GridMate`: `CarrierCommand`).
/// Internal to gridmate — consumers go through `CarrierImpl::send` /
/// `disconnect` / etc.
pub enum CarrierCommand {
    /// Send a message on a specific channel and priority
    SendMessage {
        message: MessageData,
        priority: usize,
    },

    /// Request graceful disconnect
    Disconnect(DisconnectReason),
}

/// Carrier-level events emitted from the per-connection carrier driver
/// task back to the application's owning task (`GridMate`:
/// `CarrierEvent`; despite the name, no actual OS thread is
/// involved — both ends run on `IoTaskPool`).
///
/// Internal to gridmate — application code consumes the higher-level
/// [`crate::session_service::Event`] surface instead, which both
/// `SessionServiceHandle` (client) and `ServerListenerHandle` (server)
/// emit by translating this enum + adding session/peer context.
#[derive(Debug, Clone)]
pub enum CarrierEvent {
    /// Connection established successfully
    Connected { version: u32 },

    /// Connection lost or disconnected
    Disconnected { reason: DisconnectReason },

    /// Message received on a channel
    // Both readers of these fields are feature-gated (the `client`
    // `SessionServiceActor` and the `server` translator), so a `transport`-only
    // build constructs the variant with nobody consuming it. Deleting the
    // fields would break `client`/`server`, so scope the allow to that config.
    #[cfg_attr(not(any(feature = "client", feature = "server")), allow(dead_code))]
    MessageReceived { channel: u8, data: bytes::Bytes },

    /// Error occurred in carrier thread
    Error { description: String },
}

// Spawned tasks use `IoTaskPool` directly today (see the
// `CarrierDriver` and per-session forwarder spawn sites); a
// `Spawner` trait is pending to decouple from Bevy.
