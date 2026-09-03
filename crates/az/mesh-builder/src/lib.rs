//! Native glTF/GLB to Azoth mesh product pipeline.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

mod asset;
pub mod codec;
mod fingerprint;
pub mod import;
#[cfg(feature = "bevy")]
pub mod loader;
mod rigged;
pub mod skeleton;
pub mod skinned;

use az_asset_builder::{
    AssetBuilderPattern, BuildProduct, BuildRule, BuildRuleRegistration, BuilderId,
    CreateJobsRequest, CreateJobsResponse, CreateJobsResult, JobContext, JobDescriptor,
    ProcessJobRequest, ProcessJobResponse, ProcessJobResult, ProductDependency,
    ProductDependencyFlags, ProductFormat, ProductFormatId, ProductFormatRegistration,
    SourceFileDependency, SourceFormat, SourceSchemaRegistration, SourceSchemaType,
    product_format_id, resolve_referenced_product_id, source_schema_type,
};
pub use az_mesh::{MESH_ASSET_TYPE, MESH_ASSET_TYPE_NAME, MeshAssetData};
use uuid::uuid;

pub use asset::{MeshAsset, MeshMaterialSlot, MeshPrimitive};
pub use codec::{MeshCodecError, decode_mesh_asset, encode_mesh_asset};
pub use import::{ImportedMesh, MeshImportError, gltf_source_dependencies, import_gltf};
pub use skeleton::{
    SKELETON_ASSET_TYPE, SKELETON_ASSET_TYPE_NAME, SKELETON_BUILDER_ID, SKELETON_BUILDER_NAME,
    SKELETON_FORMAT_ID, SKELETON_PRODUCT_EXTENSION, SKELETON_PRODUCT_SUB_ID,
    SKELETON_SOURCE_SCHEMA, SKELETON_SOURCE_SCHEMA_NAME, SkeletonAssetData, SkeletonProductError,
    SkeletonProductFormat, SkeletonSourceFormat, skeleton_build_rule, skeleton_product_path,
    validate_skeleton_glb,
};
pub use skinned::{
    SKINNED_MESH_ASSET_TYPE, SKINNED_MESH_ASSET_TYPE_NAME, SKINNED_MESH_BUILDER_ID,
    SKINNED_MESH_BUILDER_NAME, SKINNED_MESH_FORMAT_ID, SKINNED_MESH_PRODUCT_EXTENSION,
    SKINNED_MESH_PRODUCT_SUB_ID, SKINNED_MESH_SOURCE_SCHEMA, SKINNED_MESH_SOURCE_SCHEMA_NAME,
    SkinnedMeshAssetData, SkinnedMeshProductError, SkinnedMeshProductFormat,
    SkinnedMeshSourceFormat, skinned_mesh_build_rule, skinned_mesh_product_path,
    validate_skinned_mesh_glb,
};

pub const MESH_MAGIC: &[u8; 8] = b"AZMESH\0\0";
pub const MESH_PRODUCT_VERSION: u32 = 1;
pub const MESH_PRODUCT_EXTENSION: &str = "azmesh";
pub const MESH_SOURCE_SCHEMA_NAME: &str = "azoth.mesh.Source";

pub struct MeshSourceFormat;

impl SourceFormat for MeshSourceFormat {
    const SCHEMA: Option<SourceSchemaType> =
        Some(SourceSchemaType::__from_static(MESH_SOURCE_SCHEMA_NAME));
    const SCHEMA_TYPES: &'static [SourceSchemaType] =
        &[SourceSchemaType::__from_static(MESH_SOURCE_SCHEMA_NAME)];
    const PATTERNS: &'static [AssetBuilderPattern] = &[
        AssetBuilderPattern::static_wildcard("*.gltf"),
        AssetBuilderPattern::static_wildcard("*.glb"),
    ];
}

pub struct MeshProductFormat;

impl ProductFormat for MeshProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static(MESH_ASSET_TYPE_NAME);
    const VERSION: u32 = MESH_PRODUCT_VERSION;
    type Asset = MeshAssetData;
}

pub const MESH_FORMAT_ID: ProductFormatId = product_format_id::<MeshProductFormat>();
pub const MESH_SOURCE_SCHEMA: SourceSchemaType = source_schema_type::<MeshSourceFormat>();
pub const MESH_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("d03a558b-4bd6-4d13-a75d-82e2eb257711"));
pub const MESH_BUILDER_NAME: &str = "azoth.mesh";
pub const MESH_PRODUCT_SUB_ID: u32 = 1;

/// The product formats this crate owns, for a host contribution to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 3] {
    [
        ProductFormatRegistration::for_format::<MeshProductFormat>(),
        ProductFormatRegistration::for_format::<SkeletonProductFormat>(),
        ProductFormatRegistration::for_format::<SkinnedMeshProductFormat>(),
    ]
}

/// The glTF authoring schemas this crate owns.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 3] {
    [
        SourceSchemaRegistration::for_source::<MeshSourceFormat>()
            .with_label("Mesh")
            .with_category("Rendering")
            .with_editable_file("meshes", &["gltf", "glb"]),
        SourceSchemaRegistration::for_source::<SkeletonSourceFormat>()
            .with_label("Skeleton")
            .with_category("Animation")
            .with_editable_file("characters", &["skeleton.glb"]),
        SourceSchemaRegistration::for_source::<SkinnedMeshSourceFormat>()
            .with_label("Skinned Mesh")
            .with_category("Rendering")
            .with_editable_file("characters", &["skinnedmesh.glb"]),
    ]
}

/// The mesh, skeleton, and skinned-mesh build rules.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 3] {
    [
        BuildRuleRegistration::new(MESH_BUILDER_NAME, MESH_BUILDER_ID, mesh_build_rule)
            .with_priority(80),
        BuildRuleRegistration::new(
            SKELETON_BUILDER_NAME,
            SKELETON_BUILDER_ID,
            skeleton_build_rule,
        )
        .with_priority(90),
        BuildRuleRegistration::new(
            SKINNED_MESH_BUILDER_NAME,
            SKINNED_MESH_BUILDER_ID,
            skinned_mesh_build_rule,
        )
        .with_priority(90),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

#[must_use]
pub fn mesh_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<MeshSourceFormat>()
        .named(MESH_BUILDER_NAME)
        .id(MESH_BUILDER_ID)
        .version(2)
        .analysis_fingerprint(fingerprint::mesh_analysis_fingerprint())
        .produces::<MeshProductFormat>()
        .create_jobs(create_mesh_jobs)
        .process(process_mesh_job)
}

fn create_mesh_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let dependencies = match gltf_source_dependencies(&request.source_path, request.source_bytes) {
        Ok(dependencies) => dependencies
            .into_iter()
            .map(SourceFileDependency::Path)
            .collect(),
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "mesh job discovery failed");
            return CreateJobsResponse {
                result: CreateJobsResult::Failed,
                ..Default::default()
            };
        }
    };
    CreateJobsResponse {
        jobs: request
            .platforms
            .iter()
            .copied()
            .map(JobDescriptor::default_for_platform)
            .collect(),
        source_dependencies: dependencies,
        result: CreateJobsResult::Success,
    }
}

fn process_mesh_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let imported = match import_gltf(
        request.source_root,
        &request.source_path,
        request.source_bytes,
    ) {
        Ok(imported) => imported,
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "mesh import failed");
            return failed_process_response();
        }
    };
    let bytes = match encode_mesh_asset(&imported.asset) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "mesh encoding failed");
            return failed_process_response();
        }
    };
    let product = match BuildProduct::for_format::<MeshProductFormat>(
        mesh_product_path(&request.source_path),
        MESH_PRODUCT_SUB_ID,
        bytes,
    ) {
        Ok(product) => product,
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "mesh product path failed");
            return failed_process_response();
        }
    };
    let product_dependencies = match imported
        .material_sources
        .iter()
        .map(|source| {
            resolve_referenced_product_id(
                request.source_root,
                source,
                az_material_builder::MATERIAL_PRODUCT_SUB_ID,
            )
            .map(|dependency_id| {
                (
                    0,
                    ProductDependency {
                        dependency_id,
                        flags: ProductDependencyFlags::default(),
                    },
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(dependencies) => dependencies,
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "mesh material identity failed");
            return failed_process_response();
        }
    };
    ProcessJobResponse {
        products: vec![product],
        product_dependencies,
        result: ProcessJobResult::Success,
    }
}

fn failed_process_response() -> ProcessJobResponse {
    ProcessJobResponse {
        result: ProcessJobResult::Failed,
        ..Default::default()
    }
}

#[must_use]
pub fn mesh_product_path(source_path: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    let stem = normalized
        .strip_suffix(".gltf")
        .or_else(|| normalized.strip_suffix(".glb"))
        .unwrap_or(normalized.as_str());
    format!("{stem}.{MESH_PRODUCT_EXTENSION}")
}

#[cfg(feature = "bevy")]
pub struct MeshAssetPlugin;

#[cfg(feature = "bevy")]
impl bevy::app::Plugin for MeshAssetPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        use bevy::asset::AssetApp as _;
        app.init_asset::<MeshAsset>()
            .init_asset_loader::<loader::MeshAssetLoader>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{BuildRuleRegistry, composed_product_format, composed_source_schemas};
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, declare_caps,
    };

    const OWNER: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.mesh-builder-tests"),
        contribution: ContributionId::new("builders"),
        roles: &[],
    };

    declare_caps!(MeshBuilderCaps:);

    struct Meshes;

    impl Contribution for Meshes {
        type Caps = MeshBuilderCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            OWNER
        }

        fn register(&self, ctx: &mut GemContext<'_, MeshBuilderCaps>) {
            register(ctx);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Meshes, ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    #[test]
    fn mesh_pipeline_registers_source_product_and_rule() {
        let composer = composed();
        let registries = composer.registries();

        assert!(
            composed_source_schemas(registries)
                .iter()
                .any(|schema| schema.entry.schema_type() == MESH_SOURCE_SCHEMA)
        );
        assert!(composed_product_format(registries, MESH_FORMAT_ID).is_some());

        let registry = BuildRuleRegistry::compose(&JobContext::new(registries));
        assert!(registry.iter().any(|rule| rule.id == MESH_BUILDER_ID));
    }

    #[test]
    fn every_mesh_rule_dispatches_under_its_own_id() {
        let composer = composed();
        let registries = composer.registries();
        let registry = BuildRuleRegistry::compose(&JobContext::new(registries));
        let ids = registry.iter().map(|rule| rule.id).collect::<Vec<_>>();

        // Skeleton and skinned-mesh outrank the broad `*.glb` mesh rule, so a
        // rigged source reaches its own builder rather than the mesh codec.
        assert_eq!(
            ids,
            [
                SKELETON_BUILDER_ID,
                SKINNED_MESH_BUILDER_ID,
                MESH_BUILDER_ID
            ]
        );
        assert_eq!(
            registry
                .matching_source(
                    "characters/hero.skeleton.glb",
                    Some(SKELETON_SOURCE_SCHEMA_NAME)
                )
                .map(|rule| rule.id)
                .collect::<Vec<_>>(),
            [SKELETON_BUILDER_ID]
        );
        assert_eq!(
            registry
                .matching_source("meshes/crate.glb", Some(MESH_SOURCE_SCHEMA_NAME))
                .map(|rule| rule.id)
                .collect::<Vec<_>>(),
            [MESH_BUILDER_ID]
        );
    }

    /// The registration's name and id are the compose key and the job key;
    /// the factory sets them again on the resolved rule. A registration paired
    /// with the wrong factory would file jobs under a rule that never ran, so
    /// assert the pairing rather than reading the three lists side by side.
    #[test]
    fn every_registration_names_the_rule_its_factory_builds() {
        let registries = az_gem_contract::Registries::new();
        let context = JobContext::new(&registries);

        for registration in build_rules() {
            let rule = registration.rule(&context);
            assert_eq!(registration.name(), rule.name);
            assert_eq!(registration.id(), rule.id);
        }
    }

    #[test]
    fn mesh_rule_claims_gltf_and_glb_only() {
        let registries = az_gem_contract::Registries::new();
        let rule = mesh_build_rule(&JobContext::new(&registries));
        assert!(rule.matches_source("meshes/test.glb", Some(MESH_SOURCE_SCHEMA_NAME)));
        assert!(rule.matches_source("meshes/test.gltf", Some(MESH_SOURCE_SCHEMA_NAME)));
        assert!(!rule.matches_source("meshes/test.azmesh", Some(MESH_SOURCE_SCHEMA_NAME)));
    }

    #[test]
    fn mesh_product_path_replaces_source_extension() {
        assert_eq!(mesh_product_path("meshes/test.glb"), "meshes/test.azmesh");
        assert_eq!(mesh_product_path("models/test.gltf"), "models/test.azmesh");
    }

    #[test]
    fn gltf_external_buffers_are_source_dependencies() {
        let bytes = br#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"triangle.bin","byteLength":42}]
        }"#;
        assert_eq!(
            gltf_source_dependencies("meshes/triangle.gltf", bytes).unwrap(),
            ["meshes/triangle.bin"]
        );
    }
}
