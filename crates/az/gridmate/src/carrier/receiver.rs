//! Carrier-level receive facade — compose
//! [`super::decoder::decode_datagram`] with
//! [`super::reassembler::Reassembler`] into one "feed a UDP datagram,
//! drain the messages it makes deliverable" handle.
//!
//! This is the canonical entry point for the receive pipeline. Two
//! kinds of caller use it:
//!
//! - Production peer state ([`super::connection_state::ConnectionState`])
//!   holds one `Receiver` privately and wraps it with dedup + ACK +
//!   retransmit + time-based logic. The decode steps inside
//!   `process_incoming_datagram` no longer have their own
//!   implementation — they go through `Receiver`.
//! - Capture, replay, and test tooling constructs a `Receiver` directly and feeds datagrams
//!   in capture order. No carrier lifecycle bookkeeping is required —
//!   the reassembler's reliable-ordering gate still enforces correct
//!   delivery order against a passive byte stream.
//!
//! `Receiver` returns reassembled `MessageData`; application protocols decide
//! how to interpret each body.

use bytes::Bytes;

use super::datagram::DatagramHeader;
use super::decoder::{DecodeError, DecodedDatagram, decode_datagram};
use super::reassembler::{Drain, Reassembler};

/// Stateful carrier receive handle: pure decode (`decode_datagram`)
/// plus per-channel reassembly ([`Reassembler`]). No lifecycle.
#[derive(Default)]
pub struct Receiver {
    reassembler: Reassembler,
}

impl Receiver {
    /// Fresh receiver. All channels start with no buffered frames
    /// and the "nothing delivered yet" reliable-seq sentinel.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the underlying reassembler. Tests and diagnostic
    /// tooling occasionally want to inspect the delivery cursor
    /// (`received_reliable_seq_num`) without going through
    /// [`Self::feed`].
    #[must_use]
    pub const fn reassembler(&self) -> &Reassembler {
        &self.reassembler
    }

    /// Feed one UDP datagram. Returns a [`FedDatagram`] borrow that:
    ///
    /// 1. Carries the parsed [`DatagramHeader`] for the caller's
    ///    lifecycle work (dedup, ACK history, time-since-last-recv).
    /// 2. Yields the messages the datagram (plus prior buffered
    ///    chunks) made deliverable. System-channel frames pass
    ///    through unaltered; application-channel frames go through
    ///    the reassembler and emerge in delivery order.
    ///
    /// The borrow is `&mut self` for the lifetime of `FedDatagram`,
    /// so callers must drain (`for msg in fed.messages() { … }`) or
    /// drop the value before feeding another datagram.
    ///
    /// # Errors
    ///
    /// Returns whatever [`decode_datagram`] returns for a malformed
    /// datagram: [`DecodeError::Header`] when the header is truncated or its
    /// marker bytes do not match the current wire format, or
    /// [`DecodeError::Decompress`] when a compressed payload fails LZ4.
    /// A datagram that decodes but delivers nothing is `Ok` with an
    /// empty [`FedDatagram::messages`]; the reassembler holds partial
    /// or out-of-order frames rather than erroring.
    pub fn feed(&mut self, data: Bytes) -> Result<FedDatagram<'_>, DecodeError> {
        let DecodedDatagram {
            header,
            payload,
            frames,
        } = decode_datagram(data)?;
        for frame in frames {
            // Every frame — system or application — flows through
            // the reassembler so the iterator below has a single
            // source of "what's deliverable now." Lifecycle reactions
            // (ACK frame processing, handshake state) happen one
            // layer up where the caller pattern-matches on
            // `channel == SYSTEM_CHANNEL` after `messages()` yields.
            self.reassembler.accept(frame);
        }
        Ok(FedDatagram {
            header,
            payload,
            receiver: self,
        })
    }
}

/// Borrow of the `Receiver` after feeding one datagram. Holds the
/// parsed [`DatagramHeader`] and offers an iterator over messages
/// that became deliverable as a result of the feed.
///
/// Drop or fully drain this before feeding the next datagram — the
/// underlying borrow is `&mut Receiver`, so the borrow checker
/// enforces the discipline at compile time.
pub struct FedDatagram<'a> {
    /// Carrier-level header (`seq_num`, `is_compressed` flag). Held
    /// here so the caller can run lifecycle work (dedup / ACK) even
    /// when no messages drained out of this feed.
    pub header: DatagramHeader,
    /// (Possibly decompressed) payload backing the frames the
    /// reassembler is staging. Kept alive in the result so the
    /// `MessageData::data` slices the reassembler holds remain valid
    /// until they're popped.
    pub payload: Bytes,
    receiver: &'a mut Receiver,
}

impl<'a> FedDatagram<'a> {
    /// Drain every message this feed (plus any backlog cleared by
    /// it) made deliverable. The iterator borrows from the inner
    /// reassembler; collect or step it before the `FedDatagram`
    /// drops.
    pub const fn messages(&mut self) -> Drain<'_> {
        self.receiver.reassembler.drain()
    }

    /// Consume the `FedDatagram`, returning the drain iterator. The
    /// iterator carries the reassembler borrow forward so callers
    /// can write `for (ch, msg) in fed.into_messages()` without an
    /// intermediate `let` binding.
    pub fn into_messages(self) -> Drain<'a> {
        self.receiver.reassembler.drain()
    }
}
