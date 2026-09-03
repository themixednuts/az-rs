//! Derived identity for the mesh, skeleton, and skinned-mesh build rules.
//!
//! `encode_mesh_asset` is a hand-rolled little-endian byte writer: field
//! order, length prefixes, and attribute packing exist only inside that
//! function. Nothing about them appears in `MeshAsset` or any other type
//! declaration, so hashing declared structure would miss exactly the change
//! that silently invalidates every mesh product. The build-time fingerprint of
//! this crate's sources is what covers it.
//!
//! Skeleton and skinned-mesh own no codec — they validate the source GLB and
//! pass its bytes through. What they do declare is typed asset data at a
//! format version with no Azoth header in the bytes, which makes that version
//! a catalog assertion nothing can check by reading a product. Hashing
//! `az-mesh`, where those asset identities live, is what makes the assertion
//! checkable.
//!
//! The two fingerprints differ in one component. Mesh products embed material
//! product paths resolved through `az-material-builder`, so a change there
//! changes mesh bytes; the passthrough rules never touch it and must not
//! reprocess when it moves.

use std::sync::OnceLock;

use az_asset_builder::fingerprint::{AnalysisFingerprint, cached};

const MESH_DOMAIN: &str = "azoth.mesh-analysis/v1:";
const PASSTHROUGH_DOMAIN: &str = "azoth.mesh-passthrough-analysis/v1:";

/// Identity for `azoth.mesh`, whose products are framed by this crate's codec
/// and reference material products resolved through `az-material-builder`.
pub fn mesh_analysis_fingerprint() -> &'static str {
    static FINGERPRINT: OnceLock<String> = OnceLock::new();
    cached(&FINGERPRINT, || {
        AnalysisFingerprint::new(MESH_DOMAIN)
            .field("mesh-builder-sources", crate::SOURCE_FINGERPRINT)
            .field("mesh-sources", az_mesh::SOURCE_FINGERPRINT)
            .field(
                "material-builder-sources",
                az_material_builder::SOURCE_FINGERPRINT,
            )
            .finish()
    })
}

/// Identity for `azoth.skeleton` and `azoth.skinned-mesh`, which copy source
/// bytes and only assert an asset identity over them.
pub fn passthrough_analysis_fingerprint() -> &'static str {
    static FINGERPRINT: OnceLock<String> = OnceLock::new();
    cached(&FINGERPRINT, || {
        AnalysisFingerprint::new(PASSTHROUGH_DOMAIN)
            .field("mesh-builder-sources", crate::SOURCE_FINGERPRINT)
            .field("mesh-sources", az_mesh::SOURCE_FINGERPRINT)
            .finish()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_components_are_real_build_time_fingerprints() {
        for (label, value) in [
            ("mesh-builder", crate::SOURCE_FINGERPRINT),
            ("mesh", az_mesh::SOURCE_FINGERPRINT),
            ("material-builder", az_material_builder::SOURCE_FINGERPRINT),
        ] {
            assert!(
                value.starts_with("azoth.source-tree/v1:"),
                "{label} published `{value}` instead of a source-tree fingerprint"
            );
        }
    }

    #[test]
    fn codec_owning_and_passthrough_rules_have_distinct_identities() {
        assert_ne!(
            mesh_analysis_fingerprint(),
            passthrough_analysis_fingerprint()
        );
    }

    #[test]
    fn passthrough_identity_excludes_the_material_path_resolver() {
        // A change under `az-material-builder` must not reprocess skeletons.
        let with_material = AnalysisFingerprint::new(PASSTHROUGH_DOMAIN)
            .field("mesh-builder-sources", crate::SOURCE_FINGERPRINT)
            .field("mesh-sources", az_mesh::SOURCE_FINGERPRINT)
            .field("material-builder-sources", "anything")
            .finish();
        assert_ne!(passthrough_analysis_fingerprint(), with_material);
    }

    #[test]
    fn mesh_identity_tracks_the_material_path_resolver() {
        // Mesh products embed material product paths, so they must.
        let swapped = AnalysisFingerprint::new(MESH_DOMAIN)
            .field("mesh-builder-sources", crate::SOURCE_FINGERPRINT)
            .field("mesh-sources", az_mesh::SOURCE_FINGERPRINT)
            .field("material-builder-sources", "a-different-resolver")
            .finish();
        assert_ne!(mesh_analysis_fingerprint(), swapped);
    }

    #[test]
    fn rules_publish_the_derived_fingerprints() {
        let registries = az_gem_contract::Registries::new();
        let context = az_asset_builder::JobContext::new(&registries);

        assert_eq!(
            crate::mesh_build_rule(&context).analysis_fingerprint,
            mesh_analysis_fingerprint()
        );
        assert_eq!(
            crate::skeleton::skeleton_build_rule(&context).analysis_fingerprint,
            passthrough_analysis_fingerprint()
        );
        assert_eq!(
            crate::skinned::skinned_mesh_build_rule(&context).analysis_fingerprint,
            passthrough_analysis_fingerprint()
        );
    }
}
