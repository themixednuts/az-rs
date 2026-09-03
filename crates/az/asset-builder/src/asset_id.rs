//! Asset-id derivation helpers for the asset build SDK.
//!
//! The nominal `AZ::Data::AssetId` type is owned by `az-core`; this module only
//! contains build-pipeline helpers that derive asset ids from normalized source
//! paths.

use az_core::AssetId;
use az_core::uuid::AzUuidExt;
use az_filesystem::normalize_source_path;
use uuid::Uuid;

/// Derive an [`AssetId`] from a source path and product sub id.
///
/// This is Lumberyard `AssetCatalog::GenerateAssetIdTEMP`: `CreateName` of the
/// normalized path plus `sub_id`. Use it for a source's *own* identity when no
/// azmeta sidecar exists. Do **not** use it to embed a reference to another
/// already-registered product — that is [`crate::resolve_referenced_product_id`],
/// the cook-time equivalent of `GetAssetIdByPath`.
#[must_use]
pub fn source_asset_id(source_path: &str, sub_id: u32) -> AssetId {
    AssetId::new(source_guid(source_path), sub_id)
}

/// Derive the `guid` half of an [`AssetId`] from a source path.
///
/// Matches Lumberyard's catalog path hashing: normalize the virtual source path
/// to lowercase forward slashes, then run `AzCore` `Uuid::CreateName`.
#[must_use]
pub fn source_guid(source_path: &str) -> Uuid {
    let normalized = normalize_source_path(source_path);
    Uuid::create_name(normalized.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_name_matches_azcore_for_hello() {
        let uuid = Uuid::create_name(b"hello");
        assert_eq!(uuid.to_string(), "aaf4c61d-dcc5-58a2-9abe-de0f3b482cd9");
    }

    #[test]
    fn create_name_empty_returns_nil() {
        assert!(Uuid::create_name(b"").is_nil());
    }

    #[test]
    fn source_guid_is_stable_across_path_case_and_separators() {
        let a = source_guid("objects/Foo.cgf");
        let b = source_guid("Objects\\foo.CGF");
        assert_eq!(a, b);
    }

    #[test]
    fn source_guid_ignores_redundant_and_trailing_separators() {
        // Redundant internal slashes and a trailing slash normalize away, so
        // `AZ::Uuid::CreateName` folds the two spellings to the same GUID.
        assert_eq!(normalize_source_path("a//b/"), normalize_source_path("a/b"));
        assert_eq!(source_guid("a//b/"), source_guid("a/b"));
    }

    #[test]
    fn source_asset_id_distinguishes_sub_ids() {
        let path = "models/foo.cgf";
        let primary = source_asset_id(path, 0);
        let lod1 = source_asset_id(path, 1);

        assert_eq!(primary.guid, lod1.guid);
        assert_ne!(primary, lod1);
    }
}
