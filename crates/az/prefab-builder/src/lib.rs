//! Typed Prefab source processing and asset-builder integration.
//!
//! [`PrefabDocument`](az_prefab::PrefabDocument) source is resolved into a
//! temporary Bevy world and emitted only as the engine-owned AZSCENE product.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

mod azscene;
mod fingerprint;

use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use az_asset_builder::{
    AssetBuilderPattern, BuildProduct, BuildRule, BuildRuleRegistration, BuilderId,
    CreateJobsRequest, CreateJobsResponse, CreateJobsResult, JobContext, JobDescriptor,
    ProcessJobRequest, ProcessJobResponse, ProcessJobResult, ProductDependency, ProductFormat,
    ProductFormatId, ProductFormatRegistration, SourceFileDependency, SourceFileEditCodecContext,
    SourceFileEditCodecRegistration, SourceFileEditDocument, SourceFileEditError,
    SourceFileEditLoadRequest, SourceFileEditLoadResult, SourceFileEditSaveRequest,
    SourceFileEditSaveResult, SourceFormat, SourceSchemaRegistration, SourceSchemaType,
    product_format_id, source_schema_type,
};
use az_core::{
    ReflectedValueEncoding, ReflectedValueEnvelope, component::ComponentLoweringRegistration,
};
use az_gem_contract::GemContext;
use az_prefab::{
    PREFAB_COLLECTION_SOURCE_TYPE, PREFAB_SOURCE_TYPE, PrefabCodec, PrefabCollectionCodec,
    PrefabType,
};
pub use az_scene::{AZSCENE_ASSET_TYPE, AZSCENE_ASSET_TYPE_NAME, AzSceneAssetData};
pub use azscene::{
    CompiledAzScene, OverrideTargetFailure, PrefabAzSceneProcessError, azscene_product_path,
    engine_prefab_type_registry, lowerings, prefab_type_registry,
    process_prefab_collection_to_azscenes, process_prefab_to_azscene,
};
use bevy::ecs::reflect::AppTypeRegistry;
pub use fingerprint::prefab_cook_analysis_fingerprint;
use tracing::warn;
use uuid::uuid;

pub struct PrefabSourceFormat;

impl SourceFormat for PrefabSourceFormat {
    const SCHEMA: Option<SourceSchemaType> =
        Some(SourceSchemaType::__from_static(PREFAB_SOURCE_TYPE));
    const SCHEMA_TYPES: &'static [SourceSchemaType] =
        &[SourceSchemaType::__from_static(PREFAB_SOURCE_TYPE)];
    const PATTERNS: &'static [AssetBuilderPattern] =
        &[AssetBuilderPattern::static_wildcard("*.prefab.ron")];
}

pub struct PrefabCollectionSourceFormat;

impl SourceFormat for PrefabCollectionSourceFormat {
    const SCHEMA: Option<SourceSchemaType> = Some(SourceSchemaType::__from_static(
        PREFAB_COLLECTION_SOURCE_TYPE,
    ));
    const SCHEMA_TYPES: &'static [SourceSchemaType] = &[SourceSchemaType::__from_static(
        PREFAB_COLLECTION_SOURCE_TYPE,
    )];
    const PATTERNS: &'static [AssetBuilderPattern] =
        &[AssetBuilderPattern::static_wildcard("*.prefabs.ron")];
}

pub struct AzSceneProductFormat;

impl ProductFormat for AzSceneProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static("azoth.scene.azscene");
    const VERSION: u32 = az_scene::AZSCENE_FORMAT_VERSION;
    type Asset = AzSceneAssetData;
}

pub const AZSCENE_FORMAT_ID: ProductFormatId = product_format_id::<AzSceneProductFormat>();
pub const PREFAB_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("990a48df-830e-4d51-9ec6-bbb2b3e9d706"));
pub const PREFAB_COLLECTION_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("3991e172-ad36-4d42-abb2-133b858faef5"));
pub const AZSCENE_PRODUCT_SUB_ID: u32 = 1;
pub const PREFAB_SOURCE_SCHEMA: SourceSchemaType = source_schema_type::<PrefabSourceFormat>();
pub const PREFAB_COLLECTION_SOURCE_SCHEMA: SourceSchemaType =
    source_schema_type::<PrefabCollectionSourceFormat>();

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// The AZSCENE byte contract this crate owns.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::for_format::<AzSceneProductFormat>()]
}

/// The two Prefab authoring source schemas.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 2] {
    [
        SourceSchemaRegistration::for_source::<PrefabSourceFormat>()
            .with_label("Prefab")
            .with_category("Authoring")
            .with_creatable_document_schema(PREFAB_SOURCE_TYPE),
        SourceSchemaRegistration::for_source::<PrefabCollectionSourceFormat>()
            .with_label("Prefab Collection")
            .with_category("Authoring")
            .with_editable_file("prefabs", &["prefabs.ron"]),
    ]
}

/// The Prefab source codec, resolved against the active worker's composed type
/// registry on every invocation.
#[must_use]
pub fn source_file_edit_codecs() -> [SourceFileEditCodecRegistration; 1] {
    [SourceFileEditCodecRegistration::for_source::<
        PrefabSourceFormat,
    >(load_prefab_source, save_prefab_source)]
}

/// The two Prefab build rules.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 2] {
    [
        BuildRuleRegistration::new("azoth.prefab", PREFAB_BUILDER_ID, prefab_build_rule)
            .with_priority(75),
        BuildRuleRegistration::new(
            "azoth.prefab-collection",
            PREFAB_COLLECTION_BUILDER_ID,
            prefab_collection_build_rule,
        )
        .with_priority(75),
    ]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut GemContext<'_, D>) {
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<SourceSchemaRegistration>()
        .register_many(source_schemas());
    ctx.registrar::<SourceFileEditCodecRegistration>()
        .register_many(source_file_edit_codecs());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

/// The Prefab rule, fingerprinted against the Prefab types its host composed.
#[must_use]
pub fn prefab_build_rule(context: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<PrefabSourceFormat>()
        .named("azoth.prefab")
        .id(PREFAB_BUILDER_ID)
        .version(1)
        .analysis_fingerprint(prefab_cook_analysis_fingerprint(context))
        .produces::<AzSceneProductFormat>()
        .create_jobs(prefab_jobs)
        .process(prefab_job)
}

/// The Prefab-collection rule, fingerprinted against its host's composition.
#[must_use]
pub fn prefab_collection_build_rule(context: &JobContext<'_>) -> BuildRule {
    BuildRule::for_source::<PrefabCollectionSourceFormat>()
        .named("azoth.prefab-collection")
        .id(PREFAB_COLLECTION_BUILDER_ID)
        .version(1)
        .analysis_fingerprint(prefab_cook_analysis_fingerprint(context))
        .produces::<AzSceneProductFormat>()
        .create_jobs(prefab_collection_jobs)
        .process(prefab_collection_job)
}

#[must_use]
pub const fn prefab_source_patterns() -> &'static [AssetBuilderPattern] {
    <PrefabSourceFormat as SourceFormat>::PATTERNS
}

/// The processing registry for one job: engine Prefab types plus whatever the
/// host composed.
///
/// A host that composed no Prefab types gets the engine registry — the same
/// answer the retired `UNCOMPOSED` constant produced, except that it now
/// means "this composition has none", not "a rule callback cannot reach one".
// Registered as a `BuildRuleFactory`-style callback, so the `&JobContext`
// parameter is fixed by that signature; taking it by value would not compile.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn job_type_registry(context: &JobContext<'_>) -> AppTypeRegistry {
    context
        .registry::<PrefabType>()
        .map_or_else(engine_prefab_type_registry, prefab_type_registry)
}

fn source_edit_type_registry(
    context: SourceFileEditCodecContext<'_>,
) -> Result<AppTypeRegistry, SourceFileEditError> {
    context
        .registry::<PrefabType>()
        .map(prefab_type_registry)
        .ok_or_else(|| {
            SourceFileEditError::failed(
                "active asset worker composed no Prefab type registry for source authoring",
            )
        })
}

// The codec and the decoded document borrow from this type-registry read guard for
// the rest of the function, so the guard cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
fn load_prefab_source(
    context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditLoadRequest<'_>,
) -> SourceFileEditLoadResult {
    let source = std::str::from_utf8(request.bytes)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    let registry = source_edit_type_registry(context)?;
    let registry = registry.read();
    let codec = PrefabCodec::new(&registry)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    let document = codec
        .decode(source)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    let canonical = codec
        .encode(&document)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    Ok(SourceFileEditDocument::new(
        PREFAB_SOURCE_TYPE,
        ReflectedValueEnvelope::typed_ron(PREFAB_SOURCE_TYPE, canonical),
    ))
}

// The codec and the decoded document borrow from this type-registry read guard for
// the rest of the function, so the guard cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
fn save_prefab_source(
    context: SourceFileEditCodecContext<'_>,
    request: &SourceFileEditSaveRequest<'_>,
) -> SourceFileEditSaveResult {
    if request.root_schema != PREFAB_SOURCE_TYPE || request.value.type_path != PREFAB_SOURCE_TYPE {
        return Err(SourceFileEditError::unsupported(format!(
            "Prefab codec cannot save root `{}` with reflected type `{}`",
            request.root_schema, request.value.type_path
        )));
    }
    if request.value.encoding != ReflectedValueEncoding::TypedRon {
        return Err(SourceFileEditError::failed(
            "Prefab source value is not typed RON",
        ));
    }
    let source = std::str::from_utf8(&request.value.payload)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    let registry = source_edit_type_registry(context)?;
    let registry = registry.read();
    let codec = PrefabCodec::new(&registry)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    let document = codec
        .decode(source)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))?;
    codec
        .encode(&document)
        .map(String::into_bytes)
        .map_err(|error| SourceFileEditError::failed(error.to_string()))
}

/// The lowerings for one job: whatever the host composed, and nothing else.
///
/// An absent registry now means an uncomposed host rather than "the engine's
/// share is elsewhere", and lowering with an empty set silently drops every
/// faceted component out of the scene it was supposed to build — so it is said
/// out loud. The three pipeline roles all name the `types` and `runtime`
/// bundles, so a real worker never takes this branch.
// Registered as a `BuildRuleFactory`-style callback, so the `&JobContext`
// parameter is fixed by that signature; taking it by value would not compile.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn job_lowerings(context: &JobContext<'_>) -> Vec<ComponentLoweringRegistration> {
    let Some(composed) = context.registry::<ComponentLoweringRegistration>() else {
        warn!(
            "processing a Prefab against a host that composed no component lowerings; \
             every faceted component will be dropped from the scene"
        );
        return Vec::new();
    };
    lowerings(composed)
}

fn prefab_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    if request.source_schema_type != Some(PREFAB_SOURCE_TYPE) {
        return skipped_jobs();
    }

    let source_dependencies =
        match prefab_source_dependencies(&job_type_registry(request.context), request.source_bytes)
        {
            Ok(dependencies) => dependencies
                .into_iter()
                .map(SourceFileDependency::Path)
                .collect(),
            Err(error) => {
                warn!(
                    source_path = %request.source_path,
                    %error,
                    "failed to plan typed Prefab AZSCENE processing"
                );
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
            .map(|platform| JobDescriptor::default_for_platform(*platform))
            .collect(),
        source_dependencies,
        result: CreateJobsResult::Success,
    }
}

fn prefab_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    if request.source_schema_type != Some(PREFAB_SOURCE_TYPE) {
        return failed_process_response();
    }
    let source = match std::str::from_utf8(request.source_bytes) {
        Ok(source) => source,
        Err(error) => {
            warn!(source_path = %request.source_path, %error, "Prefab source is not UTF-8");
            return failed_process_response();
        }
    };
    let app_registry = job_type_registry(request.context);
    let compiled = match process_prefab_to_azscene(
        &request.source_path,
        source,
        &app_registry,
        &job_lowerings(request.context),
        |nested_path| read_nested_source(request.source_root, nested_path),
    ) {
        Ok(compiled) => compiled,
        Err(error) => {
            warn!(
                source_path = %request.source_path,
                %error,
                "failed to process typed Prefab to AZSCENE"
            );
            return failed_process_response();
        }
    };
    let catalog_aliases = match validated_catalog_aliases(&compiled) {
        Ok(aliases) => aliases,
        Err(error) => {
            warn!(source_path = %request.source_path, %error, "invalid Prefab catalog alias");
            return failed_process_response();
        }
    };
    let product = match BuildProduct::for_primary_source_format::<AzSceneProductFormat>(
        request,
        compiled.product_path,
        AZSCENE_PRODUCT_SUB_ID,
        compiled.bytes,
    ) {
        Ok(product) => product.with_catalog_aliases(catalog_aliases),
        Err(error) => {
            warn!(source_path = %request.source_path, %error, "invalid AZSCENE product path");
            return failed_process_response();
        }
    };

    ProcessJobResponse {
        products: vec![product],
        product_dependencies: compiled
            .asset_dependencies
            .into_iter()
            .map(|asset_id| (0, ProductDependency::new(asset_id)))
            .collect(),
        result: ProcessJobResult::Success,
    }
}

fn prefab_collection_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    if request.source_schema_type != Some(PREFAB_COLLECTION_SOURCE_TYPE) {
        return skipped_jobs();
    }

    let source_dependencies = match prefab_collection_source_dependencies(
        &job_type_registry(request.context),
        request.source_bytes,
    ) {
        Ok(dependencies) => dependencies
            .into_iter()
            .map(SourceFileDependency::Path)
            .collect(),
        Err(error) => {
            warn!(
                source_path = %request.source_path,
                %error,
                "failed to plan typed Prefab collection AZSCENE products"
            );
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
            .map(|platform| JobDescriptor::default_for_platform(*platform))
            .collect(),
        source_dependencies,
        result: CreateJobsResult::Success,
    }
}

fn prefab_collection_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    if request.source_schema_type != Some(PREFAB_COLLECTION_SOURCE_TYPE) {
        return failed_process_response();
    }
    let source = match std::str::from_utf8(request.source_bytes) {
        Ok(source) => source,
        Err(error) => {
            warn!(source_path = %request.source_path, %error, "Prefab collection source is not UTF-8");
            return failed_process_response();
        }
    };
    let app_registry = job_type_registry(request.context);
    let registry = app_registry.read();
    let collection = match PrefabCollectionCodec::new(&registry)
        .and_then(|codec| codec.decode(source))
    {
        Ok(collection) => collection,
        Err(error) => {
            warn!(source_path = %request.source_path, %error, "invalid Prefab collection source");
            return failed_process_response();
        }
    };
    if let Some(preserved) = request.preserved_source_asset_id {
        warn!(
            source_path = %request.source_path,
            preserved_sub_id = preserved.sub_id,
            "Prefab collections require source-GUID identity metadata without a primary product sub-ID"
        );
        return failed_process_response();
    }
    drop(registry);
    let compiled_entries = match process_prefab_collection_to_azscenes(
        &collection,
        &app_registry,
        &job_lowerings(request.context),
        |nested_path| read_nested_source(request.source_root, nested_path),
    ) {
        Ok(entries) => entries,
        Err(error) => {
            warn!(source_path = %request.source_path, %error, "failed to build Prefab collection");
            return failed_process_response();
        }
    };
    let mut products = Vec::with_capacity(compiled_entries.len());
    let mut product_dependencies = Vec::new();
    let mut catalog_paths = BTreeSet::new();
    for (sub_id, compiled) in compiled_entries {
        let aliases = match validated_catalog_aliases(&compiled) {
            Ok(aliases) => aliases,
            Err(error) => {
                warn!(
                    source_path = %request.source_path,
                    sub_id,
                    %error,
                    "invalid Prefab collection catalog path"
                );
                return failed_process_response();
            }
        };
        if !catalog_paths.insert(compiled.product_path.to_ascii_lowercase())
            || aliases
                .iter()
                .any(|alias| !catalog_paths.insert(alias.as_str().to_ascii_lowercase()))
        {
            warn!(
                source_path = %request.source_path,
                sub_id,
                "Prefab collection assigns one catalog path to multiple products"
            );
            return failed_process_response();
        }
        let product = match BuildProduct::for_format::<AzSceneProductFormat>(
            compiled.product_path,
            sub_id,
            compiled.bytes,
        ) {
            Ok(product) => product.with_catalog_aliases(aliases),
            Err(error) => {
                warn!(
                    source_path = %request.source_path,
                    sub_id,
                    %error,
                    "invalid AZSCENE product path"
                );
                return failed_process_response();
            }
        };
        let product_index = products.len();
        product_dependencies.extend(
            compiled
                .asset_dependencies
                .into_iter()
                .map(|asset_id| (product_index, ProductDependency::new(asset_id))),
        );
        products.push(product);
    }

    ProcessJobResponse {
        products,
        product_dependencies,
        result: ProcessJobResult::Success,
    }
}

// The codec and the decoded document borrow from this type-registry read guard for
// the rest of the function, so the guard cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
fn prefab_source_dependencies(
    app_registry: &AppTypeRegistry,
    source_bytes: &[u8],
) -> Result<Vec<String>, String> {
    let source = std::str::from_utf8(source_bytes).map_err(|error| error.to_string())?;
    let registry = app_registry.read();
    let document = PrefabCodec::new(&registry)
        .map_err(|error| error.to_string())?
        .decode(source)
        .map_err(|error| error.to_string())?;
    Ok(document
        .direct_dependencies()
        .map(|dependency| dependency.as_str().to_owned())
        .collect())
}

// The codec and the decoded document borrow from this type-registry read guard for
// the rest of the function, so the guard cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
fn prefab_collection_source_dependencies(
    app_registry: &AppTypeRegistry,
    source_bytes: &[u8],
) -> Result<Vec<String>, String> {
    let source = std::str::from_utf8(source_bytes).map_err(|error| error.to_string())?;
    let registry = app_registry.read();
    let collection = PrefabCollectionCodec::new(&registry)
        .and_then(|codec| codec.decode(source))
        .map_err(|error| error.to_string())?;
    let internal = collection
        .iter()
        .map(|(_, entry)| entry.source_path().as_str().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    Ok(collection
        .iter()
        .flat_map(|(_, entry)| entry.document().direct_dependencies())
        .filter(|dependency| !internal.contains(&dependency.as_str().to_ascii_lowercase()))
        .map(|dependency| dependency.as_str().to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

/// Validates and lowers one compiled scene's catalog aliases to product paths.
///
/// # Errors
///
/// Returns a description of the first alias that is not a valid product path,
/// or a fixed message if any alias equals the scene's own canonical product
/// path (compared case-insensitively).
pub fn validated_catalog_aliases(
    compiled: &CompiledAzScene,
) -> Result<Vec<az_asset_builder::ProductPath>, String> {
    let aliases = compiled
        .catalog_aliases
        .iter()
        .map(az_asset_builder::ProductPath::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if aliases
        .iter()
        .any(|alias| alias.as_str().eq_ignore_ascii_case(&compiled.product_path))
    {
        return Err("catalog alias duplicates its canonical product path".to_owned());
    }
    Ok(aliases)
}

fn read_nested_source(source_root: &Path, nested_path: &str) -> Result<String, String> {
    let relative = Path::new(nested_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "nested Prefab path `{nested_path}` must be a normalized asset-relative path"
        ));
    }
    let path = source_root.join(relative);
    std::fs::read_to_string(&path)
        .map_err(|error| format!("read nested Prefab `{}`: {error}", path.display()))
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
    use std::{any::TypeId, collections::BTreeMap};

    use super::*;
    use az_asset::{AssetId, UntypedAssetRef};
    use az_asset_builder::{
        BuildRuleRegistry, CreateJobsRequest, PlatformId, ProcessJobRequest, SourceFileDependency,
        SourceFileEditCodecContext, SourceFileEditLoadRequest, SourceFileEditSaveRequest,
        SourcePath, composed_product_format, composed_source_file_edit_codec,
    };
    use az_gem_contract::{
        ComposeError, Composer, Contribution, ContributionDescriptor, ContributionId, GemContext,
        GemId, GemTargetRole, ProductActivation, Registries, Registry, declare_caps,
    };
    use az_prefab::{
        EntityAlias, InstanceAlias, Prefab, PrefabAssetPath, PrefabCollection,
        PrefabCollectionEntry, PrefabDocument, PrefabEntity, PrefabInstance, ReflectPrefab,
        SparseValue,
    };
    use az_work::CancellationToken;
    use bevy::{
        ecs::reflect::ReflectComponent,
        prelude::Component,
        reflect::{Reflect, TypePath, std_traits::ReflectDefault},
    };

    #[derive(Reflect)]
    struct LinkedProjectPrefabType;

    #[derive(Reflect)]
    struct LinkedSiblingPrefabType;

    #[derive(Component, Reflect, Default, Prefab)]
    #[reflect(Component, Default, Prefab)]
    #[prefab(tag = "azoth.test.AssetComponent", version = 1)]
    struct LinkedAssetComponent {
        reference: UntypedAssetRef,
    }

    declare_caps!(ProjectCaps:);

    const PROJECT: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.prefab-builder-tests"),
        contribution: ContributionId::new("runtime"),
        roles: &[],
    };
    const SIBLING: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.prefab-builder-tests"),
        contribution: ContributionId::new("authoring"),
        roles: &[],
    };
    const RIVAL: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.prefab-builder-rival"),
        contribution: ContributionId::new("runtime"),
        roles: &[],
    };
    const BUILDERS: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.prefab-builder-tests"),
        contribution: ContributionId::new("builders"),
        roles: &[],
    };

    /// Stands in for a project gem's runtime contribution: several authored
    /// types registered as one batch through the ordinary registrar.
    struct Project;

    impl Contribution for Project {
        type Caps = ProjectCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            PROJECT
        }

        fn register(&self, ctx: &mut GemContext<'_, ProjectCaps>) {
            ctx.registrar::<PrefabType>().register_many([
                PrefabType::of::<LinkedProjectPrefabType>(),
                PrefabType::of::<UntypedAssetRef>(),
                PrefabType::of::<LinkedAssetComponent>(),
            ]);
        }
    }

    /// A second contribution under the same gem id — the case the retired
    /// `(owner, package, version, registration)` dedup key existed to keep
    /// distinct. Attribution now separates them without a key at all.
    struct Sibling;

    impl Contribution for Sibling {
        type Caps = ProjectCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            SIBLING
        }

        fn register(&self, ctx: &mut GemContext<'_, ProjectCaps>) {
            ctx.registrar::<PrefabType>()
                .register(PrefabType::of::<LinkedSiblingPrefabType>());
        }
    }

    /// The engine builder crate's own contribution — the same `register` a
    /// composing host calls.
    struct Builders;

    impl Contribution for Builders {
        type Caps = ProjectCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            BUILDERS
        }

        fn register(&self, ctx: &mut GemContext<'_, ProjectCaps>) {
            crate::register(ctx);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Project, ProductActivation::default())
            .expect("an empty floor composes");
        composer
            .add(Sibling, ProductActivation::default())
            .expect("an empty floor composes");
        composer
            .add(Builders, ProductActivation::default())
            .expect("an empty floor composes");
        composer
    }

    fn composed_prefab_types(composer: &Composer) -> &Registry<PrefabType> {
        composer
            .registries()
            .get::<PrefabType>()
            .expect("both contributions registered Prefab types")
    }

    #[test]
    fn composed_prefab_types_extend_the_processing_registry() {
        let composer = composed();
        let registry = prefab_type_registry(composed_prefab_types(&composer));
        let (has_project_type, has_sibling_type) = {
            let registry = registry.read();
            (
                registry
                    .get(TypeId::of::<LinkedProjectPrefabType>())
                    .is_some(),
                registry
                    .get(TypeId::of::<LinkedSiblingPrefabType>())
                    .is_some(),
            )
        };
        assert!(has_project_type, "composed registry keeps the project type");
        assert!(has_sibling_type, "composed registry keeps the sibling type");
    }

    #[test]
    fn composed_prefab_source_codec_preserves_project_types_through_load_and_save() {
        let composer = composed();
        let registry = prefab_type_registry(composed_prefab_types(&composer));
        let asset_id = AssetId::new(uuid::Uuid::from_u128(17), 3);
        let expected = PrefabDocument {
            type_versions: BTreeMap::from([("azoth.test.AssetComponent".to_owned(), 1)]),
            entities: BTreeMap::from([(
                EntityAlias::new("root").unwrap(),
                PrefabEntity {
                    entity_id: None,
                    parent: None,
                    components: BTreeMap::from([(
                        "azoth.test.AssetComponent".to_owned(),
                        SparseValue::try_new(Box::new(LinkedAssetComponent {
                            reference: UntypedAssetRef::from_id(asset_id, AZSCENE_ASSET_TYPE),
                        }))
                        .unwrap(),
                    )]),
                },
            )]),
            ..Default::default()
        };
        let source = PrefabCodec::new(&registry.read())
            .unwrap()
            .encode(&expected)
            .unwrap();
        let codec = composed_source_file_edit_codec(composer.registries(), PREFAB_SOURCE_SCHEMA)
            .expect("Prefab source codec is composed")
            .entry;
        let context = SourceFileEditCodecContext::new(composer.registries());
        let document = codec
            .load(
                context,
                &SourceFileEditLoadRequest {
                    schema_type: PREFAB_SOURCE_SCHEMA,
                    source_path: "prefabs/root.prefab.ron",
                    bytes: source.as_bytes(),
                },
            )
            .unwrap();
        let saved = codec
            .save(
                context,
                &SourceFileEditSaveRequest::new(
                    PREFAB_SOURCE_SCHEMA,
                    "prefabs/root.prefab.ron",
                    &document.root_schema,
                    &document.value,
                    &document.objects,
                    &document.codec_state,
                ),
            )
            .unwrap();
        let round_tripped = PrefabCodec::new(&registry.read())
            .unwrap()
            .decode(std::str::from_utf8(&saved).unwrap())
            .unwrap();

        assert_eq!(saved, source.as_bytes());
        assert_eq!(round_tripped.type_versions, expected.type_versions);
        assert!(
            round_tripped
                .entities
                .get(&EntityAlias::new("root").unwrap())
                .unwrap()
                .components
                .contains_key("azoth.test.AssetComponent")
        );
    }

    #[test]
    fn every_composed_entry_is_attributed_to_its_contribution() {
        let composer = composed();
        let report = composer.finalize().expect("composition is valid");
        let prefab_types = report
            .entries
            .iter()
            .filter(|entry| entry.registry == "prefab-type")
            .collect::<Vec<_>>();

        assert_eq!(prefab_types.len(), 4);
        assert!(
            prefab_types
                .iter()
                .all(|entry| entry.instance.gem.as_str() == "azoth.prefab-builder-tests")
        );
        let sibling = prefab_types
            .iter()
            .find(|entry| entry.key == LinkedSiblingPrefabType::type_path())
            .expect("the sibling type is composed");
        assert_eq!(sibling.instance.contribution.as_str(), "authoring");
    }

    #[test]
    fn two_contributions_claiming_one_type_path_fail_composition() {
        struct Rival;

        impl Contribution for Rival {
            type Caps = ProjectCaps;

            fn descriptor(&self) -> ContributionDescriptor {
                RIVAL
            }

            fn register(&self, ctx: &mut GemContext<'_, ProjectCaps>) {
                ctx.registrar::<PrefabType>()
                    .register(PrefabType::of::<LinkedProjectPrefabType>());
            }
        }

        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer.add(Project, ProductActivation::default()).unwrap();
        composer.add(Rival, ProductActivation::default()).unwrap();

        let ComposeError::Duplicate {
            registry,
            key,
            first,
            second,
        } = composer.finalize().unwrap_err()
        else {
            panic!("a repeated Prefab type path must fail composition");
        };
        assert_eq!(registry, "prefab-type");
        assert_eq!(key, LinkedProjectPrefabType::type_path());
        assert_eq!(first.gem.as_str(), "azoth.prefab-builder-tests");
        assert_eq!(second.gem.as_str(), "azoth.prefab-builder-rival");
    }

    /// The cook-determinism gap the retired `UNCOMPOSED` constant left open:
    /// a gem-contributed Prefab type has to reach the fingerprint a host
    /// actually publishes, not just a fingerprint helper called directly.
    ///
    /// This walks the real path — compose, resolve the registry the worker
    /// dispatches from, read the rule the way the builder catalog does.
    #[test]
    fn a_composed_prefab_type_moves_the_published_analysis_fingerprint() {
        fn published_fingerprint(composer: &Composer) -> String {
            BuildRuleRegistry::compose(&JobContext::new(composer.registries()))
                .by_id(PREFAB_BUILDER_ID)
                .expect("the composed host dispatches the Prefab rule")
                .analysis_fingerprint
                .clone()
        }

        // A host with the builder contribution and nothing else: engine
        // Prefab types only.
        let mut engine_only = Composer::new(GemTargetRole::AssetWorker);
        engine_only
            .add(Builders, ProductActivation::default())
            .unwrap();

        let with_project_types = published_fingerprint(&composed());
        assert_ne!(
            with_project_types,
            published_fingerprint(&engine_only),
            "a contributed Prefab type must re-fingerprint AZSCENE products"
        );
        assert!(with_project_types.starts_with("azoth.prefab-analysis/v2:"));
    }

    #[test]
    fn a_composed_host_dispatches_the_prefab_rules_in_priority_order() {
        let composer = composed();
        let registry = BuildRuleRegistry::compose(&JobContext::new(composer.registries()));
        assert!(registry.by_id(PREFAB_BUILDER_ID).is_some());
        assert!(registry.by_id(PREFAB_COLLECTION_BUILDER_ID).is_some());
        assert_eq!(
            registry
                .match_path("prefabs/door.prefab.ron")
                .map(|rule| rule.name),
            Some("azoth.prefab")
        );
    }

    /// The root document for the catalog-alias job test: one aliased entity
    /// referencing `dependency_id`, plus one nested child instance.
    fn catalog_alias_root_document(dependency_id: AssetId) -> PrefabDocument {
        PrefabDocument {
            catalog_aliases: std::iter::once(
                az_prefab::PrefabCatalogAlias::new("compat/prefabs/root.dynamicslice").unwrap(),
            )
            .collect(),
            type_versions: BTreeMap::from([("azoth.test.AssetComponent".to_owned(), 1)]),
            entities: BTreeMap::from([(
                EntityAlias::new("root").unwrap(),
                PrefabEntity {
                    entity_id: None,
                    parent: None,
                    components: BTreeMap::from([(
                        "azoth.test.AssetComponent".to_owned(),
                        SparseValue::try_new(Box::new(LinkedAssetComponent {
                            reference: UntypedAssetRef::from_id(dependency_id, AZSCENE_ASSET_TYPE),
                        }))
                        .unwrap(),
                    )]),
                },
            )]),
            instances: BTreeMap::from([(
                InstanceAlias::new("child").unwrap(),
                PrefabInstance {
                    source: PrefabAssetPath::new("prefabs/child.prefab.ron").unwrap(),
                    parent: None,
                    overrides: Vec::new(),
                },
            )]),
            ..Default::default()
        }
    }

    #[test]
    fn prefab_job_emits_one_azscene_with_declared_catalog_aliases() {
        let composer = composed();
        let prefab_types = composed_prefab_types(&composer);
        let context = JobContext::new(composer.registries());
        let registry = prefab_type_registry(prefab_types);
        let child = PrefabDocument::default();
        let child_source = PrefabCodec::new(&registry.read())
            .unwrap()
            .encode(&child)
            .unwrap();
        let dependency_id = AssetId::new(uuid::Uuid::from_u128(7), 11);
        let root = catalog_alias_root_document(dependency_id);
        let root_source = PrefabCodec::new(&registry.read())
            .unwrap()
            .encode(&root)
            .unwrap();
        let source_root = tempfile::tempdir().unwrap();
        let prefab_root = source_root.path().join("prefabs");
        std::fs::create_dir_all(&prefab_root).unwrap();
        std::fs::write(prefab_root.join("child.prefab.ron"), child_source).unwrap();
        let platforms = [PlatformId::new_static("server")];
        let source_uuid = uuid!("a660eeeb-ebc7-5cb7-be6b-ab11eb831731");
        let preserved_asset_id = AssetId::new(source_uuid, 2);
        let create = prefab_jobs(&CreateJobsRequest {
            builder_id: PREFAB_BUILDER_ID,
            source_path: SourcePath::new("prefabs/root.prefab.ron"),
            source_root: source_root.path(),
            source_uuid,
            source_schema_type: Some(PREFAB_SOURCE_SCHEMA.as_str()),
            source_bytes: root_source.as_bytes(),
            platforms: &platforms,
            context: &context,
        });

        assert_eq!(create.result, CreateJobsResult::Success);
        assert_eq!(create.jobs.len(), 1);
        assert_eq!(
            create.source_dependencies,
            vec![SourceFileDependency::Path(
                "prefabs/child.prefab.ron".to_owned()
            )]
        );

        let cancellation = CancellationToken::new();
        let process = prefab_job(&ProcessJobRequest {
            builder_id: PREFAB_BUILDER_ID,
            source_path: SourcePath::new("prefabs/root.prefab.ron"),
            source_root: source_root.path(),
            source_uuid,
            preserved_source_asset_id: Some(preserved_asset_id),
            source_schema_type: Some(PREFAB_SOURCE_SCHEMA.as_str()),
            source_bytes: root_source.as_bytes(),
            platform: "server",
            job_key: create.jobs[0].job_key.as_str(),
            cache_root: source_root.path(),
            cancellation: &cancellation,
            context: &context,
        });

        assert_eq!(process.result, ProcessJobResult::Success);
        assert_eq!(
            process.product_dependencies,
            vec![(0, ProductDependency::new(dependency_id))]
        );
        let [product] = process.products.as_slice() else {
            panic!("prefab job should emit exactly one product");
        };
        assert_eq!(product.asset_type, AZSCENE_ASSET_TYPE);
        assert_eq!(product.format, AZSCENE_FORMAT_ID);
        assert_eq!(product.format_version, az_scene::AZSCENE_FORMAT_VERSION);
        assert_eq!(product.asset_id(source_uuid), preserved_asset_id);
        assert_eq!(product.product_path.as_str(), "prefabs/root.scn.bin");
        assert_eq!(
            product.catalog_aliases,
            vec![az_asset_builder::ProductPath::new("compat/prefabs/root.dynamicslice").unwrap()]
        );
        let mut world = bevy::prelude::World::new();
        let instance = {
            let registry = registry.read();
            az_scene::read_scene_asset_from_reader(product.bytes.as_slice(), &registry)
                .unwrap()
                .materialize(&mut world, &registry)
                .unwrap()
        };
        let [entity] = instance.entities.as_slice() else {
            panic!("prefab job should materialize exactly one entity");
        };
        assert_eq!(
            world
                .get::<LinkedAssetComponent>(*entity)
                .map(|component| component.reference.asset_id),
            Some(dependency_id)
        );
    }

    /// Two aliased sub-documents where the first instances the second, so the
    /// nested reference stays internal to the collection.
    fn aliased_prefab_collection() -> PrefabCollection {
        let mut first = PrefabDocument::default();
        first
            .catalog_aliases
            .insert(az_prefab::PrefabCatalogAlias::new("slices/first.dynamicslice").unwrap());
        first.instances.insert(
            InstanceAlias::new("second").unwrap(),
            PrefabInstance {
                source: PrefabAssetPath::new("prefabs/second.prefab.ron").unwrap(),
                parent: None,
                overrides: Vec::new(),
            },
        );
        let mut second = PrefabDocument::default();
        second
            .catalog_aliases
            .insert(az_prefab::PrefabCatalogAlias::new("slices/second.dynamicslice").unwrap());
        PrefabCollection::try_from_entries([
            (
                0x11,
                PrefabCollectionEntry::new("prefabs/first.prefab.ron", first).unwrap(),
            ),
            (
                0x22,
                PrefabCollectionEntry::new("prefabs/second.prefab.ron", second).unwrap(),
            ),
        ])
        .unwrap()
    }

    #[test]
    fn prefab_collection_job_emits_each_sub_id_without_internal_source_dependencies() {
        let composer = composed();
        let prefab_types = composed_prefab_types(&composer);
        let context = JobContext::new(composer.registries());
        let app_registry = prefab_type_registry(prefab_types);
        let collection = aliased_prefab_collection();
        let collection_source = {
            let registry = app_registry.read();
            PrefabCollectionCodec::new(&registry)
                .unwrap()
                .encode(&collection)
                .unwrap()
        };
        let source_root = tempfile::tempdir().unwrap();
        let platforms = [PlatformId::new_static("server")];
        let source_uuid = uuid!("11835fa2-8e74-5b52-9de9-b7a993c96746");
        let create = prefab_collection_jobs(&CreateJobsRequest {
            builder_id: PREFAB_COLLECTION_BUILDER_ID,
            source_path: SourcePath::new("prefabs/source.prefabs.ron"),
            source_root: source_root.path(),
            source_uuid,
            source_schema_type: Some(PREFAB_COLLECTION_SOURCE_SCHEMA.as_str()),
            source_bytes: collection_source.as_bytes(),
            platforms: &platforms,
            context: &context,
        });

        assert_eq!(create.result, CreateJobsResult::Success);
        assert!(create.source_dependencies.is_empty());

        let cancellation = CancellationToken::new();
        let process = prefab_collection_job(&ProcessJobRequest {
            builder_id: PREFAB_COLLECTION_BUILDER_ID,
            source_path: SourcePath::new("prefabs/source.prefabs.ron"),
            source_root: source_root.path(),
            source_uuid,
            preserved_source_asset_id: None,
            source_schema_type: Some(PREFAB_COLLECTION_SOURCE_SCHEMA.as_str()),
            source_bytes: collection_source.as_bytes(),
            platform: "server",
            job_key: create.jobs[0].job_key.as_str(),
            cache_root: source_root.path(),
            cancellation: &cancellation,
            context: &context,
        });

        assert_eq!(process.result, ProcessJobResult::Success);
        assert_eq!(process.products.len(), 2);
        assert_eq!(
            process
                .products
                .iter()
                .map(|product| (product.sub_id, product.product_path.as_str()))
                .collect::<Vec<_>>(),
            [
                (0x11, "prefabs/first.scn.bin"),
                (0x22, "prefabs/second.scn.bin"),
            ]
        );
        assert_eq!(
            process.products[0].catalog_aliases[0].as_str(),
            "slices/first.dynamicslice"
        );
        assert_eq!(
            process.products[1].catalog_aliases[0].as_str(),
            "slices/second.dynamicslice"
        );

        let invalid_primary_identity = prefab_collection_job(&ProcessJobRequest {
            builder_id: PREFAB_COLLECTION_BUILDER_ID,
            source_path: SourcePath::new("prefabs/source.prefabs.ron"),
            source_root: source_root.path(),
            source_uuid,
            preserved_source_asset_id: Some(AssetId::new(source_uuid, 0x11)),
            source_schema_type: Some(PREFAB_COLLECTION_SOURCE_SCHEMA.as_str()),
            source_bytes: collection_source.as_bytes(),
            platform: "server",
            job_key: create.jobs[0].job_key.as_str(),
            cache_root: source_root.path(),
            cancellation: &cancellation,
            context: &context,
        });
        assert_eq!(invalid_primary_identity.result, ProcessJobResult::Failed);
        assert!(invalid_primary_identity.products.is_empty());
    }

    #[test]
    fn azscene_product_format_registers_byte_contract() {
        let composer = composed();
        let azscene = composed_product_format(composer.registries(), AZSCENE_FORMAT_ID)
            .expect("AZSCENE product format is composed");
        assert_eq!(
            azscene.entry.current_version(),
            az_scene::AZSCENE_FORMAT_VERSION
        );
    }

    #[test]
    fn prefab_rule_emits_only_azscene() {
        let registries = Registries::new();
        let prefab = prefab_build_rule(&JobContext::new(&registries));
        assert!(prefab.matches_source("prefabs/door.prefab.ron", Some(PREFAB_SOURCE_TYPE)));
        assert!(!prefab.matches_source("scenes/main.scene.ron", None));
    }
}
