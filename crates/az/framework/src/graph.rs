//! Runtime visual graph assets.
//!
//! The editor/project-host graph model compiles to `AZGIR` products before
//! runtime. This module exposes the Bevy loader from `az-graph-runtime` and
//! keeps `az-framework` dependent on the runtime product contract, not on the
//! build/editor graph compiler crates.

pub use az_graph_runtime::bevy_asset::{
    PackedGraphAsset, PackedGraphAssetLoader, PackedGraphAssetPlugin, PackedGraphLoadError,
};
