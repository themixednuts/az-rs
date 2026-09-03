//! Native terrain asset-builder integration.
//!
//! Terrain authored documents are editor/source data. This crate processes them
//! into runtime terrain products owned by `az-terrain-runtime`.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

mod fingerprint;
mod source_document;

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
use az_core::{AssetData, AssetType, ReflectedValueEncoding, ReflectedValueEnvelope};
use az_terrain::{
    TERRAIN_HEIGHTMAP_EXTENSION, TERRAIN_HEIGHTMAP_PATH_PREFIX, TERRAIN_HEIGHTMAP_SCHEMA_NAME,
    TERRAIN_LAYER_SET_EXTENSION, TERRAIN_LAYER_SET_PATH_PREFIX, TERRAIN_LAYER_SET_SCHEMA_NAME,
    TERRAIN_REGION_EXTENSION, TERRAIN_REGION_PATH_PREFIX, TERRAIN_REGION_SCHEMA_NAME,
    TERRAIN_SOURCE_ROOT, TERRAIN_WORLD_EXTENSION, TERRAIN_WORLD_PATH_PREFIX,
    TERRAIN_WORLD_SCHEMA_NAME, TerrainHeightmapSource, TerrainLayerSetSource, TerrainRegionSource,
    TerrainWorldSource,
};
pub use az_terrain_runtime::{
    SurfaceTag, TERRAIN_HEIGHTMAP_MAGIC, TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION,
    TERRAIN_HEIGHTMAP_PRODUCT_VERSION, TERRAIN_LAYER_SET_MAGIC,
    TERRAIN_LAYER_SET_PRODUCT_EXTENSION, TERRAIN_LAYER_SET_PRODUCT_VERSION, TERRAIN_REGION_MAGIC,
    TERRAIN_REGION_PRODUCT_EXTENSION, TERRAIN_REGION_PRODUCT_VERSION, TERRAIN_WORLD_MAGIC,
    TERRAIN_WORLD_PRODUCT_EXTENSION, TERRAIN_WORLD_PRODUCT_VERSION, TerrainAssetCodecError,
    TerrainBounds, TerrainConstantHeightSource, TerrainCoord, TerrainHeightGraphSource,
    TerrainHeightImageSource, TerrainHeightRange, TerrainHeightSource, TerrainHeightTilesSource,
    TerrainHeightmapAsset, TerrainImageChannel, TerrainLayer, TerrainLayerSetAsset,
    TerrainRegionAsset, TerrainRegionRef, TerrainResolution, TerrainSurfaceChannel,
    TerrainSurfaceGraphSource, TerrainSurfaceImageSource, TerrainSurfaceSource,
    TerrainSurfaceWeightsSource, TerrainWorldAsset, decode_terrain_heightmap_asset,
    decode_terrain_layer_set_asset, decode_terrain_region_asset, decode_terrain_world_asset,
};
pub use source_document::{
    TerrainProcessError, TerrainProcessedProduct, TerrainSourceDocumentArtifact,
    TerrainSourceDocumentError, collect_terrain_source_dependencies,
    encode_terrain_heightmap_source_document, encode_terrain_layer_set_source_document,
    encode_terrain_region_source_document, encode_terrain_world_source_document,
    process_terrain_source, terrain_heightmap_source_document_artifact,
    terrain_heightmap_source_path, terrain_layer_set_source_document_artifact,
    terrain_region_coord_segment, terrain_region_display_name, terrain_region_layers_source_path,
    terrain_region_source_document_artifact, terrain_region_source_path,
    terrain_runtime_asset_path, terrain_source_segment, terrain_world_layers_source_path,
    terrain_world_source_document_artifact, terrain_world_source_path,
};
use uuid::uuid;

pub struct TerrainWorldSourceFormat;

impl SourceFormat for TerrainWorldSourceFormat {
    const SCHEMA: Option<SourceSchemaType> =
        Some(SourceSchemaType::__from_static(TERRAIN_WORLD_SCHEMA_NAME));
    const SCHEMA_TYPES: &'static [SourceSchemaType] =
        &[SourceSchemaType::__from_static(TERRAIN_WORLD_SCHEMA_NAME)];
    const PATTERNS: &'static [AssetBuilderPattern] =
        &[AssetBuilderPattern::static_wildcard("*.azterrain.ron")];
}

pub struct TerrainRegionSourceFormat;

impl SourceFormat for TerrainRegionSourceFormat {
    const SCHEMA: Option<SourceSchemaType> =
        Some(SourceSchemaType::__from_static(TERRAIN_REGION_SCHEMA_NAME));
    const SCHEMA_TYPES: &'static [SourceSchemaType] =
        &[SourceSchemaType::__from_static(TERRAIN_REGION_SCHEMA_NAME)];
    const PATTERNS: &'static [AssetBuilderPattern] = &[AssetBuilderPattern::static_wildcard(
        "*.azterrain-region.ron",
    )];
}

pub struct TerrainLayerSetSourceFormat;

impl SourceFormat for TerrainLayerSetSourceFormat {
    const SCHEMA: Option<SourceSchemaType> = Some(SourceSchemaType::__from_static(
        TERRAIN_LAYER_SET_SCHEMA_NAME,
    ));
    const SCHEMA_TYPES: &'static [SourceSchemaType] = &[SourceSchemaType::__from_static(
        TERRAIN_LAYER_SET_SCHEMA_NAME,
    )];
    const PATTERNS: &'static [AssetBuilderPattern] = &[AssetBuilderPattern::static_wildcard(
        "*.azterrain-layers.ron",
    )];
}

pub struct TerrainHeightmapSourceFormat;

impl SourceFormat for TerrainHeightmapSourceFormat {
    const SCHEMA: Option<SourceSchemaType> = Some(SourceSchemaType::__from_static(
        TERRAIN_HEIGHTMAP_SCHEMA_NAME,
    ));
    const SCHEMA_TYPES: &'static [SourceSchemaType] = &[SourceSchemaType::__from_static(
        TERRAIN_HEIGHTMAP_SCHEMA_NAME,
    )];
    const PATTERNS: &'static [AssetBuilderPattern] = &[AssetBuilderPattern::static_wildcard(
        "*.azterrain-heightmap.ron",
    )];
}

pub struct TerrainSourceFormat;

impl SourceFormat for TerrainSourceFormat {
    const SCHEMA: Option<SourceSchemaType> = None;
    const SCHEMA_TYPES: &'static [SourceSchemaType] = TERRAIN_SOURCE_SCHEMAS;
    const PATTERNS: &'static [AssetBuilderPattern] = &[
        AssetBuilderPattern::static_wildcard("*.azterrain.ron"),
        AssetBuilderPattern::static_wildcard("*.azterrain-region.ron"),
        AssetBuilderPattern::static_wildcard("*.azterrain-layers.ron"),
        AssetBuilderPattern::static_wildcard("*.azterrain-heightmap.ron"),
    ];
}

pub struct TerrainWorldProductFormat;

impl ProductFormat for TerrainWorldProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static("azoth.terrain.world");
    const VERSION: u32 = TERRAIN_WORLD_PRODUCT_VERSION;
    type Asset = TerrainWorldAsset;
}

pub struct TerrainRegionProductFormat;

impl ProductFormat for TerrainRegionProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static("azoth.terrain.region");
    const VERSION: u32 = TERRAIN_REGION_PRODUCT_VERSION;
    type Asset = TerrainRegionAsset;
}

pub struct TerrainLayerSetProductFormat;

impl ProductFormat for TerrainLayerSetProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static("azoth.terrain.layer-set");
    const VERSION: u32 = TERRAIN_LAYER_SET_PRODUCT_VERSION;
    type Asset = TerrainLayerSetAsset;
}

pub struct TerrainHeightmapProductFormat;

impl ProductFormat for TerrainHeightmapProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static("azoth.terrain.heightmap");
    const VERSION: u32 = TERRAIN_HEIGHTMAP_PRODUCT_VERSION;
    type Asset = TerrainHeightmapAsset;
}

pub const TERRAIN_WORLD_FORMAT_ID: ProductFormatId =
    product_format_id::<TerrainWorldProductFormat>();
pub const TERRAIN_REGION_FORMAT_ID: ProductFormatId =
    product_format_id::<TerrainRegionProductFormat>();
pub const TERRAIN_LAYER_SET_FORMAT_ID: ProductFormatId =
    product_format_id::<TerrainLayerSetProductFormat>();
pub const TERRAIN_HEIGHTMAP_FORMAT_ID: ProductFormatId =
    product_format_id::<TerrainHeightmapProductFormat>();

pub const TERRAIN_WORLD_ASSET_TYPE_NAME: &str = "azoth.terrain.world";
pub const TERRAIN_REGION_ASSET_TYPE_NAME: &str = "azoth.terrain.region";
pub const TERRAIN_LAYER_SET_ASSET_TYPE_NAME: &str = "azoth.terrain.layer-set";
pub const TERRAIN_HEIGHTMAP_ASSET_TYPE_NAME: &str = "azoth.terrain.heightmap";

pub const TERRAIN_WORLD_ASSET_TYPE: AssetType = TerrainWorldAsset::ASSET_TYPE;
pub const TERRAIN_REGION_ASSET_TYPE: AssetType = TerrainRegionAsset::ASSET_TYPE;
pub const TERRAIN_LAYER_SET_ASSET_TYPE: AssetType = TerrainLayerSetAsset::ASSET_TYPE;
pub const TERRAIN_HEIGHTMAP_ASSET_TYPE: AssetType = TerrainHeightmapAsset::ASSET_TYPE;

pub const TERRAIN_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("58c3d7b2-bb57-4d30-bf9a-c4079150f3a1"));
pub const TERRAIN_BUILD_JOB_KEY: &str = "cook-terrain";
pub const TERRAIN_PRODUCT_SUB_ID: u32 = 1;

pub const TERRAIN_WORLD_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<TerrainWorldSourceFormat>();
pub const TERRAIN_REGION_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<TerrainRegionSourceFormat>();
pub const TERRAIN_LAYER_SET_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<TerrainLayerSetSourceFormat>();
pub const TERRAIN_HEIGHTMAP_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<TerrainHeightmapSourceFormat>();
pub const TERRAIN_SOURCE_SCHEMAS: &[SourceSchemaType] = &[
    TERRAIN_WORLD_SOURCE_SCHEMA,
    TERRAIN_REGION_SOURCE_SCHEMA,
    TERRAIN_LAYER_SET_SOURCE_SCHEMA,
    TERRAIN_HEIGHTMAP_SOURCE_SCHEMA,
];

fn load_terrain_world_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditLoadRequest<'_>,
) -> SourceFileEditLoadResult {
    reflected_source_document::<TerrainWorldSource>(TERRAIN_WORLD_SCHEMA_NAME, request.bytes)
}

fn save_terrain_world_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditSaveRequest<'_>,
) -> SourceFileEditSaveResult {
    save_reflected_source::<TerrainWorldSource>(TERRAIN_WORLD_SCHEMA_NAME, request)
}

fn load_terrain_region_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditLoadRequest<'_>,
) -> SourceFileEditLoadResult {
    reflected_source_document::<TerrainRegionSource>(TERRAIN_REGION_SCHEMA_NAME, request.bytes)
}

fn save_terrain_region_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditSaveRequest<'_>,
) -> SourceFileEditSaveResult {
    save_reflected_source::<TerrainRegionSource>(TERRAIN_REGION_SCHEMA_NAME, request)
}

fn load_terrain_layer_set_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditLoadRequest<'_>,
) -> SourceFileEditLoadResult {
    reflected_source_document::<TerrainLayerSetSource>(TERRAIN_LAYER_SET_SCHEMA_NAME, request.bytes)
}

fn save_terrain_layer_set_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditSaveRequest<'_>,
) -> SourceFileEditSaveResult {
    save_reflected_source::<TerrainLayerSetSource>(TERRAIN_LAYER_SET_SCHEMA_NAME, request)
}

fn load_terrain_heightmap_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditLoadRequest<'_>,
) -> SourceFileEditLoadResult {
    reflected_source_document::<TerrainHeightmapSource>(
        TERRAIN_HEIGHTMAP_SCHEMA_NAME,
        request.bytes,
    )
}

fn save_terrain_heightmap_source(
    _context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditSaveRequest<'_>,
) -> SourceFileEditSaveResult {
    save_reflected_source::<TerrainHeightmapSource>(TERRAIN_HEIGHTMAP_SCHEMA_NAME, request)
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
            "terrain source value is not typed RON",
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
pub const fn product_formats() -> [ProductFormatRegistration; 4] {
    [
        ProductFormatRegistration::for_format::<TerrainWorldProductFormat>(),
        ProductFormatRegistration::for_format::<TerrainRegionProductFormat>(),
        ProductFormatRegistration::for_format::<TerrainLayerSetProductFormat>(),
        ProductFormatRegistration::for_format::<TerrainHeightmapProductFormat>(),
    ]
}

/// The source schemas this crate owns, for a composing host to register.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 4] {
    [
        SourceSchemaRegistration::for_source::<TerrainWorldSourceFormat>()
            .with_label("Terrain World")
            .with_category("Terrain")
            .with_editable_file(TERRAIN_WORLD_PATH_PREFIX, &[TERRAIN_WORLD_EXTENSION]),
        SourceSchemaRegistration::for_source::<TerrainRegionSourceFormat>()
            .with_label("Terrain Region")
            .with_category("Terrain")
            .with_editable_file(TERRAIN_REGION_PATH_PREFIX, &[TERRAIN_REGION_EXTENSION]),
        SourceSchemaRegistration::for_source::<TerrainLayerSetSourceFormat>()
            .with_label("Terrain Layer Set")
            .with_category("Terrain")
            .with_editable_file(
                TERRAIN_LAYER_SET_PATH_PREFIX,
                &[TERRAIN_LAYER_SET_EXTENSION],
            ),
        SourceSchemaRegistration::for_source::<TerrainHeightmapSourceFormat>()
            .with_label("Terrain Heightmap")
            .with_category("Terrain")
            .with_editable_file(
                TERRAIN_HEIGHTMAP_PATH_PREFIX,
                &[TERRAIN_HEIGHTMAP_EXTENSION],
            ),
    ]
}

/// The source-file edit codecs this crate owns, for a composing host to
/// register.
#[must_use]
pub fn source_file_edit_codecs() -> [SourceFileEditCodecRegistration; 4] {
    [
        SourceFileEditCodecRegistration::for_source::<TerrainWorldSourceFormat>(
            load_terrain_world_source,
            save_terrain_world_source,
        ),
        SourceFileEditCodecRegistration::for_source::<TerrainRegionSourceFormat>(
            load_terrain_region_source,
            save_terrain_region_source,
        ),
        SourceFileEditCodecRegistration::for_source::<TerrainLayerSetSourceFormat>(
            load_terrain_layer_set_source,
            save_terrain_layer_set_source,
        ),
        SourceFileEditCodecRegistration::for_source::<TerrainHeightmapSourceFormat>(
            load_terrain_heightmap_source,
            save_terrain_heightmap_source,
        ),
    ]
}

/// The build rules this crate owns, for a composing host to register.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 1] {
    [
        BuildRuleRegistration::new("azoth.terrain", TERRAIN_BUILDER_ID, terrain_build_rule)
            .with_priority(75),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<SourceFileEditCodecRegistration>()
        .register_many(source_file_edit_codecs());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

#[must_use]
pub fn terrain_build_rule(_: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<TerrainSourceFormat>()
        .named("azoth.terrain")
        .id(TERRAIN_BUILDER_ID)
        .version(2)
        .analysis_fingerprint(fingerprint::terrain_analysis_fingerprint())
        .produces_dynamic_products()
        .create_jobs(create_terrain_jobs)
        .process(process_terrain_job)
}

#[must_use]
pub const fn terrain_source_patterns() -> &'static [AssetBuilderPattern] {
    <TerrainSourceFormat as SourceFormat>::PATTERNS
}

#[must_use]
pub fn is_terrain_source_schema(schema_type: &str) -> bool {
    TERRAIN_SOURCE_SCHEMAS
        .iter()
        .any(|schema| schema.as_str() == schema_type)
}

#[must_use]
pub fn terrain_product_path(source_path: &str, product_extension: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    let stem = strip_known_terrain_source_extension(&normalized).unwrap_or(normalized.as_str());
    if stem == "terrain" || stem.starts_with("terrain/") {
        format!("{stem}.{product_extension}")
    } else {
        format!("terrain/{stem}.{product_extension}")
    }
}

fn strip_known_terrain_source_extension(path: &str) -> Option<&str> {
    for extension in [
        TERRAIN_WORLD_EXTENSION,
        TERRAIN_REGION_EXTENSION,
        TERRAIN_LAYER_SET_EXTENSION,
        TERRAIN_HEIGHTMAP_EXTENSION,
    ] {
        if let Some(stem) = path.strip_suffix(extension) {
            return stem.strip_suffix('.');
        }
    }
    None
}

fn create_terrain_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let Some(source_schema_type) = request.source_schema_type else {
        return skipped_jobs();
    };
    if !is_terrain_source_schema(source_schema_type) {
        return skipped_jobs();
    }

    let dependencies =
        match collect_terrain_source_dependencies(source_schema_type, request.source_bytes) {
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
        jobs: request
            .platforms
            .iter()
            .map(|platform| JobDescriptor {
                job_key: az_asset_builder::JobKey::new(TERRAIN_BUILD_JOB_KEY)
                    .expect("terrain job key is static and valid"),
                platform: *platform,
                job_dependencies: Vec::new(),
                critical: false,
            })
            .collect(),
        source_dependencies: dependencies,
        result: CreateJobsResult::Success,
    }
}

fn process_terrain_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let Some(source_schema_type) = request.source_schema_type else {
        return failed_process_response();
    };
    let Ok(dependencies) =
        collect_terrain_source_dependencies(source_schema_type, request.source_bytes)
    else {
        return failed_process_response();
    };
    let Ok(product) = process_terrain_source(
        &request.source_path,
        source_schema_type,
        request.source_bytes,
    ) else {
        return failed_process_response();
    };
    // The terrain rule handles a closed set of typed source descriptors; the
    // concrete product descriptor is selected after decoding the source schema.
    let Ok(product) = BuildProduct::from_dynamic_parts(
        product.product_path,
        product.asset_type,
        product.format,
        product.format_version,
        product.sub_id,
        product.bytes,
    ) else {
        return failed_process_response();
    };

    let product_dependencies = match dependencies
        .into_iter()
        .map(|source| {
            resolve_referenced_product_id(request.source_root, &source, TERRAIN_PRODUCT_SUB_ID).map(
                |dependency_id| {
                    (
                        0,
                        ProductDependency {
                            dependency_id,
                            flags: ProductDependencyFlags::default(),
                        },
                    )
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(dependencies) => dependencies,
        Err(error) => {
            tracing::warn!(source = %request.source_path, %error, "terrain dependency identity failed");
            return failed_process_response();
        }
    };

    ProcessJobResponse {
        products: vec![product],
        product_dependencies,
        result: ProcessJobResult::Success,
    }
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

#[must_use]
pub fn terrain_source_document_prefix(schema_type: SourceSchemaType) -> Option<&'static str> {
    if schema_type == TERRAIN_WORLD_SOURCE_SCHEMA {
        Some(TERRAIN_WORLD_PATH_PREFIX)
    } else if schema_type == TERRAIN_REGION_SOURCE_SCHEMA {
        Some(TERRAIN_REGION_PATH_PREFIX)
    } else if schema_type == TERRAIN_LAYER_SET_SOURCE_SCHEMA {
        Some(TERRAIN_LAYER_SET_PATH_PREFIX)
    } else if schema_type == TERRAIN_HEIGHTMAP_SOURCE_SCHEMA {
        Some(TERRAIN_HEIGHTMAP_PATH_PREFIX)
    } else {
        None
    }
}

#[must_use]
pub const fn terrain_source_root() -> &'static str {
    TERRAIN_SOURCE_ROOT
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::{
        BuildRuleRegistry, composed_product_formats, composed_source_file_edit_codecs,
        composed_source_schemas,
    };
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, Registries, declare_caps,
    };

    declare_caps!(TerrainCaps:);

    const OWNER: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.terrain-builder-tests"),
        contribution: ContributionId::new("assets"),
        roles: &[],
    };

    /// This crate contributed the way a host's glue contributes it.
    struct Terrain;

    impl Contribution for Terrain {
        type Caps = TerrainCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            OWNER
        }

        fn register(&self, ctx: &mut GemContext<'_, TerrainCaps>) {
            super::register(ctx);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Terrain, ProductActivation::default())
            .expect("terrain registrations require no host capability");
        composer
    }

    #[test]
    fn every_family_reaches_the_composed_host() {
        let composer = composed();
        let registries = composer.registries();

        let formats = composed_product_formats(registries)
            .into_iter()
            .map(|registration| registration.entry.id())
            .collect::<Vec<_>>();
        assert!(formats.contains(&TERRAIN_WORLD_FORMAT_ID));
        assert!(formats.contains(&TERRAIN_REGION_FORMAT_ID));
        assert!(formats.contains(&TERRAIN_LAYER_SET_FORMAT_ID));
        assert!(formats.contains(&TERRAIN_HEIGHTMAP_FORMAT_ID));

        let schemas = composed_source_schemas(registries)
            .into_iter()
            .map(|registration| registration.entry.schema_type())
            .collect::<Vec<_>>();
        for schema in TERRAIN_SOURCE_SCHEMAS {
            assert!(schemas.contains(schema), "{schema} is not composed");
        }

        let codecs = composed_source_file_edit_codecs(registries)
            .into_iter()
            .map(|registration| registration.entry.schema_type())
            .collect::<Vec<_>>();
        for schema in TERRAIN_SOURCE_SCHEMAS {
            assert!(codecs.contains(schema), "{schema} has no composed codec");
        }

        let rules = BuildRuleRegistry::compose(&JobContext::new(registries));
        let rule = rules
            .iter()
            .find(|rule| rule.id == TERRAIN_BUILDER_ID)
            .expect("terrain build rule is composed");
        assert!(rule.matches_source(
            "terrain/overworld.azterrain.ron",
            Some(TERRAIN_WORLD_SCHEMA_NAME)
        ));
    }

    #[test]
    fn registration_identity_matches_the_rule_it_resolves() {
        let registries = Registries::new();
        let context = JobContext::new(&registries);
        for registration in build_rules() {
            let rule = registration.rule(&context);
            assert_eq!(registration.name(), rule.name);
            assert_eq!(registration.id(), rule.id);
        }
    }
}
