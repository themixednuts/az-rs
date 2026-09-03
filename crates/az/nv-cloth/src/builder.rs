use std::collections::BTreeSet;
use std::path::Path;

use az_asset_builder::{
    BuildProduct, BuildRule, BuilderId, CreateJobsRequest, CreateJobsResponse, CreateJobsResult,
    JobContext, JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult,
    ProductDependency, SourceFileDependency, SourceMetaError, TypedBuildProduct,
    normalize_source_path, resolve_referenced_product_id,
};
use uuid::uuid;

use crate::{
    CLOTH_FABRIC_PRODUCT_EXTENSION, CLOTH_FABRIC_PRODUCT_SUB_ID, CLOTH_MATERIAL_PRODUCT_EXTENSION,
    CLOTH_MATERIAL_PRODUCT_SUB_ID, ClothFabricAsset, ClothFabricProductFormat, ClothFabricSource,
    ClothFabricSourceFormat, ClothMaterialAsset, ClothMaterialProductFormat, ClothMaterialSource,
    ClothMaterialSourceFormat, SOURCE_VERSION, write_cloth_fabric, write_cloth_material,
};

pub const FABRIC_NAME: &str = "azoth.nvcloth.cloth-fabric";
pub const MATERIAL_NAME: &str = "azoth.nvcloth.cloth-material";
pub const FABRIC_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("20d3e45c-a98d-4f66-9d9e-28c46d874e65"));
pub const MATERIAL_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("2ca0113a-6652-4146-8f40-69eb1fc7f79e"));
const BUILDER_VERSION: u32 = 1;

#[must_use]
pub fn cloth_fabric_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<ClothFabricSourceFormat>()
        .named(FABRIC_NAME)
        .id(FABRIC_BUILDER_ID)
        .version(BUILDER_VERSION)
        .produces::<ClothFabricProductFormat>()
        .create_jobs(create_fabric_jobs)
        .process(process_fabric_job)
}

#[must_use]
pub fn cloth_material_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<ClothMaterialSourceFormat>()
        .named(MATERIAL_NAME)
        .id(MATERIAL_BUILDER_ID)
        .version(BUILDER_VERSION)
        .produces::<ClothMaterialProductFormat>()
        .create_jobs(create_material_jobs)
        .process(process_material_job)
}

fn create_fabric_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let source = match ron::de::from_bytes::<ClothFabricSource>(request.source_bytes) {
        Ok(source) if source.version == SOURCE_VERSION && source.fabric.validate().is_ok() => {
            source
        }
        Ok(source) => {
            tracing::warn!(
                source = %request.source_path,
                version = source.version,
                "cloth-fabric job discovery rejected invalid source"
            );
            return failed_create_jobs();
        }
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "cloth-fabric job discovery failed");
            return failed_create_jobs();
        }
    };
    let mut paths = BTreeSet::new();
    source.fabric.visit_assets(|_, path| {
        paths.insert(path.as_str().to_owned());
    });
    CreateJobsResponse {
        jobs: request
            .platforms
            .iter()
            .copied()
            .map(JobDescriptor::default_for_platform)
            .collect(),
        source_dependencies: paths.into_iter().map(SourceFileDependency::Path).collect(),
        result: CreateJobsResult::Success,
    }
}

fn create_material_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    match ron::de::from_bytes::<ClothMaterialSource>(request.source_bytes) {
        Ok(source) if source.version == SOURCE_VERSION && source.material.validate().is_ok() => {
            CreateJobsResponse {
                jobs: request
                    .platforms
                    .iter()
                    .copied()
                    .map(JobDescriptor::default_for_platform)
                    .collect(),
                result: CreateJobsResult::Success,
                ..CreateJobsResponse::default()
            }
        }
        Ok(source) => {
            tracing::warn!(
                source = %request.source_path,
                version = source.version,
                "cloth-material job discovery rejected invalid source"
            );
            failed_create_jobs()
        }
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "cloth-material job discovery failed");
            failed_create_jobs()
        }
    }
}

const fn failed_create_jobs() -> CreateJobsResponse {
    CreateJobsResponse {
        jobs: Vec::new(),
        source_dependencies: Vec::new(),
        result: CreateJobsResult::Failed,
    }
}

fn process_fabric_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    match build_fabric_product(
        request.source_root,
        &request.source_path,
        request.source_bytes,
    ) {
        Ok((product, dependencies)) => ProcessJobResponse {
            products: vec![product],
            product_dependencies: dependencies,
            result: ProcessJobResult::Success,
        },
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "cloth-fabric product failed");
            ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            }
        }
    }
}

fn process_material_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    match build_material_product(&request.source_path, request.source_bytes) {
        Ok(product) => ProcessJobResponse {
            products: vec![product],
            result: ProcessJobResult::Success,
            ..ProcessJobResponse::default()
        },
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "cloth-material product failed");
            ProcessJobResponse {
                result: ProcessJobResult::Failed,
                ..ProcessJobResponse::default()
            }
        }
    }
}

pub fn build_fabric_product(
    source_root: &Path,
    source_path: &str,
    source_bytes: &[u8],
) -> Result<(BuildProduct, Vec<(usize, ProductDependency)>), ClothBuildError> {
    let source: ClothFabricSource = ron::de::from_bytes(source_bytes)?;
    if source.version != SOURCE_VERSION {
        return Err(ClothBuildError::UnsupportedVersion {
            version: source.version,
        });
    }
    source.fabric.validate()?;
    let fabric = source.fabric.try_map_assets(|kind, path| {
        resolve_referenced_product_id(source_root, path.as_str(), kind.product_sub_id())
    })?;
    let asset = ClothFabricAsset::new(fabric);
    let mut bytes = Vec::new();
    write_cloth_fabric(&asset, &mut bytes)?;
    let product = TypedBuildProduct::<ClothFabricProductFormat>::from_trusted_path(
        cloth_fabric_product_path(source_path),
        CLOTH_FABRIC_PRODUCT_SUB_ID,
        bytes,
    )
    .erase();
    let mut ids = BTreeSet::new();
    asset.fabric().visit_assets(|_, id| {
        ids.insert(*id);
    });
    Ok((
        product,
        ids.into_iter()
            .map(|id| (0, ProductDependency::new(id)))
            .collect(),
    ))
}

pub fn build_material_product(
    source_path: &str,
    source_bytes: &[u8],
) -> Result<BuildProduct, ClothBuildError> {
    let source: ClothMaterialSource = ron::de::from_bytes(source_bytes)?;
    if source.version != SOURCE_VERSION {
        return Err(ClothBuildError::UnsupportedVersion {
            version: source.version,
        });
    }
    source.material.validate()?;
    let asset = ClothMaterialAsset::new(source.material);
    let mut bytes = Vec::new();
    write_cloth_material(&asset, &mut bytes)?;
    Ok(
        TypedBuildProduct::<ClothMaterialProductFormat>::from_trusted_path(
            cloth_material_product_path(source_path),
            CLOTH_MATERIAL_PRODUCT_SUB_ID,
            bytes,
        )
        .erase(),
    )
}

#[must_use]
pub fn cloth_fabric_product_path(source_path: &str) -> String {
    product_path(source_path, ".cloth.ron", CLOTH_FABRIC_PRODUCT_EXTENSION)
}

#[must_use]
pub fn cloth_material_product_path(source_path: &str) -> String {
    product_path(
        source_path,
        ".clothmaterial.ron",
        CLOTH_MATERIAL_PRODUCT_EXTENSION,
    )
}

fn product_path(source_path: &str, source_suffix: &str, product_extension: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized
        .strip_suffix(source_suffix)
        .unwrap_or(&normalized);
    format!("{stem}.{product_extension}")
}

#[derive(Debug, thiserror::Error)]
pub enum ClothBuildError {
    #[error("parse cloth source: {0}")]
    Ron(#[from] ron::error::SpannedError),
    #[error("unsupported cloth source version {version}")]
    UnsupportedVersion { version: u32 },
    #[error(transparent)]
    Validation(#[from] crate::ClothValidationError),
    #[error(transparent)]
    Codec(#[from] crate::ClothCodecError),
    #[error("read referenced source metadata: {0:?}")]
    SourceMeta(SourceMetaError),
}

impl From<SourceMetaError> for ClothBuildError {
    fn from(error: SourceMetaError) -> Self {
        Self::SourceMeta(error)
    }
}
