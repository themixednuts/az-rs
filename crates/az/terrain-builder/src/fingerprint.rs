//! Derived identity for the `azoth.terrain` build rule.
//!
//! This rule owns four product formats — world, region, layer set, and
//! heightmap — and encodes none of them itself. All four codecs, and the
//! format versions they write, live in `az-terrain-runtime`. That split is
//! precisely why a declared identity fails here: an encoder can change in a
//! crate the rule merely depends on, leaving every value the rule declares
//! untouched while every product byte moves.
//!
//! The fingerprint therefore hashes `az-terrain-runtime` alongside this
//! crate's own source-document parsing and job analysis, plus `az-terrain`
//! for the asset identities the products are catalogued under.

use std::sync::OnceLock;

use az_asset_builder::fingerprint::{AnalysisFingerprint, cached};

const DOMAIN: &str = "azoth.terrain-analysis/v1:";

pub fn terrain_analysis_fingerprint() -> &'static str {
    static FINGERPRINT: OnceLock<String> = OnceLock::new();
    cached(&FINGERPRINT, || {
        AnalysisFingerprint::new(DOMAIN)
            .field("terrain-builder-sources", crate::SOURCE_FINGERPRINT)
            .field(
                "terrain-runtime-codec-sources",
                az_terrain_runtime::SOURCE_FINGERPRINT,
            )
            .field("terrain-sources", az_terrain::SOURCE_FINGERPRINT)
            .finish()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_components_are_real_build_time_fingerprints() {
        for (label, value) in [
            ("terrain-builder", crate::SOURCE_FINGERPRINT),
            ("terrain-runtime", az_terrain_runtime::SOURCE_FINGERPRINT),
            ("terrain", az_terrain::SOURCE_FINGERPRINT),
        ] {
            assert!(
                value.starts_with("azoth.source-tree/v1:"),
                "{label} published `{value}` instead of a source-tree fingerprint"
            );
        }
    }

    #[test]
    fn the_rule_publishes_the_derived_fingerprint() {
        let registries = az_gem_contract::Registries::new();
        let rule = crate::terrain_build_rule(&az_asset_builder::JobContext::new(&registries));
        assert_eq!(rule.analysis_fingerprint, terrain_analysis_fingerprint());
        assert!(rule.analysis_fingerprint.starts_with(DOMAIN));
    }

    #[test]
    fn the_out_of_crate_codec_fingerprint_is_load_bearing() {
        // The whole point of this rule's fingerprint: all four encoders live
        // in another crate, so that crate's identity must reach the value.
        let swapped = AnalysisFingerprint::new(DOMAIN)
            .field("terrain-builder-sources", crate::SOURCE_FINGERPRINT)
            .field("terrain-runtime-codec-sources", "a-different-codec")
            .field("terrain-sources", az_terrain::SOURCE_FINGERPRINT)
            .finish();
        assert_ne!(terrain_analysis_fingerprint(), swapped);
    }
}
