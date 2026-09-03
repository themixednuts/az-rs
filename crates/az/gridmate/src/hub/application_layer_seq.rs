//! `Amazon::ApplicationLayerSeqAck`.
//!
//! This is the application-level acknowledgement carried by `ActionList` input.
//! It is owned by ContainerShared/GridMate, not by the game-specific message
//! that happens to transport it.

use arrayvec::ArrayVec;

use super::SequenceNumber;
use crate::serialize::{Marshaler, MarshalerError, ReadBuffer, VlqU64, WriteBuffer};

/// Native `k_MaxAppSeqValues`.
pub const MAX_APPLICATION_LAYER_SEQ_ACK_ENTRIES: usize = 16;

/// One native 0x18-byte entry from the bounded acknowledgement collection.
///
/// Native runtime validation proves the two serialized values and their wire
/// order, but does not name their semantics. They stay private so
/// protocol consumers cannot build behavior around guessed field names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ApplicationLayerSeqAckEntry {
    value_at_0x10: u64,
    value_at_0x08: u64,
}

impl Marshaler for ApplicationLayerSeqAckEntry {
    fn marshal(&self, wb: &mut WriteBuffer) {
        VlqU64::new(self.value_at_0x10).marshal(wb);
        VlqU64::new(self.value_at_0x08).marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self {
            value_at_0x10: rb.field("value_at_0x10", |rb| VlqU64::unmarshal(rb))?.get(),
            value_at_0x08: rb.field("value_at_0x08", |rb| VlqU64::unmarshal(rb))?.get(),
        })
    }
}

/// Application-level sequence acknowledgement embedded in client input.
///
/// The trailing pair is a newest sequence plus a 64-bit preceding-sequence
/// mask. Bit zero acknowledges `newest - 1`, bit 63 acknowledges
/// `newest - 64`. This is distinct from [`SequenceNumber`]'s ordinary
/// optional wire representation: all values here are raw VLQ-u64 values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationLayerSeqAck {
    entries: ArrayVec<ApplicationLayerSeqAckEntry, MAX_APPLICATION_LAYER_SEQ_ACK_ENTRIES>,
    newest_sequence: SequenceNumber,
    preceding_mask: u64,
}

impl Default for ApplicationLayerSeqAck {
    fn default() -> Self {
        Self {
            entries: ArrayVec::new(),
            newest_sequence: SequenceNumber::Invalid,
            preceding_mask: 0,
        }
    }
}

impl ApplicationLayerSeqAck {
    #[must_use]
    pub fn new(newest_sequence: SequenceNumber, preceding_mask: u64) -> Self {
        Self {
            entries: ArrayVec::new(),
            newest_sequence,
            preceding_mask,
        }
    }

    /// A valid acknowledgement object that does not identify a real sequence.
    /// Input may use this shape before it has a concrete sequence.
    #[must_use]
    pub fn valid_non_sequence() -> Self {
        Self::new(SequenceNumber::ValidNonSequence, 0)
    }

    #[must_use]
    pub const fn newest_sequence(&self) -> SequenceNumber {
        self.newest_sequence
    }

    #[must_use]
    pub const fn preceding_mask(&self) -> u64 {
        self.preceding_mask
    }

    #[must_use]
    pub const fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Whether the trailing acknowledgement window covers `sequence`.
    #[must_use]
    pub const fn acknowledges(&self, sequence: SequenceNumber) -> bool {
        let (Some(newest), Some(sequence)) = (self.newest_sequence.as_seq(), sequence.as_seq())
        else {
            return false;
        };
        let Some(distance) = newest.checked_sub(sequence) else {
            return false;
        };
        match distance {
            0 => true,
            1..=64 => self.preceding_mask & (1_u64 << (distance - 1)) != 0,
            _ => false,
        }
    }
}

impl Marshaler for ApplicationLayerSeqAck {
    fn marshal(&self, wb: &mut WriteBuffer) {
        self.entries.marshal(wb);
        VlqU64::new(self.newest_sequence.wire_value()).marshal(wb);
        VlqU64::new(self.preceding_mask).marshal(wb);
    }

    fn unmarshal(rb: &mut ReadBuffer) -> Result<Self, MarshalerError> {
        Ok(Self {
            entries: rb.field("entries", ArrayVec::unmarshal)?,
            newest_sequence: SequenceNumber::from(
                rb.field("newest_sequence", |rb| VlqU64::unmarshal(rb))?,
            ),
            preceding_mask: rb
                .field("preceding_mask", |rb| VlqU64::unmarshal(rb))?
                .get(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::buffer::CARRIER_ENDIAN;

    #[test]
    fn empty_valid_ack_matches_initial_input_shape() {
        let ack = ApplicationLayerSeqAck::valid_non_sequence();
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        ack.marshal(&mut wb);
        assert_eq!(wb.as_slice(), &[0, 0, 0]);

        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, wb.as_slice());
        assert_eq!(ApplicationLayerSeqAck::unmarshal(&mut rb).unwrap(), ack);
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn entry_wire_order_and_ack_window_are_exact() {
        let mut entries = ArrayVec::new();
        entries.push(ApplicationLayerSeqAckEntry {
            value_at_0x10: 0x81,
            value_at_0x08: 7,
        });
        let ack = ApplicationLayerSeqAck {
            entries,
            newest_sequence: SequenceNumber::Seq(70),
            preceding_mask: (1 << 0) | (1 << 63),
        };

        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        ack.marshal(&mut wb);
        assert_eq!(&wb.as_slice()[..4], &[1, 0x81, 0x02, 7]);
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, wb.as_slice());
        assert_eq!(ApplicationLayerSeqAck::unmarshal(&mut rb).unwrap(), ack);
        assert_eq!(rb.left(), 0);
        assert!(ack.acknowledges(SequenceNumber::Seq(70)));
        assert!(ack.acknowledges(SequenceNumber::Seq(69)));
        assert!(ack.acknowledges(SequenceNumber::Seq(6)));
        assert!(!ack.acknowledges(SequenceNumber::Seq(68)));
        assert!(!ack.acknowledges(SequenceNumber::Seq(5)));
    }
}
