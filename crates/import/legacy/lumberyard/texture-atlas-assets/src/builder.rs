//! `AssetBuilder` adapter for Lumberyard `TextureAtlas` index sources.
//!
//! The atlas source already carries engine-resolved texture paths, so the job
//! reads nothing from the host's composition.

use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, TypedBuildProduct,
};
use az_filesystem::engine_path_with_extension_key;
use az_gem_texture_atlas::write_texture_atlas_asset;
use uuid::uuid;

use crate::{TextureAtlasProductFormat, TextureAtlasSource, TextureAtlasSourceFormat};

pub const NAME: &str = "texture-atlas.atlas";
pub const ID: BuilderId = BuilderId::new(uuid!("a0b48596-7292-43ee-f65c-c4e1927380a6"));
pub const VERSION: u32 = 1;

#[must_use]
pub fn desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<TextureAtlasSourceFormat>()
        .named(NAME)
        .id(ID)
        .version(VERSION)
        .produces::<TextureAtlasProductFormat>()
        .create_jobs(create_jobs)
        .process(process_job)
}

fn create_jobs(req: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let jobs = req
        .platforms
        .iter()
        .copied()
        .map(JobDescriptor::default_for_platform)
        .collect();
    CreateJobsResponse {
        jobs,
        ..CreateJobsResponse::default()
    }
}

fn process_job(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let product = match transform_product(&req.source_path, req.source_bytes) {
        Ok(product) => product,
        Err(err) => {
            tracing::warn!(source = %req.source_path, error = %err, "texture atlas product failed");
            return ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            };
        }
    };

    ProcessJobResponse {
        products: vec![product],
        result: ProcessJobResult::Success,
        ..ProcessJobResponse::default()
    }
}

/// Compiles a texture-atlas authoring source into its packed build product.
///
/// # Errors
///
/// Returns [`TextureAtlasProductError::Deserialize`] if `source_bytes` is not
/// a well-formed `TextureAtlasSource` RON document,
/// [`TextureAtlasProductError::Transform`] if packing the regions overflows a
/// `u32` name offset, length or coordinate, and
/// [`TextureAtlasProductError::Serialize`] if the packed asset cannot be
/// written to its binary product format.
pub fn transform_product(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BuildProduct, TextureAtlasProductError> {
    let source = ron::de::from_bytes::<TextureAtlasSource>(source_bytes)?;
    let asset = source.to_asset()?;
    let mut bytes = Vec::new();
    write_texture_atlas_asset(&asset, &mut bytes)?;

    Ok(
        TypedBuildProduct::<TextureAtlasProductFormat>::from_trusted_path(
            engine_path_with_extension_key(
                source_path,
                "texture-atlases",
                "texture-atlas.bin",
                Some("atlas.ron"),
            ),
            0,
            bytes,
        )
        .erase(),
    )
}

#[derive(Debug, thiserror::Error)]
pub enum TextureAtlasProductError {
    #[error("parse texture atlas source RON: {0}")]
    Deserialize(#[from] ron::error::SpannedError),
    #[error("transform texture atlas asset: {0}")]
    Transform(#[from] crate::TextureAtlasTransformError),
    #[error("serialize texture atlas product: {0}")]
    Serialize(#[from] az_gem_texture_atlas::TextureAtlasAssetFormatError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_contract::Registries;

    #[test]
    fn descriptor_claims_texture_atlas_sources() {
        let registries = Registries::new();
        let desc = desc(&JobContext::new(&registries));

        assert_eq!(desc.name, NAME);
        assert_eq!(desc.id, ID);
        assert_eq!(desc.version, VERSION);
        assert!(desc.matches("ui/lyshineui/images/chat.atlas.ron"));
        assert!(!desc.matches("lyshineui/images/chat.texatlasidx"));
        assert!(!desc.matches("lyshineui/images/chat.sprite"));
    }

    #[test]
    fn create_jobs_emits_default_job_for_each_platform() {
        let registries = Registries::new();
        let context = JobContext::new(&registries);
        let platforms = [
            az_asset_builder::DEFAULT_PLATFORM_ID,
            az_asset_builder::PlatformId::new_static("server"),
        ];
        let req = CreateJobsRequest::new(
            ID,
            "lyshineui/images/chat.texatlasidx",
            std::path::Path::new(""),
            uuid::Uuid::nil(),
            None,
            b"",
            &platforms,
            &context,
        );

        let response = create_jobs(&req);

        assert_eq!(response.jobs.len(), 2);
        assert_eq!(response.jobs[0].platform, "pc");
        assert_eq!(response.jobs[1].platform, "server");
        assert_eq!(response.jobs[0].job_key, "default");
        assert_eq!(response.jobs[1].job_key, "default");
    }
}
