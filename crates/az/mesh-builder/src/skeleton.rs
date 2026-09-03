//! Self-contained glTF skeleton products.

use az_asset_builder::{
    AssetBuilderPattern, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, ProductFormat,
    ProductFormatId, SourceFormat, SourceSchemaType, TypedBuildProduct, product_format_id,
    source_schema_type,
};
pub use az_mesh::{SKELETON_ASSET_TYPE, SKELETON_ASSET_TYPE_NAME, SkeletonAssetData};
use thiserror::Error;
use uuid::uuid;

use crate::rigged::{RiggedGltfError, validate_rigged_glb};

pub const SKELETON_SOURCE_SCHEMA_NAME: &str = "azoth.mesh.SkeletonSource";
pub const SKELETON_PRODUCT_EXTENSION: &str = "rig.glb";
pub const SKELETON_PRODUCT_SUB_ID: u32 = 1;
pub const SKELETON_BUILDER_NAME: &str = "azoth.skeleton";
pub const SKELETON_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("04ce4d38-2feb-49dc-b058-91929ce3ec69"));

pub struct SkeletonSourceFormat;

impl SourceFormat for SkeletonSourceFormat {
    const SCHEMA: Option<SourceSchemaType> =
        Some(SourceSchemaType::__from_static(SKELETON_SOURCE_SCHEMA_NAME));
    const SCHEMA_TYPES: &'static [SourceSchemaType] =
        &[SourceSchemaType::__from_static(SKELETON_SOURCE_SCHEMA_NAME)];
    const PATTERNS: &'static [AssetBuilderPattern] =
        &[AssetBuilderPattern::static_wildcard("*.skeleton.glb")];
}

pub struct SkeletonProductFormat;

impl ProductFormat for SkeletonProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static(SKELETON_ASSET_TYPE_NAME);
    const VERSION: u32 = 1;
    type Asset = SkeletonAssetData;
}

pub const SKELETON_FORMAT_ID: ProductFormatId = product_format_id::<SkeletonProductFormat>();
pub const SKELETON_SOURCE_SCHEMA: SourceSchemaType = source_schema_type::<SkeletonSourceFormat>();

#[must_use]
pub fn skeleton_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<SkeletonSourceFormat>()
        .named(SKELETON_BUILDER_NAME)
        .id(SKELETON_BUILDER_ID)
        .version(1)
        .analysis_fingerprint(crate::fingerprint::passthrough_analysis_fingerprint())
        .produces::<SkeletonProductFormat>()
        .create_jobs(create_jobs)
        .process(process_job)
}

fn create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    CreateJobsResponse {
        jobs: request
            .platforms
            .iter()
            .copied()
            .map(JobDescriptor::default_for_platform)
            .collect(),
        ..CreateJobsResponse::default()
    }
}

fn process_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    if let Err(error) = validate_skeleton_glb(&request.source_path, request.source_bytes) {
        tracing::warn!(source = %request.source_path, %error, "skeleton product failed");
        return ProcessJobResponse {
            result: ProcessJobResult::Failed,
            ..ProcessJobResponse::default()
        };
    }
    ProcessJobResponse {
        products: vec![
            TypedBuildProduct::<SkeletonProductFormat>::from_trusted_path(
                skeleton_product_path(&request.source_path),
                SKELETON_PRODUCT_SUB_ID,
                request.source_bytes.to_vec(),
            )
            .erase(),
        ],
        result: ProcessJobResult::Success,
        ..ProcessJobResponse::default()
    }
}

#[must_use]
pub fn skeleton_product_path(source_path: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    let stem = normalized
        .strip_suffix(".skeleton.glb")
        .unwrap_or(normalized.as_str());
    format!("{stem}.{SKELETON_PRODUCT_EXTENSION}")
}

#[derive(Debug, Error)]
pub enum SkeletonProductError {
    #[error(transparent)]
    RiggedGltf(#[from] RiggedGltfError),
}

/// Accept a `.skeleton.glb` source only if it carries a well-formed rig.
///
/// # Errors
///
/// Returns [`SkeletonProductError::RiggedGltf`] wrapping any error
/// [`validate_rigged_glb`] produces: the bytes do not parse as glTF, a buffer is
/// external instead of embedded, the binary chunk is absent, the document
/// declares no skins, or a skin has no joints, no inverse bind matrices, a
/// matrix count that disagrees with its joint count, or a non-finite matrix.
pub fn validate_skeleton_glb(source_path: &str, bytes: &[u8]) -> Result<(), SkeletonProductError> {
    validate_rigged_glb(source_path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use az_asset_builder::{ProductFormat, SourceFormat};

    use super::*;

    #[test]
    fn skeleton_rule_is_schema_and_suffix_specific() {
        let registries = az_gem_contract::Registries::new();
        let rule = skeleton_build_rule(&JobContext::new(&registries));
        assert!(rule.matches_source(
            "characters/player/hero.skeleton.glb",
            Some(SKELETON_SOURCE_SCHEMA_NAME)
        ));
        assert!(!rule.matches_source(
            "characters/player/hero.skinnedmesh.glb",
            Some(SKELETON_SOURCE_SCHEMA_NAME)
        ));
        assert_eq!(SkeletonProductFormat::ID, SKELETON_FORMAT_ID);
        assert_eq!(SkeletonSourceFormat::SCHEMA, Some(SKELETON_SOURCE_SCHEMA));
        assert_eq!(
            skeleton_product_path("characters/player/hero.skeleton.glb"),
            "characters/player/hero.rig.glb"
        );
    }
}
