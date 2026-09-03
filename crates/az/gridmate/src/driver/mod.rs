//! Driver Layer — DTLS-encrypted UDP transport.
//!
//! `GridMate`: `SecureSocketDriver`.
//!
//! `SecureConnection` is the per-connection DTLS state machine
//! (initiator / responder). On the server side `MultiPeerListener`
//! demuxes a shared UDP socket into per-peer `SecureConnection`s.

pub mod error;
#[cfg(feature = "server")]
pub mod multi_peer;
#[cfg(feature = "transport")]
pub(crate) mod recv_ring;
#[cfg(feature = "transport")]
pub mod secure_connection;
#[cfg(feature = "server")]
pub mod test_cert;

pub use error::DriverError;
#[cfg(feature = "server")]
pub use multi_peer::MultiPeerListener;
#[cfg(feature = "server")]
pub(crate) use multi_peer::{MultiPeerEvent, PeerGeneration};
#[cfg(feature = "client")]
pub use secure_connection::SecureConnectionBuilder;
#[cfg(feature = "server")]
pub use secure_connection::SecureSocketListener;
#[cfg(feature = "transport")]
pub use secure_connection::{Established, SecureConnection};
#[cfg(feature = "server")]
pub use test_cert::generate_self_signed_cert;
