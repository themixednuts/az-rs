use std::collections::BTreeSet;
use std::path::Path;

use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, CreateJobsResult,
    JobContext, JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult,
    ProductDependency, SourceFileDependency, SourceMetaError, TypedBuildProduct,
    normalize_source_path, resolve_referenced_product_id,
};
use uuid::uuid;

use crate::character::definition::{
    CHARACTER_DEFINITION_PRODUCT_EXTENSION, CHARACTER_DEFINITION_PRODUCT_SUB_ID,
    CharacterDefinitionAsset, CharacterDefinitionCodecError, CharacterDefinitionProductFormat,
    CharacterDefinitionSource, CharacterDefinitionSourceFormat, write_character_definition,
};

pub const NAME: &str = "azoth.animation.character-definition";
pub const ID: BuilderId = BuilderId::new(uuid!("30ea5907-93f8-451d-9d18-ed53d30ef772"));
pub const VERSION: u32 = 1;

#[must_use]
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "Signature is pinned by `az_asset_builder::BuildRuleFactory`               (`fn(&JobContext<'_>) -> BuildRule`); a by-value parameter would               not coerce."
)]
pub fn desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<CharacterDefinitionSourceFormat>()
        .named(NAME)
        .id(ID)
        .version(VERSION)
        .produces::<CharacterDefinitionProductFormat>()
        .create_jobs(create_jobs)
        .process(process_job)
}

fn create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let source = match ron::de::from_bytes::<CharacterDefinitionSource>(request.source_bytes) {
        Ok(source) => source,
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "character-definition job discovery failed");
            return CreateJobsResponse {
                result: CreateJobsResult::Failed,
                ..CreateJobsResponse::default()
            };
        }
    };
    let mut dependency_paths = BTreeSet::new();
    source.visit_assets(|_, path| {
        dependency_paths.insert(path.as_str().to_owned());
    });
    CreateJobsResponse {
        jobs: request
            .platforms
            .iter()
            .copied()
            .map(JobDescriptor::default_for_platform)
            .collect(),
        source_dependencies: dependency_paths
            .into_iter()
            .map(SourceFileDependency::Path)
            .collect(),
        result: CreateJobsResult::Success,
    }
}

fn process_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let (product, product_dependencies) = match transform_product(
        request.source_root,
        &request.source_path,
        request.source_bytes,
    ) {
        Ok(product) => product,
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "character-definition product failed");
            return ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            };
        }
    };

    ProcessJobResponse {
        products: vec![product],
        product_dependencies,
        result: ProcessJobResult::Success,
    }
}

pub fn transform_product(
    source_root: &Path,
    source_path: &str,
    source_bytes: &[u8],
) -> Result<(BuildProduct, Vec<(usize, ProductDependency)>), CharacterDefinitionProductError> {
    let source: CharacterDefinitionSource = ron::de::from_bytes(source_bytes)?;
    let definition = source.try_map_assets(|kind, path| {
        resolve_referenced_product_id(source_root, path.as_str(), kind.product_sub_id())
    })?;
    let asset = CharacterDefinitionAsset::new(definition);
    let mut bytes = Vec::new();
    write_character_definition(&asset, &mut bytes)?;
    let product = TypedBuildProduct::<CharacterDefinitionProductFormat>::from_trusted_path(
        character_definition_product_path(source_path),
        CHARACTER_DEFINITION_PRODUCT_SUB_ID,
        bytes,
    )
    .erase();
    let mut dependency_ids = BTreeSet::new();
    asset.definition().visit_assets(|_, asset_id| {
        dependency_ids.insert(*asset_id);
    });
    let dependencies = dependency_ids
        .into_iter()
        .map(|asset_id| (0, ProductDependency::new(asset_id)))
        .collect();
    Ok((product, dependencies))
}

#[must_use]
pub fn character_definition_product_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized
        .strip_suffix(".character.ron")
        .unwrap_or(&normalized);
    format!("{stem}.{CHARACTER_DEFINITION_PRODUCT_EXTENSION}")
}

#[derive(Debug, thiserror::Error)]
pub enum CharacterDefinitionProductError {
    #[error("parse character-definition source: {0}")]
    Ron(#[from] ron::error::SpannedError),
    #[error("encode character-definition product: {0}")]
    Codec(#[from] CharacterDefinitionCodecError),
    #[error("read referenced source metadata: {0:?}")]
    SourceMeta(SourceMetaError),
}

impl From<SourceMetaError> for CharacterDefinitionProductError {
    fn from(error: SourceMetaError) -> Self {
        Self::SourceMeta(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::definition::CharacterMirroring;
    use az_asset_builder::source_asset_id;
    use az_core::AssetPathBuf;

    #[test]
    fn descriptor_claims_only_canonical_character_sources() {
        let registries = az_gem_contract::Registries::new();
        let descriptor = desc(&JobContext::new(&registries));

        assert!(descriptor.matches("characters/hero.character.ron"));
        assert!(!descriptor.matches("characters/hero.cdf"));
        assert!(!descriptor.matches("characters/hero.cdf.ron"));
    }

    #[test]
    fn product_contains_asset_ids_and_dependency_edges() {
        let source = CharacterDefinitionSource {
            model: AssetPathBuf::new("characters/hero.skinnedmesh.glb").unwrap(),
            parameters: None,
            material: None,
            keep_models_in_memory: false,
            mirroring: CharacterMirroring::default(),
            attachments: Vec::new(),
        };
        let bytes = ron::ser::to_string(&source).unwrap();

        let (product, dependencies) = transform_product(
            Path::new("."),
            "characters/hero.character.ron",
            bytes.as_bytes(),
        )
        .unwrap();

        assert_eq!(product.sub_id, CHARACTER_DEFINITION_PRODUCT_SUB_ID);
        assert_eq!(product.product_path.as_str(), "characters/hero.azcharacter");
        assert_eq!(dependencies.len(), 1);
        assert_eq!(
            dependencies[0].1.dependency_id,
            source_asset_id("characters/hero.skinnedmesh.glb", 1)
        );
    }
}
