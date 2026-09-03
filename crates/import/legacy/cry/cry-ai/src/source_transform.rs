//! Legacy `CryAI` generated navigation cache classification.

use std::path::Path;

use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, normalize_source_path,
};

use crate::BaiKind;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BaiNavigationCacheTransform;

impl LegacySourceTransform for BaiNavigationCacheTransform {
    type Error = BaiNavigationCacheTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        if !is_legacy_bai_navigation_cache_source(&input.source_path) {
            return Err(BaiNavigationCacheTransformError::UnsupportedPath {
                path: input.source_path.to_string(),
            });
        }

        let path = input.source_path.to_string();
        let family = BaiKind::from_path(Path::new(&path))
            .map(|kind| format!(" {kind:?}"))
            .unwrap_or_default();
        Ok(LegacySourceOutput::Excluded {
            reason: format!(
                "legacy CryAI{family} navigation cache {path} is generated editor output; Azoth rebuilds navigation from terrain, Prefab, and navigation authoring sources"
            ),
        })
    }
}

#[must_use]
pub fn is_legacy_bai_navigation_cache_source(source_path: &str) -> bool {
    Path::new(&normalize_source_path(source_path))
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bai"))
}

#[derive(Debug, thiserror::Error)]
pub enum BaiNavigationCacheTransformError {
    #[error("unsupported CryAI navigation cache path {path}")]
    UnsupportedPath { path: String },
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{LegacySourceInput, LegacySourceOutput, LegacySourceTransform};

    use super::*;

    #[test]
    fn classifies_only_legacy_bai_navigation_caches() {
        assert!(is_legacy_bai_navigation_cache_source(
            "Levels/world/areas.bai"
        ));
        assert!(!is_legacy_bai_navigation_cache_source(
            "levels/world/areas.bai.ron"
        ));
        assert!(!is_legacy_bai_navigation_cache_source(
            "levels/world/areas.bin"
        ));
    }

    #[test]
    fn transform_excludes_bai_navigation_cache_from_authoring_source() {
        let output = BaiNavigationCacheTransform
            .transform(LegacySourceInput::new("Levels/World/mnmnav.bai", &[]))
            .unwrap();

        assert_eq!(output.artifact(), None);
        match output {
            LegacySourceOutput::Excluded { reason } => {
                assert!(reason.contains("generated editor output"));
                assert!(reason.contains("levels/world/mnmnav.bai"));
                assert!(reason.contains("MnmNavigation"));
            }
            other => panic!("expected excluded BAI cache, got {other:?}"),
        }
    }
}
