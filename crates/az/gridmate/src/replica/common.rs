//! Common types for the Replica system.
//!
//! Mirrors `GridMate/Replica/ReplicaCommon.h`.

/// Unique identifier for a Replica instance on the network.
/// `0` = invalid.
pub type ReplicaId = u32;

/// Unique identifier for a peer in the session.
/// `0` = invalid.
pub type PeerId = u32;

/// Revision stamp — increases every time a replicated field changes on the master.
pub type Revision = u64;

pub const INVALID_REPLICA_ID: ReplicaId = 0;
pub const INVALID_PEER_ID: PeerId = 0;

/// Max replicated fields per native replicated-state payload
/// (`GM_MAX_DATASETS_IN_CHUNK`).
pub const MAX_DATASETS_IN_CHUNK: usize = 32;

/// Max chunks per Replica (from `GM_MAX_CHUNKS_PER_REPLICA`).
pub const MAX_CHUNKS_PER_REPLICA: usize = 64;
