//! Marshaling for `GridMate` Replica command messages on the system channel.
//!
//! Ported from Lumberyard's
//! `dev/Code/Framework/GridMate/GridMate/Replica/`:
//! - `ReplicaDefs.h` — `ReservedIds`, `ReplicaMarshalFlags`
//! - `Replica.cpp::Replica::Marshal` (line 541)
//! - `Tasks/ReplicaMarshalTasks.cpp::ReplicaMarshalTaskBase::MarshalNewReplica` (line 55)
//!
//! The output of [`CmdNewProxy::marshal`] is a stream of raw bytes
//! intended to be sent on the carrier's system channel (channel 0)
//! with reliability flagged.
//!
//! ## `ReplicaMgr` message wire format
//!
//! `ReplicaManager::_Unmarshal` (`ReplicaMgr.cpp:1005`) reads each
//! carrier message as:
//!
//! ```text
//! [u32 timestamp]              // per-message — set by GetTimeForNetworkTimestamp()
//! while bytes left:
//!   [u32 cmdId]                // ReservedId or repId
//!   [body — depends on cmdId]
//! ```
//!
//! Each `_Unmarshal` invocation drains the entire receive buffer; one
//! buffer may carry multiple cmds, but our outbound only sends one
//! `Cmd_NewProxy` per message so the body length matches `result.m_numBytes`
//! exactly.
//!
//! ## `Cmd_NewProxy` body (after timestamp + cmdId)
//!
//! Concretely (big-endian for raw integer writes; VLQ for VlqU32-marshaled
//! values):
//!
//! ```text
//! [u8  isSyncStage]
//! [u8  isMigratable]
//! [u32 createTime]
//! [u32 ownerSeq]
//! [u32 repId]
//! [PackedSize payloadLen]      // VLQ(bytes * 8)
//! [VLQ u64 chunkManifest]      // bitmask: 1 bit per chunk slot present
//! for each chunk in the manifest:
//!     [PackedSize chunkLen]    // VLQ(bytes * 8)
//!     [chunk payload bytes]    // = classId (u32) + ctorData + groupData
//! ```
//!
//! For a `Cmd_NewProxy` with `ReplicaMarshalFlags::INCLUDE_CTOR_DATA`,
//! each chunk's payload starts with `[u32 classId][MarshalCtorData()]`
//! followed by the chunk's native wire body.

use crate::serialize::{buffer::WriteBuffer, vlq::VlqU64Marshaler};

/// `ReservedIds` from `ReplicaDefs.h` (line 41-58).
///
/// IDs 0..=6 are command/control reservations; 7 is the `SessionInfo`
/// replica ID. Application replicas start at `Max_Reserved_Cmd_Or_Id`
/// (= 8). Compatibility peers may use larger application replica IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ReservedId {
    Invalid = 0,
    CmdGreetings = 1,
    /// `Cmd_NewProxy` — notify the peer that a new replica proxy
    /// should be instantiated on its side.
    CmdNewProxy = 2,
    CmdDestroyProxy = 3,
    CmdNewOwner = 4,
    CmdHeartbeat = 5,
    // 6 = Cmd_Count (placeholder, not a real cmd id)
    RepIdSessionInfo = 7,
}

/// Per-call marshal hints, mirrored from
/// `ReplicaDefs.h::ReplicaMarshalFlags::Type` (line 60-76).
///
/// We use plain `u32` bit constants rather than the [`bitflags`]
/// crate to keep the gridmate dep set small; the values are only
/// consumed by the marshal pipeline for documentation purposes
/// (the wire bytes don't include the flags — they steer marshal
/// behaviour, not wire shape).
#[allow(dead_code)]
pub mod marshal_flags {
    pub const INCLUDE_DATASETS: u32 = 1 << 0;
    pub const FORCE_DIRTY: u32 = 1 << 1;
    pub const AUTHORITATIVE: u32 = 1 << 2;
    pub const RELIABLE: u32 = 1 << 3;
    pub const INCLUDE_CTOR_DATA: u32 = 1 << 4;
    pub const OMIT_UNMODIFIED: u32 = 1 << 5;
    pub const FORCE_RELIABLE: u32 = 1 << 6;

    /// `Cmd_NewProxy` flag set per `ReplicaDefs.h:73`:
    /// `IncludeDatasets | OmitUnmodified | Authoritative | Reliable | ForceReliable`.
    pub const NEW_PROXY: u32 =
        INCLUDE_DATASETS | OMIT_UNMODIFIED | AUTHORITATIVE | RELIABLE | FORCE_RELIABLE;
}

/// A single chunk inside a [`CmdNewProxy`] payload.
///
/// `class_id` is the chunk class CRC32 (e.g. `0x06ac5c28` for
/// `NetBindingComponentChunk`). `ctor_data` is the per-class
/// `MarshalCtorData` output — for `NetBindingComponentChunk` this is
/// empty (chunks that override `MarshalCtorData` write into it). The
/// real chunk wire (replicated fields + `BaseState` + Collections) lives in
/// `body`, which the caller produces from the chunk's own marshaler.
#[derive(Debug, Clone, Default)]
pub struct ReplicaChunkWire {
    /// Chunk class id — `AZ::Crc32(class_name)`. The launcher's
    /// `sub_1463458b0` chunk descriptor factory keys on this to find
    /// the registered factory entry.
    pub class_id: u32,
    /// Optional ctor data emitted by the chunk descriptor's
    /// `MarshalCtorData`. Empty for chunks that don't override it
    /// (e.g. `NetBindingComponentChunk` — its replicated fields carry all
    /// spawn info).
    pub ctor_data: Vec<u8>,
    /// Chunk body — `chunk->Marshal(mc, iChunk)` output, i.e. the
    /// 3-group `[replicated fields][BaseState][Collections]` wire sequence.
    pub body: Vec<u8>,
}

impl ReplicaChunkWire {
    #[must_use]
    pub const fn new(class_id: u32) -> Self {
        Self {
            class_id,
            ctor_data: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Total length of this chunk's payload after the
    /// `[PackedSize chunkLen]` prefix (i.e. classId + ctorData + body).
    const fn payload_len(&self) -> usize {
        4 + self.ctor_data.len() + self.body.len()
    }

    fn marshal_into(&self, out: &mut Vec<u8>) {
        // Big-endian u32 — matches `WriteBuffer::Write(AZ::u32)` with
        // the carrier's default endian on Windows. We pin big-endian
        // here because the launcher's carrier is initialized with
        // `EndianType::BigEndian`.
        out.extend_from_slice(&self.class_id.to_be_bytes());
        out.extend_from_slice(&self.ctor_data);
        out.extend_from_slice(&self.body);
    }
}

/// Builder for one `Cmd_NewProxy` wire message.
///
/// Construct, fill the fields, append chunks, then call [`marshal`]
/// to produce the raw bytes for the system channel send.
///
/// [`marshal`]: CmdNewProxy::marshal
#[derive(Debug, Clone)]
pub struct CmdNewProxy {
    /// Per-message timestamp prefix. `ReplicaManager::_Unmarshal`
    /// reads this as `mc.m_timestamp` BEFORE the cmdId switch loop —
    /// it sits at the very start of every carrier message on the
    /// `ReplicaMgr` commChannel. Lumberyard fills this with
    /// `GetTimeForNetworkTimestamp()` (an `AZ::u32` tick counter).
    /// For us, a fresh wall-clock-ms-mod-2^32 value works since the
    /// launcher only uses this as a tie-breaker for stale-message
    /// detection.
    pub timestamp: u32,
    /// `m_isSyncStage` on the source replica — `false` for a fresh
    /// player binding emitted post-handshake.
    pub is_sync_stage: bool,
    /// `m_isMigratable` — `NetBindingComponentChunk` returns `true`
    /// from `IsReplicaMigratable()`, so for that chunk we set this
    /// to `true`. For chunks that don't migrate, `false` is fine.
    pub is_migratable: bool,
    /// `m_createTime` — replica creation timestamp in the
    /// `ReplicaManager`'s tick units. We seed this from the
    /// server's GridMate-local wall clock (millis since session
    /// start) since the launcher mainly uses it as a tie-breaker.
    pub create_time: u32,
    /// `m_replicaStatus->m_ownerSeq` — ownership-version counter.
    /// For new replicas we initialise this to 0; subsequent owner
    /// changes (`Cmd_NewOwner`) would bump it.
    pub owner_seq: u32,
    /// Replica id — the application-level id the client uses to
    /// route subsequent updates back to this replica instance. For
    /// `NetBindingComponentChunk` the dev's reference uses `0x110`.
    pub rep_id: u32,
    /// Per-chunk wire payloads in chunk-slot order. The chunk
    /// manifest bitmask is computed from this list's length (one bit
    /// per slot, low bit = slot 0).
    pub chunks: Vec<ReplicaChunkWire>,
}

impl CmdNewProxy {
    /// Build a `Cmd_NewProxy` for a single-chunk replica (the common
    /// shape for `NetBindingComponentChunk` etc). Caller is expected
    /// to set `timestamp` before [`marshal`]ing.
    ///
    /// [`marshal`]: CmdNewProxy::marshal
    #[must_use]
    pub fn single_chunk(rep_id: u32, chunk: ReplicaChunkWire) -> Self {
        Self {
            timestamp: 0,
            is_sync_stage: false,
            is_migratable: true,
            create_time: 0,
            owner_seq: 0,
            rep_id,
            chunks: vec![chunk],
        }
    }

    /// Marshal the full `Cmd_NewProxy` into `wb`. Output is a
    /// self-contained byte stream ready for one carrier `send` on
    /// the `ReplicaMgr` commChannel.
    pub fn marshal(&self, wb: &mut WriteBuffer) {
        // ── Per-message prefix: m_timestamp (ReplicaMgr.cpp:1021) ──
        // Read first by `ReplicaManager::_Unmarshal` BEFORE the cmdId
        // switch loop. Without this prefix the launcher consumes our
        // cmdId bytes as a timestamp and then sees `Invalid_Cmd_Or_Id`,
        // silently dropping the message.
        wb.write_u32(self.timestamp);

        // ── Header (ReplicaMarshalTaskBase::MarshalNewReplica) ──
        wb.write_u32(ReservedId::CmdNewProxy as u32);
        wb.write_u8(u8::from(self.is_sync_stage));
        wb.write_u8(u8::from(self.is_migratable));
        wb.write_u32(self.create_time);
        wb.write_u32(self.owner_seq);

        // ── Replica::Marshal body ──
        // [u32 repId][PackedSize payloadLen][VLQ chunkManifest][per-chunk: PackedSize chunkLen + payload]
        wb.write_u32(self.rep_id);

        // Build the payload (manifest + chunk frames) into a side
        // buffer so we can compute the total `payloadLen` as a
        // single VLQ-encoded `PackedSize` value before writing.
        let manifest: u64 = if self.chunks.is_empty() {
            0
        } else if self.chunks.len() >= 64 {
            // Lumberyard's `GM_MAX_CHUNKS_PER_REPLICA` is 64; the
            // bitset's `to_ullong()` lossy-converts above that.
            u64::MAX
        } else {
            (1u64 << self.chunks.len()) - 1
        };

        let mut payload = Vec::new();
        // VLQ-u64 manifest. We borrow our own `WriteBuffer` for the
        // VLQ encoding so we get the same byte layout as everywhere
        // else; pop the bytes out and concatenate.
        let manifest_bytes = vlq_u64_to_bytes(manifest, wb.endian());
        payload.extend_from_slice(&manifest_bytes);

        for chunk in &self.chunks {
            let chunk_payload_len = chunk.payload_len();
            let chunk_len_bytes = packed_size_to_bytes(chunk_payload_len, wb.endian());
            payload.extend_from_slice(&chunk_len_bytes);
            chunk.marshal_into(&mut payload);
        }

        let payload_size_bytes = packed_size_to_bytes(payload.len(), wb.endian());
        wb.write_bytes(&payload_size_bytes);
        wb.write_bytes(&payload);
    }
}

/// Builder for one `Cmd_Greetings` wire message — the first message
/// every peer sends on the `ReplicaMgr` commChannel.
///
/// Ported from `ReplicaManager::SendGreetings` (`ReplicaMgr.cpp:1421`):
///
/// ```cpp
/// peer->GetReliableOutBuffer().Write(Cmd_Greetings);     // u32
/// peer->GetReliableOutBuffer().Write(GetLocalPeerId());  // u32
/// peer->GetReliableOutBuffer().Write(IsSyncHost());      // bool
/// if (IsSyncHost()) {
///     peer->GetReliableOutBuffer().Write(ReserveIdBlock(...));  // u32 RepIdSeed
/// }
/// peer->SendBuffer(carrier, m_commChannel, GetTimeForNetworkTimestamp());
/// ```
///
/// `SendBuffer` writes a per-message `[u32 timestamp]` prefix
/// (Lumberyard `Carrier::SendBuffer`), so the on-wire bytes are:
///
/// ```text
/// [u32 timestamp]
/// [u32 cmdId = 1]                  // Cmd_Greetings
/// [u32 localPeerId]
/// [bool isSyncHost]
/// if isSyncHost:
///     [u32 firstSeed]              // RepIdSeed for the receiver's local block
/// ```
#[derive(Debug, Clone, Copy)]
pub struct CmdGreetings {
    /// Per-message `ReplicaMgr` timestamp — same role as in
    /// [`CmdNewProxy::timestamp`].
    pub timestamp: u32,
    /// Our local peer id. Lumberyard uses `AZ::Crc32` of the local
    /// session id but the launcher only checks it for uniqueness
    /// across remote peers, so any stable non-zero u32 works for our
    /// single-launcher case.
    pub local_peer_id: u32,
    /// `true` when we are the replication host (always true for our
    /// server). When set, the launcher reads `first_seed` and starts
    /// its local replica-id allocator from there.
    pub is_sync_host: bool,
    /// `RepIdSeed` for the receiver's `m_localIdBlocks.AddBlock`.
    /// Per `ReplicaMgr.cpp:470::ReserveIdBlock`, the host allocates
    /// `Max_Reserved_Cmd_Or_Id` (= 8) for itself and hands each
    /// remote peer the next `GM_REPIDS_PER_BLOCK` (= 1<<25) block.
    /// For the first non-host peer that is `8 + (1 << 25) = 0x02000008`.
    /// Only marshaled when `is_sync_host == true`.
    pub first_seed: u32,
}

impl CmdGreetings {
    /// `Max_Reserved_Cmd_Or_Id + GM_REPIDS_PER_BLOCK` — the
    /// first non-host peer's id-block start. Per
    /// `ReplicaCommon.h:32` (`GM_REPIDS_PER_BLOCK = 1<<25`) and
    /// `ReplicaDefs.h::Max_Reserved_Cmd_Or_Id = 8`.
    pub const FIRST_PEER_REP_ID_SEED: u32 = 8 + (1u32 << 25);

    #[must_use]
    pub const fn host(timestamp: u32, local_peer_id: u32) -> Self {
        Self {
            timestamp,
            local_peer_id,
            is_sync_host: true,
            first_seed: Self::FIRST_PEER_REP_ID_SEED,
        }
    }

    pub fn marshal(&self, wb: &mut WriteBuffer) {
        wb.write_u32(self.timestamp);
        wb.write_u32(ReservedId::CmdGreetings as u32);
        wb.write_u32(self.local_peer_id);
        wb.write_u8(u8::from(self.is_sync_host));
        if self.is_sync_host {
            wb.write_u32(self.first_seed);
        }
    }
}

/// Receive-side view of a single `ReplicaMgr` cmd parsed from the
/// inbound buffer.
///
/// Currently we only model the cmds we plan to handle (Greetings); the
/// rest land in [`ReplicaCmdInbound::Other`] so the unrecognized-cmd
/// path is observable rather than a parse failure.
#[derive(Debug, Clone)]
pub enum ReplicaCmdInbound {
    /// `Cmd_Greetings` from the peer. Contains the launcher's
    /// `peerId` and whether the peer claims `IsSyncHost`.
    Greetings {
        peer_id: u32,
        is_sync_host: bool,
        first_seed: Option<u32>,
    },
    /// Any cmd we don't decode yet — carries the raw cmd id so the
    /// dispatcher can log it for visibility.
    Other { cmd_id: u32 },
}

/// Parse a full `ReplicaMgr` message off the wire.
///
/// Wire shape:
///
/// ```text
/// [u32 timestamp]
/// repeated:
///   [u32 cmdId]
///   [body — variant-specific]
/// ```
///
/// Returns the leading `(timestamp, Vec<ReplicaCmdInbound>)`.
/// Returns `None` if the buffer is malformed (truncated) — caller
/// can log and drop.
#[must_use]
pub fn parse_replica_mgr_message(
    bytes: &[u8],
    endian: crate::serialize::buffer::Endian,
) -> Option<(u32, Vec<ReplicaCmdInbound>)> {
    use crate::serialize::buffer::ReadBuffer;
    let mut rb = ReadBuffer::new(endian, bytes);
    let timestamp = rb.read_u32().ok()?;
    let mut cmds = Vec::new();
    while rb.left() > 0 {
        // Each cmd header is a raw u32 cmdId.
        let cmd_id = rb.read_u32().ok()?;
        match cmd_id {
            // Cmd_Greetings
            1 => {
                let peer_id = rb.read_u32().ok()?;
                let is_sync_host = rb.read_u8().ok()? != 0;
                let first_seed = if is_sync_host {
                    rb.read_u32().ok()
                } else {
                    None
                };
                cmds.push(ReplicaCmdInbound::Greetings {
                    peer_id,
                    is_sync_host,
                    first_seed,
                });
            }
            // For other cmds we can't determine the body length
            // without per-cmd parsers. Record the cmd id and stop
            // (the bytes after may be a malformed continuation).
            other => {
                cmds.push(ReplicaCmdInbound::Other { cmd_id: other });
                break;
            }
        }
    }
    Some((timestamp, cmds))
}

/// Encode a `PackedSize` value (Lumberyard: `bytes * 8 + bits`) as
/// `VlqU32`. Thin wrapper around [`super::super::serialize::compression_marshal::PackedSize`]'s
/// `Marshaler` impl, which mirrors `Marshaler<PackedSize>::Marshal` in
/// `CompressionMarshal.h:617`:
///
/// ```cpp
/// wb.Write(static_cast<AZ::u32>(value.GetTotalSizeInBits()), VlqU32Marshaler());
/// ```
fn packed_size_to_bytes(byte_count: usize, endian: crate::serialize::buffer::Endian) -> Vec<u8> {
    use crate::serialize::{compression_marshal::PackedSize, marshaler::Marshaler};
    let bytes = u32::try_from(byte_count)
        .expect("PackedSize byte count must fit in u32 / 8 — saturate to u32::MAX/8 otherwise");
    let mut tmp = WriteBuffer::new(endian);
    PackedSize::from_bytes(bytes).marshal(&mut tmp);
    tmp.into_vec()
}

/// Encode a `u64` as `VlqU64` into a fresh byte vector — used for
/// the chunk manifest in [`CmdNewProxy::marshal`].
fn vlq_u64_to_bytes(value: u64, endian: crate::serialize::buffer::Endian) -> Vec<u8> {
    let mut tmp = WriteBuffer::new(endian);
    VlqU64Marshaler.marshal(&mut tmp, value);
    tmp.into_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::buffer::CARRIER_ENDIAN;

    #[test]
    fn reserved_id_values_match_lumberyard() {
        // `ReplicaDefs.h:41` — verify the wire values match the
        // launcher's expectations.
        assert_eq!(ReservedId::Invalid as u32, 0);
        assert_eq!(ReservedId::CmdGreetings as u32, 1);
        assert_eq!(ReservedId::CmdNewProxy as u32, 2);
        assert_eq!(ReservedId::CmdDestroyProxy as u32, 3);
        assert_eq!(ReservedId::CmdNewOwner as u32, 4);
        assert_eq!(ReservedId::CmdHeartbeat as u32, 5);
        assert_eq!(ReservedId::RepIdSessionInfo as u32, 7);
    }

    #[test]
    fn new_proxy_flags_match_lumberyard() {
        // `ReplicaDefs.h:73`
        use marshal_flags::*;
        assert_eq!(
            NEW_PROXY,
            INCLUDE_DATASETS | OMIT_UNMODIFIED | AUTHORITATIVE | RELIABLE | FORCE_RELIABLE
        );
        // Sanity check: not equal to FullSync (= IncludeDatasets |
        // ForceDirty | ... ) because OMIT_UNMODIFIED differs.
        assert_ne!(
            NEW_PROXY,
            INCLUDE_DATASETS | FORCE_DIRTY | AUTHORITATIVE | RELIABLE | FORCE_RELIABLE
        );
    }

    #[test]
    fn packed_size_for_empty_chunk_is_zero_vlq() {
        let bytes = packed_size_to_bytes(0, CARRIER_ENDIAN);
        // VLQ(0 bits) = single byte 0x00.
        assert_eq!(bytes, vec![0x00]);
    }

    #[test]
    fn packed_size_for_38_byte_chunk() {
        // 38 bytes * 8 = 304 bits. 304 < 16384, so VLQ-u32 produces
        // 2 bytes: 0x80 | (304 & 0x3f) = 0xb0, then (304 >> 6) = 4.
        let bytes = packed_size_to_bytes(38, CARRIER_ENDIAN);
        assert_eq!(bytes, vec![0xb0, 0x04]);
    }

    #[test]
    fn cmd_greetings_host_round_trip() {
        // Outbound Cmd_Greetings from a host server. Wire shape per
        // `ReplicaMgr.cpp:1421::SendGreetings`:
        //   [u32 timestamp][u32 cmdId=1][u32 peerId][bool=1][u32 firstSeed]
        // = 4 + 4 + 4 + 1 + 4 = 17 bytes.
        let greet = CmdGreetings::host(0xDEAD_BEEF, 0xCAFE_BABE);
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        greet.marshal(&mut wb);
        let bytes = wb.into_vec();
        assert_eq!(bytes.len(), 17);
        assert_eq!(
            &bytes,
            &[
                0xDE, 0xAD, 0xBE, 0xEF, // timestamp
                0x00, 0x00, 0x00, 0x01, // cmdId = Cmd_Greetings
                0xCA, 0xFE, 0xBA, 0xBE, // localPeerId
                0x01, // isSyncHost = true
                0x02, 0x00, 0x00, 0x08, // firstSeed = 0x02000008
            ]
        );
    }

    #[test]
    fn cmd_greetings_parser_round_trip_inbound_launcher_greetings() {
        // Inbound Cmd_Greetings from the launcher (non-host).
        // The launcher's outBuffer.Write sequence per
        // `ReplicaMgr.cpp:1421` skips the firstSeed write since
        // `IsSyncHost()` is false on the launcher side. Wire:
        //   [u32 timestamp][u32 cmdId=1][u32 peerId][bool=0]
        // = 4 + 4 + 4 + 1 = 13 bytes.
        let bytes = [
            0x11, 0x22, 0x33, 0x44, // timestamp
            0x00, 0x00, 0x00, 0x01, // cmdId = Cmd_Greetings
            0x55, 0x66, 0x77, 0x88, // remote peer id
            0x00, // isSyncHost = false
        ];
        let (timestamp, cmds) = parse_replica_mgr_message(&bytes, CARRIER_ENDIAN).expect("parses");
        assert_eq!(timestamp, 0x1122_3344);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            ReplicaCmdInbound::Greetings {
                peer_id,
                is_sync_host,
                first_seed,
            } => {
                assert_eq!(*peer_id, 0x5566_7788);
                assert!(!*is_sync_host);
                assert!(first_seed.is_none());
            }
            other @ ReplicaCmdInbound::Other { .. } => {
                panic!("expected Greetings, got {other:?}")
            }
        }
    }

    #[test]
    fn cmd_greetings_parser_handles_unknown_cmd() {
        // Unknown cmd id is preserved verbatim so the dispatcher
        // can log it; parsing stops there since we can't compute
        // the body length.
        let bytes = [
            0x00, 0x00, 0x00, 0x05, // timestamp
            0x00, 0x00, 0x00, 0x05, // cmdId = 5 (Cmd_Heartbeat — not yet handled)
            0xff, 0xff, // trailing bytes (won't be parsed)
        ];
        let (_, cmds) = parse_replica_mgr_message(&bytes, CARRIER_ENDIAN).expect("parses");
        assert_eq!(cmds.len(), 1);
        match cmds[0] {
            ReplicaCmdInbound::Other { cmd_id } => assert_eq!(cmd_id, 5),
            ref other @ ReplicaCmdInbound::Greetings { .. } => {
                panic!("expected Other, got {other:?}")
            }
        }
    }

    #[test]
    fn single_chunk_new_proxy_header() {
        // Construct a Cmd_NewProxy with one empty chunk and verify
        // the header bytes match the expected wire shape.
        let cmd = CmdNewProxy {
            timestamp: 0x1234_5678,
            is_sync_stage: false,
            is_migratable: true,
            create_time: 0,
            owner_seq: 0,
            rep_id: 0x0110,
            chunks: vec![ReplicaChunkWire::new(0x06ac_5c28)],
        };
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        cmd.marshal(&mut wb);
        let bytes = wb.into_vec();

        // Header (Big-endian, since CARRIER_ENDIAN == BigEndian):
        // 12 34 56 78      timestamp (per-message ReplicaMgr prefix)
        // 00 00 00 02      cmdId = Cmd_NewProxy
        // 00               is_sync_stage = false
        // 01               is_migratable = true
        // 00 00 00 00      create_time
        // 00 00 00 00      owner_seq
        // 00 00 01 10      rep_id = 0x110
        assert_eq!(
            &bytes[..22],
            &[
                0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x10,
            ]
        );

        // Payload starts at byte 22. With one empty chunk (only the
        // 4-byte classId, no ctor or body) the layout is:
        //
        // [PackedSize payloadLen] [VLQ manifest] [PackedSize chunkLen] [u32 classId]
        //
        // - manifest = 0b1 (one chunk) → VLQ-u64 = 0x01 (1 byte)
        // - chunkLen = 4 bytes = 32 bits → VLQ-u32(32) = 0x20 (1 byte, since 32 < 0x80)
        // - classId = 0x06ac5c28 → 4 bytes BE
        //
        // Total inner payload = 1 (manifest) + 1 (chunkLen) + 4 (classId) = 6 bytes
        // payloadLen = 6 bytes = 48 bits → VLQ-u32(48) = 0x30 (1 byte).
        assert_eq!(
            &bytes[22..],
            &[
                0x30, // payloadLen = 48 bits (= 6 bytes)
                0x01, // manifest = 1
                0x20, // chunkLen = 32 bits (= 4 bytes)
                0x06, 0xac, 0x5c, 0x28, // classId
            ]
        );
    }
}
