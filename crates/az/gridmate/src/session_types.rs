//! Transport-independent session identity and network event values.
//!
//! These types are shared by protocol/reflection crates and live outside the
//! socket-owning session service so asset builders can describe reflected
//! messages without linking a network transport.

use bytes::Bytes;

/// Session index type. Newtype for type safety — prevents mixing
/// session indices with other usize values.
///
/// `Deref<Target = usize>` lets indexing and log callsites write
/// `*session_id` without exposing transport/session-service ownership.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    derive_more::Deref,
    derive_more::Display,
    derive_more::From,
    derive_more::Into,
)]
pub struct SessionId(pub usize);

impl SessionId {
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }
}

/// Unified network event surface emitted by client and server transports.
#[derive(Debug, Clone)]
pub enum Event {
    /// Data received on a session (raw decrypted carrier bytes, no envelope parsing).
    Received {
        session: SessionId,
        channel: u8,
        data: Bytes,
    },

    /// Typed `IMessage` envelope received after wire framing has been decoded.
    TypedReceived {
        session: SessionId,
        channel: u8,
        type_index: u32,
        correlation_id: Option<uuid::Uuid>,
        data: Bytes,
    },

    /// Data accepted for send on a session.
    Sent {
        session: SessionId,
        data: Bytes,
        channel: u8,
        reliability: crate::carrier::DataReliability,
        priority: usize,
        type_index: Option<u32>,
    },

    /// Session connected and carrier ready for typed traffic.
    Ready { session: SessionId },

    /// Session disconnected.
    Disconnected { session: SessionId, reason: String },

    /// Transport-level error not bound to a specific session.
    Error { description: String },
}
