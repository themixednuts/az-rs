//! Component-asset-path → on-disk-product-path helpers.
//!
//! Identity-on-pak-source: with the `engine_map` translation layer
//! gone these are just normalisation passes plus the
//! `.slice` / `.dynamicslice` → `.slice.meta` variant rewrite.
//! Callers used to reach for a `Res<LmbrCentralSceneAssetResolver>`
//! Bevy resource that wrapped these two functions; that wrapper is
//! gone — call the free fns directly.

use az_asset::normalize_source_path;

/// Normalise a component-authored asset path string.
///
/// Returns `None` for empty/whitespace input; otherwise yields the
/// lowercased forward-slash form that matches both the on-disk
/// layout and the `AssetCatalog` lookup key.
#[must_use]
pub fn resolve_scene_asset_path(asset_path: &str) -> Option<String> {
    let asset_path = normalize_source_path(asset_path);
    (!asset_path.is_empty()).then_some(asset_path)
}

/// Same as [`resolve_scene_asset_path`] but honours a slice-variant suffix.
///
/// When `variant` is present and `asset_path` ends in `.slice` /
/// `.dynamicslice`, returns the synthetic
/// `<stem>_<variant>.slice.meta` path that the extractor uses to
/// address that variant's transformed scene bytes; otherwise falls
/// through to the base resolver.
#[must_use]
pub fn resolve_scene_asset_path_with_variant(
    asset_path: &str,
    variant: Option<&str>,
) -> Option<String> {
    let variant = variant.map(str::trim).filter(|v| !v.is_empty());
    if let Some(variant) = variant
        && let Some(metadata_path) = slice_metadata_source_path(asset_path, variant)
        && let Some(scene_path) = resolve_scene_asset_path(&metadata_path)
    {
        return Some(scene_path);
    }
    resolve_scene_asset_path(asset_path)
}

/// Build the synthetic source path that addresses one variant of a
/// slice's `.slice.meta` companion.
///
/// Returns `None` if `variant` is empty or `source_path` doesn't
/// end in `.slice` / `.dynamicslice`.
#[must_use]
pub fn slice_metadata_source_path(source_path: &str, variant: &str) -> Option<String> {
    let source_path = normalize_source_path(source_path);
    let variant = normalize_variant_suffix(variant)?;
    let stem = source_path
        .strip_suffix(".dynamicslice")
        .or_else(|| source_path.strip_suffix(".slice"))?;
    Some(format!("{stem}_{variant}.slice.meta"))
}

/// Resolve the `.slice.meta` companion for a slice and an optional variant.
///
/// Unlike [`slice_metadata_source_path`], this also covers the base metadata
/// record used when the authored variant is empty.
#[must_use]
pub fn slice_metadata_source_path_with_variant(
    source_path: &str,
    variant: Option<&str>,
) -> Option<String> {
    let source_path = normalize_source_path(source_path);
    let stem = source_path
        .strip_suffix(".dynamicslice")
        .or_else(|| source_path.strip_suffix(".slice"))?;
    Some(variant.and_then(normalize_variant_suffix).map_or_else(
        || format!("{stem}.slice.meta"),
        |variant| format!("{stem}_{variant}.slice.meta"),
    ))
}

fn normalize_variant_suffix(variant: &str) -> Option<String> {
    let normalized = normalize_source_path(variant);
    (!normalized.is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_normalises_and_rejects_empty() {
        assert_eq!(
            resolve_scene_asset_path("Slices/World/Foo.slice").as_deref(),
            Some("slices/world/foo.slice")
        );
        assert_eq!(resolve_scene_asset_path("  ").as_deref(), None);
        assert_eq!(resolve_scene_asset_path("").as_deref(), None);
    }

    #[test]
    fn resolve_with_variant_produces_synthetic_slice_meta_path() {
        assert_eq!(
            resolve_scene_asset_path_with_variant(
                "slices/gatherables/master_tree.dynamicslice",
                Some("OakTree_a"),
            )
            .as_deref(),
            Some("slices/gatherables/master_tree_oaktree_a.slice.meta"),
        );
    }

    #[test]
    fn builds_synthetic_variant_path_from_dynamic_slice() {
        assert_eq!(
            slice_metadata_source_path("Slices/Gatherables/Master_Tree.dynamicslice", "OakTree_a")
                .as_deref(),
            Some("slices/gatherables/master_tree_oaktree_a.slice.meta"),
        );
    }

    #[test]
    fn preserves_variant_path_components_instead_of_flattening_them() {
        assert_eq!(
            slice_metadata_source_path("slices/world/tree.dynamicslice", "season/winter")
                .as_deref(),
            Some("slices/world/tree_season/winter.slice.meta"),
        );
    }

    #[test]
    fn rejects_non_slice_sources_and_empty_variants() {
        assert!(slice_metadata_source_path("foo/bar.cgf", "v1").is_none());
        assert!(slice_metadata_source_path("foo/bar.slice", "").is_none());
    }

    #[test]
    fn resolves_base_and_variant_metadata_companions() {
        assert_eq!(
            slice_metadata_source_path_with_variant(
                "Slices/Gatherables/Master_Tree.dynamicslice",
                None,
            )
            .as_deref(),
            Some("slices/gatherables/master_tree.slice.meta"),
        );
        assert_eq!(
            slice_metadata_source_path_with_variant(
                "Slices/Gatherables/Master_Tree.dynamicslice",
                Some(" OakTree_A "),
            )
            .as_deref(),
            Some("slices/gatherables/master_tree_oaktree_a.slice.meta"),
        );
    }

    #[test]
    fn resolve_with_variant_falls_through_when_variant_is_absent_or_blank() {
        assert_eq!(
            resolve_scene_asset_path_with_variant("slices/world/foo.slice", None).as_deref(),
            Some("slices/world/foo.slice"),
        );
        assert_eq!(
            resolve_scene_asset_path_with_variant("slices/world/foo.slice", Some(" ")).as_deref(),
            Some("slices/world/foo.slice"),
        );
    }
}
