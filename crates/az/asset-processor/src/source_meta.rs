//! Source-metadata sidecar contract.
//!
//! Owned by [`az_asset_builder::source_meta`]. This module re-exports that
//! contract so existing processor call sites keep a stable `source_meta::`
//! path.

pub use az_asset_builder::{
    SOURCE_META_SIDECAR_SUFFIX, SOURCE_META_SPEC, SourceAssetMeta, SourceMetaError,
    read_source_asset_meta, resolve_referenced_product_id, resolve_source_asset_guid,
    serialize_source_asset_meta, source_meta_sidecar_path,
};
