// DatagramData - Group of MessageData
// Following GridMate Carrier.cpp lines 113-128

use super::message::MessageData;
use super::types::{PRIORITY_MAX, SequenceNumber};
use crate::serialize::{ReadBuffer, error::MarshalerError};

/// Parsed `GridMate` datagram header.
///
/// The current wire shape is a compression marker, the required compressor byte,
/// and a big-endian sequence number.
#[derive(Debug, Clone, Copy)]
pub struct DatagramHeader {
    pub is_compressed: bool,
    pub sequence_number: SequenceNumber,
}

impl DatagramHeader {
    pub const SIZE: usize = 4;
    pub const UNCOMPRESSED: u8 = 0x80;
    pub const COMPRESSED: u8 = 0x81;
    pub const HAS_COMPRESSOR: u8 = 1;

    /// Stream-read the `GridMate` datagram header from `rb`. Advances
    /// `rb` by exactly [`DatagramHeader::SIZE`] bytes on success;
    /// `rb.remaining()` after the call is the datagram payload.
    ///
    /// # Errors
    ///
    /// Returns [`MarshalerError::InvalidDiscriminant`] unless the marker is
    /// `0x80` or `0x81` and the compressor byte is exactly one. A truncated
    /// header returns [`MarshalerError::BufferUnderrun`].
    pub fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        let is_compressed = match rb.read_u8()? {
            Self::UNCOMPRESSED => false,
            Self::COMPRESSED => true,
            value => return Err(MarshalerError::InvalidDiscriminant { value }),
        };
        match rb.read_u8()? {
            Self::HAS_COMPRESSOR => {}
            value => return Err(MarshalerError::InvalidDiscriminant { value }),
        }
        let sequence_number = SequenceNumber::from(rb.read_u16()?);

        Ok(Self {
            is_compressed,
            sequence_number,
        })
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;
    use crate::serialize::buffer::CARRIER_ENDIAN;

    #[test]
    fn current_header_preserves_marker_and_sequence() {
        let mut rb = ReadBuffer::new(
            CARRIER_ENDIAN,
            &[
                DatagramHeader::COMPRESSED,
                DatagramHeader::HAS_COMPRESSOR,
                0x12,
                0x34,
            ],
        );
        let header = DatagramHeader::unmarshal(&mut rb).unwrap();

        assert!(header.is_compressed);
        assert_eq!(header.sequence_number.get(), 0x1234);
        assert!(rb.is_empty());
    }

    #[test]
    fn current_header_requires_the_compressor_marker() {
        let mut rb = ReadBuffer::new(
            CARRIER_ENDIAN,
            &[DatagramHeader::UNCOMPRESSED, 0, 0x12, 0x34],
        );

        assert!(matches!(
            DatagramHeader::unmarshal(&mut rb),
            Err(MarshalerError::InvalidDiscriminant { value: 0 })
        ));
    }

    #[test]
    fn current_header_rejects_old_markers() {
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &[1, 1, 0x12, 0x34]);
        assert!(matches!(
            DatagramHeader::unmarshal(&mut rb),
            Err(MarshalerError::InvalidDiscriminant { value: 1 })
        ));
    }
}

/// Flow control data for datagram (`GridMate`: `TrafficControl::DataGramControlData`)
#[derive(Debug, Clone)]
pub struct DataGramControlData {
    /// Datagram sequence number
    pub sequence_number: SequenceNumber,

    /// Total size of datagram
    pub size: u16,

    /// Effective size (payload without system messages)
    pub effective_size: u16,

    /// Timestamp when sent
    pub sent_time: std::time::Instant,
}

impl DataGramControlData {
    #[must_use]
    pub fn new(sequence_number: SequenceNumber) -> Self {
        Self {
            sequence_number,
            size: 0,
            effective_size: 0,
            sent_time: std::time::Instant::now(),
        }
    }
}

/// Carrier datagram (`GridMate`: `DatagramData` struct)
/// A group of `MessageData`
pub struct DatagramData {
    /// Flow control data
    pub flow_control: DataGramControlData,

    /// Size of data in toResend list (not including headers)
    pub resend_data_size: u16,

    /// Lists of reliable messages that were part of datagram (by priority)
    /// May need to resend them
    pub to_resend: [Vec<MessageData>; PRIORITY_MAX],

    /// ACK callbacks for this datagram
    pub ack_callbacks: Vec<Box<dyn FnOnce() + Send>>,
}

impl DatagramData {
    /// Create new datagram (`GridMate` pattern)
    #[must_use]
    pub fn new(sequence_number: SequenceNumber) -> Self {
        Self {
            flow_control: DataGramControlData::new(sequence_number),
            resend_data_size: 0,
            // One resend queue per priority — sized by `PRIORITY_MAX`
            // so renumbering `DataPriority` doesn't quietly truncate.
            to_resend: std::array::from_fn(|_| Vec::new()),
            ack_callbacks: Vec::new(),
        }
    }
}

/// Datagram history list constants (`GridMate`: `DataGramHistoryList`)
pub mod history {
    use super::SequenceNumber;

    /// Maximum number of ACKs before removing from history
    pub const MAX_NUM_ACKS: u8 = 3;

    /// Maximum number of bytes in datagram history
    pub const MAX_NUM_BYTES: u8 = 64;

    /// Datagram history ID size (512 = 64 * 8 bits)
    pub const HISTORY_SIZE: usize = (MAX_NUM_BYTES as usize) * 8;

    /// History element (`GridMate`: `DataGramHistoryList::Element`)
    #[derive(Debug, Clone, Copy)]
    pub struct HistoryElement {
        pub sequence_number: SequenceNumber,
        /// -1 if slot not used, otherwise count of ACKs sent
        pub num_acks_sent: i32,
    }

    impl Default for HistoryElement {
        fn default() -> Self {
            Self {
                sequence_number: SequenceNumber::ZERO,
                num_acks_sent: -1,
            }
        }
    }
}
