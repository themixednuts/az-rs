// GridMate Carrier module (mirrors GridMate/Carrier/)
// Contains SecureSocketDriver implementation

pub mod carrier_desc;
#[cfg(feature = "transport")]
mod carrier_impl;
#[cfg(feature = "transport")]
pub(crate) mod carrier_thread;
pub mod connection_security;
#[cfg(feature = "transport")]
pub mod connection_state;
pub mod datagram;
#[cfg(feature = "transport")]
pub(crate) mod datagram_history;
pub mod decoder;
#[cfg(feature = "transport")]
pub mod handshake_retry;
#[cfg(feature = "transport")]
pub mod io;
pub mod message;
pub mod mtu;
pub mod reassembler;
#[cfg(feature = "transport")]
pub mod receive_stream;
pub mod receiver;
pub mod state;
pub mod system_message;
#[cfg(feature = "transport")]
pub(crate) mod thread_message;
pub mod types;

// Re-export main types
pub use crate::state::{Connected, Connecting};
#[cfg(feature = "transport")]
pub use carrier_impl::{CarrierImpl, Disconnected, Disconnecting};
// Re-export with Carrier prefix for clarity in usage
pub use crate::state::{Connected as CarrierConnected, Connecting as CarrierConnecting};
pub use carrier_desc::{CarrierDesc, CarrierProtocolProfile};
#[cfg(feature = "transport")]
pub use carrier_impl::{
    Disconnected as CarrierDisconnected, Disconnecting as CarrierDisconnecting,
};
pub use datagram::{DataGramControlData, DatagramData, DatagramHeader};
pub use decoder::{DecodeError, DecodedDatagram, decode_datagram};
#[cfg(feature = "transport")]
pub use io::CarrierTransport;
#[cfg(feature = "server")]
pub use io::ChannelTransport;
#[cfg(feature = "client")]
pub use io::DtlsTransport;
pub use message::{DataPriority, DataReliability, MessageData, MessageFlags};
pub use reassembler::Reassembler;
pub use receiver::{FedDatagram, Receiver};
pub use state::StateMachine;

pub use types::{
    CarrierConnectionState, CarrierStats, DisconnectReason, FlowInformation, MAX_CHANNELS,
    PRIORITY_MAX, ReceiveResult, ReceiveState, SYSTEM_CHANNEL, SequenceNumber,
};
