//! `AssetBuilder` adapters for legacy `LmbrCentral` source assets.

use az_asset::EngineTextureFormat;
use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult, TypedBuildProduct,
};
use az_filesystem::{engine_path, engine_path_with_extension_key};
use az_gem_lmbr_central::{
    MaterialAsset, write_material_asset, write_material_override_asset, write_vertex_shape_asset,
};
use uuid::uuid;

use crate::{
    MaterialOverrideProductFormat, MaterialProductFormat, MaterialSource, MaterialSourceFormat,
    VertexShapeProductFormat, VolumeShapeSource, VolumeShapeSourceFormat, parse_material_asset,
    parse_material_override_asset,
};

pub const MATERIAL_NAME: &str = "lmbr-central.material";
pub const MATERIAL_ID: BuilderId = BuilderId::new(uuid!("6c704152-3e5d-4faa-b218-80ad5e3f4c62"));
pub const MATERIAL_VERSION: u32 = 1;
pub const VOLUME_SHAPE_NAME: &str = "lmbr-central.volume-shape";
pub const VOLUME_SHAPE_ID: BuilderId =
    BuilderId::new(uuid!("9fa37485-6181-42dd-e54b-b3d081627f95"));
pub const VOLUME_SHAPE_VERSION: u32 = 1;

#[must_use]
pub fn material_desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<MaterialSourceFormat>()
        .named(MATERIAL_NAME)
        .id(MATERIAL_ID)
        .version(MATERIAL_VERSION)
        .produces::<MaterialProductFormat>()
        .create_jobs(create_jobs)
        .process(material_process_job)
}

#[must_use]
pub fn volume_shape_desc(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<VolumeShapeSourceFormat>()
        .named(VOLUME_SHAPE_NAME)
        .id(VOLUME_SHAPE_ID)
        .version(VOLUME_SHAPE_VERSION)
        .produces::<VertexShapeProductFormat>()
        .create_jobs(create_jobs)
        .process(volume_shape_process_job)
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

/// The engine texture format a build job rewrites material texture references
/// to.
///
/// The host's composition carries no texture-transcode profile, so the job path
/// uses the engine default. Callers that select a profile — an importer, or a
/// worker configured for KTX2 — pass it explicitly to
/// [`transform_material_product`].
const JOB_TEXTURE_FORMAT: EngineTextureFormat = EngineTextureFormat::Dds;

fn material_process_job(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let product =
        match transform_material_product(&req.source_path, req.source_bytes, JOB_TEXTURE_FORMAT) {
            Ok(product) => product,
            Err(err) => {
                tracing::warn!(source = %req.source_path, error = %err, "material product failed");
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

fn volume_shape_process_job(req: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let product = match transform_volume_shape_product(&req.source_path, req.source_bytes) {
        Ok(product) => product,
        Err(err) => {
            tracing::warn!(source = %req.source_path, error = %err, "volume shape product failed");
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

/// Builds the runtime material product from a `.material.ron` or legacy
/// `.mtl` source.
///
/// # Errors
///
/// Returns [`MaterialProductError::DeserializeRon`] when a `.material.ron`
/// source is not valid RON, [`MaterialProductError::Transform`] when a legacy
/// `.mtl` source does not parse, and [`MaterialProductError::Serialize`] when
/// the product cannot be written.
pub fn transform_material_product(
    source_path: &str,
    source_bytes: &[u8],
    texture_format: EngineTextureFormat,
) -> Result<BuildProduct, MaterialProductError> {
    let asset = parse_material_product_source(source_path, source_bytes, texture_format)?;
    let mut bytes = Vec::new();
    write_material_asset(&asset, &mut bytes)?;

    Ok(
        TypedBuildProduct::<MaterialProductFormat>::from_trusted_path(
            engine_path(source_path, "materials", "material.bin"),
            0,
            bytes,
        )
        .erase(),
    )
}

fn parse_material_product_source(
    source_path: &str,
    source_bytes: &[u8],
    texture_format: EngineTextureFormat,
) -> Result<MaterialAsset, MaterialProductError> {
    if source_path.to_ascii_lowercase().ends_with(".material.ron") {
        let source = ron::de::from_bytes::<MaterialSource>(source_bytes)
            .map_err(MaterialProductError::DeserializeRon)?;
        return Ok(source.to_asset());
    }

    parse_material_asset(source_path, source_bytes, texture_format)
        .map_err(MaterialProductError::Transform)
}

/// Builds the runtime material-override product from a legacy override XML
/// source.
///
/// # Errors
///
/// Returns [`MaterialOverrideProductError::Transform`] when the XML does not
/// parse as a `MaterialParamsOverride` document, and
/// [`MaterialOverrideProductError::Serialize`] when the product cannot be
/// written.
pub fn transform_material_override_product(
    source_path: &str,
    source_bytes: &[u8],
    texture_format: EngineTextureFormat,
) -> Result<BuildProduct, MaterialOverrideProductError> {
    let asset = parse_material_override_asset(source_path, source_bytes, texture_format)?;
    let mut bytes = Vec::new();
    write_material_override_asset(&asset, &mut bytes)?;

    Ok(
        TypedBuildProduct::<MaterialOverrideProductFormat>::from_trusted_path(
            engine_path(source_path, "materials", "material-override.bin"),
            0,
            bytes,
        )
        .erase(),
    )
}

/// Builds the runtime vertex-shape product from a `.shape.ron` source.
///
/// # Errors
///
/// Returns [`VolumeShapeProductError::ParseSource`] when `source_bytes` is not
/// valid [`VolumeShapeSource`] RON, and [`VolumeShapeProductError::Serialize`]
/// when the product cannot be written.
pub fn transform_volume_shape_product(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BuildProduct, VolumeShapeProductError> {
    let source = VolumeShapeSource::from_ron_bytes(source_bytes)?;
    let asset = source.to_asset();
    let mut bytes = Vec::new();
    write_vertex_shape_asset(&asset, &mut bytes)?;

    Ok(
        TypedBuildProduct::<VertexShapeProductFormat>::from_trusted_path(
            volume_shape_product_path(source_path),
            0,
            bytes,
        )
        .erase(),
    )
}

#[must_use]
pub fn volume_shape_product_path(source_path: &str) -> String {
    engine_path_with_extension_key(source_path, "", "vshapec", Some("shape.ron"))
}

#[derive(Debug, thiserror::Error)]
pub enum MaterialProductError {
    #[error("transform material asset: {0}")]
    Transform(#[from] crate::MaterialTransformError),
    #[error("deserialize material source RON: {0}")]
    DeserializeRon(ron::error::SpannedError),
    #[error("serialize material product: {0}")]
    Serialize(#[from] az_gem_lmbr_central::MaterialAssetFormatError),
}

#[derive(Debug, thiserror::Error)]
pub enum MaterialOverrideProductError {
    #[error("transform material override asset: {0}")]
    Transform(#[from] crate::MaterialTransformError),
    #[error("serialize material override product: {0}")]
    Serialize(#[from] az_gem_lmbr_central::MaterialAssetFormatError),
}

#[derive(Debug, thiserror::Error)]
pub enum VolumeShapeProductError {
    #[error("parse volume shape source RON: {0}")]
    ParseSource(#[from] ron::error::SpannedError),
    #[error("serialize vertex shape product: {0}")]
    Serialize(#[from] az_gem_lmbr_central::VertexShapeAssetFormatError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_contract::Registries;

    #[test]
    fn descriptor_claims_material_sources() {
        let registries = Registries::new();
        let desc = material_desc(&JobContext::new(&registries));

        assert_eq!(desc.name, MATERIAL_NAME);
        assert_eq!(desc.id, MATERIAL_ID);
        assert_eq!(desc.version, MATERIAL_VERSION);
        assert!(desc.matches("materials/tree.material.ron"));
        assert!(!desc.matches("materials/tree.mtl"));
        assert!(!desc.matches("materials/tree.xml"));
    }

    #[test]
    fn descriptor_claims_volume_shape_sources() {
        let registries = Registries::new();
        let desc = volume_shape_desc(&JobContext::new(&registries));

        assert_eq!(desc.name, VOLUME_SHAPE_NAME);
        assert_eq!(desc.id, VOLUME_SHAPE_ID);
        assert_eq!(desc.version, VOLUME_SHAPE_VERSION);
        assert!(desc.matches("volumes/region.shape.ron"));
        assert!(!desc.matches("volumes/region.vshapec"));
        assert!(!desc.matches("shapes/region.rnr"));
    }

    #[test]
    fn volume_shape_builder_cooks_shape_ron_to_runtime_vshapec_product_path() {
        let source =
            VolumeShapeSource::from_legacy("Volumes/Region.vshapec", &vertex_shape_bytes())
                .unwrap();
        let source_bytes = source.to_ron_bytes().unwrap();

        let product = transform_volume_shape_product("volumes/region.shape.ron", &source_bytes)
            .expect("volume shape product");

        assert_eq!(product.product_path.as_str(), "volumes/region.vshapec");
        assert_eq!(product.asset_type, crate::ids::VOLUME_SHAPE);
        assert_eq!(
            product.format,
            crate::product_formats::LMBR_CENTRAL_VERTEX_SHAPE
        );
        let decoded = az_gem_lmbr_central::read_vertex_shape_asset(&product.bytes).unwrap();
        assert_eq!(
            decoded.vertices,
            vec![bevy::prelude::Vec3::new(1.0, 2.0, 0.0)]
        );
        assert_eq!(decoded.metadata[0].key, "TerritoryId");
        assert_eq!(decoded.metadata[0].value, "14:@Example");
        assert_exact(decoded.height, 32.0);
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
            VOLUME_SHAPE_ID,
            "volumes/region.shape.ron",
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

    /// Compares a round-tripped `f32` bit-exactly.
    ///
    /// The fixture writes the same little-endian pattern the reader decodes,
    /// so any difference is a decode bug rather than accumulated error; an
    /// epsilon window would hide exactly the bugs this asserts against.
    #[track_caller]
    fn assert_exact(actual: f32, expected: f32) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{actual} != {expected}"
        );
    }

    fn vertex_shape_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        bytes.extend_from_slice(&0.0f32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        write_string(&mut bytes, "TerritoryId");
        write_string(&mut bytes, "14:@Example");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&32.0f32.to_le_bytes());
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes
    }

    fn write_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&u32::try_from(value.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
}
