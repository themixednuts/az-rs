//! MTU + chunk-size arithmetic.
//!
//! The carrier splits oversized messages into chunks constrained by the UDP
//! packet limit after IP, UDP, DTLS, datagram, and frame headers are reserved.
//! The arithmetic follows Lumberyard `GridMate`'s
//! `dev/Code/Framework/GridMate/GridMate/Carrier/Carrier.cpp`.

use super::datagram::DatagramHeader;

/// Lumberyard `GridMate`'s default internet packet limit.
pub const MAX_UDP_PACKET_SIZE: usize = 1400;

/// IPv4 and UDP header bytes.
pub const UDP_IP_OVERHEAD: usize = 28;

/// DTLS record header size (OpenSSL: `DTLS1_RT_HEADER_LENGTH = 13`).
pub const DTLS_HEADER_SIZE: usize = 13;

/// DTLS cipher overhead for AES-GCM.
pub const DTLS_CIPHER_OVERHEAD: usize = 30;

/// Maximum datagram size after transport overhead.
///
pub const MAX_DATAGRAM_SIZE: usize =
    MAX_UDP_PACKET_SIZE - UDP_IP_OVERHEAD - DTLS_HEADER_SIZE - DTLS_CIPHER_OVERHEAD;

/// Optional carrier frame header fields.
pub mod header_sizes {
    pub const FLAGS: usize = 1; // u8
    pub const DATA_SIZE: usize = 2; // u16
    pub const CHANNEL_INFO: usize = 1; // u8
    pub const SPLIT_PACKET_INFO: usize = 2; // SequenceNumber (u16)
    pub const SEQUENCE_NUMBER: usize = 2; // SequenceNumber (u16)
    pub const SEQUENCE_RELIABLE_NUMBER: usize = 2; // SequenceNumber (u16)
}

/// Maximum message header size (all optional fields present).
///
pub const MAX_MESSAGE_HEADER_SIZE: usize = header_sizes::FLAGS
    + header_sizes::DATA_SIZE
    + header_sizes::CHANNEL_INFO
    + header_sizes::SPLIT_PACKET_INFO
    + header_sizes::SEQUENCE_NUMBER
    + header_sizes::SEQUENCE_RELIABLE_NUMBER;

/// Maximum message data size per chunk.
///
pub const MAX_MESSAGE_DATA_SIZE: usize =
    MAX_DATAGRAM_SIZE - DatagramHeader::SIZE - MAX_MESSAGE_HEADER_SIZE;

/// Half of sequence number space (used for chunk limit).
///
pub const SEQUENCE_NUMBER_HALF_SPAN: usize = 32768;

/// Maximum number of chunks before sequence ordering becomes ambiguous.
pub const MAX_NUM_CHUNKS: usize = SEQUENCE_NUMBER_HALF_SPAN - 1;

/// Calculate number of chunks needed for a message.
///
#[inline]
#[must_use]
pub const fn chunks_needed(data_size: usize) -> usize {
    if data_size <= MAX_MESSAGE_DATA_SIZE {
        1
    } else {
        1 + ((data_size - 1) / MAX_MESSAGE_DATA_SIZE)
    }
}
