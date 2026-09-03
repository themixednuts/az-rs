//! Derived identity for the material-type and material build rules.
//!
//! `encode_material_type_asset` and `encode_material_asset` are hand-rolled
//! byte writers in this crate. Their field order, length prefixes, and
//! property packing live only in those function bodies, invisible to every
//! type declaration and to every runtime registry — so only a build-time hash
//! of this crate's sources can notice when they change.
//!
//! `az-material` contributes the schema names and asset identities the
//! products are catalogued under, so it is hashed alongside.
//!
//! Both rules share one identity because both codecs live in the same crate:
//! any edit that changes one crate fingerprint changes it for both, and
//! pretending otherwise would only invent a distinction the mechanism cannot
//! actually make. The rules stay distinguishable regardless, since a
//! descriptor hash also covers the rule's name and GUID.

use std::sync::OnceLock;

use az_asset_builder::fingerprint::{AnalysisFingerprint, cached};

const DOMAIN: &str = "azoth.material-analysis/v1:";

pub fn material_analysis_fingerprint() -> &'static str {
    static FINGERPRINT: OnceLock<String> = OnceLock::new();
    cached(&FINGERPRINT, || {
        AnalysisFingerprint::new(DOMAIN)
            .field("material-builder-sources", crate::SOURCE_FINGERPRINT)
            .field("material-sources", az_material::SOURCE_FINGERPRINT)
            .finish()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_components_are_real_build_time_fingerprints() {
        for (label, value) in [
            ("material-builder", crate::SOURCE_FINGERPRINT),
            ("material", az_material::SOURCE_FINGERPRINT),
        ] {
            assert!(
                value.starts_with("azoth.source-tree/v1:"),
                "{label} published `{value}` instead of a source-tree fingerprint"
            );
        }
    }

    #[test]
    fn both_rules_publish_the_derived_fingerprint() {
        let registries = az_gem_contract::Registries::new();
        let context = az_asset_builder::JobContext::new(&registries);
        for rule in [
            crate::material_type_build_rule(&context),
            crate::material_build_rule(&context),
        ] {
            assert_eq!(rule.analysis_fingerprint, material_analysis_fingerprint());
            assert!(rule.analysis_fingerprint.starts_with(DOMAIN));
        }
    }

    #[test]
    fn the_codec_crate_fingerprint_is_load_bearing() {
        let swapped = AnalysisFingerprint::new(DOMAIN)
            .field("material-builder-sources", "a-different-codec")
            .field("material-sources", az_material::SOURCE_FINGERPRINT)
            .finish();
        assert_ne!(material_analysis_fingerprint(), swapped);
    }
}
