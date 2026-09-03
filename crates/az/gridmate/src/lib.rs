//! `GridMate` transport, replication, and AZ network reflection primitives.
//!
//! This crate is the framework-level Rust port of Lumberyard/O3DE `GridMate`
//! concepts. Optional native registry snapshots are selected explicitly for
//! debug validation rather than becoming runtime protocol dependencies.

#![allow(
    clippy::upper_case_acronyms,
    async_fn_in_trait  // Intentional: using Rust 2024 edition async fn in traits
)]

extern crate self as gridmate;

use std::io;

// Re-export derive macros
pub use gridmate_derive::{
    ChunkMarshaler, ClassDesc, FixedReplicatedStateFields, Marshaler, Message, NetworkBase,
    ReplicatedState,
};

pub mod az;
#[cfg(all(debug_assertions, feature = "transport"))]
pub mod capture;
pub mod carrier;
pub mod driver;
pub mod ebus;
pub mod hub;
pub mod message;
pub mod pervasives;
mod protocol_time;
pub mod registration;
pub mod replica;
pub mod serialize;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "transport")]
pub mod session;
#[cfg(feature = "transport")]
pub mod session_service;
#[cfg(not(feature = "transport"))]
pub mod session_service {
    //! Transport-independent session identifiers and event values.

    pub use crate::session_types::{Event, SessionId};
}
pub mod session_types;
pub mod spawn;
pub mod state;

// Re-export public API for applications
pub use crate::carrier::StateMachine;
#[cfg(feature = "transport")]
pub use carrier::CarrierImpl;
pub use carrier::{
    CarrierConnectionState, CarrierDesc, CarrierProtocolProfile, CarrierStats, DataPriority,
    DataReliability, DisconnectReason, FlowInformation, ReceiveResult, ReceiveState,
    SYSTEM_CHANNEL,
};
pub use driver::DriverError;
#[cfg(feature = "transport")]
pub use driver::SecureConnection;
pub use hub::{
    FieldGroup, FieldGroupMut, FieldVector, FieldVectorMut, FilterValue, FixedReplicatedState,
    FixedReplicatedStateFields, FixedStateRegister, Fragment, FragmentBase, FragmentCategory,
    FragmentRegistration, Fragments, GdeMetadataReplicatedState, I_FRAGMENT_TYPE_ID,
    MarshalContext, NamedField, NamedFieldMut, REPLICATED_STATE_TYPE_ID, ReplicatedState,
    ReplicatedStateBundle, ReplicatedStateBundleView, ReplicatedStateConstants,
    ReplicationControlData, ReplicationPerformanceData, StateFragmentTypeId,
};
pub use message::Message;
pub use registration::register;
pub use serialize::replicated_field::ReplicatedFieldHandlerBase;
#[cfg(feature = "server")]
pub use server::{OutboundTyped, ServerListenerHandle};
#[cfg(feature = "transport")]
pub use session::GridSession;
#[cfg(feature = "client")]
pub use session_service::SessionServiceHandle;
#[cfg(feature = "transport")]
pub use session_service::{OutboundSink, Outgoing};
pub use session_types::{Event, SessionId};
pub use spawn::{BoxedFuture, Spawner, set_spawner, spawn_detached};

// ============================================================================
// Error Types
// ============================================================================

use thiserror::Error;

/// `GridMate` error type
#[derive(Debug, Error)]
pub enum GridMateError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Driver error: {0}")]
    Driver(#[from] driver::error::DriverError),

    #[error("SSL/DTLS error: {0}")]
    Ssl(String),

    #[error("Operation timed out")]
    Timeout,

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Handshake failed: {0}")]
    HandshakeFailed(String),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Carrier error: {0}")]
    Carrier(String),

    #[error("Channel error: {0}")]
    Channel(String),
}

pub type Result<T> = std::result::Result<T, GridMateError>;

#[cfg(test)]
mod architecture_tests {
    #[test]
    fn build_script_is_project_agnostic_by_default() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read gridmate Cargo.toml");
        let build_script =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/build.rs"))
                .expect("read gridmate build.rs");

        assert!(
            manifest.contains("native-type-registry-debug = []"),
            "native type-registry snapshots must be an explicit debug-validation feature"
        );
        assert!(
            manifest.contains("default = []"),
            "GridMate transport roles must be selected explicitly"
        );
        for forbidden in ["default = [\"client\"]", "steam-locator", "dotenvy"] {
            assert!(
                !manifest.contains(forbidden),
                "GridMate's default framework build must not depend on `{forbidden}`"
            );
        }

        for forbidden in ["steam_locator::", "dotenvy::", "EOSSDK-Win64-Shipping"] {
            assert!(
                !build_script.contains(forbidden),
                "GridMate build.rs must not probe or stage project runtime assets with `{forbidden}`"
            );
        }
        assert!(
            build_script.contains("CARGO_FEATURE_NATIVE_TYPE_REGISTRY_DEBUG")
                && build_script.contains("AZOTH_TYPEREGISTRY"),
            "GridMate build.rs must load native debug tables only through explicit feature/env selection"
        );
        assert!(
            build_script.contains("env::var(\"DEBUG\")"),
            "GridMate build.rs must keep native snapshot loading debug-only"
        );
        assert!(
            build_script.contains("manifest_dir.ancestors()"),
            "GridMate build.rs must discover the workspace root after nested source-tree moves"
        );
    }

    #[test]
    fn carrier_default_protocol_profile_is_project_agnostic() {
        let carrier_desc = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/carrier/carrier_desc.rs"
        ))
        .expect("read carrier_desc.rs");

        for forbidden in ["vendor-runtime-assets", "project-registry-snapshot"] {
            assert!(
                !carrier_desc.contains(forbidden),
                "GridMate's default carrier profile must not encode project compatibility via `{forbidden}`"
            );
        }
    }
}
