//! Native material asset-builder integration.
//!
//! Material property documents are editor/source data owned by `az-material`.
//! This engine-owned crate processes them into compiled property-table products
//! (adopted decision 3 — renderer consumption and the graph/shader path are
//! explicitly out of scope for v1):
//!
//! - `azoth.material.MaterialType` source -> [`MaterialTypeAsset`] product
//!   (`azoth.material.type`, `materials/**.azmaterialtype`);
//! - `azoth.material.Material` source -> [`MaterialAsset`] product
//!   (`azoth.material.material`, `materials/**.azmaterial`) carrying resolved
//!   property bindings plus the compiled material-type reference.
//!
//! The decode-only runtime loaders live behind the `bevy` feature and reuse
//! the typed structs from `az_material::value`.

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
mod compile;
mod fingerprint;

#[cfg(feature = "bevy")]
pub mod loader;

use az_asset_builder::{
    AssetBuilderPattern, BuildProduct, BuildRule, BuildRuleRegistration, BuilderId,
    CreateJobsRequest, CreateJobsResponse, CreateJobsResult, JobContext, JobDescriptor,
    ProcessJobRequest, ProcessJobResponse, ProcessJobResult, ProductDependency,
    ProductDependencyFlags, ProductFormat, ProductFormatId, ProductFormatRegistration,
    SourceFileDependency, SourceFileEditCodecContext, SourceFileEditCodecRegistration,
    SourceFileEditDocument, SourceFileEditError, SourceFileEditLoadRequest,
    SourceFileEditLoadResult, SourceFileEditSaveRequest, SourceFileEditSaveResult, SourceFormat,
    SourceSchemaRegistration, SourceSchemaType, product_format_id, resolve_referenced_product_id,
    source_schema_type,
};
use az_core::{
    AssetData, AssetType, AssetTypeRegistration, AzRtti, AzTypeInfo, ReflectedValueEncoding,
    ReflectedValueEnvelope,
};
use az_material::{
    MATERIAL_EXTENSION, MATERIAL_PATH_PREFIX, MATERIAL_SCHEMA_NAME, MATERIAL_TYPE_EXTENSION,
    MATERIAL_TYPE_PATH_PREFIX, MATERIAL_TYPE_SCHEMA_NAME, MaterialSource, MaterialTypeSource,
};
use uuid::uuid;

pub use asset::{MaterialAsset, MaterialTypeAsset};
pub use codec::{
    MaterialAssetCodecError, decode_material_asset, decode_material_type_asset,
    encode_material_asset, encode_material_type_asset,
};
pub use compile::{
    CompiledMaterial, CompiledMaterialType, MaterialCompileError, compile_material_document,
    compile_material_type_document, material_product_path, material_type_product_path,
};

/// On-disk magic tag for compiled material-type products.
pub const MATERIAL_TYPE_MAGIC: &[u8; 8] = b"AZMTLT\0\0";
/// Current writer version for the compiled material-type byte format.
pub const MATERIAL_TYPE_PRODUCT_VERSION: u32 = 1;
/// Catalog product-path extension for compiled material-type products.
pub const MATERIAL_TYPE_PRODUCT_EXTENSION: &str = "azmaterialtype";

/// On-disk magic tag for compiled material products.
pub const MATERIAL_MAGIC: &[u8; 8] = b"AZMTLI\0\0";
/// Current writer version for the compiled material byte format.
pub const MATERIAL_PRODUCT_VERSION: u32 = 1;
/// Catalog product-path extension for compiled material products.
pub const MATERIAL_PRODUCT_EXTENSION: &str = "azmaterial";

pub struct MaterialTypeAssetData;

impl AzTypeInfo for MaterialTypeAssetData {
    const NAME: &'static str = "Azoth::MaterialTypeAsset";
    const TYPE_ID: uuid::Uuid = uuid!("c4864388-ddc0-43cc-9450-c7b7aadeeaff");
}

impl AzRtti for MaterialTypeAssetData {}

impl AssetData for MaterialTypeAssetData {
    const STABLE_NAME: &'static str = "azoth.material.type";
}

pub struct MaterialAssetData;

impl AzTypeInfo for MaterialAssetData {
    const NAME: &'static str = "Azoth::MaterialAsset";
    const TYPE_ID: uuid::Uuid = uuid!("a3f3caac-e0f2-479c-9094-054047a4eade");
}

impl AzRtti for MaterialAssetData {}

impl AssetData for MaterialAssetData {
    const STABLE_NAME: &'static str = "azoth.material.material";
}

pub struct MaterialTypeSourceFormat;

impl SourceFormat for MaterialTypeSourceFormat {
    const SCHEMA: Option<SourceSchemaType> =
        Some(SourceSchemaType::__from_static(MATERIAL_TYPE_SCHEMA_NAME));
    const SCHEMA_TYPES: &'static [SourceSchemaType] =
        &[SourceSchemaType::__from_static(MATERIAL_TYPE_SCHEMA_NAME)];
    const PATTERNS: &'static [AssetBuilderPattern] =
        &[AssetBuilderPattern::static_wildcard("*.azmaterialtype.ron")];
}

pub struct MaterialSourceFormat;

impl SourceFormat for MaterialSourceFormat {
    const SCHEMA: Option<SourceSchemaType> =
        Some(SourceSchemaType::__from_static(MATERIAL_SCHEMA_NAME));
    const SCHEMA_TYPES: &'static [SourceSchemaType] =
        &[SourceSchemaType::__from_static(MATERIAL_SCHEMA_NAME)];
    const PATTERNS: &'static [AssetBuilderPattern] =
        &[AssetBuilderPattern::static_wildcard("*.azmaterial.ron")];
}

pub struct MaterialTypeProductFormat;

impl ProductFormat for MaterialTypeProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static("azoth.material.type");
    const VERSION: u32 = MATERIAL_TYPE_PRODUCT_VERSION;
    type Asset = MaterialTypeAssetData;
}

pub struct MaterialProductFormat;

impl ProductFormat for MaterialProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static("azoth.material.material");
    const VERSION: u32 = MATERIAL_PRODUCT_VERSION;
    type Asset = MaterialAssetData;
}

pub const MATERIAL_TYPE_FORMAT_ID: ProductFormatId =
    product_format_id::<MaterialTypeProductFormat>();
pub const MATERIAL_FORMAT_ID: ProductFormatId = product_format_id::<MaterialProductFormat>();

pub const MATERIAL_TYPE_ASSET_TYPE_NAME: &str = "azoth.material.type";
pub const MATERIAL_ASSET_TYPE_NAME: &str = "azoth.material.material";

pub const MATERIAL_TYPE_ASSET_TYPE: AssetType = MaterialTypeAssetData::ASSET_TYPE;
pub const MATERIAL_ASSET_TYPE: AssetType = MaterialAssetData::ASSET_TYPE;

pub const MATERIAL_TYPE_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("bb7cb528-fa59-4f10-bb58-ee1e1907d917"));
pub const MATERIAL_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("c998e1e2-280f-4aee-bf38-43a32b159b70"));
pub const MATERIAL_TYPE_PRODUCT_SUB_ID: u32 = 1;
pub const MATERIAL_PRODUCT_SUB_ID: u32 = 1;

pub const MATERIAL_TYPE_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<MaterialTypeSourceFormat>();
pub const MATERIAL_SOURCE_SCHEMA: SourceSchemaType = source_schema_type::<MaterialSourceFormat>();

fn load_material_type_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditLoadRequest<'_>,
) -> SourceFileEditLoadResult {
    reflected_source_document::<MaterialTypeSource>(MATERIAL_TYPE_SCHEMA_NAME, request.bytes)
}

fn save_material_type_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditSaveRequest<'_>,
) -> SourceFileEditSaveResult {
    save_reflected_source::<MaterialTypeSource>(MATERIAL_TYPE_SCHEMA_NAME, request)
}

fn load_material_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditLoadRequest<'_>,
) -> SourceFileEditLoadResult {
    reflected_source_document::<MaterialSource>(MATERIAL_SCHEMA_NAME, request.bytes)
}

fn save_material_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditSaveRequest<'_>,
) -> SourceFileEditSaveResult {
    save_reflected_source::<MaterialSource>(MATERIAL_SCHEMA_NAME, request)
}

fn reflected_source_document<T>(type_path: &'static str, bytes: &[u8]) -> SourceFileEditLoadResult
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let source = ron::de::from_bytes::<T>(bytes)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    let payload = ron::ser::to_string_pretty(&source, ron::ser::PrettyConfig::new())
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    Ok(SourceFileEditDocument::new(
        type_path,
        ReflectedValueEnvelope::typed_ron(type_path, payload),
    ))
}

fn save_reflected_source<T>(
    type_path: &'static str,
    request: &SourceFileEditSaveRequest<'_>,
) -> SourceFileEditSaveResult
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    if request.root_schema != type_path || request.value.type_path != type_path {
        return Err(SourceFileEditError::unsupported(format!(
            "source codec for `{type_path}` cannot save root `{}` with reflected type `{}`",
            request.root_schema, request.value.type_path
        )));
    }
    if request.value.encoding != ReflectedValueEncoding::TypedRon {
        return Err(SourceFileEditError::failed(
            "material source value is not typed RON",
        ));
    }
    let source = ron::de::from_bytes::<T>(&request.value.payload)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    ron::ser::to_string_pretty(&source, ron::ser::PrettyConfig::new())
        .map(String::into_bytes)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// The product formats this crate owns, for a composing host to register.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 2] {
    [
        ProductFormatRegistration::for_format::<MaterialTypeProductFormat>(),
        ProductFormatRegistration::for_format::<MaterialProductFormat>(),
    ]
}

/// The asset types this crate owns, for a composing host to register.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 2] {
    [
        AssetTypeRegistration::for_asset::<MaterialTypeAssetData>()
            .with_owner("az-material-builder"),
        AssetTypeRegistration::for_asset::<MaterialAssetData>().with_owner("az-material-builder"),
    ]
}

/// The source schemas this crate owns, for a composing host to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 2] {
    [
        SourceSchemaRegistration::for_source::<MaterialTypeSourceFormat>()
            .with_label("Material Type")
            .with_category("Rendering")
            .with_editable_file(MATERIAL_TYPE_PATH_PREFIX, &[MATERIAL_TYPE_EXTENSION]),
        SourceSchemaRegistration::for_source::<MaterialSourceFormat>()
            .with_label("Material")
            .with_category("Rendering")
            .with_editable_file(MATERIAL_PATH_PREFIX, &[MATERIAL_EXTENSION]),
    ]
}

/// The source-file edit codecs this crate owns, for a composing host to
/// register.
#[must_use]
pub fn source_file_edit_codecs() -> [SourceFileEditCodecRegistration; 2] {
    [
        SourceFileEditCodecRegistration::for_source::<MaterialTypeSourceFormat>(
            load_material_type_source,
            save_material_type_source,
        ),
        SourceFileEditCodecRegistration::for_source::<MaterialSourceFormat>(
            load_material_source,
            save_material_source,
        ),
    ]
}

/// The build rules this crate owns, for a composing host to register.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 2] {
    [
        BuildRuleRegistration::new(
            "azoth.material.type",
            MATERIAL_TYPE_BUILDER_ID,
            material_type_build_rule,
        )
        .with_priority(75),
        BuildRuleRegistration::new("azoth.material", MATERIAL_BUILDER_ID, material_build_rule)
            .with_priority(75),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<AssetTypeRegistration>()
        .register_many(asset_types());
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<SourceFileEditCodecRegistration>()
        .register_many(source_file_edit_codecs());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

#[must_use]
pub fn material_type_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<MaterialTypeSourceFormat>()
        .named("azoth.material.type")
        .id(MATERIAL_TYPE_BUILDER_ID)
        .version(1)
        .analysis_fingerprint(fingerprint::material_analysis_fingerprint())
        .produces::<MaterialTypeProductFormat>()
        .create_jobs(create_material_type_jobs)
        .process(process_material_type_job)
}

#[must_use]
pub fn material_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<MaterialSourceFormat>()
        .named("azoth.material")
        .id(MATERIAL_BUILDER_ID)
        .version(2)
        .analysis_fingerprint(fingerprint::material_analysis_fingerprint())
        .produces::<MaterialProductFormat>()
        .create_jobs(create_material_jobs)
        .process(process_material_job)
}

fn create_material_type_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    if request.source_schema_type != Some(MATERIAL_TYPE_SCHEMA_NAME) {
        return skipped_jobs();
    }

    let dependencies = match compile::material_type_source_dependencies(request.source_bytes) {
        Ok(dependencies) => dependencies
            .into_iter()
            .map(SourceFileDependency::Path)
            .collect(),
        Err(_) => {
            return CreateJobsResponse {
                result: CreateJobsResult::Failed,
                ..Default::default()
            };
        }
    };

    CreateJobsResponse {
        jobs: default_jobs(request),
        source_dependencies: dependencies,
        result: CreateJobsResult::Success,
    }
}

fn create_material_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    if request.source_schema_type != Some(MATERIAL_SCHEMA_NAME) {
        return skipped_jobs();
    }

    let dependencies = match compile::material_source_dependencies(request.source_bytes) {
        Ok(dependencies) => dependencies
            .into_iter()
            .map(SourceFileDependency::Path)
            .collect(),
        Err(_) => {
            return CreateJobsResponse {
                result: CreateJobsResult::Failed,
                ..Default::default()
            };
        }
    };

    CreateJobsResponse {
        jobs: default_jobs(request),
        source_dependencies: dependencies,
        result: CreateJobsResult::Success,
    }
}

fn process_material_type_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    if request.source_schema_type != Some(MATERIAL_TYPE_SCHEMA_NAME) {
        return failed_process_response();
    }
    let Ok(compiled) = compile_material_type_document(&request.source_path, request.source_bytes)
    else {
        return failed_process_response();
    };
    let Ok(product) = BuildProduct::for_format::<MaterialTypeProductFormat>(
        compiled.product_path,
        MATERIAL_TYPE_PRODUCT_SUB_ID,
        compiled.bytes,
    ) else {
        return failed_process_response();
    };

    ProcessJobResponse {
        products: vec![product],
        product_dependencies: Vec::new(),
        result: ProcessJobResult::Success,
    }
}

fn process_material_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    if request.source_schema_type != Some(MATERIAL_SCHEMA_NAME) {
        return failed_process_response();
    }
    let Ok(compiled) = compile_material_document(&request.source_path, request.source_bytes) else {
        return failed_process_response();
    };
    let Ok(product) = BuildProduct::for_format::<MaterialProductFormat>(
        compiled.product_path,
        MATERIAL_PRODUCT_SUB_ID,
        compiled.bytes,
    ) else {
        return failed_process_response();
    };

    let mut product_dependencies = Vec::new();
    match resolve_referenced_product_id(
        request.source_root,
        &compiled.material_type_source,
        MATERIAL_TYPE_PRODUCT_SUB_ID,
    ) {
        Ok(dependency_id) => product_dependencies.push((
            0,
            ProductDependency {
                dependency_id,
                flags: ProductDependencyFlags::default(),
            },
        )),
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "material type identity failed");
            return failed_process_response();
        }
    }
    if let Some(parent_source) = &compiled.parent_source {
        match resolve_referenced_product_id(
            request.source_root,
            parent_source,
            MATERIAL_PRODUCT_SUB_ID,
        ) {
            Ok(dependency_id) => product_dependencies.push((
                0,
                ProductDependency {
                    dependency_id,
                    flags: ProductDependencyFlags::default(),
                },
            )),
            Err(error) => {
                tracing::warn!(source = %request.source_path, %error, "parent material identity failed");
                return failed_process_response();
            }
        }
    }

    ProcessJobResponse {
        products: vec![product],
        product_dependencies,
        result: ProcessJobResult::Success,
    }
}

fn default_jobs(request: &CreateJobsRequest<'_>) -> Vec<JobDescriptor> {
    request
        .platforms
        .iter()
        .map(|platform| JobDescriptor::default_for_platform(*platform))
        .collect()
}

const fn skipped_jobs() -> CreateJobsResponse {
    CreateJobsResponse {
        jobs: Vec::new(),
        source_dependencies: Vec::new(),
        result: CreateJobsResult::Skipped,
    }
}

const fn failed_process_response() -> ProcessJobResponse {
    ProcessJobResponse {
        products: Vec::new(),
        product_dependencies: Vec::new(),
        result: ProcessJobResult::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{BuildRuleRegistry, composed_product_format};
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, Registries, declare_caps,
    };

    declare_caps!(MaterialCaps:);

    const OWNER: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.material-builder-tests"),
        contribution: ContributionId::new("assets"),
        roles: &[],
    };

    /// This crate contributed the way a host's glue contributes it.
    struct Material;

    impl Contribution for Material {
        type Caps = MaterialCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            OWNER
        }

        fn register(&self, ctx: &mut GemContext<'_, MaterialCaps>) {
            super::register(ctx);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Material, ProductActivation::default())
            .expect("material registrations require no host capability");
        composer
    }

    #[test]
    fn material_product_formats_register_byte_contracts() {
        let composer = composed();
        let registries = composer.registries();

        let material_type = composed_product_format(registries, MATERIAL_TYPE_FORMAT_ID)
            .expect("material type product format is composed");
        assert_eq!(
            material_type.entry.current_version(),
            MATERIAL_TYPE_PRODUCT_VERSION
        );

        let material = composed_product_format(registries, MATERIAL_FORMAT_ID)
            .expect("material product format is composed");
        assert_eq!(material.entry.current_version(), MATERIAL_PRODUCT_VERSION);
    }

    /// Both byte contracts trace back to the contribution that registered
    /// them. That provenance is what the hand-written owner string used to
    /// approximate, and the compose report is the copy that cannot drift.
    #[test]
    fn material_product_formats_are_attributed_to_their_contribution() {
        let report = composed().finalize().expect("composition is valid");
        let formats = report
            .entries
            .iter()
            .filter(|entry| entry.registry == "product-format")
            .collect::<Vec<_>>();

        assert_eq!(formats.len(), product_formats().len());
        assert!(formats.iter().all(|entry| {
            entry.instance.gem.as_str() == "azoth.material-builder-tests"
                && entry.instance.contribution.as_str() == "assets"
        }));
    }

    #[test]
    fn material_rules_claim_disjoint_schema_sets() {
        let registries = Registries::new();
        let context = JobContext::new(&registries);
        let material_type = material_type_build_rule(&context);
        let material = material_build_rule(&context);

        assert!(material_type.matches_source(
            "materials/types/standard.azmaterialtype.ron",
            Some(MATERIAL_TYPE_SCHEMA_NAME)
        ));
        assert!(
            !material_type
                .matches_source("materials/wood.azmaterial.ron", Some(MATERIAL_SCHEMA_NAME))
        );
        assert!(
            material.matches_source("materials/wood.azmaterial.ron", Some(MATERIAL_SCHEMA_NAME))
        );
        assert!(!material.matches_source(
            "materials/types/standard.azmaterialtype.ron",
            Some(MATERIAL_TYPE_SCHEMA_NAME)
        ));
    }

    #[test]
    fn build_rules_are_composed() {
        let composer = composed();
        let registry = BuildRuleRegistry::compose(&JobContext::new(composer.registries()));
        let ids = registry.iter().map(|rule| rule.id).collect::<Vec<_>>();

        assert!(ids.contains(&MATERIAL_TYPE_BUILDER_ID));
        assert!(ids.contains(&MATERIAL_BUILDER_ID));
    }

    #[test]
    fn registration_names_match_the_rules_they_resolve() {
        let registries = Registries::new();
        let context = JobContext::new(&registries);
        for registration in build_rules() {
            assert_eq!(registration.name(), registration.rule(&context).name);
            assert_eq!(registration.id(), registration.rule(&context).id);
        }
    }
}
