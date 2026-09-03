//! `GridMate` replica manager command support.
//!
//! Replicated-state payloads are modeled through
//! [`crate::hub::Fragment`], [`crate::hub::ReplicatedState`], and
//! [`crate::hub::FixedReplicatedState`]. This module retains the replica
//! manager command envelopes that still exist below hub `StateBundle` traffic.
//!
pub mod cmd_marshal;
pub mod common;
pub mod preserved_base_state;

pub use cmd_marshal::{
    CmdGreetings, CmdNewProxy, ReplicaChunkWire, ReplicaCmdInbound, ReservedId, marshal_flags,
    parse_replica_mgr_message,
};
pub use common::*;
pub use preserved_base_state::PreservedBaseState;
