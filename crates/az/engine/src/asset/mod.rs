//! Asset facade for the base engine crate.
//!
//! `az-engine` deliberately does not own runtime asset inventory. Azoth
//! runtime inventory is the Asset Catalog mounted by `az-framework` from
//! session-provided package roots: native `assetcatalog.bin` plus loose,
//! `azpack`, or compatible pak payloads. Keeping this crate to Bevy asset
//! type re-exports prevents a second bundle manifest/catalog path from
//! becoming part of the base engine ABI.

/// Prelude for asset types
pub mod prelude {
    // Re-export Bevy asset types for convenience
    pub use bevy::asset::{Asset, AssetLoader, AssetPath, AssetServer, Assets, Handle, LoadState};
    pub use bevy::prelude::AssetEvent;
}
