//! Secret resolution for trusted Azoth hosts and tools.
//!
//! This crate is intentionally absent from game-client and title-runtime
//! dependency graphs. Those processes receive narrow capabilities, never a
//! general secret resolver.

mod backend;
mod config;
mod env;
mod error;
mod local;
mod reference;
mod registry;
mod resolved;
mod router;

pub use backend::{BackendCapabilities, SecretBackend, SecretBatchFuture, SecretFuture};
pub use config::SecretMountConfig;
pub use env::EnvSecretBackend;
pub use error::SecretError;
pub use local::{
    LocalSecretStore, MasterKeyProvider, OsKeychainMasterKey, ProvisionSecrets, ResolvedMasterKey,
};
pub use reference::SecretRef;
pub use registry::{
    BackendBuildContext, BackendFactory, BackendFactoryFuture, ENV_BACKEND_ID, LOCAL_BACKEND_ID,
    SecretBackendRegistry, register_core_backends,
};
pub use resolved::ResolvedSecret;
pub use router::SecretRouter;
