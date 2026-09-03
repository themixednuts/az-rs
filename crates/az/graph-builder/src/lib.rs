//! Universal build-rule bridge for Azoth visual graph runtime products.
//!
//! `az-node-graph` owns editor-facing graph documents and descriptors. This
//! crate adapts registered graph types into the asset-builder pipeline and
//! compiles runtime graph products for loaders. Project/gem graph types remain
//! inventory registrations; the universal editor never links the project code
//! directly.

/// Fingerprint of this crate's own Rust sources, derived at build time by
/// `az-build-fingerprint`.
///
/// Asset build rules compose this into their analysis fingerprint so that
/// changing the code behind a product's bytes invalidates products built by
/// the older code. Nothing here is hand-maintained: editing any file under
/// `src/` changes the value.
pub const SOURCE_FINGERPRINT: &str = env!("AZ_SOURCE_FINGERPRINT");

mod fingerprint;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use az_asset_builder::JobContext;
use az_asset_builder::{
    AssetBuilderPattern, BuildProduct, BuildRule, BuildRuleRegistration, BuilderId,
    CreateJobsRequest, CreateJobsResponse, CreateJobsResult, JobDescriptor, JobKey,
    ProcessJobRequest, ProcessJobResponse, ProcessJobResult, ProductFormat, ProductFormatId,
    ProductFormatPolicy, ProductFormatRegistration, SourceMatcher, SourceSchemaType,
};
use az_core::{
    AssetData, AssetType, AssetTypeRegistration, AzRtti, AzTypeInfo, ReflectedValueEncoding,
    ReflectedValueEnvelope,
};
use az_gem_contract::Registries;
pub use az_graph_runtime::{
    AOT_GRAPH_MANIFEST_MAGIC, AOT_GRAPH_MANIFEST_PRODUCT_EXTENSION, AOT_GRAPH_MANIFEST_VERSION,
    AotGraphManifest, AotGraphManifestError, AotGraphManifestHeader, PACKED_GRAPH_IR_MAGIC,
    PACKED_GRAPH_IR_PRODUCT_EXTENSION, PACKED_GRAPH_IR_VERSION, PackedGraphBindError,
    PackedGraphBindLookupError, PackedGraphBoundNode, PackedGraphConnection,
    PackedGraphExecutionPlan, PackedGraphExecutor, PackedGraphInputValue, PackedGraphInputValueRef,
    PackedGraphIr, PackedGraphIrError, PackedGraphIrHeader, PackedGraphNode,
    PackedGraphNodeBindRequest, PackedGraphNodeBinder, PackedGraphNodeInputValues, PackedGraphPort,
    PackedGraphPortCapacity, PackedGraphPortDirection, PackedGraphPortValue, PackedGraphProduct,
    PackedGraphRuntimeFlags, PackedRuntimeBinding, PackedRuntimeBindingRef,
    decode_aot_graph_manifest, decode_packed_graph_ir,
};
use az_node_graph::{
    GeneratedRustGraphAbi, GraphCompilerBackendDescriptor, GraphCompilerBackendKind,
    GraphConnection, GraphConnectionId, GraphNode, GraphNodeId, GraphPortRef,
    GraphSourceWorkflowKind, GraphTypeCatalog, GraphTypeCatalogError, GraphTypeDescriptor,
    NodePortDescriptor, NodePortDirection, NodePortId, NodePortValue, NodeRuntimeBinding,
    NodeTypeCatalog, NodeTypeCatalogError, NodeTypeCatalogHashError, NodeTypeDescriptor,
    RuntimeGraphExecutionStrategy, RuntimeGraphProductDescriptor, RustCallResult,
    RustDataflowOutput, RustDataflowParameter, RustDataflowParameterSource, RustNodeCallAbi,
    RustTypedDataflowNodeCall, RustValuePassing, VisualGraphDocument, VisualGraphDocumentIoError,
    VisualGraphValidationError, decode_visual_graph_document_ron, graph_types,
};
use thiserror::Error;
use uuid::{Uuid, uuid};

pub struct PackedGraphIrAssetData;

impl AzTypeInfo for PackedGraphIrAssetData {
    const NAME: &'static str = "AzGraph::PackedGraphIrAssetData";
    const TYPE_ID: Uuid = uuid!("6b3058f1-2ddb-4be1-a2e7-a4cf404aeb70");
}

impl AzRtti for PackedGraphIrAssetData {}

impl AssetData for PackedGraphIrAssetData {
    const STABLE_NAME: &'static str = "azoth.graph.packed-ir";
}

pub struct AotGraphManifestAssetData;

impl AzTypeInfo for AotGraphManifestAssetData {
    const NAME: &'static str = "AzGraph::AotGraphManifestAssetData";
    const TYPE_ID: Uuid = uuid!("da70f970-4d91-4f16-a1a8-71526332a935");
}

impl AzRtti for AotGraphManifestAssetData {}

impl AssetData for AotGraphManifestAssetData {
    const STABLE_NAME: &'static str = "azoth.graph.aot-manifest";
}

pub struct GeneratedRustGraphSourceAssetData;

impl AzTypeInfo for GeneratedRustGraphSourceAssetData {
    const NAME: &'static str = "AzGraph::GeneratedRustGraphSourceAssetData";
    const TYPE_ID: Uuid = uuid!("c6de7260-0d1b-4f6e-a60f-f567f774513d");
}

impl AzRtti for GeneratedRustGraphSourceAssetData {}

impl AssetData for GeneratedRustGraphSourceAssetData {
    const STABLE_NAME: &'static str = "azoth.graph.generated-rust-source";
}

pub struct PackedGraphIrProductFormat;

impl ProductFormat for PackedGraphIrProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static("azoth.graph.packed-ir");
    const VERSION: u32 = PACKED_GRAPH_IR_VERSION;
    type Asset = PackedGraphIrAssetData;
}

pub struct AotGraphManifestProductFormat;

impl ProductFormat for AotGraphManifestProductFormat {
    const ID: ProductFormatId = ProductFormatId::__from_static("azoth.graph.aot-manifest");
    const VERSION: u32 = AOT_GRAPH_MANIFEST_VERSION;
    type Asset = AotGraphManifestAssetData;
}

#[derive(ProductFormat)]
#[product_format(
    id = "azoth.graph.generated-rust-source",
    version = 1,
    asset = GeneratedRustGraphSourceAssetData
)]
pub struct GeneratedRustGraphSourceProductFormat;

pub const PACKED_GRAPH_IR_PRODUCT_FORMAT_VERSION: u32 =
    <PackedGraphIrProductFormat as ProductFormat>::VERSION;
pub const PACKED_GRAPH_IR_FORMAT_ID: ProductFormatId =
    <PackedGraphIrProductFormat as ProductFormat>::ID;
pub const PACKED_GRAPH_IR_ASSET_TYPE_NAME: &str =
    <PackedGraphIrAssetData as AssetData>::STABLE_NAME;
pub const PACKED_GRAPH_IR_ASSET_TYPE: AssetType = <PackedGraphIrAssetData as AssetData>::ASSET_TYPE;
pub const AOT_GRAPH_MANIFEST_PRODUCT_FORMAT_VERSION: u32 =
    <AotGraphManifestProductFormat as ProductFormat>::VERSION;
pub const AOT_GRAPH_MANIFEST_FORMAT_ID: ProductFormatId =
    <AotGraphManifestProductFormat as ProductFormat>::ID;
pub const GENERATED_RUST_GRAPH_SOURCE_PRODUCT_FORMAT_VERSION: u32 =
    <GeneratedRustGraphSourceProductFormat as ProductFormat>::VERSION;
pub const GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID: ProductFormatId =
    <GeneratedRustGraphSourceProductFormat as ProductFormat>::ID;
pub const GENERATED_RUST_GRAPH_SOURCE_ASSET_TYPE_NAME: &str =
    <GeneratedRustGraphSourceAssetData as AssetData>::STABLE_NAME;
pub const GENERATED_RUST_GRAPH_SOURCE_ASSET_TYPE: AssetType =
    <GeneratedRustGraphSourceAssetData as AssetData>::ASSET_TYPE;
pub const GENERATED_RUST_GRAPH_SOURCE_PRODUCT_EXTENSION: &str = "generated.rs";
/// Relative path beneath an ADR-0030 per-project data entry.
pub const GENERATED_RUST_GRAPH_SOURCE_ROOT: &str = "derived/graphs";
pub const GENERATED_RUST_GRAPH_SOURCE_MODULE_FILE: &str = "azoth_generated_graphs.rs";
/// Item each AOT Rust graph product exports: its own registration, as data.
///
/// The compiler emits it beside the entry function it names, so the value and
/// the code it dispatches to are written by one pass and cannot drift.
pub const GENERATED_RUST_GRAPH_REGISTRATION_ITEM: &str = "REGISTRATION";
/// Per-product module the projection wraps each product in.
///
/// One module per product keeps two graphs that share an entry-module prefix
/// from colliding when the projection `include!`s them side by side, and gives
/// the projection a path to each product's registration without parsing it.
///
/// It also fixes where a product's paths resolve: a runtime binding or context
/// type naming crate-local code must be `crate::`-rooted, because the product
/// no longer lands at the crate root. Extern-crate paths are unaffected — those
/// resolve from any module.
const GENERATED_RUST_GRAPH_MODULE_PREFIX: &str = "graph";
const GENERATED_RUST_GRAPH_REGISTER_DOC: &str = "\n/// Register every AOT graph compiled into this crate.\n///\n/// Generated, so a gem cannot ship compiled graphs and forget to register\n/// them: the contribution's `register` body calls this and the list below is\n/// the build's own answer to what was compiled.\n";
pub const RUNTIME_GRAPH_PRODUCT_SUB_ID: u32 = 1;
pub const GENERATED_RUST_GRAPH_SOURCE_PRODUCT_SUB_ID: u32 = 2;
pub const GRAPH_COMPILER_BUILDER_ID: BuilderId =
    BuilderId::new(uuid!("e622e48c-5c8a-4b28-a64e-e080d5aa51f1"));
pub const GRAPH_COMPILER_JOB_KEY: &str = "compile-graph-runtime-product";

/// Result of projecting engine-managed generated Rust graph products into one
/// Cargo build-script module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedRustGraphSourceModule {
    pub module_path: PathBuf,
    pub source_count: usize,
}

/// Failure to link the current generated Rust graph product projection.
#[derive(Debug, Error)]
pub enum GeneratedRustGraphSourceModuleError {
    #[error("failed to create generated graph source root {path}: {source}")]
    CreateSourceRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to inspect generated graph source path {path}: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("generated graph source root {path} is not a directory")]
    SourceRootNotDirectory { path: PathBuf },

    #[error("failed to write generated graph module {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "package `{package}` projects AOT graphs, but no `{GEM_MANIFEST_FILE}` declares it: no \
         manifest in this crate's own directory, nor in any ancestor of it. A contribution crate \
         lives inside its gem, at or below the manifest that declares it."
    )]
    UndeclaredGem { package: String },

    #[error("`{path}` could not be read as a gem manifest: {reason}")]
    UnreadableGem { path: PathBuf, reason: String },

    #[error(
        "`{path}` gives package `{package}` no contribution declaring `{GRAPH_PRODUCT}`, so the \
         projection this writes would never be registered and the graphs would be missing at \
         runtime. Add it to the contribution's stanza:\n\n    products = [\"{GRAPH_PRODUCT}\"]"
    )]
    UndeclaredProduct { path: PathBuf, package: String },
}

/// The file a gem is declared in, spelled where a build script can reach it.
/// `az_project::GEM_MANIFEST_FILE` is the same name in the crate that owns the
/// schema.
const GEM_MANIFEST_FILE: &str = "gem.toml";
/// The `products` name for AOT graphs — `az_project::GemProduct::Graphs`'s
/// serde spelling, the same one `#[contribution]` matches on.
const GRAPH_PRODUCT: &str = "graphs";

/// Emit the build-script module that registers all current AOT Rust graph
/// products beneath an ADR-0030 `ProjectDataPaths::graphs_dir()` root.
///
/// The source root is supplied explicitly so callers must resolve the typed
/// project data-home path rather than falling back to a repository-local
/// directory. A missing root is materialized as a valid empty projection before
/// any graph has been compiled. This also gives Cargo an existing, stable
/// `rerun-if-changed` anchor; watching a missing path would make every build
/// script invocation stale.
///
/// `crate_root` and `package` are the calling build script's own
/// `CARGO_MANIFEST_DIR` and `CARGO_PKG_NAME`. They are taken as arguments
/// rather than read from the environment so this stays a pure function, and
/// they are required because calling this *is* half a declaration: the gem
/// manifest carries the other half, and generated glue reads only the manifest.
/// A crate that projects graphs its manifest does not declare would write a
/// projection nothing ever calls, so that disagreement is refused here — this
/// side is the only one that can see it.
///
/// # Errors
///
/// Returns an error when the gem manifest does not declare this package's
/// graphs, when the source root cannot be materialized, when an existing source
/// root is not a directory, when its contents cannot be inspected, or when the
/// generated module cannot be written to `out_dir`.
pub fn write_generated_rust_graph_source_module(
    source_root: &Path,
    out_dir: &Path,
    crate_root: &Path,
    package: &str,
) -> Result<GeneratedRustGraphSourceModule, GeneratedRustGraphSourceModuleError> {
    require_declared_graphs(crate_root, package)?;
    if !source_root.exists() {
        std::fs::create_dir_all(source_root).map_err(|source| {
            GeneratedRustGraphSourceModuleError::CreateSourceRoot {
                path: source_root.to_path_buf(),
                source,
            }
        })?;
    }
    if !source_root.is_dir() {
        return Err(
            GeneratedRustGraphSourceModuleError::SourceRootNotDirectory {
                path: source_root.to_path_buf(),
            },
        );
    }
    println!("cargo:rerun-if-changed={}", source_root.display());
    let mut sources = Vec::new();
    collect_generated_rust_graph_sources(source_root, &mut sources)?;
    sources.sort();

    let module_path = out_dir.join(GENERATED_RUST_GRAPH_SOURCE_MODULE_FILE);
    let mut module = String::from("// @generated by az-graph-builder. Do not edit by hand.\n\n");
    for (index, source) in sources.iter().enumerate() {
        writeln!(
            module,
            "pub mod {GENERATED_RUST_GRAPH_MODULE_PREFIX}{index} {{\n    include!({:?});\n}}",
            source.to_string_lossy().as_ref()
        )
        .expect("writing to a String cannot fail");
    }
    module.push_str(GENERATED_RUST_GRAPH_REGISTER_DOC);
    module.push_str("pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {\n    ctx.registrar::<az_graph_runtime::AotGraphRuntimeRegistration>()\n        .register_many([\n");
    for index in 0..sources.len() {
        writeln!(
            module,
            "            {GENERATED_RUST_GRAPH_MODULE_PREFIX}{index}::{GENERATED_RUST_GRAPH_REGISTRATION_ITEM},"
        )
        .expect("writing to a String cannot fail");
    }
    module.push_str("        ]);\n}\n");
    std::fs::create_dir_all(out_dir).map_err(|source| {
        GeneratedRustGraphSourceModuleError::Write {
            path: out_dir.to_path_buf(),
            source,
        }
    })?;
    std::fs::write(&module_path, module).map_err(|source| {
        GeneratedRustGraphSourceModuleError::Write {
            path: module_path.clone(),
            source,
        }
    })?;
    Ok(GeneratedRustGraphSourceModule {
        module_path,
        source_count: sources.len(),
    })
}

/// Refuse to project graphs a gem manifest does not declare.
///
/// A narrow reader on purpose, not a second copy of the schema: `az-project`
/// owns and validates it, and this reads one fact — whether some
/// `[[contributions]]` stanza for this package lists `graphs` in `products`.
/// Every other key stays invisible, so growing the schema cannot break a build.
fn require_declared_graphs(
    crate_root: &Path,
    package: &str,
) -> Result<(), GeneratedRustGraphSourceModuleError> {
    let path = crate_root
        .ancestors()
        .map(|directory| directory.join(GEM_MANIFEST_FILE))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| GeneratedRustGraphSourceModuleError::UndeclaredGem {
            package: package.to_string(),
        })?;
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(&path).map_err(|error| {
        GeneratedRustGraphSourceModuleError::UnreadableGem {
            path: path.clone(),
            reason: error.to_string(),
        }
    })?;
    let manifest = text.parse::<toml::Table>().map_err(|error| {
        GeneratedRustGraphSourceModuleError::UnreadableGem {
            path: path.clone(),
            reason: error.to_string(),
        }
    })?;
    let declared = manifest
        .get("contributions")
        .and_then(toml::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(toml::Value::as_table)
        .filter(|stanza| stanza.get("package").and_then(toml::Value::as_str) == Some(package))
        .filter_map(|stanza| stanza.get("products"))
        .filter_map(toml::Value::as_array)
        .any(|products| {
            products
                .iter()
                .any(|product| product.as_str() == Some(GRAPH_PRODUCT))
        });
    if declared {
        Ok(())
    } else {
        Err(GeneratedRustGraphSourceModuleError::UndeclaredProduct {
            path,
            package: package.to_string(),
        })
    }
}

fn collect_generated_rust_graph_sources(
    directory: &Path,
    sources: &mut Vec<PathBuf>,
) -> Result<(), GeneratedRustGraphSourceModuleError> {
    let entries = std::fs::read_dir(directory).map_err(|source| {
        GeneratedRustGraphSourceModuleError::Inspect {
            path: directory.to_path_buf(),
            source,
        }
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| GeneratedRustGraphSourceModuleError::Inspect {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type =
            entry
                .file_type()
                .map_err(|source| GeneratedRustGraphSourceModuleError::Inspect {
                    path: path.clone(),
                    source,
                })?;
        if file_type.is_dir() {
            collect_generated_rust_graph_sources(&path, sources)?;
        } else if file_type.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("rs")
        {
            println!("cargo:rerun-if-changed={}", path.display());
            sources.push(path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// The graph product byte contracts this crate owns.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 3] {
    [
        ProductFormatRegistration::for_format::<PackedGraphIrProductFormat>(),
        ProductFormatRegistration::for_format::<AotGraphManifestProductFormat>(),
        ProductFormatRegistration::for_format::<GeneratedRustGraphSourceProductFormat>(),
    ]
}

/// The graph asset types this crate owns.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 2] {
    [
        AssetTypeRegistration::for_asset::<PackedGraphIrAssetData>().with_owner("az-graph-builder"),
        AssetTypeRegistration::for_asset::<GeneratedRustGraphSourceAssetData>()
            .with_owner("az-graph-builder"),
    ]
}

/// The graph compiler rule.
#[must_use]
pub fn build_rules() -> [BuildRuleRegistration; 1] {
    [BuildRuleRegistration::new(
        "az.graph.compiler",
        GRAPH_COMPILER_BUILDER_ID,
        graph_compiler_build_rule,
    )
    .with_priority(50)]
}

/// Register this crate's asset-pipeline contributions into a composing host.
pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {
    ctx.registrar::<ProductFormatRegistration>()
        .register_many(product_formats());
    ctx.registrar::<AssetTypeRegistration>()
        .register_many(asset_types());
    ctx.registrar::<BuildRuleRegistration>()
        .register_many(build_rules());
}

/// Kept for `GraphCompilerBackendRegistration`, the one registry in this crate
/// still discovered at link time.
#[doc(hidden)]
pub use az_asset_builder::inventory;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredGraphSourceSchema {
    pub schema_type: String,
    pub owner: String,
    pub label: String,
    pub category: String,
    pub authoring: RegisteredGraphSourceAuthoring,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisteredGraphSourceAuthoring {
    ProjectDocument {
        schema_type: String,
    },
    File {
        source_root: String,
        default_path_prefix: String,
        extensions: Vec<String>,
        can_create: bool,
        can_edit: bool,
    },
}

pub struct GraphCompileRequest<'a> {
    pub source_path: &'a str,
    pub source_uuid: uuid::Uuid,
    pub platform: &'a str,
    pub document: &'a VisualGraphDocument,
    pub graph_type: &'a GraphTypeDescriptor,
    pub node_catalog: &'a NodeTypeCatalog,
    /// The host composition this compile resolves asset types and product
    /// formats against.
    pub registries: &'a Registries,
}

impl std::fmt::Debug for GraphCompileRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The composition is a heterogeneous store with no useful rendering;
        // everything that identifies this compile is above it.
        f.debug_struct("GraphCompileRequest")
            .field("source_path", &self.source_path)
            .field("source_uuid", &self.source_uuid)
            .field("platform", &self.platform)
            .field("document", &self.document)
            .field("graph_type", &self.graph_type)
            .field("node_catalog", &self.node_catalog)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCompileOutput {
    pub products: Vec<GraphCompileProduct>,
}

impl GraphCompileOutput {
    #[must_use]
    pub fn single(product: GraphCompileProduct) -> Self {
        Self {
            products: vec![product],
        }
    }

    #[must_use]
    pub fn runtime_product(&self) -> Option<&GraphCompileProduct> {
        self.products
            .iter()
            .find(|product| product.role == GraphCompileProductRole::RuntimeProduct)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCompileProduct {
    pub role: GraphCompileProductRole,
    pub product_path: String,
    pub asset_type: AssetType,
    pub product_format: ProductFormatId,
    pub product_format_version: u32,
    pub sub_id: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphCompileProductRole {
    RuntimeProduct,
    GeneratedRustSource,
}

impl GraphCompileProductRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeProduct => "runtime product",
            Self::GeneratedRustSource => "generated Rust source",
        }
    }
}

pub trait GraphCompilerBackend: Sync {
    fn backend_id(&self) -> &'static str;
    fn supports(&self, backend: &GraphCompilerBackendDescriptor) -> bool;

    /// Compiles `request` into this backend's product.
    ///
    /// # Errors
    ///
    /// Returns a [`GraphCompileError`] chosen by the implementation when the
    /// request's graph cannot be lowered — an unsupported backend descriptor, a
    /// node the backend has no lowering for, or a malformed graph topology.
    fn compile(
        &self,
        request: GraphCompileRequest<'_>,
    ) -> Result<GraphCompileOutput, GraphCompileError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PackedIrCompiler;

#[derive(Debug, Clone, Copy, Default)]
pub struct GeneratedRustAotCompiler;

impl GraphCompilerBackend for PackedIrCompiler {
    fn backend_id(&self) -> &'static str {
        "az.graph-builder.packed-ir"
    }

    fn supports(&self, backend: &GraphCompilerBackendDescriptor) -> bool {
        matches!(&backend.kind, GraphCompilerBackendKind::PackedIr { .. })
    }

    fn compile(
        &self,
        request: GraphCompileRequest<'_>,
    ) -> Result<GraphCompileOutput, GraphCompileError> {
        compile_packed_ir(&request)
    }
}

impl GraphCompilerBackend for GeneratedRustAotCompiler {
    fn backend_id(&self) -> &'static str {
        "az.graph-builder.generated-rust-aot"
    }

    fn supports(&self, backend: &GraphCompilerBackendDescriptor) -> bool {
        matches!(
            &backend.kind,
            GraphCompilerBackendKind::GeneratedRust {
                abi: GeneratedRustGraphAbi::ContextSchedule | GeneratedRustGraphAbi::TypedDataflow,
                ..
            }
        )
    }

    fn compile(
        &self,
        request: GraphCompileRequest<'_>,
    ) -> Result<GraphCompileOutput, GraphCompileError> {
        compile_aot_graph_products(&request)
    }
}

#[derive(Debug, Error)]
pub enum GraphCompileError {
    #[error("visual graph source decode failed: {0}")]
    DecodeSource(#[from] VisualGraphDocumentIoError),
    #[error("visual graph source is not UTF-8: {0}")]
    SourceUtf8(#[from] std::str::Utf8Error),
    #[error("registered graph type catalog is invalid: {0}")]
    GraphTypeCatalog(#[from] GraphTypeCatalogError),
    #[error("registered node type catalog is invalid: {0}")]
    NodeTypeCatalog(#[from] NodeTypeCatalogError),
    #[error("failed to hash registered node type catalog: {0}")]
    NodeTypeCatalogHash(#[from] NodeTypeCatalogHashError),
    #[error("visual graph document is invalid: {0}")]
    ValidateDocument(#[from] VisualGraphValidationError),
    #[error("source schema `{source_schema}` is not a registered runtime graph type")]
    UnknownRuntimeGraphType { source_schema: String },
    #[error(
        "visual graph document declares graph type `{actual}`, but build source schema is `{expected}`"
    )]
    GraphTypeMismatch { expected: String, actual: String },
    #[error("graph type `{graph_type}` has no compiler backend")]
    MissingCompilerBackend { graph_type: String },
    #[error("graph type `{graph_type}` has no runtime product descriptor")]
    MissingRuntimeProduct { graph_type: String },
    #[error("graph type `{graph_type}` uses unsupported compiler backend `{backend}`")]
    UnsupportedCompilerBackend { graph_type: String, backend: String },
    #[error("runtime graph asset type `{asset_type}` is not registered")]
    UnregisteredRuntimeAssetType { asset_type: String },
    #[error(
        "graph type `{graph_type}` compiler emitted asset type `{actual}` but runtime product declares `{expected}`"
    )]
    RuntimeAssetTypeMismatch {
        graph_type: String,
        expected: AssetType,
        actual: AssetType,
    },
    #[error("graph type `{graph_type}` compiler emitted no runtime graph product")]
    MissingRuntimeCompileProduct { graph_type: String },
    #[error("graph type `{graph_type}` compiler emitted more than one runtime graph product")]
    MultipleRuntimeCompileProducts { graph_type: String },
    #[error(
        "graph type `{graph_type}` compiler emitted product role `{role}` with unsupported asset type `{asset_type}`"
    )]
    UnsupportedProductRoleAssetType {
        graph_type: String,
        role: &'static str,
        asset_type: AssetType,
    },
    #[error("runtime graph product format `{format}` is not registered")]
    UnregisteredProductFormat { format: String },
    #[error(
        "runtime graph product format `{format}` writer version mismatch: registered {expected}, emitted {actual}"
    )]
    ProductFormatVersionMismatch {
        format: String,
        expected: u32,
        actual: u32,
    },
    #[error("graph type `{graph_type}` runtime strategy is not {expected}")]
    UnsupportedRuntimeExecutionStrategy {
        graph_type: String,
        expected: &'static str,
    },
    #[error("node `{node_id}` type `{node_type}` has no runtime binding")]
    MissingNodeRuntimeBinding {
        node_id: GraphNodeId,
        node_type: String,
    },
    #[error(
        "node `{node_id}` type `{node_type}` uses runtime binding `{binding_kind}`, but generated Rust graphs require Rust symbol bindings"
    )]
    UnsupportedGeneratedRustNodeBinding {
        node_id: GraphNodeId,
        node_type: String,
        binding_kind: &'static str,
    },
    #[error(
        "generated Rust context-schedule graph `{graph_type}` cannot compile input value on node `{node_id}` port `{port_id}`; use a typed dataflow, material, shader, or domain compiler backend"
    )]
    UnsupportedGeneratedRustInputValue {
        graph_type: String,
        node_id: GraphNodeId,
        port_id: NodePortId,
    },
    #[error(
        "generated Rust context-schedule graph `{graph_type}` cannot compile non-execution connection `{connection_id}` from port `{from_port_id}` to `{to_port_id}`; use a typed dataflow, material, shader, or domain compiler backend"
    )]
    UnsupportedGeneratedRustDataConnection {
        graph_type: String,
        connection_id: GraphConnectionId,
        from_port_id: NodePortId,
        to_port_id: NodePortId,
    },
    #[error(
        "generated Rust typed-dataflow node `{node_id}` type `{node_type}` must use typed-dataflow Rust call ABI"
    )]
    UnsupportedGeneratedRustDataflowNodeAbi {
        node_id: GraphNodeId,
        node_type: String,
    },
    #[error(
        "generated Rust typed-dataflow node `{node_id}` type `{node_type}` references Rust type `{rust_type}` which is invalid: {reason}"
    )]
    InvalidGeneratedRustDataflowRustType {
        node_id: GraphNodeId,
        node_type: String,
        rust_type: String,
        reason: String,
    },
    #[error(
        "generated Rust typed-dataflow node `{node_id}` type `{node_type}` maps field accessor `{field}` which is not a Rust identifier"
    )]
    InvalidGeneratedRustDataflowField {
        node_id: GraphNodeId,
        node_type: String,
        field: String,
    },
    #[error(
        "generated Rust typed-dataflow node `{node_id}` port `{port_id}` has no connection, input value, or port default"
    )]
    MissingGeneratedRustDataflowInput {
        node_id: GraphNodeId,
        port_id: NodePortId,
    },
    #[error(
        "generated Rust typed-dataflow node `{node_id}` port `{port_id}` has {connection_count} incoming data connections; typed parameters accept exactly one value"
    )]
    MultipleGeneratedRustDataflowInputConnections {
        node_id: GraphNodeId,
        port_id: NodePortId,
        connection_count: usize,
    },
    #[error(
        "generated Rust typed-dataflow node `{node_id}` port `{port_id}` has authored input value but no typed parameter mapping"
    )]
    UnmappedGeneratedRustDataflowInputValue {
        node_id: GraphNodeId,
        port_id: NodePortId,
    },
    #[error(
        "generated Rust typed-dataflow connection `{connection_id}` targets port `{to_port_id}` that has no typed parameter mapping"
    )]
    UnmappedGeneratedRustDataflowConnectionInput {
        connection_id: GraphConnectionId,
        to_port_id: NodePortId,
    },
    #[error(
        "generated Rust typed-dataflow connection `{connection_id}` sources port `{from_port_id}` that has no typed output mapping"
    )]
    UnmappedGeneratedRustDataflowConnectionOutput {
        connection_id: GraphConnectionId,
        from_port_id: NodePortId,
    },
    #[error(
        "generated Rust typed-dataflow node `{node_id}` port `{port_id}` value cannot be encoded as Rust type `{rust_type}`"
    )]
    UnsupportedGeneratedRustDataflowInputLiteral {
        node_id: GraphNodeId,
        port_id: NodePortId,
        rust_type: String,
    },
    #[error(
        "generated Rust typed-dataflow node `{node_id}` output port `{port_id}` feeds {consumer_count} by-value consumers; declare borrowed parameters or an explicit clone node"
    )]
    MovedGeneratedRustDataflowValueFanout {
        node_id: GraphNodeId,
        port_id: NodePortId,
        consumer_count: usize,
    },
    #[error(
        "generated Rust typed-dataflow node `{node_id}` output port `{port_id}` has mutable fan-out; use one mutable consumer or split with an explicit node"
    )]
    MutableGeneratedRustDataflowValueFanout {
        node_id: GraphNodeId,
        port_id: NodePortId,
        consumer_count: usize,
    },
    #[error("generated Rust path `{path}` is invalid: {reason}")]
    InvalidRustPath { path: String, reason: String },
    #[error("failed to encode generated Rust graph source: {0}")]
    EncodeGeneratedRust(#[from] std::fmt::Error),
    #[error("visual graph contains a compile-time dependency cycle")]
    CyclicGraph,
    #[error("{field} count {count} exceeds u32 range")]
    CountOverflow { field: &'static str, count: usize },
    #[error("failed to encode packed graph IR: {0}")]
    Encode(#[from] std::io::Error),
}

static PACKED_IR_COMPILER: PackedIrCompiler = PackedIrCompiler;
static GENERATED_RUST_AOT_COMPILER: GeneratedRustAotCompiler = GeneratedRustAotCompiler;

#[derive(Clone, Copy)]
pub struct GraphCompilerBackendRegistration {
    priority: i32,
    backend: &'static dyn GraphCompilerBackend,
}

impl GraphCompilerBackendRegistration {
    #[must_use]
    pub const fn new(backend: &'static dyn GraphCompilerBackend) -> Self {
        Self {
            priority: 0,
            backend,
        }
    }

    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub const fn priority(&self) -> i32 {
        self.priority
    }

    #[must_use]
    pub const fn backend(&self) -> &'static dyn GraphCompilerBackend {
        self.backend
    }
}

az_asset_builder::inventory::collect!(GraphCompilerBackendRegistration);

az_asset_builder::inventory::submit! {
    GraphCompilerBackendRegistration::new(&PACKED_IR_COMPILER).with_priority(100)
}

az_asset_builder::inventory::submit! {
    GraphCompilerBackendRegistration::new(&GENERATED_RUST_AOT_COMPILER).with_priority(90)
}

#[must_use]
pub fn registered_graph_compiler_backends() -> Vec<&'static dyn GraphCompilerBackend> {
    let mut registrations = az_asset_builder::inventory::iter::<GraphCompilerBackendRegistration>
        .into_iter()
        .copied()
        .collect::<Vec<_>>();
    registrations.sort_by(|left, right| {
        right.priority().cmp(&left.priority()).then_with(|| {
            left.backend()
                .backend_id()
                .cmp(right.backend().backend_id())
        })
    });
    registrations
        .into_iter()
        .map(|registration| registration.backend())
        .collect()
}

#[must_use]
pub fn default_graph_compiler_backends() -> Vec<&'static dyn GraphCompilerBackend> {
    registered_graph_compiler_backends()
}

/// Lists the source schema for every graph type registered in `registries`.
///
/// # Errors
///
/// Returns any [`GraphTypeCatalogError`] [`GraphTypeCatalog::compose`] returns
/// when the registered graph types do not compose into a valid catalog.
pub fn graph_source_schemas(
    registries: &Registries,
) -> Result<Vec<RegisteredGraphSourceSchema>, GraphTypeCatalogError> {
    let catalog = GraphTypeCatalog::compose(1, 0, registries)?;
    Ok(catalog
        .graph_types
        .iter()
        .map(graph_type_source_schema)
        .collect())
}

/// Compiles a graph source document using the globally registered backends.
///
/// # Errors
///
/// Returns any [`GraphCompileError`] [`compile_graph_source_with_backends`]
/// returns.
pub fn compile_graph_source(
    source_path: &str,
    source_uuid: uuid::Uuid,
    source_schema_type: &str,
    source_bytes: &[u8],
    platform: &str,
    registries: &Registries,
) -> Result<GraphCompileOutput, GraphCompileError> {
    let backends = registered_graph_compiler_backends();
    compile_graph_source_with_backends(
        source_path,
        source_uuid,
        source_schema_type,
        source_bytes,
        platform,
        registries,
        &backends,
    )
}

/// Decodes `source_bytes` as a visual graph document and compiles it with the
/// first backend in `backends` that supports the graph type.
///
/// # Errors
///
/// Returns [`GraphCompileError::GraphTypeMismatch`] if the document's declared
/// graph type differs from `source_schema_type`, or
/// [`GraphCompileError::UnknownRuntimeGraphType`] if no registered graph type
/// carries that schema id. Non-UTF-8 `source_bytes` and RON decode failures
/// convert into [`GraphCompileError`], as do catalog composition errors and any
/// error [`compile_graph_with_backends`] returns.
pub fn compile_graph_source_with_backends(
    source_path: &str,
    source_uuid: uuid::Uuid,
    source_schema_type: &str,
    source_bytes: &[u8],
    platform: &str,
    registries: &Registries,
    backends: &[&dyn GraphCompilerBackend],
) -> Result<GraphCompileOutput, GraphCompileError> {
    let document = decode_visual_graph_document_ron(std::str::from_utf8(source_bytes)?)?;
    if document.graph_type != source_schema_type {
        return Err(GraphCompileError::GraphTypeMismatch {
            expected: source_schema_type.to_string(),
            actual: document.graph_type,
        });
    }

    let graph_catalog = GraphTypeCatalog::compose(1, 0, registries)?;
    let graph_type = graph_catalog
        .graph_types
        .iter()
        .find(|graph_type| graph_type.id.as_str() == source_schema_type)
        .ok_or_else(|| GraphCompileError::UnknownRuntimeGraphType {
            source_schema: source_schema_type.to_string(),
        })?;
    let node_catalog = NodeTypeCatalog::compose(1, 0, registries)?;
    let request = GraphCompileRequest {
        source_path,
        source_uuid,
        platform,
        document: &document,
        graph_type,
        node_catalog: &node_catalog,
        registries,
    };

    compile_graph_with_backends(request, backends)
}

/// Dispatches `request` to the first backend in `backends` that supports the
/// graph type's declared compiler backend, then validates the products it emits.
///
/// # Errors
///
/// Returns [`GraphCompileError::MissingCompilerBackend`] if the graph type
/// declares no backend, [`GraphCompileError::MissingRuntimeProduct`] if it
/// declares no runtime product, [`GraphCompileError::UnsupportedCompilerBackend`]
/// if no supplied backend supports it, and [`GraphCompileError::GraphTypeMismatch`]
/// if the document's graph type differs from the descriptor's. Also propagates
/// contract validation, node-catalog validation, the backend's own compile
/// error, and output validation failures.
pub fn compile_graph_with_backends(
    request: GraphCompileRequest<'_>,
    backends: &[&dyn GraphCompilerBackend],
) -> Result<GraphCompileOutput, GraphCompileError> {
    let backend = request
        .graph_type
        .compiler_backend
        .as_ref()
        .ok_or_else(|| GraphCompileError::MissingCompilerBackend {
            graph_type: request.graph_type.id.as_str().to_string(),
        })?;
    let runtime_product = request.graph_type.runtime_product.as_ref().ok_or_else(|| {
        GraphCompileError::MissingRuntimeProduct {
            graph_type: request.graph_type.id.as_str().to_string(),
        }
    })?;
    let graph_type_id = request.graph_type.id.as_str().to_string();
    let runtime_asset_type_name = runtime_product.asset_type.clone();
    let request_registries = request.registries;
    validate_graph_compile_contract(&request)?;
    if request.document.graph_type != request.graph_type.id.as_str() {
        return Err(GraphCompileError::GraphTypeMismatch {
            expected: request.graph_type.id.as_str().to_string(),
            actual: request.document.graph_type.clone(),
        });
    }
    request.document.validate_against(request.node_catalog)?;

    let compiler = backends
        .iter()
        .find(|compiler| compiler.supports(backend))
        .ok_or_else(|| GraphCompileError::UnsupportedCompilerBackend {
            graph_type: request.graph_type.id.as_str().to_string(),
            backend: backend.id.clone(),
        })?;
    let output = compiler.compile(request)?;
    validate_graph_compile_output(
        &graph_type_id,
        &runtime_asset_type_name,
        &output,
        request_registries,
    )?;
    Ok(output)
}

/// Lowers `request` to the packed graph IR runtime product.
///
/// # Errors
///
/// Returns [`GraphCompileError::MissingCompilerBackend`] if the graph type
/// declares no backend, [`GraphCompileError::UnsupportedCompilerBackend`] if the
/// declared backend is not `PackedIr`, [`GraphCompileError::MissingRuntimeProduct`]
/// if it declares no runtime product, and [`GraphCompileError::GraphTypeMismatch`]
/// if the document's graph type differs from the descriptor's. Also propagates
/// contract validation, node-catalog validation, runtime-binding validation,
/// asset-type resolution, and IR encoding failures.
pub fn compile_packed_ir(
    request: &GraphCompileRequest<'_>,
) -> Result<GraphCompileOutput, GraphCompileError> {
    let backend = request
        .graph_type
        .compiler_backend
        .as_ref()
        .ok_or_else(|| GraphCompileError::MissingCompilerBackend {
            graph_type: request.graph_type.id.as_str().to_string(),
        })?;
    let GraphCompilerBackendKind::PackedIr { ir_schema } = &backend.kind else {
        return Err(GraphCompileError::UnsupportedCompilerBackend {
            graph_type: request.graph_type.id.as_str().to_string(),
            backend: backend.id.clone(),
        });
    };
    let runtime_product = request.graph_type.runtime_product.as_ref().ok_or_else(|| {
        GraphCompileError::MissingRuntimeProduct {
            graph_type: request.graph_type.id.as_str().to_string(),
        }
    })?;
    validate_graph_compile_contract(request)?;
    if request.document.graph_type != request.graph_type.id.as_str() {
        return Err(GraphCompileError::GraphTypeMismatch {
            expected: request.graph_type.id.as_str().to_string(),
            actual: request.document.graph_type.clone(),
        });
    }
    request.document.validate_against(request.node_catalog)?;
    validate_node_runtime_bindings(request.document, request.node_catalog)?;

    let asset_type = runtime_graph_asset_type(runtime_product, request.registries)?;
    let bytes = encode_packed_ir(request, backend, ir_schema, runtime_product)?;

    Ok(GraphCompileOutput::single(GraphCompileProduct {
        role: GraphCompileProductRole::RuntimeProduct,
        product_path: packed_ir_product_path(request.source_path),
        asset_type,
        product_format: PACKED_GRAPH_IR_FORMAT_ID,
        product_format_version: PACKED_GRAPH_IR_PRODUCT_FORMAT_VERSION,
        sub_id: RUNTIME_GRAPH_PRODUCT_SUB_ID,
        bytes,
    }))
}

/// Lowers `request` to the generated-Rust AOT products (manifest plus source).
///
/// # Errors
///
/// Returns [`GraphCompileError::MissingCompilerBackend`] if the graph type
/// declares no backend, [`GraphCompileError::UnsupportedCompilerBackend`] if the
/// declared backend is not `GeneratedRust`, [`GraphCompileError::MissingRuntimeProduct`]
/// if it declares no runtime product, [`GraphCompileError::UnsupportedRuntimeExecutionStrategy`]
/// if that product is not AOT-compiled code, and [`GraphCompileError::GraphTypeMismatch`]
/// if the document's graph type differs from the descriptor's. Also propagates
/// contract validation, node-catalog and ABI-specific binding validation, asset-type
/// resolution, entry-symbol and Rust path formation, and source/manifest encoding failures.
pub fn compile_aot_graph_products(
    request: &GraphCompileRequest<'_>,
) -> Result<GraphCompileOutput, GraphCompileError> {
    let backend = request
        .graph_type
        .compiler_backend
        .as_ref()
        .ok_or_else(|| GraphCompileError::MissingCompilerBackend {
            graph_type: request.graph_type.id.as_str().to_string(),
        })?;
    let GraphCompilerBackendKind::GeneratedRust { abi, .. } = &backend.kind else {
        return Err(GraphCompileError::UnsupportedCompilerBackend {
            graph_type: request.graph_type.id.as_str().to_string(),
            backend: backend.id.clone(),
        });
    };
    let runtime_product = request.graph_type.runtime_product.as_ref().ok_or_else(|| {
        GraphCompileError::MissingRuntimeProduct {
            graph_type: request.graph_type.id.as_str().to_string(),
        }
    })?;
    let RuntimeGraphExecutionStrategy::AotCompiledCode {
        language,
        package,
        entry_symbol,
        context_type,
    } = &runtime_product.execution_strategy
    else {
        return Err(GraphCompileError::UnsupportedRuntimeExecutionStrategy {
            graph_type: request.graph_type.id.as_str().to_string(),
            expected: "AOT compiled code",
        });
    };
    validate_graph_compile_contract(request)?;
    if request.document.graph_type != request.graph_type.id.as_str() {
        return Err(GraphCompileError::GraphTypeMismatch {
            expected: request.graph_type.id.as_str().to_string(),
            actual: request.document.graph_type.clone(),
        });
    }
    request.document.validate_against(request.node_catalog)?;
    validate_generated_rust_node_bindings(request.document, request.node_catalog)?;
    match abi {
        GeneratedRustGraphAbi::ContextSchedule => {
            validate_generated_rust_context_schedule(request)?;
        }
        GeneratedRustGraphAbi::TypedDataflow => {
            validate_generated_rust_typed_dataflow(request, context_type)?;
        }
    }
    let asset_type = runtime_graph_asset_type(runtime_product, request.registries)?;
    let context_type = rust_path_segments(context_type)?.join("::");
    let compiled_entry_symbol = generated_rust_aot_entry_symbol(entry_symbol, request.source_uuid)?;
    let manifest_bytes = encode_aot_graph_manifest(
        request,
        runtime_product,
        language,
        package,
        &compiled_entry_symbol,
        &context_type,
    )?;
    let source_bytes = match abi {
        GeneratedRustGraphAbi::ContextSchedule => encode_generated_rust_context_schedule_source(
            request,
            package,
            &compiled_entry_symbol,
            &context_type,
        )?,
        GeneratedRustGraphAbi::TypedDataflow => encode_generated_rust_typed_dataflow_source(
            request,
            package,
            &compiled_entry_symbol,
            &context_type,
        )?,
    };

    Ok(GraphCompileOutput {
        products: vec![
            GraphCompileProduct {
                role: GraphCompileProductRole::RuntimeProduct,
                product_path: aot_graph_manifest_product_path(request.source_path),
                asset_type,
                product_format: AOT_GRAPH_MANIFEST_FORMAT_ID,
                product_format_version: AOT_GRAPH_MANIFEST_PRODUCT_FORMAT_VERSION,
                sub_id: RUNTIME_GRAPH_PRODUCT_SUB_ID,
                bytes: manifest_bytes,
            },
            GraphCompileProduct {
                role: GraphCompileProductRole::GeneratedRustSource,
                product_path: generated_rust_graph_source_product_path(request.source_path),
                asset_type: GENERATED_RUST_GRAPH_SOURCE_ASSET_TYPE,
                product_format: GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID,
                product_format_version: GENERATED_RUST_GRAPH_SOURCE_PRODUCT_FORMAT_VERSION,
                sub_id: GENERATED_RUST_GRAPH_SOURCE_PRODUCT_SUB_ID,
                bytes: source_bytes,
            },
        ],
    })
}

fn validate_graph_compile_contract(
    request: &GraphCompileRequest<'_>,
) -> Result<(), GraphCompileError> {
    GraphTypeCatalog::try_new(1, 0, vec![request.graph_type.clone()])?;
    Ok(())
}

fn validate_graph_compile_output(
    graph_type: &str,
    runtime_asset_type_name: &str,
    output: &GraphCompileOutput,
    registries: &Registries,
) -> Result<(), GraphCompileError> {
    let expected_runtime_asset_type =
        az_core::composed_asset_type_by_name(registries, runtime_asset_type_name)
            .ok_or_else(|| GraphCompileError::UnregisteredRuntimeAssetType {
                asset_type: runtime_asset_type_name.to_string(),
            })?
            .asset_type();
    let mut runtime_product_count = 0usize;
    for product in &output.products {
        match product.role {
            GraphCompileProductRole::RuntimeProduct => {
                runtime_product_count += 1;
                if product.asset_type != expected_runtime_asset_type {
                    return Err(GraphCompileError::RuntimeAssetTypeMismatch {
                        graph_type: graph_type.to_string(),
                        expected: expected_runtime_asset_type,
                        actual: product.asset_type,
                    });
                }
            }
            GraphCompileProductRole::GeneratedRustSource => {
                if product.asset_type != GENERATED_RUST_GRAPH_SOURCE_ASSET_TYPE {
                    return Err(GraphCompileError::UnsupportedProductRoleAssetType {
                        graph_type: graph_type.to_string(),
                        role: product.role.as_str(),
                        asset_type: product.asset_type,
                    });
                }
            }
        }
        let format = az_asset_builder::composed_product_format(registries, product.product_format)
            .ok_or_else(|| GraphCompileError::UnregisteredProductFormat {
                format: product.product_format.as_str().to_string(),
            })?;
        if product.product_format_version != format.entry.current_version() {
            return Err(GraphCompileError::ProductFormatVersionMismatch {
                format: product.product_format.as_str().to_string(),
                expected: format.entry.current_version(),
                actual: product.product_format_version,
            });
        }
    }
    match runtime_product_count {
        0 => Err(GraphCompileError::MissingRuntimeCompileProduct {
            graph_type: graph_type.to_string(),
        }),
        1 => Ok(()),
        _ => Err(GraphCompileError::MultipleRuntimeCompileProducts {
            graph_type: graph_type.to_string(),
        }),
    }
}

fn runtime_graph_asset_type(
    runtime_product: &RuntimeGraphProductDescriptor,
    registries: &Registries,
) -> Result<AssetType, GraphCompileError> {
    az_core::composed_asset_type_by_name(registries, &runtime_product.asset_type)
        .ok_or_else(|| GraphCompileError::UnregisteredRuntimeAssetType {
            asset_type: runtime_product.asset_type.clone(),
        })
        .map(|registration| registration.asset_type())
}

/// The graph compiler rule, claiming exactly the graph types its host composed
/// a compiler backend for.
///
/// The claim is resolved here rather than declared as a constant because it is
/// a function of the composition: a gem that contributes a compilable graph
/// type widens this rule's schema set, and one that does not leaves it alone.
#[must_use]
pub fn graph_compiler_build_rule(context: &JobContext<'_>) -> BuildRule {
    BuildRule {
        name: "az.graph.compiler",
        id: GRAPH_COMPILER_BUILDER_ID,
        primary_source: SourceMatcher::Schemas {
            patterns: graph_compiler_patterns(),
            schemas: compilable_graph_type_schemas(context.registries()).into(),
        },
        source_dependencies: &[],
        version: 1,
        analysis_fingerprint: fingerprint::analysis(*context),
        product_formats: ProductFormatPolicy::Dynamic,
        create_jobs: create_graph_compiler_jobs,
        process_job: process_graph_compiler_job,
    }
}

fn create_graph_compiler_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
    let Some(source_schema_type) = request.source_schema_type else {
        return skipped_jobs();
    };
    if !is_compilable_graph_type(request.context.registries(), source_schema_type) {
        return skipped_jobs();
    }

    CreateJobsResponse {
        jobs: request
            .platforms
            .iter()
            .map(|platform| JobDescriptor {
                job_key: JobKey::new(GRAPH_COMPILER_JOB_KEY)
                    .expect("graph compiler job key is static and valid"),
                platform: *platform,
                job_dependencies: Vec::new(),
                critical: false,
            })
            .collect(),
        source_dependencies: Vec::new(),
        result: CreateJobsResult::Success,
    }
}

fn process_graph_compiler_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let Some(source_schema_type) = request.source_schema_type else {
        return failed_process_response();
    };
    let Ok(output) = compile_graph_source(
        &request.source_path,
        request.source_uuid,
        source_schema_type,
        request.source_bytes,
        request.platform,
        request.context.registries(),
    ) else {
        return failed_process_response();
    };
    let mut products = Vec::with_capacity(output.products.len());
    for product in output.products {
        // Graph compiler products bind to runtime graph descriptors registered by projects/gems.
        let Ok(product) = BuildProduct::from_dynamic_parts(
            product.product_path,
            product.asset_type,
            product.product_format,
            product.product_format_version,
            product.sub_id,
            product.bytes,
        ) else {
            return failed_process_response();
        };
        products.push(product);
    }

    ProcessJobResponse {
        products,
        product_dependencies: Vec::new(),
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

fn graph_compiler_patterns() -> &'static [AssetBuilderPattern] {
    static PATTERNS: OnceLock<Box<[AssetBuilderPattern]>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| vec![AssetBuilderPattern::wildcard("*")].into_boxed_slice())
        .as_ref()
}

/// The schema identities this host's composition makes compilable.
///
/// `SourceSchemaType` holds a `&'static str`, and a composed graph type's id
/// is only known at runtime, so each name is leaked once. The set is finite —
/// one entry per compilable graph type in the composition — and a host
/// composes once, so this does not grow with work.
fn compilable_graph_type_schemas(registries: &Registries) -> Vec<SourceSchemaType> {
    compilable_graph_type_names(registries)
        .into_iter()
        .map(|schema| SourceSchemaType::from_dynamic_parts(String::leak(schema)))
        .collect()
}

fn compilable_graph_type_names(registries: &Registries) -> Vec<String> {
    graph_types(registries)
        .into_iter()
        .filter(is_compilable_graph_type_descriptor)
        .map(|graph_type| graph_type.id.as_str().to_string())
        .collect()
}

fn is_compilable_graph_type(registries: &Registries, source_schema_type: &str) -> bool {
    graph_types(registries).into_iter().any(|graph_type| {
        graph_type.id.as_str() == source_schema_type
            && is_compilable_graph_type_descriptor(&graph_type)
    })
}

fn is_compilable_graph_type_descriptor(graph_type: &GraphTypeDescriptor) -> bool {
    if !graph_type.execution_mode.requires_runtime_product() {
        return false;
    }
    let Some(backend) = graph_type.compiler_backend.as_ref() else {
        return false;
    };
    registered_graph_compiler_backends()
        .into_iter()
        .any(|compiler| compiler.supports(backend))
}

fn graph_type_source_schema(graph_type: &GraphTypeDescriptor) -> RegisteredGraphSourceSchema {
    let category = graph_type.category_path.join("/");
    let authoring = match graph_type.source_workflow.kind {
        GraphSourceWorkflowKind::ProjectDocument => {
            RegisteredGraphSourceAuthoring::ProjectDocument {
                schema_type: graph_type
                    .source_workflow
                    .source_schema
                    .clone()
                    .unwrap_or_else(|| graph_type.id.as_str().to_string()),
            }
        }
        GraphSourceWorkflowKind::File => RegisteredGraphSourceAuthoring::File {
            source_root: graph_type
                .source_workflow
                .source_root
                .clone()
                .unwrap_or_else(|| az_asset_builder::PROJECT_SOURCE_ROOT.to_string()),
            default_path_prefix: graph_type
                .source_workflow
                .default_path_prefix
                .clone()
                .unwrap_or_default(),
            extensions: graph_type
                .source_workflow
                .default_extension
                .iter()
                .cloned()
                .collect(),
            can_create: false,
            can_edit: true,
        },
    };

    RegisteredGraphSourceSchema {
        schema_type: graph_type.id.as_str().to_string(),
        owner: graph_type.source_workflow.workflow_id.clone(),
        label: graph_type.display_name.clone(),
        category,
        authoring,
    }
}

fn validate_node_runtime_bindings(
    document: &VisualGraphDocument,
    catalog: &NodeTypeCatalog,
) -> Result<(), GraphCompileError> {
    for node in &document.nodes {
        let descriptor = catalog
            .node_type_version(&node.node_type, node.node_type_version)
            .expect("document validation guarantees node descriptors are present");
        if descriptor.runtime_binding.is_none() {
            return Err(GraphCompileError::MissingNodeRuntimeBinding {
                node_id: node.id,
                node_type: node.node_type.as_str().to_string(),
            });
        }
    }
    Ok(())
}

fn validate_generated_rust_node_bindings(
    document: &VisualGraphDocument,
    catalog: &NodeTypeCatalog,
) -> Result<(), GraphCompileError> {
    for node in &document.nodes {
        let descriptor = catalog
            .node_type_version(&node.node_type, node.node_type_version)
            .expect("document validation guarantees node descriptors are present");
        let Some(binding) = descriptor.runtime_binding.as_ref() else {
            return Err(GraphCompileError::MissingNodeRuntimeBinding {
                node_id: node.id,
                node_type: node.node_type.as_str().to_string(),
            });
        };
        match binding {
            NodeRuntimeBinding::RustSymbol {
                symbol, call_abi, ..
            } => {
                validate_rust_path(symbol)?;
                validate_rust_node_call_abi(node, descriptor, call_abi)?;
            }
            other => {
                return Err(GraphCompileError::UnsupportedGeneratedRustNodeBinding {
                    node_id: node.id,
                    node_type: node.node_type.as_str().to_string(),
                    binding_kind: runtime_binding_kind(other),
                });
            }
        }
    }
    Ok(())
}

fn validate_rust_node_call_abi(
    node: &GraphNode,
    descriptor: &az_node_graph::NodeTypeDescriptor,
    call_abi: &RustNodeCallAbi,
) -> Result<(), GraphCompileError> {
    if let RustNodeCallAbi::TypedDataflow(dataflow) = call_abi {
        for parameter in &dataflow.parameters {
            validate_rust_type_for_node(node, descriptor, &parameter.rust_type)?;
        }
        match &dataflow.output {
            RustDataflowOutput::None => {}
            RustDataflowOutput::Single { rust_type, .. } => {
                validate_rust_type_for_node(node, descriptor, rust_type)?;
            }
            RustDataflowOutput::StructFields { rust_type, fields } => {
                validate_rust_type_for_node(node, descriptor, rust_type)?;
                for field in fields {
                    validate_rust_type_for_node(node, descriptor, &field.rust_type)?;
                    validate_rust_identifier(field.field.as_str(), field.field.as_str()).map_err(
                        |_| GraphCompileError::InvalidGeneratedRustDataflowField {
                            node_id: node.id,
                            node_type: descriptor.id.as_str().to_string(),
                            field: field.field.clone(),
                        },
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn validate_rust_type_for_node(
    node: &GraphNode,
    descriptor: &az_node_graph::NodeTypeDescriptor,
    rust_type: &str,
) -> Result<(), GraphCompileError> {
    rust_path_segments(rust_type).map_err(|error| match error {
        GraphCompileError::InvalidRustPath { reason, .. } => {
            GraphCompileError::InvalidGeneratedRustDataflowRustType {
                node_id: node.id,
                node_type: descriptor.id.as_str().to_string(),
                rust_type: rust_type.to_string(),
                reason,
            }
        }
        other => other,
    })?;
    Ok(())
}

fn validate_generated_rust_context_schedule(
    request: &GraphCompileRequest<'_>,
) -> Result<(), GraphCompileError> {
    for node in &request.document.nodes {
        if let Some(port_id) = node.input_values.keys().next() {
            return Err(GraphCompileError::UnsupportedGeneratedRustInputValue {
                graph_type: request.graph_type.id.as_str().to_string(),
                node_id: node.id,
                port_id: *port_id,
            });
        }
    }

    let nodes_by_id = request
        .document
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    for connection in &request.document.connections {
        let from_node = nodes_by_id
            .get(&connection.from.node_id)
            .expect("document validation guarantees source connection nodes exist");
        let to_node = nodes_by_id
            .get(&connection.to.node_id)
            .expect("document validation guarantees target connection nodes exist");
        let from_descriptor = request
            .node_catalog
            .node_type_version(&from_node.node_type, from_node.node_type_version)
            .expect("document validation guarantees source node descriptors exist");
        let to_descriptor = request
            .node_catalog
            .node_type_version(&to_node.node_type, to_node.node_type_version)
            .expect("document validation guarantees target node descriptors exist");
        let from_port = from_descriptor
            .ports
            .iter()
            .find(|port| port.id == connection.from.port_id)
            .expect("document validation guarantees source ports exist");
        let to_port = to_descriptor
            .ports
            .iter()
            .find(|port| port.id == connection.to.port_id)
            .expect("document validation guarantees target ports exist");
        if !from_port.value.is_execution() || !to_port.value.is_execution() {
            return Err(GraphCompileError::UnsupportedGeneratedRustDataConnection {
                graph_type: request.graph_type.id.as_str().to_string(),
                connection_id: connection.id,
                from_port_id: connection.from.port_id,
                to_port_id: connection.to.port_id,
            });
        }
    }

    Ok(())
}

fn validate_generated_rust_typed_dataflow(
    request: &GraphCompileRequest<'_>,
    context_type: &str,
) -> Result<(), GraphCompileError> {
    validate_rust_path(context_type)?;
    let nodes_by_id = request
        .document
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();

    validate_dataflow_nodes(request, context_type)?;
    validate_dataflow_connection_endpoints(request, &nodes_by_id)?;
    validate_dataflow_connection_fanout(request, &nodes_by_id)?;
    Ok(())
}

/// Checks every node's typed-dataflow call: its runtime-context parameters
/// agree with the graph's context type, and every ABI input port is satisfied
/// by a connection, an authored value, or a port default.
fn validate_dataflow_nodes(
    request: &GraphCompileRequest<'_>,
    context_type: &str,
) -> Result<(), GraphCompileError> {
    let incoming_data_connections = incoming_data_connections(request);

    for node in &request.document.nodes {
        let descriptor = descriptor_for_node(request.node_catalog, node);
        let dataflow = typed_dataflow_call_for_node(node, descriptor)?;
        validate_dataflow_runtime_context_parameters(node, descriptor, dataflow, context_type)?;
        let mapped_inputs = dataflow_input_ports(dataflow);
        for port_id in node.input_values.keys() {
            if !mapped_inputs.contains(port_id) {
                return Err(GraphCompileError::UnmappedGeneratedRustDataflowInputValue {
                    node_id: node.id,
                    port_id: *port_id,
                });
            }
        }
        for port_id in &mapped_inputs {
            if let Some(connections) = incoming_data_connections.get(&(node.id, *port_id)) {
                if connections.len() > 1 {
                    return Err(
                        GraphCompileError::MultipleGeneratedRustDataflowInputConnections {
                            node_id: node.id,
                            port_id: *port_id,
                            connection_count: connections.len(),
                        },
                    );
                }
                continue;
            }
            if node.input_values.contains_key(port_id) {
                continue;
            }
            let port = descriptor
                .ports
                .iter()
                .find(|candidate| candidate.id == *port_id)
                .expect("node catalog validation guarantees ABI input ports exist");
            if port.default_value.is_some() {
                continue;
            }
            return Err(GraphCompileError::MissingGeneratedRustDataflowInput {
                node_id: node.id,
                port_id: *port_id,
            });
        }
    }

    Ok(())
}

/// Every runtime-context parameter must be taken by reference rather than by
/// value, and must name the graph's own context type.
fn validate_dataflow_runtime_context_parameters(
    node: &GraphNode,
    descriptor: &NodeTypeDescriptor,
    dataflow: &RustTypedDataflowNodeCall,
    context_type: &str,
) -> Result<(), GraphCompileError> {
    for parameter in &dataflow.parameters {
        if parameter.source != RustDataflowParameterSource::RuntimeContext {
            continue;
        }
        if parameter.passing == RustValuePassing::ByValue {
            return Err(GraphCompileError::InvalidGeneratedRustDataflowRustType {
                node_id: node.id,
                node_type: descriptor.id.as_str().to_string(),
                rust_type: parameter.rust_type.clone(),
                reason: "runtime context parameters must be passed by shared or mutable reference"
                    .to_string(),
            });
        }
        let normalized_parameter_type = rust_path_segments(&parameter.rust_type)?.join("::");
        let normalized_context_type = rust_path_segments(context_type)?.join("::");
        if normalized_parameter_type != normalized_context_type {
            return Err(GraphCompileError::InvalidGeneratedRustDataflowRustType {
                node_id: node.id,
                node_type: descriptor.id.as_str().to_string(),
                rust_type: parameter.rust_type.clone(),
                reason: format!(
                    "runtime context parameter must use graph context type `{normalized_context_type}`"
                ),
            });
        }
    }

    Ok(())
}

/// Both endpoints of every data connection must be mapped by the typed-dataflow
/// ABI: the target port to a parameter, the source port to an output.
fn validate_dataflow_connection_endpoints(
    request: &GraphCompileRequest<'_>,
    nodes_by_id: &BTreeMap<GraphNodeId, &GraphNode>,
) -> Result<(), GraphCompileError> {
    for connection in &request.document.connections {
        let Some((from_descriptor, from_port, to_descriptor, to_port)) =
            connection_port_descriptors(request, nodes_by_id, connection)
        else {
            continue;
        };
        if !from_port.value.is_data() || !to_port.value.is_data() {
            continue;
        }

        let to_node = nodes_by_id
            .get(&connection.to.node_id)
            .expect("document validation guarantees target connection nodes exist");
        let to_dataflow = typed_dataflow_call_for_node(to_node, to_descriptor)?;
        if dataflow_input_parameter(to_dataflow, connection.to.port_id).is_none() {
            return Err(
                GraphCompileError::UnmappedGeneratedRustDataflowConnectionInput {
                    connection_id: connection.id,
                    to_port_id: connection.to.port_id,
                },
            );
        }

        let from_node = nodes_by_id
            .get(&connection.from.node_id)
            .expect("document validation guarantees source connection nodes exist");
        let from_dataflow = typed_dataflow_call_for_node(from_node, from_descriptor)?;
        if dataflow_output_type(from_dataflow, connection.from.port_id).is_none() {
            return Err(
                GraphCompileError::UnmappedGeneratedRustDataflowConnectionOutput {
                    connection_id: connection.id,
                    from_port_id: connection.from.port_id,
                },
            );
        }
    }

    Ok(())
}

fn validate_dataflow_connection_fanout(
    request: &GraphCompileRequest<'_>,
    nodes_by_id: &BTreeMap<GraphNodeId, &GraphNode>,
) -> Result<(), GraphCompileError> {
    let mut consumers = BTreeMap::<(GraphNodeId, NodePortId), Vec<RustValuePassing>>::new();
    for connection in &request.document.connections {
        let Some((_from_descriptor, from_port, to_descriptor, to_port)) =
            connection_port_descriptors(request, nodes_by_id, connection)
        else {
            continue;
        };
        if !from_port.value.is_data() || !to_port.value.is_data() {
            continue;
        }
        let to_node = nodes_by_id
            .get(&connection.to.node_id)
            .expect("document validation guarantees target connection nodes exist");
        let to_dataflow = typed_dataflow_call_for_node(to_node, to_descriptor)?;
        if let Some(parameter) = dataflow_input_parameter(to_dataflow, connection.to.port_id) {
            consumers
                .entry((connection.from.node_id, connection.from.port_id))
                .or_default()
                .push(parameter.passing);
        }
    }

    for ((node_id, port_id), passing) in consumers {
        let consumer_count = passing.len();
        if consumer_count <= 1 {
            continue;
        }
        if passing.contains(&RustValuePassing::ByMutableRef) {
            return Err(GraphCompileError::MutableGeneratedRustDataflowValueFanout {
                node_id,
                port_id,
                consumer_count,
            });
        }
        if passing.contains(&RustValuePassing::ByValue) {
            return Err(GraphCompileError::MovedGeneratedRustDataflowValueFanout {
                node_id,
                port_id,
                consumer_count,
            });
        }
    }
    Ok(())
}

fn incoming_data_connections(
    request: &GraphCompileRequest<'_>,
) -> BTreeMap<(GraphNodeId, NodePortId), Vec<GraphConnectionId>> {
    let nodes_by_id = request
        .document
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut incoming = BTreeMap::<(GraphNodeId, NodePortId), Vec<GraphConnectionId>>::new();
    for connection in &request.document.connections {
        let Some((_from_descriptor, from_port, _to_descriptor, to_port)) =
            connection_port_descriptors(request, &nodes_by_id, connection)
        else {
            continue;
        };
        if from_port.value.is_data() && to_port.value.is_data() {
            incoming
                .entry((connection.to.node_id, connection.to.port_id))
                .or_default()
                .push(connection.id);
        }
    }
    incoming
}

fn connection_port_descriptors<'a>(
    request: &'a GraphCompileRequest<'_>,
    nodes_by_id: &BTreeMap<GraphNodeId, &GraphNode>,
    connection: &GraphConnection,
) -> Option<(
    &'a NodeTypeDescriptor,
    &'a NodePortDescriptor,
    &'a NodeTypeDescriptor,
    &'a NodePortDescriptor,
)> {
    let from_node = nodes_by_id.get(&connection.from.node_id)?;
    let to_node = nodes_by_id.get(&connection.to.node_id)?;
    let from_descriptor = descriptor_for_node(request.node_catalog, from_node);
    let to_descriptor = descriptor_for_node(request.node_catalog, to_node);
    let from_port = from_descriptor
        .ports
        .iter()
        .find(|port| port.id == connection.from.port_id)?;
    let to_port = to_descriptor
        .ports
        .iter()
        .find(|port| port.id == connection.to.port_id)?;
    Some((from_descriptor, from_port, to_descriptor, to_port))
}

fn descriptor_for_node<'a>(
    catalog: &'a NodeTypeCatalog,
    node: &GraphNode,
) -> &'a NodeTypeDescriptor {
    catalog
        .node_type_version(&node.node_type, node.node_type_version)
        .expect("document validation guarantees node descriptors are present")
}

fn typed_dataflow_call_for_node<'a>(
    node: &GraphNode,
    descriptor: &'a NodeTypeDescriptor,
) -> Result<&'a RustTypedDataflowNodeCall, GraphCompileError> {
    let Some(NodeRuntimeBinding::RustSymbol {
        call_abi: RustNodeCallAbi::TypedDataflow(dataflow),
        ..
    }) = descriptor.runtime_binding.as_ref()
    else {
        return Err(GraphCompileError::UnsupportedGeneratedRustDataflowNodeAbi {
            node_id: node.id,
            node_type: node.node_type.as_str().to_string(),
        });
    };
    Ok(dataflow)
}

fn dataflow_input_ports(dataflow: &RustTypedDataflowNodeCall) -> BTreeSet<NodePortId> {
    dataflow
        .parameters
        .iter()
        .filter_map(|parameter| match parameter.source {
            RustDataflowParameterSource::RuntimeContext => None,
            RustDataflowParameterSource::InputPort { port } => Some(port),
        })
        .collect()
}

fn dataflow_input_parameter(
    dataflow: &RustTypedDataflowNodeCall,
    port_id: NodePortId,
) -> Option<&RustDataflowParameter> {
    dataflow
        .parameters
        .iter()
        .find(|parameter| matches!(parameter.source, RustDataflowParameterSource::InputPort { port } if port == port_id))
}

fn dataflow_output_type(dataflow: &RustTypedDataflowNodeCall, port_id: NodePortId) -> Option<&str> {
    match &dataflow.output {
        RustDataflowOutput::Single { port, rust_type } if *port == port_id => Some(rust_type),
        RustDataflowOutput::StructFields { fields, .. } => fields
            .iter()
            .find(|field| field.port == port_id)
            .map(|field| field.rust_type.as_str()),
        RustDataflowOutput::None | RustDataflowOutput::Single { .. } => None,
    }
}

const fn runtime_binding_kind(binding: &NodeRuntimeBinding) -> &'static str {
    match binding {
        NodeRuntimeBinding::RustSymbol { .. } => "RustSymbol",
        NodeRuntimeBinding::AssetBuilder { .. } => "AssetBuilder",
        NodeRuntimeBinding::RuntimeComponent { .. } => "RuntimeComponent",
        NodeRuntimeBinding::External { .. } => "External",
    }
}

fn encode_packed_ir(
    request: &GraphCompileRequest<'_>,
    backend: &GraphCompilerBackendDescriptor,
    ir_schema: &str,
    runtime_product: &RuntimeGraphProductDescriptor,
) -> Result<Vec<u8>, GraphCompileError> {
    let mut nodes = request.document.nodes.iter().collect::<Vec<_>>();
    nodes.sort_by_key(|node| node.id);
    let node_indices = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect::<BTreeMap<_, _>>();
    let mut connections = request.document.connections.iter().collect::<Vec<_>>();
    connections.sort_by_key(|connection| connection.id);
    let execution_order = topological_order(&nodes, &connections, &node_indices)?;
    let node_catalog_hash = request.node_catalog.content_hash()?;

    let mut strings = StringTable::default();
    let graph_type_index = strings.intern(request.graph_type.id.as_str())?;
    let product_kind_index = strings.intern(&runtime_product.product_kind)?;
    let backend_index = strings.intern(&backend.id)?;
    let ir_schema_index = strings.intern(ir_schema)?;
    for marker in &backend.capability_markers {
        strings.intern(marker)?;
    }
    intern_nodes(&mut strings, &nodes, request.node_catalog)?;

    let mut bytes = Vec::new();
    bytes.write_all(PACKED_GRAPH_IR_MAGIC)?;
    write_u32(&mut bytes, PACKED_GRAPH_IR_VERSION)?;
    write_u32(&mut bytes, runtime_product_flags(runtime_product))?;
    write_u32(&mut bytes, request.graph_type.version)?;
    write_u32(&mut bytes, request.node_catalog.catalog_version)?;
    bytes.write_all(&node_catalog_hash)?;
    write_uuid(&mut bytes, request.source_uuid)?;
    write_u32(&mut bytes, graph_type_index)?;
    write_u32(&mut bytes, product_kind_index)?;
    write_u32(&mut bytes, backend_index)?;
    write_u32(&mut bytes, ir_schema_index)?;
    write_u32_checked(&mut bytes, "strings", strings.len())?;
    write_u32_checked(
        &mut bytes,
        "backend markers",
        backend.capability_markers.len(),
    )?;
    write_u32_checked(&mut bytes, "nodes", nodes.len())?;
    write_u32_checked(
        &mut bytes,
        "ports",
        total_port_count(&nodes, request.node_catalog),
    )?;
    write_u32_checked(&mut bytes, "input values", total_input_value_count(&nodes))?;
    write_u32_checked(&mut bytes, "connections", connections.len())?;
    write_u32_checked(&mut bytes, "execution order", execution_order.len())?;

    strings.write(&mut bytes)?;
    for marker in &backend.capability_markers {
        write_u32(
            &mut bytes,
            strings.index(marker).expect("marker was interned"),
        )?;
    }
    write_nodes(&mut bytes, &strings, &nodes, request.node_catalog)?;
    write_connections(
        &mut bytes,
        &nodes,
        &node_indices,
        &connections,
        request.node_catalog,
    )?;
    for node_index in execution_order {
        write_u32(&mut bytes, checked_u32("execution node index", node_index)?)?;
    }

    Ok(bytes)
}

fn encode_aot_graph_manifest(
    request: &GraphCompileRequest<'_>,
    runtime_product: &RuntimeGraphProductDescriptor,
    language: &str,
    package: &str,
    entry_symbol: &str,
    context_type: &str,
) -> Result<Vec<u8>, GraphCompileError> {
    let node_catalog_hash = request.node_catalog.content_hash()?;

    let mut bytes = Vec::new();
    bytes.write_all(AOT_GRAPH_MANIFEST_MAGIC)?;
    write_u32(&mut bytes, AOT_GRAPH_MANIFEST_VERSION)?;
    write_u32(&mut bytes, runtime_product_flags(runtime_product))?;
    write_u32(&mut bytes, request.graph_type.version)?;
    write_u32(&mut bytes, request.node_catalog.catalog_version)?;
    bytes.write_all(&node_catalog_hash)?;
    write_uuid(&mut bytes, request.source_uuid)?;
    write_text(&mut bytes, request.graph_type.id.as_str())?;
    write_text(&mut bytes, &runtime_product.product_kind)?;
    write_text(&mut bytes, language)?;
    write_text(&mut bytes, package)?;
    write_text(&mut bytes, entry_symbol)?;
    write_text(&mut bytes, context_type)?;
    Ok(bytes)
}

fn generated_rust_aot_entry_symbol(
    entry_symbol: &str,
    source_uuid: uuid::Uuid,
) -> Result<String, GraphCompileError> {
    let segments = rust_path_segments(entry_symbol)?;
    let Some((first, rest)) = segments.split_first() else {
        return Err(GraphCompileError::InvalidRustPath {
            path: entry_symbol.to_string(),
            reason: "entry symbol is empty".to_string(),
        });
    };
    let source_suffix = source_uuid.simple();
    let symbol = if rest.is_empty() {
        format!("{first}_{source_suffix}")
    } else {
        let mut rewritten = Vec::with_capacity(segments.len());
        rewritten.push(format!("{first}_{source_suffix}"));
        rewritten.extend(rest.iter().map(|segment| (*segment).to_string()));
        rewritten.join("::")
    };
    validate_rust_path(&symbol)?;
    Ok(symbol)
}

fn encode_generated_rust_context_schedule_source(
    request: &GraphCompileRequest<'_>,
    package: &str,
    entry_symbol: &str,
    context_type: &str,
) -> Result<Vec<u8>, GraphCompileError> {
    let entry_segments = rust_path_segments(entry_symbol)?;
    let context_type = rust_path_segments(context_type)?.join("::");
    let Some((entry_fn, entry_modules)) = entry_segments.split_last() else {
        return Err(GraphCompileError::InvalidRustPath {
            path: entry_symbol.to_string(),
            reason: "entry symbol is empty".to_string(),
        });
    };
    let mut nodes = request.document.nodes.iter().collect::<Vec<_>>();
    nodes.sort_by_key(|node| node.id);
    let node_indices = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect::<BTreeMap<_, _>>();
    let mut connections = request.document.connections.iter().collect::<Vec<_>>();
    connections.sort_by_key(|connection| connection.id);
    let execution_order = topological_order(&nodes, &connections, &node_indices)?;

    let mut source = String::new();
    writeln!(
        &mut source,
        "// Generated by az-graph-builder from `{}`. Do not edit by hand.",
        request.source_path
    )?;
    writeln!(&mut source)?;
    for module in entry_modules {
        writeln!(&mut source, "pub mod {module} {{")?;
    }
    let indent = "    ".repeat(entry_modules.len());
    writeln!(&mut source, "{indent}pub fn {entry_fn}(")?;
    writeln!(
        &mut source,
        "{indent}    mut context: az_graph_runtime::AotGraphExecuteContext<'_>,"
    )?;
    writeln!(
        &mut source,
        "{indent}) -> Result<(), az_graph_runtime::AotGraphExecutionError> {{"
    )?;
    writeln!(
        &mut source,
        "{indent}    let runtime_context = context.runtime_context_mut::<{context_type}>()?;"
    )?;
    for node_index in execution_order {
        let node = nodes[node_index];
        let descriptor = request
            .node_catalog
            .node_type_version(&node.node_type, node.node_type_version)
            .expect("document validation guarantees node descriptors are present");
        let NodeRuntimeBinding::RustSymbol { symbol, .. } = descriptor
            .runtime_binding
            .as_ref()
            .expect("generated Rust binding validation guarantees a binding")
        else {
            unreachable!("generated Rust binding validation accepts only Rust symbols");
        };
        let symbol = rust_path_segments(symbol)?.join("::");
        writeln!(&mut source, "{indent}    {symbol}(&mut *runtime_context)?;")?;
    }
    writeln!(&mut source, "{indent}    Ok(())")?;
    writeln!(&mut source, "{indent}}}")?;
    for depth in (0..entry_modules.len()).rev() {
        let indent = "    ".repeat(depth);
        writeln!(&mut source, "{indent}}}")?;
    }
    write_aot_graph_registration(
        &mut source,
        package,
        entry_symbol,
        request.graph_type.id.as_str(),
        &context_type,
    )?;
    Ok(source.into_bytes())
}

/// Close an AOT Rust graph product with its own registration, as data.
///
/// The product exports a `const` rather than submitting itself anywhere: the
/// projection module that `include!`s these products is what registers them,
/// through the composing host's registrar, so a graph reaches a runtime only
/// by way of a composition that owns it.
fn write_aot_graph_registration(
    source: &mut String,
    package: &str,
    entry_symbol: &str,
    graph_type: &str,
    context_type: &str,
) -> Result<(), GraphCompileError> {
    writeln!(source)?;
    writeln!(
        source,
        "/// This graph's registration, composed by the crate's generated glue."
    )?;
    writeln!(
        source,
        "pub const {GENERATED_RUST_GRAPH_REGISTRATION_ITEM}: az_graph_runtime::AotGraphRuntimeRegistration ="
    )?;
    writeln!(
        source,
        "    az_graph_runtime::AotGraphRuntimeRegistration::new("
    )?;
    writeln!(source, "        {package:?},")?;
    writeln!(source, "        {entry_symbol:?},")?;
    writeln!(source, "        {graph_type:?},")?;
    writeln!(source, "        {context_type:?},")?;
    writeln!(source, "        {entry_symbol},")?;
    writeln!(source, "    );")?;
    Ok(())
}

fn encode_generated_rust_typed_dataflow_source(
    request: &GraphCompileRequest<'_>,
    package: &str,
    entry_symbol: &str,
    context_type: &str,
) -> Result<Vec<u8>, GraphCompileError> {
    let entry_segments = rust_path_segments(entry_symbol)?;
    let context_type = rust_path_segments(context_type)?.join("::");
    let Some((entry_fn, entry_modules)) = entry_segments.split_last() else {
        return Err(GraphCompileError::InvalidRustPath {
            path: entry_symbol.to_string(),
            reason: "entry symbol is empty".to_string(),
        });
    };
    let mut nodes = request.document.nodes.iter().collect::<Vec<_>>();
    nodes.sort_by_key(|node| node.id);
    let node_indices = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect::<BTreeMap<_, _>>();
    let mut connections = request.document.connections.iter().collect::<Vec<_>>();
    connections.sort_by_key(|connection| connection.id);
    let execution_order = topological_order(&nodes, &connections, &node_indices)?;
    let incoming_by_target = incoming_data_connections_by_target(
        &connections,
        request.node_catalog,
        &nodes,
        &node_indices,
    );
    let mutable_outputs =
        mutable_dataflow_outputs(&connections, request.node_catalog, &nodes, &node_indices)?;

    let mut source = String::new();
    let indent = write_generated_rust_header(
        &mut source,
        request.source_path,
        entry_fn,
        entry_modules,
        &context_type,
    )?;

    for node_index in execution_order {
        let node = nodes[node_index];
        let descriptor = descriptor_for_node(request.node_catalog, node);
        let NodeRuntimeBinding::RustSymbol { symbol, .. } = descriptor
            .runtime_binding
            .as_ref()
            .expect("generated Rust binding validation guarantees a binding")
        else {
            unreachable!("generated Rust binding validation accepts only Rust symbols");
        };
        let dataflow = typed_dataflow_call_for_node(node, descriptor)?;
        let symbol = rust_path_segments(symbol)?.join("::");
        let mut prelude = Vec::<String>::new();
        let mut arguments = Vec::<String>::new();
        for parameter in &dataflow.parameters {
            let expression = dataflow_parameter_expression(
                node,
                descriptor,
                parameter,
                &incoming_by_target,
                &mut prelude,
            )?;
            arguments.push(expression);
        }
        for line in prelude {
            writeln!(&mut source, "{indent}    {line}")?;
        }
        let call = format!("{symbol}({})", arguments.join(", "));
        write_dataflow_call(
            &mut source,
            indent.as_str(),
            node,
            dataflow,
            &call,
            &mutable_outputs,
        )?;
    }
    writeln!(&mut source, "{indent}    Ok(())")?;
    writeln!(&mut source, "{indent}}}")?;
    for depth in (0..entry_modules.len()).rev() {
        let indent = "    ".repeat(depth);
        writeln!(&mut source, "{indent}}}")?;
    }
    write_aot_graph_registration(
        &mut source,
        package,
        entry_symbol,
        request.graph_type.id.as_str(),
        &context_type,
    )?;
    Ok(source.into_bytes())
}

/// Writes the generated file's banner, the module nesting the entry symbol's
/// path implies, and the entry function's signature and runtime-context binding.
///
/// Returns the indentation the entry function's body sits at, which is set by
/// how deeply `entry_modules` nests it.
fn write_generated_rust_header(
    source: &mut String,
    source_path: &str,
    entry_fn: &str,
    entry_modules: &[&str],
    context_type: &str,
) -> Result<String, GraphCompileError> {
    writeln!(
        source,
        "// Generated by az-graph-builder from `{source_path}`. Do not edit by hand."
    )?;
    writeln!(source)?;
    for module in entry_modules {
        writeln!(source, "pub mod {module} {{")?;
    }
    let indent = "    ".repeat(entry_modules.len());
    writeln!(source, "{indent}pub fn {entry_fn}(")?;
    writeln!(
        source,
        "{indent}    mut context: az_graph_runtime::AotGraphExecuteContext<'_>,"
    )?;
    writeln!(
        source,
        "{indent}) -> Result<(), az_graph_runtime::AotGraphExecutionError> {{"
    )?;
    writeln!(
        source,
        "{indent}    let runtime_context = context.runtime_context_mut::<{context_type}>()?;"
    )?;
    Ok(indent)
}

fn dataflow_parameter_expression(
    node: &GraphNode,
    descriptor: &NodeTypeDescriptor,
    parameter: &RustDataflowParameter,
    incoming_by_target: &BTreeMap<(GraphNodeId, NodePortId), &GraphConnection>,
    prelude: &mut Vec<String>,
) -> Result<String, GraphCompileError> {
    let value = match parameter.source {
        RustDataflowParameterSource::RuntimeContext => {
            return Ok(match parameter.passing {
                RustValuePassing::ByValue => "runtime_context".to_string(),
                RustValuePassing::BySharedRef => "&*runtime_context".to_string(),
                RustValuePassing::ByMutableRef => "&mut *runtime_context".to_string(),
            });
        }
        RustDataflowParameterSource::InputPort { port } => {
            if let Some(connection) = incoming_by_target.get(&(node.id, port)) {
                dataflow_output_local(connection.from.node_id, connection.from.port_id)
            } else {
                let port_descriptor = descriptor
                    .ports
                    .iter()
                    .find(|candidate| candidate.id == port)
                    .expect("node catalog validation guarantees ABI input ports exist");
                let value = node
                    .input_values
                    .get(&port)
                    .or(port_descriptor.default_value.as_ref())
                    .ok_or(GraphCompileError::MissingGeneratedRustDataflowInput {
                        node_id: node.id,
                        port_id: port,
                    })?;
                let local = dataflow_input_local(node.id, port);
                let literal =
                    rust_literal_for_reflected_value(value, &parameter.rust_type, node.id, port)?;
                let mutable = if parameter.passing == RustValuePassing::ByMutableRef {
                    "mut "
                } else {
                    ""
                };
                prelude.push(format!(
                    "let {mutable}{local}: {} = {literal};",
                    rust_path_segments(&parameter.rust_type)?.join("::")
                ));
                local
            }
        }
    };
    Ok(match parameter.passing {
        RustValuePassing::ByValue => value,
        RustValuePassing::BySharedRef => format!("&{value}"),
        RustValuePassing::ByMutableRef => format!("&mut {value}"),
    })
}

fn write_dataflow_call(
    source: &mut String,
    indent: &str,
    node: &GraphNode,
    dataflow: &RustTypedDataflowNodeCall,
    call: &str,
    mutable_outputs: &BTreeSet<(GraphNodeId, NodePortId)>,
) -> Result<(), GraphCompileError> {
    let suffix = match dataflow.result {
        RustCallResult::Plain => "",
        RustCallResult::Result => "?",
    };
    match &dataflow.output {
        RustDataflowOutput::None => {
            writeln!(source, "{indent}    {call}{suffix};")?;
        }
        RustDataflowOutput::Single { port, rust_type } => {
            let local = dataflow_output_local(node.id, *port);
            let mutable = if mutable_outputs.contains(&(node.id, *port)) {
                "mut "
            } else {
                ""
            };
            let rust_type = rust_path_segments(rust_type)?.join("::");
            writeln!(
                source,
                "{indent}    let {mutable}{local}: {rust_type} = {call}{suffix};"
            )?;
        }
        RustDataflowOutput::StructFields { rust_type, fields } => {
            let result_local = dataflow_result_local(node.id);
            let rust_type = rust_path_segments(rust_type)?.join("::");
            writeln!(
                source,
                "{indent}    let {result_local}: {rust_type} = {call}{suffix};"
            )?;
            for field in fields {
                let local = dataflow_output_local(node.id, field.port);
                let mutable = if mutable_outputs.contains(&(node.id, field.port)) {
                    "mut "
                } else {
                    ""
                };
                let rust_type = rust_path_segments(&field.rust_type)?.join("::");
                writeln!(
                    source,
                    "{indent}    let {mutable}{local}: {rust_type} = {result_local}.{};",
                    field.field
                )?;
            }
        }
    }
    Ok(())
}

fn incoming_data_connections_by_target<'a>(
    connections: &[&'a GraphConnection],
    catalog: &NodeTypeCatalog,
    nodes: &[&GraphNode],
    node_indices: &BTreeMap<GraphNodeId, usize>,
) -> BTreeMap<(GraphNodeId, NodePortId), &'a GraphConnection> {
    let mut incoming = BTreeMap::new();
    for connection in connections {
        let from_node = nodes[*node_indices
            .get(&connection.from.node_id)
            .expect("document validation guarantees source nodes exist")];
        let to_node = nodes[*node_indices
            .get(&connection.to.node_id)
            .expect("document validation guarantees target nodes exist")];
        let from_descriptor = descriptor_for_node(catalog, from_node);
        let to_descriptor = descriptor_for_node(catalog, to_node);
        let from_port = from_descriptor
            .ports
            .iter()
            .find(|port| port.id == connection.from.port_id)
            .expect("document validation guarantees source ports exist");
        let to_port = to_descriptor
            .ports
            .iter()
            .find(|port| port.id == connection.to.port_id)
            .expect("document validation guarantees target ports exist");
        if from_port.value.is_data() && to_port.value.is_data() {
            incoming.insert((connection.to.node_id, connection.to.port_id), *connection);
        }
    }
    incoming
}

fn mutable_dataflow_outputs(
    connections: &[&GraphConnection],
    catalog: &NodeTypeCatalog,
    nodes: &[&GraphNode],
    node_indices: &BTreeMap<GraphNodeId, usize>,
) -> Result<BTreeSet<(GraphNodeId, NodePortId)>, GraphCompileError> {
    let mut outputs = BTreeSet::new();
    for connection in connections {
        let from_node = nodes[*node_indices
            .get(&connection.from.node_id)
            .expect("document validation guarantees source nodes exist")];
        let to_node = nodes[*node_indices
            .get(&connection.to.node_id)
            .expect("document validation guarantees target nodes exist")];
        let from_descriptor = descriptor_for_node(catalog, from_node);
        let to_descriptor = descriptor_for_node(catalog, to_node);
        let from_port = from_descriptor
            .ports
            .iter()
            .find(|port| port.id == connection.from.port_id)
            .expect("document validation guarantees source ports exist");
        let to_port = to_descriptor
            .ports
            .iter()
            .find(|port| port.id == connection.to.port_id)
            .expect("document validation guarantees target ports exist");
        if !from_port.value.is_data() || !to_port.value.is_data() {
            continue;
        }
        let to_dataflow = typed_dataflow_call_for_node(to_node, to_descriptor)?;
        if dataflow_input_parameter(to_dataflow, connection.to.port_id)
            .is_some_and(|parameter| parameter.passing == RustValuePassing::ByMutableRef)
        {
            outputs.insert((connection.from.node_id, connection.from.port_id));
        }
    }
    Ok(outputs)
}

fn dataflow_result_local(node_id: GraphNodeId) -> String {
    format!("node_{}_result", rust_uuid_ident(node_id))
}

fn dataflow_input_local(node_id: GraphNodeId, port_id: NodePortId) -> String {
    format!("node_{}_input_{}", rust_uuid_ident(node_id), port_id.0)
}

fn dataflow_output_local(node_id: GraphNodeId, port_id: NodePortId) -> String {
    format!("node_{}_output_{}", rust_uuid_ident(node_id), port_id.0)
}

fn rust_uuid_ident(node_id: GraphNodeId) -> String {
    node_id.as_uuid().to_string().replace('-', "_")
}

fn rust_literal_for_reflected_value(
    value: &ReflectedValueEnvelope,
    rust_type: &str,
    node_id: GraphNodeId,
    port_id: NodePortId,
) -> Result<String, GraphCompileError> {
    let rust_type = rust_path_segments(rust_type)?.join("::");
    let unsupported = || GraphCompileError::UnsupportedGeneratedRustDataflowInputLiteral {
        node_id,
        port_id,
        rust_type: rust_type.clone(),
    };
    if value.encoding != ReflectedValueEncoding::TypedRon {
        return Err(unsupported());
    }
    let source = std::str::from_utf8(&value.payload).map_err(|_| unsupported())?;
    let literal = match rust_type.as_str() {
        "bool" => ron::from_str::<bool>(source)
            .map(|value| value.to_string())
            .map_err(|_| unsupported())?,
        "i8" => ron::from_str::<i8>(source)
            .map(|value| format!("{value}_i8"))
            .map_err(|_| unsupported())?,
        "i16" => ron::from_str::<i16>(source)
            .map(|value| format!("{value}_i16"))
            .map_err(|_| unsupported())?,
        "i32" => ron::from_str::<i32>(source)
            .map(|value| format!("{value}_i32"))
            .map_err(|_| unsupported())?,
        "i64" => ron::from_str::<i64>(source)
            .map(|value| format!("{value}_i64"))
            .map_err(|_| unsupported())?,
        "isize" => ron::from_str::<isize>(source)
            .map(|value| format!("{value}_isize"))
            .map_err(|_| unsupported())?,
        "u8" => ron::from_str::<u8>(source)
            .map(|value| format!("{value}_u8"))
            .map_err(|_| unsupported())?,
        "u16" => ron::from_str::<u16>(source)
            .map(|value| format!("{value}_u16"))
            .map_err(|_| unsupported())?,
        "u32" => ron::from_str::<u32>(source)
            .map(|value| format!("{value}_u32"))
            .map_err(|_| unsupported())?,
        "u64" => ron::from_str::<u64>(source)
            .map(|value| format!("{value}_u64"))
            .map_err(|_| unsupported())?,
        "usize" => ron::from_str::<usize>(source)
            .map(|value| format!("{value}_usize"))
            .map_err(|_| unsupported())?,
        "f32" => ron::from_str::<f32>(source)
            .ok()
            .filter(|value| value.is_finite())
            .map(|value| format!("{value:?}_f32"))
            .ok_or_else(unsupported)?,
        "f64" => ron::from_str::<f64>(source)
            .ok()
            .filter(|value| value.is_finite())
            .map(|value| format!("{value:?}_f64"))
            .ok_or_else(unsupported)?,
        "String" | "std::string::String" | "alloc::string::String" => {
            ron::from_str::<String>(source)
                .map(|value| format!("{value:?}.to_string()"))
                .map_err(|_| unsupported())?
        }
        _ => return Err(unsupported()),
    };
    Ok(literal)
}

fn intern_nodes(
    strings: &mut StringTable,
    nodes: &[&GraphNode],
    catalog: &NodeTypeCatalog,
) -> Result<(), GraphCompileError> {
    for node in nodes {
        strings.intern(node.node_type.as_str())?;
        let descriptor = catalog
            .node_type_version(&node.node_type, node.node_type_version)
            .expect("document validation guarantees node descriptors are present");
        for port in &descriptor.ports {
            strings.intern(&port.name)?;
            match &port.value {
                NodePortValue::Execution => {}
                NodePortValue::Data { schema_type } => {
                    strings.intern(schema_type)?;
                }
                NodePortValue::DynamicData {
                    group,
                    accepted_schema_types,
                } => {
                    strings.intern(group)?;
                    for schema_type in accepted_schema_types {
                        strings.intern(schema_type)?;
                    }
                }
            }
        }
        intern_runtime_binding(
            strings,
            descriptor
                .runtime_binding
                .as_ref()
                .expect("runtime binding was validated"),
        )?;
    }
    Ok(())
}

fn intern_runtime_binding(
    strings: &mut StringTable,
    binding: &NodeRuntimeBinding,
) -> Result<(), GraphCompileError> {
    match binding {
        NodeRuntimeBinding::RustSymbol {
            package, symbol, ..
        } => {
            strings.intern(package)?;
            strings.intern(symbol)?;
        }
        NodeRuntimeBinding::AssetBuilder { builder_id } => {
            strings.intern(builder_id)?;
        }
        NodeRuntimeBinding::RuntimeComponent { component_type } => {
            strings.intern(component_type)?;
        }
        NodeRuntimeBinding::External { kind, locator } => {
            strings.intern(kind)?;
            strings.intern(locator)?;
        }
    }
    Ok(())
}

fn write_nodes(
    writer: &mut Vec<u8>,
    strings: &StringTable,
    nodes: &[&GraphNode],
    catalog: &NodeTypeCatalog,
) -> Result<(), GraphCompileError> {
    for node in nodes {
        let descriptor = catalog
            .node_type_version(&node.node_type, node.node_type_version)
            .expect("document validation guarantees node descriptors are present");
        let ports = sorted_ports(descriptor);
        let port_indices = ports
            .iter()
            .enumerate()
            .map(|(index, port)| (port.id, index))
            .collect::<BTreeMap<_, _>>();

        write_uuid(writer, node.id.as_uuid())?;
        write_u32(
            writer,
            strings
                .index(node.node_type.as_str())
                .expect("node type was interned"),
        )?;
        write_u32(writer, node.node_type_version)?;
        write_runtime_binding(
            writer,
            strings,
            descriptor
                .runtime_binding
                .as_ref()
                .expect("runtime binding was validated"),
        )?;
        write_u32_checked(writer, "node ports", ports.len())?;
        for port in &ports {
            write_port(writer, strings, port)?;
        }
        write_u32_checked(writer, "node input values", node.input_values.len())?;
        for (port_id, value) in &node.input_values {
            let port_index = port_indices
                .get(port_id)
                .copied()
                .expect("document validation guarantees input value ports are present");
            write_u32(writer, checked_u32("input value port index", port_index)?)?;
            let mut value_bytes = Vec::new();
            write_reflected_value(&mut value_bytes, value)?;
            write_u32_checked(writer, "input value bytes", value_bytes.len())?;
            writer.write_all(&value_bytes)?;
        }
    }
    Ok(())
}

fn write_runtime_binding(
    writer: &mut Vec<u8>,
    strings: &StringTable,
    binding: &NodeRuntimeBinding,
) -> Result<(), GraphCompileError> {
    match binding {
        NodeRuntimeBinding::RustSymbol {
            package, symbol, ..
        } => {
            write_u8(writer, 1)?;
            write_u32(
                writer,
                strings
                    .index(package)
                    .expect("binding package was interned"),
            )?;
            write_u32(
                writer,
                strings.index(symbol).expect("binding symbol was interned"),
            )?;
        }
        NodeRuntimeBinding::AssetBuilder { builder_id } => {
            write_u8(writer, 2)?;
            write_u32(
                writer,
                strings
                    .index(builder_id)
                    .expect("binding builder id was interned"),
            )?;
        }
        NodeRuntimeBinding::RuntimeComponent { component_type } => {
            write_u8(writer, 3)?;
            write_u32(
                writer,
                strings
                    .index(component_type)
                    .expect("binding component type was interned"),
            )?;
        }
        NodeRuntimeBinding::External { kind, locator } => {
            write_u8(writer, 4)?;
            write_u32(
                writer,
                strings.index(kind).expect("binding kind was interned"),
            )?;
            write_u32(
                writer,
                strings
                    .index(locator)
                    .expect("binding locator was interned"),
            )?;
        }
    }
    Ok(())
}

fn write_port(
    writer: &mut Vec<u8>,
    strings: &StringTable,
    port: &NodePortDescriptor,
) -> Result<(), GraphCompileError> {
    write_u32(writer, port.id.0)?;
    write_u32(
        writer,
        strings.index(&port.name).expect("port name was interned"),
    )?;
    write_u8(
        writer,
        match port.direction {
            NodePortDirection::Input => 0,
            NodePortDirection::Output => 1,
        },
    )?;
    write_u8(
        writer,
        match port.capacity {
            az_node_graph::NodePortCapacity::Single => 0,
            az_node_graph::NodePortCapacity::Multiple => 1,
        },
    )?;
    match &port.value {
        NodePortValue::Execution => write_u8(writer, 0)?,
        NodePortValue::Data { schema_type } => {
            write_u8(writer, 1)?;
            write_u32(
                writer,
                strings
                    .index(schema_type)
                    .expect("port schema type was interned"),
            )?;
        }
        NodePortValue::DynamicData {
            group,
            accepted_schema_types,
        } => {
            write_u8(writer, 2)?;
            write_u32(
                writer,
                strings.index(group).expect("port group was interned"),
            )?;
            write_u32_checked(
                writer,
                "dynamic accepted schema types",
                accepted_schema_types.len(),
            )?;
            for schema_type in accepted_schema_types {
                write_u32(
                    writer,
                    strings
                        .index(schema_type)
                        .expect("dynamic accepted schema type was interned"),
                )?;
            }
        }
    }
    Ok(())
}

fn write_connections(
    writer: &mut Vec<u8>,
    nodes: &[&GraphNode],
    node_indices: &BTreeMap<GraphNodeId, usize>,
    connections: &[&GraphConnection],
    catalog: &NodeTypeCatalog,
) -> Result<(), GraphCompileError> {
    for connection in connections {
        write_uuid(writer, connection.id.as_uuid())?;
        write_port_ref(writer, nodes, node_indices, &connection.from, catalog)?;
        write_port_ref(writer, nodes, node_indices, &connection.to, catalog)?;
    }
    Ok(())
}

fn write_port_ref(
    writer: &mut Vec<u8>,
    nodes: &[&GraphNode],
    node_indices: &BTreeMap<GraphNodeId, usize>,
    port_ref: &GraphPortRef,
    catalog: &NodeTypeCatalog,
) -> Result<(), GraphCompileError> {
    let node_index = *node_indices
        .get(&port_ref.node_id)
        .expect("document validation guarantees connection nodes are present");
    let node = nodes[node_index];
    let descriptor = catalog
        .node_type_version(&node.node_type, node.node_type_version)
        .expect("document validation guarantees node descriptors are present");
    let port_index = sorted_ports(descriptor)
        .iter()
        .position(|port| port.id == port_ref.port_id)
        .expect("document validation guarantees connection ports are present");
    write_u32(writer, checked_u32("connection node index", node_index)?)?;
    write_u32(writer, checked_u32("connection port index", port_index)?)?;
    Ok(())
}

fn topological_order(
    nodes: &[&GraphNode],
    connections: &[&GraphConnection],
    node_indices: &BTreeMap<GraphNodeId, usize>,
) -> Result<Vec<usize>, GraphCompileError> {
    let mut incoming = vec![0usize; nodes.len()];
    let mut outgoing = vec![Vec::<usize>::new(); nodes.len()];
    let mut seen_edges = BTreeSet::new();

    for connection in connections {
        let from = *node_indices
            .get(&connection.from.node_id)
            .expect("document validation guarantees connection source nodes are present");
        let to = *node_indices
            .get(&connection.to.node_id)
            .expect("document validation guarantees connection target nodes are present");
        if seen_edges.insert((from, to)) {
            incoming[to] += 1;
            outgoing[from].push(to);
        }
    }

    let mut ready = incoming
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(nodes.len());

    while let Some(index) = ready.pop_first() {
        order.push(index);
        for next in &outgoing[index] {
            incoming[*next] -= 1;
            if incoming[*next] == 0 {
                ready.insert(*next);
            }
        }
    }

    if order.len() != nodes.len() {
        return Err(GraphCompileError::CyclicGraph);
    }
    Ok(order)
}

fn sorted_ports(descriptor: &az_node_graph::NodeTypeDescriptor) -> Vec<&NodePortDescriptor> {
    let mut ports = descriptor.ports.iter().collect::<Vec<_>>();
    ports.sort_by_key(|port| port.id);
    ports
}

fn total_port_count(nodes: &[&GraphNode], catalog: &NodeTypeCatalog) -> usize {
    nodes
        .iter()
        .map(|node| {
            catalog
                .node_type_version(&node.node_type, node.node_type_version)
                .expect("document validation guarantees node descriptors are present")
                .ports
                .len()
        })
        .sum()
}

fn total_input_value_count(nodes: &[&GraphNode]) -> usize {
    nodes.iter().map(|node| node.input_values.len()).sum()
}

fn runtime_product_flags(product: &RuntimeGraphProductDescriptor) -> u32 {
    u32::from(product.streamable) | (u32::from(product.diffable_chunks) << 1)
}

fn packed_ir_product_path(source_path: &str) -> String {
    graph_runtime_product_path(source_path, PACKED_GRAPH_IR_PRODUCT_EXTENSION)
}

fn aot_graph_manifest_product_path(source_path: &str) -> String {
    graph_runtime_product_path(source_path, AOT_GRAPH_MANIFEST_PRODUCT_EXTENSION)
}

fn generated_rust_graph_source_product_path(source_path: &str) -> String {
    graph_runtime_product_path(source_path, GENERATED_RUST_GRAPH_SOURCE_PRODUCT_EXTENSION)
}

fn graph_runtime_product_path(source_path: &str, extension: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    let path = Path::new(&normalized);
    let mut product = PathBuf::new();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        product.push(parent);
    }
    let file_stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(normalized.as_str());
    let file_name = format!("{file_stem}.{extension}");
    product.push(file_name);
    product.to_string_lossy().replace('\\', "/")
}

fn validate_rust_path(path: &str) -> Result<(), GraphCompileError> {
    rust_path_segments(path).map(|_| ())
}

fn rust_path_segments(path: &str) -> Result<Vec<&str>, GraphCompileError> {
    if path.trim() != path {
        return Err(GraphCompileError::InvalidRustPath {
            path: path.to_string(),
            reason: "path has leading or trailing whitespace".to_string(),
        });
    }
    let segments = path.split("::").collect::<Vec<_>>();
    if segments.is_empty() || segments.iter().any(|segment| segment.is_empty()) {
        return Err(GraphCompileError::InvalidRustPath {
            path: path.to_string(),
            reason: "path contains an empty segment".to_string(),
        });
    }
    for segment in &segments {
        validate_rust_identifier(path, segment)?;
    }
    Ok(segments)
}

fn validate_rust_identifier(path: &str, segment: &str) -> Result<(), GraphCompileError> {
    let segment = segment.strip_prefix("r#").unwrap_or(segment);
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return Err(GraphCompileError::InvalidRustPath {
            path: path.to_string(),
            reason: "path contains an empty segment".to_string(),
        });
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return Err(GraphCompileError::InvalidRustPath {
            path: path.to_string(),
            reason: format!("segment `{segment}` does not start with a Rust identifier character"),
        });
    }
    if chars.any(|value| !(value == '_' || value.is_ascii_alphanumeric())) {
        return Err(GraphCompileError::InvalidRustPath {
            path: path.to_string(),
            reason: format!("segment `{segment}` contains a non Rust identifier character"),
        });
    }
    Ok(())
}

#[derive(Default)]
struct StringTable {
    values: Vec<String>,
    indexes: BTreeMap<String, u32>,
}

impl StringTable {
    fn intern(&mut self, value: impl AsRef<str>) -> Result<u32, GraphCompileError> {
        let value = value.as_ref();
        if let Some(index) = self.indexes.get(value) {
            return Ok(*index);
        }
        let index = checked_u32("string table", self.values.len())?;
        self.values.push(value.to_string());
        self.indexes.insert(value.to_string(), index);
        Ok(index)
    }

    fn index(&self, value: impl AsRef<str>) -> Option<u32> {
        self.indexes.get(value.as_ref()).copied()
    }

    const fn len(&self) -> usize {
        self.values.len()
    }

    fn write(&self, writer: &mut Vec<u8>) -> Result<(), GraphCompileError> {
        for value in &self.values {
            write_text(writer, value)?;
        }
        Ok(())
    }
}

fn write_reflected_value(
    writer: &mut Vec<u8>,
    value: &ReflectedValueEnvelope,
) -> Result<(), GraphCompileError> {
    write_text(writer, &value.type_path)?;
    write_u8(
        writer,
        match value.encoding {
            ReflectedValueEncoding::BevyRemoteJson => 0,
            ReflectedValueEncoding::TypedRon => 1,
            ReflectedValueEncoding::CapnpData => 2,
        },
    )?;
    write_u32_checked(writer, "reflected value payload", value.payload.len())?;
    writer.write_all(&value.payload)?;
    Ok(())
}

fn write_text(writer: &mut Vec<u8>, value: &str) -> Result<(), GraphCompileError> {
    write_u32_checked(writer, "text bytes", value.len())?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn write_uuid(writer: &mut Vec<u8>, value: uuid::Uuid) -> std::io::Result<()> {
    writer.write_all(value.as_bytes())
}

fn write_u8(writer: &mut Vec<u8>, value: u8) -> std::io::Result<()> {
    writer.write_all(&[value])
}

fn write_u32(writer: &mut Vec<u8>, value: u32) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u32_checked(
    writer: &mut Vec<u8>,
    field: &'static str,
    count: usize,
) -> Result<(), GraphCompileError> {
    write_u32(writer, checked_u32(field, count)?)?;
    Ok(())
}

fn checked_u32(field: &'static str, count: usize) -> Result<u32, GraphCompileError> {
    u32::try_from(count).map_err(|_| GraphCompileError::CountOverflow { field, count })
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_asset_builder::SourcePath;
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, declare_caps,
    };
    use az_node_graph::{
        GraphCompilerBackendDescriptor, GraphConnection, GraphNodeCatalogRequirement,
        GraphSourceWorkflow, GraphTypeRegistration, NodePortCapacity, NodePortDescriptor,
        NodePortDirection, NodePortId, NodePortValue, NodeRuntimeBinding, NodeTypeDescriptor,
        NodeTypeRegistration, RuntimeGraphExecutionStrategy, RuntimeGraphProductDescriptor,
        RustCallResult, RustDataflowOutput, RustDataflowParameter, RustTypedDataflowNodeCall,
        RustValuePassing, encode_visual_graph_document_ron,
    };

    /// The gem a projecting crate lives in, declaring the product its build
    /// script projects.
    const GRAPH_GEM_MANIFEST: &str = r#"[manifest]
kind = "gem"
schema = "azoth.gem/v3"

[gem]
id = "local.graphs"

[[contributions]]
id = "runtime"
kind = "code"
package = "graph-gem"
roles = ["server"]
products = ["graphs"]
"#;

    /// A crate root inside a gem that declares graphs for `graph-gem`.
    fn declaring_crate(temp: &Path) -> PathBuf {
        std::fs::write(temp.join("gem.toml"), GRAPH_GEM_MANIFEST).unwrap();
        let crate_root = temp.join("runtime");
        std::fs::create_dir_all(&crate_root).unwrap();
        crate_root
    }

    #[test]
    fn generated_rust_source_module_accepts_an_empty_projection() {
        let temp = tempfile::tempdir().unwrap();
        let crate_root = declaring_crate(temp.path());
        let source_root = temp.path().join("missing-graphs");
        let out_dir = temp.path().join("out");

        let projection = write_generated_rust_graph_source_module(
            &source_root,
            &out_dir,
            &crate_root,
            "graph-gem",
        )
        .unwrap();

        assert!(source_root.is_dir());
        assert_eq!(projection.source_count, 0);
        assert_eq!(
            projection.module_path,
            out_dir.join(GENERATED_RUST_GRAPH_SOURCE_MODULE_FILE)
        );
        let module = std::fs::read_to_string(projection.module_path).unwrap();
        assert!(!module.contains("include!"), "{module}");
        assert!(
            module.contains(
                "pub fn register<D>(ctx: &mut az_gem_contract::GemContext<'_, D>) {\n    ctx.registrar::<az_graph_runtime::AotGraphRuntimeRegistration>()\n        .register_many([\n        ]);\n}\n"
            ),
            "an empty projection still registers, with nothing in it: {module}"
        );
    }

    /// The half of the declaration only this side can see: the crate compiled
    /// graphs, and the manifest generated glue reads says nothing about them,
    /// so the projection would be written and never called.
    #[test]
    fn projecting_graphs_a_gem_does_not_declare_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("gem.toml"),
            GRAPH_GEM_MANIFEST.replace("products = [\"graphs\"]\n", ""),
        )
        .unwrap();
        let crate_root = temp.path().join("runtime");
        std::fs::create_dir_all(&crate_root).unwrap();

        let error = write_generated_rust_graph_source_module(
            &temp.path().join("graphs"),
            &temp.path().join("out"),
            &crate_root,
            "graph-gem",
        )
        .expect_err("the manifest declares no graphs");

        let message = error.to_string();
        assert!(message.contains("products = [\"graphs\"]"), "{message}");
        assert!(message.contains("graph-gem"), "{message}");
        assert!(
            !temp.path().join("out").exists(),
            "a refused projection writes nothing"
        );
    }

    /// Another package's stanza declaring graphs is not this crate's
    /// declaration.
    #[test]
    fn another_packages_declaration_does_not_cover_this_crate() {
        let temp = tempfile::tempdir().unwrap();
        let crate_root = declaring_crate(temp.path());

        let error = write_generated_rust_graph_source_module(
            &temp.path().join("graphs"),
            &temp.path().join("out"),
            &crate_root,
            "other-gem",
        )
        .expect_err("the declaring stanza names another package");

        assert!(error.to_string().contains("other-gem"), "{error}");
    }

    #[test]
    fn projecting_graphs_outside_a_gem_names_the_search_rule() {
        let temp = tempfile::tempdir().unwrap();
        let crate_root = temp.path().join("runtime");
        std::fs::create_dir_all(&crate_root).unwrap();

        let error = write_generated_rust_graph_source_module(
            &temp.path().join("graphs"),
            &temp.path().join("out"),
            &crate_root,
            "graph-gem",
        )
        .expect_err("no manifest is in reach");

        let message = error.to_string();
        assert!(message.contains("nor in any ancestor"), "{message}");
    }

    #[test]
    fn generated_rust_source_module_recurses_and_sorts_rust_products() {
        let temp = tempfile::tempdir().unwrap();
        let crate_root = declaring_crate(temp.path());
        let source_root = temp.path().join("graphs");
        let nested = source_root.join("nested");
        let out_dir = temp.path().join("out");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(source_root.join("z.rs"), "pub fn z() {}\n").unwrap();
        std::fs::write(nested.join("a.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(nested.join("ignored.ron"), "()\n").unwrap();

        let projection = write_generated_rust_graph_source_module(
            &source_root,
            &out_dir,
            &crate_root,
            "graph-gem",
        )
        .unwrap();
        let module = std::fs::read_to_string(&projection.module_path).unwrap();
        let nested_include = format!("include!({:?});", nested.join("a.rs").to_string_lossy());
        let root_include = format!(
            "include!({:?});",
            source_root.join("z.rs").to_string_lossy()
        );

        assert_eq!(projection.source_count, 2);
        assert!(module.contains(&nested_include));
        assert!(module.contains(&root_include));
        assert!(module.find(&nested_include) < module.find(&root_include));
        // Each product gets its own module, and the registration list names
        // them in the same order, so what a host composes is the sorted
        // product set and nothing else.
        assert!(module.contains("pub mod graph0 {"), "{module}");
        assert!(module.contains("pub mod graph1 {"), "{module}");
        assert!(
            module
                .contains("            graph0::REGISTRATION,\n            graph1::REGISTRATION,\n"),
            "{module}"
        );
        assert!(!module.contains("ignored.ron"));
    }

    struct GeneratedGraphAssetData;

    impl AzTypeInfo for GeneratedGraphAssetData {
        const NAME: &'static str = "AzGraphBuilderTests::GeneratedGraphAssetData";
        const TYPE_ID: Uuid = uuid!("018f0c5a-0000-7000-8000-0000000000e1");
    }

    impl AzRtti for GeneratedGraphAssetData {}

    impl AssetData for GeneratedGraphAssetData {
        const STABLE_NAME: &'static str = "az.graph-builder.tests.generated-rust-product";
    }

    const GENERATED_GRAPH_ASSET_TYPE_NAME: &str =
        <GeneratedGraphAssetData as AssetData>::STABLE_NAME;
    const GENERATED_GRAPH_ASSET_TYPE: AssetType =
        <GeneratedGraphAssetData as AssetData>::ASSET_TYPE;

    fn test_node_type() -> NodeTypeDescriptor {
        NodeTypeDescriptor::new("az.graph_builder.tests.Print", 1, "Print")
            .with_port(NodePortDescriptor::new(
                NodePortId::new(1),
                "value",
                NodePortDirection::Input,
                NodePortValue::Data {
                    schema_type: "core.string".to_string(),
                },
            ))
            .with_port(NodePortDescriptor::new(
                NodePortId::new(2),
                "next",
                NodePortDirection::Output,
                NodePortValue::Execution,
            ))
            .with_port(NodePortDescriptor::execution_input(
                NodePortId::new(3),
                "in",
            ))
            .with_runtime_binding(NodeRuntimeBinding::rust_symbol(
                "az-graph-builder-tests",
                "az_graph_builder_tests::print",
            ))
    }

    fn test_data_source_node_type() -> NodeTypeDescriptor {
        NodeTypeDescriptor::new("az.graph_builder.tests.StringValue", 1, "String Value")
            .with_port(NodePortDescriptor::new(
                NodePortId::new(1),
                "value",
                NodePortDirection::Output,
                NodePortValue::Data {
                    schema_type: "core.string".to_string(),
                },
            ))
            .with_runtime_binding(NodeRuntimeBinding::rust_symbol(
                "az-graph-builder-tests",
                "az_graph_builder_tests::string_value",
            ))
    }

    fn test_dataflow_source_node_type() -> NodeTypeDescriptor {
        NodeTypeDescriptor::new(
            "az.graph_builder.tests.DataflowStringValue",
            1,
            "Dataflow String Value",
        )
        .with_port(
            NodePortDescriptor::new(
                NodePortId::new(1),
                "value",
                NodePortDirection::Output,
                NodePortValue::Data {
                    schema_type: "core.string".to_string(),
                },
            )
            .with_capacity(NodePortCapacity::Multiple),
        )
        .with_runtime_binding(NodeRuntimeBinding::rust_typed_dataflow(
            "az-graph-builder-tests",
            "az_graph_builder_tests::dataflow_string_value",
            RustTypedDataflowNodeCall::new(RustDataflowOutput::single(
                NodePortId::new(1),
                "String",
            )),
        ))
    }

    fn test_dataflow_transform_node_type() -> NodeTypeDescriptor {
        NodeTypeDescriptor::new(
            "az.graph_builder.tests.DataflowAppendSuffix",
            1,
            "Dataflow Append Suffix",
        )
        .with_port(NodePortDescriptor::new(
            NodePortId::new(1),
            "value",
            NodePortDirection::Input,
            NodePortValue::Data {
                schema_type: "core.string".to_string(),
            },
        ))
        .with_port(NodePortDescriptor::new(
            NodePortId::new(2),
            "result",
            NodePortDirection::Output,
            NodePortValue::Data {
                schema_type: "core.string".to_string(),
            },
        ))
        .with_runtime_binding(NodeRuntimeBinding::rust_typed_dataflow(
            "az-graph-builder-tests",
            "az_graph_builder_tests::append_suffix",
            RustTypedDataflowNodeCall::new(RustDataflowOutput::single(
                NodePortId::new(2),
                "String",
            ))
            .with_parameter(RustDataflowParameter::runtime_context(
                "az_graph_builder_generated_tests::RuntimeContext",
                RustValuePassing::BySharedRef,
            ))
            .with_parameter(RustDataflowParameter::input(
                NodePortId::new(1),
                "String",
                RustValuePassing::BySharedRef,
            ))
            .with_result(RustCallResult::Result),
        ))
    }

    fn test_graph_type() -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            "az.graph_builder.tests.logic",
            1,
            "Graph Builder Test Logic",
            GraphSourceWorkflow::file("az.graph_builder.tests.logic.source", "azgraph.ron")
                .with_default_path_prefix("graphs"),
            GraphCompilerBackendDescriptor::packed_ir(
                "az.graph_builder.tests.logic.compiler",
                "azoth.graph.logic-ir/v1",
            )
            .with_capability_marker("zero-cost"),
            RuntimeGraphProductDescriptor::new(
                PACKED_GRAPH_IR_ASSET_TYPE_NAME,
                "azoth.graph.logic-ir",
                RuntimeGraphExecutionStrategy::PackedIr,
            ),
        )
        .with_node_catalog(GraphNodeCatalogRequirement::new(
            "az.graph_builder.tests.nodes",
        ))
    }

    fn test_generated_rust_graph_type() -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            "az.graph_builder.tests.generated",
            1,
            "Graph Builder Generated Rust Test",
            GraphSourceWorkflow::file("az.graph_builder.tests.generated.source", "azgraph.ron")
                .with_default_path_prefix("graphs"),
            GraphCompilerBackendDescriptor::generated_rust_context_schedule(
                "az.graph_builder.tests.generated.compiler",
                "az-graph-builder-generated-tests",
                "az_graph_builder_generated_tests::compile_graph",
            )
            .with_capability_marker("generated-rust"),
            RuntimeGraphProductDescriptor::new(
                GENERATED_GRAPH_ASSET_TYPE_NAME,
                "azoth.graph.generated-rust-test",
                RuntimeGraphExecutionStrategy::aot_compiled_rust(
                    "az-graph-builder-generated-tests",
                    "az_graph_builder_generated_tests::execute_graph",
                    "az_graph_builder_generated_tests::RuntimeContext",
                ),
            ),
        )
        .with_node_catalog(GraphNodeCatalogRequirement::new(
            "az.graph_builder.tests.nodes",
        ))
    }

    fn test_generated_rust_dataflow_graph_type() -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            "az.graph_builder.tests.generated-dataflow",
            1,
            "Graph Builder Generated Rust Dataflow Test",
            GraphSourceWorkflow::file(
                "az.graph_builder.tests.generated-dataflow.source",
                "azgraph.ron",
            )
            .with_default_path_prefix("graphs"),
            GraphCompilerBackendDescriptor::generated_rust_typed_dataflow(
                "az.graph_builder.tests.generated-dataflow.compiler",
                "az-graph-builder-generated-tests",
                "az_graph_builder_generated_tests::compile_dataflow",
            )
            .with_capability_marker("generated-rust")
            .with_capability_marker("zero-cost"),
            RuntimeGraphProductDescriptor::new(
                GENERATED_GRAPH_ASSET_TYPE_NAME,
                "azoth.graph.generated-rust-dataflow-test",
                RuntimeGraphExecutionStrategy::aot_compiled_rust(
                    "az-graph-builder-generated-tests",
                    "az_graph_builder_generated_tests::execute_dataflow",
                    "az_graph_builder_generated_tests::RuntimeContext",
                ),
            ),
        )
        .with_node_catalog(GraphNodeCatalogRequirement::new(
            "az.graph_builder.tests.nodes",
        ))
    }

    declare_caps!(TestCaps:);

    const TEST_GRAPHS: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.graph-builder-tests"),
        contribution: ContributionId::new("graphs"),
        roles: &[],
    };

    /// The graph types and node types this crate's tests compile against,
    /// contributed the way a gem contributes them, alongside the crate's own
    /// asset-pipeline registrations.
    struct TestGraphs;

    impl Contribution for TestGraphs {
        type Caps = TestCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            TEST_GRAPHS
        }

        fn register(&self, ctx: &mut GemContext<'_, TestCaps>) {
            crate::register(ctx);
            ctx.registrar::<AssetTypeRegistration>().register(
                AssetTypeRegistration::for_asset::<GeneratedGraphAssetData>()
                    .with_owner("az-graph-builder-tests"),
            );
            ctx.registrar::<NodeTypeRegistration>().register_many([
                NodeTypeRegistration::new(test_node_type()),
                NodeTypeRegistration::new(test_data_source_node_type()),
                NodeTypeRegistration::new(test_dataflow_source_node_type()),
                NodeTypeRegistration::new(test_dataflow_transform_node_type()),
            ]);
            ctx.registrar::<GraphTypeRegistration>().register_many([
                GraphTypeRegistration::new(test_graph_type()),
                GraphTypeRegistration::new(test_generated_rust_graph_type()),
                GraphTypeRegistration::new(test_generated_rust_dataflow_graph_type()),
            ]);
        }
    }

    /// The composition every test in this crate compiles against; its
    /// registries are what a host would hand the graph compiler.
    pub fn test_composer() -> Composer {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(TestGraphs, az_gem_contract::ProductActivation::default())
            .expect("test graphs require no host capability");
        composer
    }

    #[test]
    fn graph_source_schemas_are_projected_from_graph_type_registrations() {
        let composer = test_composer();
        let schemas = graph_source_schemas(composer.registries()).unwrap();
        let schema = schemas
            .iter()
            .find(|schema| schema.schema_type == "az.graph_builder.tests.logic")
            .expect("test graph schema is projected");

        assert_eq!(schema.label, "Graph Builder Test Logic");
        assert_eq!(schema.owner, "az.graph_builder.tests.logic.source");
        assert_eq!(
            schema.authoring,
            RegisteredGraphSourceAuthoring::File {
                source_root: az_asset_builder::PROJECT_SOURCE_ROOT.to_string(),
                default_path_prefix: "graphs".to_string(),
                extensions: vec!["azgraph.ron".to_string()],
                can_create: false,
                can_edit: true,
            }
        );
    }

    #[test]
    fn generated_rust_aot_entry_symbols_are_source_unique() {
        let left = generated_rust_aot_entry_symbol(
            "az_graph_builder_generated_tests::execute_graph",
            uuid!("018f0c5a-0000-7000-8000-0000000000aa"),
        )
        .unwrap();
        let right = generated_rust_aot_entry_symbol(
            "az_graph_builder_generated_tests::execute_graph",
            uuid!("018f0c5a-0000-7000-8000-0000000000bb"),
        )
        .unwrap();

        assert_eq!(
            left,
            "az_graph_builder_generated_tests_018f0c5a0000700080000000000000aa::execute_graph"
        );
        assert_eq!(
            right,
            "az_graph_builder_generated_tests_018f0c5a0000700080000000000000bb::execute_graph"
        );
        assert_ne!(left, right);
    }

    #[test]
    fn packed_ir_compiler_writes_magic_and_separate_version() {
        let mut document = VisualGraphDocument::new("az.graph_builder.tests.logic");
        let mut node = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000001")),
            "az.graph_builder.tests.Print",
            1,
        );
        node.input_values.insert(
            NodePortId::new(1),
            ReflectedValueEnvelope::typed_ron("alloc::string::String", r#""hello""#),
        );
        document.nodes.push(node);

        let composer = test_composer();
        let graph_catalog = GraphTypeCatalog::compose(1, 0, composer.registries()).unwrap();
        let graph_type = graph_catalog
            .graph_types
            .iter()
            .find(|graph_type| graph_type.id.as_str() == "az.graph_builder.tests.logic")
            .unwrap();
        let node_catalog = NodeTypeCatalog::compose(1, 0, composer.registries()).unwrap();
        let output = compile_packed_ir(&GraphCompileRequest {
            source_path: "graphs/example.azgraph.ron",
            source_uuid: uuid!("018f0c5a-0000-7000-8000-0000000000aa"),
            platform: "pc",
            document: &document,
            graph_type,
            node_catalog: &node_catalog,
            registries: composer.registries(),
        })
        .unwrap();

        let product = output.runtime_product().unwrap();
        assert_eq!(output.products.len(), 1);
        assert_eq!(product.product_path, "graphs/example.azgraph.azgir");
        assert_eq!(product.asset_type, PACKED_GRAPH_IR_ASSET_TYPE);
        assert_eq!(product.product_format, PACKED_GRAPH_IR_FORMAT_ID);
        assert_eq!(product.sub_id, RUNTIME_GRAPH_PRODUCT_SUB_ID);
        assert_eq!(&product.bytes[..8], PACKED_GRAPH_IR_MAGIC);
        assert_eq!(
            u32::from_le_bytes(product.bytes[8..12].try_into().unwrap()),
            PACKED_GRAPH_IR_VERSION
        );
        assert_ne!(&product.bytes[..8], b"AZGIR001");

        let ir = decode_packed_graph_ir(&product.bytes).unwrap();
        assert_eq!(ir.header.version, PACKED_GRAPH_IR_VERSION);
        assert_eq!(ir.graph_type, "az.graph_builder.tests.logic");
        assert_eq!(ir.product_kind, "azoth.graph.logic-ir");
        assert_eq!(ir.backend, "az.graph_builder.tests.logic.compiler");
        assert_eq!(ir.ir_schema, "azoth.graph.logic-ir/v1");
        assert_eq!(ir.capability_markers, vec!["zero-cost"]);
        assert_eq!(ir.nodes.len(), 1);
        assert_eq!(ir.node_type(&ir.nodes[0]), "az.graph_builder.tests.Print");
        assert_eq!(ir.node_ports(&ir.nodes[0]).len(), 3);
        assert_eq!(ir.node_input_values(&ir.nodes[0]).len(), 1);
        let value_bytes = ir.node_input_values(&ir.nodes[0])[0].bytes;
        let type_path_len = u32::from_le_bytes(value_bytes[..4].try_into().unwrap()) as usize;
        let type_path_end = 4 + type_path_len;
        assert_eq!(&value_bytes[4..type_path_end], b"alloc::string::String");
        assert_eq!(value_bytes[type_path_end], 1);
        let payload_len = u32::from_le_bytes(
            value_bytes[type_path_end + 1..type_path_end + 5]
                .try_into()
                .unwrap(),
        ) as usize;
        assert_eq!(
            &value_bytes[type_path_end + 5..type_path_end + 5 + payload_len],
            br#""hello""#
        );
        assert_eq!(ir.execution_order, vec![0]);
        let PackedRuntimeBinding::RustSymbol { package, symbol } = ir.nodes[0].runtime_binding
        else {
            panic!("test node should compile to a Rust symbol binding");
        };
        assert_eq!(ir.string(package), Some("az-graph-builder-tests"));
        assert_eq!(ir.string(symbol), Some("az_graph_builder_tests::print"));
    }

    #[test]
    fn a_composed_graph_source_compiles_to_a_packed_ir_product() {
        let mut document = VisualGraphDocument::new("az.graph_builder.tests.logic");
        let source = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000011")),
            "az.graph_builder.tests.Print",
            1,
        );
        let target = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000012")),
            "az.graph_builder.tests.Print",
            1,
        );
        document.connections.push(GraphConnection::new(
            az_node_graph::GraphConnectionId::new(uuid!("018f0c5a-0000-7000-8000-000000000013")),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(3)),
        ));
        document.nodes.push(source);
        document.nodes.push(target);
        let bytes = encode_visual_graph_document_ron(&document)
            .unwrap()
            .into_bytes();

        let composer = test_composer();
        let output = compile_graph_source(
            "graphs/flow.azgraph.ron",
            uuid!("018f0c5a-0000-7000-8000-0000000000bb"),
            "az.graph_builder.tests.logic",
            &bytes,
            "pc",
            composer.registries(),
        )
        .unwrap();

        assert_eq!(output.products.len(), 1);
        assert_eq!(output.products[0].product_path, "graphs/flow.azgraph.azgir");
    }

    #[test]
    fn generated_rust_graphs_require_a_registered_backend() {
        assert!(
            registered_graph_compiler_backends()
                .iter()
                .any(|backend| backend.backend_id() == "az.graph-builder.generated-rust-aot")
        );
        let mut document = VisualGraphDocument::new("az.graph_builder.tests.generated");
        document.nodes.push(GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000021")),
            "az.graph_builder.tests.Print",
            1,
        ));
        let bytes = encode_visual_graph_document_ron(&document)
            .unwrap()
            .into_bytes();

        let composer = test_composer();
        let error = compile_graph_source_with_backends(
            "graphs/generated.azgraph.ron",
            uuid!("018f0c5a-0000-7000-8000-0000000000cc"),
            "az.graph_builder.tests.generated",
            &bytes,
            "pc",
            composer.registries(),
            &[],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            GraphCompileError::UnsupportedCompilerBackend { backend, .. }
                if backend == "az.graph_builder.tests.generated.compiler"
        ));

        let output = compile_graph_source(
            "graphs/generated.azgraph.ron",
            uuid!("018f0c5a-0000-7000-8000-0000000000cc"),
            "az.graph_builder.tests.generated",
            &bytes,
            "pc",
            composer.registries(),
        )
        .unwrap();

        assert_eq!(output.products.len(), 2);
        let manifest_product = output.runtime_product().unwrap();
        let source_product = output
            .products
            .iter()
            .find(|product| product.role == GraphCompileProductRole::GeneratedRustSource)
            .unwrap();
        assert_aot_manifest_product(manifest_product);
        assert_generated_rust_source_product(source_product);
    }

    /// Asserts the AOT manifest product's identity and every decoded field.
    fn assert_aot_manifest_product(manifest_product: &GraphCompileProduct) {
        assert_eq!(
            manifest_product.product_path,
            "graphs/generated.azgraph.azgaot"
        );
        assert_eq!(manifest_product.asset_type, GENERATED_GRAPH_ASSET_TYPE);
        assert_eq!(
            manifest_product.product_format,
            AOT_GRAPH_MANIFEST_FORMAT_ID
        );
        assert_eq!(manifest_product.sub_id, RUNTIME_GRAPH_PRODUCT_SUB_ID);
        assert_eq!(
            manifest_product.product_format_version,
            AOT_GRAPH_MANIFEST_PRODUCT_FORMAT_VERSION
        );
        let manifest = decode_aot_graph_manifest(&manifest_product.bytes).unwrap();
        assert_eq!(manifest.header.version, AOT_GRAPH_MANIFEST_VERSION);
        assert_eq!(
            manifest.header.source_uuid,
            uuid!("018f0c5a-0000-7000-8000-0000000000cc")
        );
        assert_eq!(manifest.graph_type, "az.graph_builder.tests.generated");
        assert_eq!(manifest.product_kind, "azoth.graph.generated-rust-test");
        assert_eq!(manifest.language, "rust");
        assert_eq!(manifest.package, "az-graph-builder-generated-tests");
        assert_eq!(
            manifest.entry_symbol,
            "az_graph_builder_generated_tests_018f0c5a0000700080000000000000cc::execute_graph"
        );
        assert_eq!(
            manifest.context_type,
            "az_graph_builder_generated_tests::RuntimeContext"
        );
    }

    /// Asserts the generated-Rust source product's identity and that its body
    /// carries the module, entry point, context binding, node call, and
    /// composition registration — and no link-time `inventory` submission.
    fn assert_generated_rust_source_product(source_product: &GraphCompileProduct) {
        assert_eq!(
            source_product.product_path,
            "graphs/generated.azgraph.generated.rs"
        );
        assert_eq!(
            source_product.asset_type,
            GENERATED_RUST_GRAPH_SOURCE_ASSET_TYPE
        );
        assert_eq!(
            source_product.product_format,
            GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID
        );
        assert_eq!(
            source_product.sub_id,
            GENERATED_RUST_GRAPH_SOURCE_PRODUCT_SUB_ID
        );
        let generated_source = std::str::from_utf8(&source_product.bytes).unwrap();
        assert!(
            generated_source.contains(
                "pub mod az_graph_builder_generated_tests_018f0c5a0000700080000000000000cc"
            ),
            "{generated_source}"
        );
        assert!(
            generated_source.contains("pub fn execute_graph("),
            "{generated_source}"
        );
        assert!(
            generated_source.contains(
                "let runtime_context = context.runtime_context_mut::<az_graph_builder_generated_tests::RuntimeContext>()?;"
            ),
            "{generated_source}"
        );
        assert!(
            generated_source.contains("az_graph_builder_tests::print(&mut *runtime_context)?;"),
            "{generated_source}"
        );
        assert!(
            generated_source.contains(
                "pub const REGISTRATION: az_graph_runtime::AotGraphRuntimeRegistration ="
            ),
            "{generated_source}"
        );
        assert!(
            !generated_source.contains("inventory"),
            "an AOT product registers through a composition, never a link-time submission: {generated_source}"
        );
    }

    #[test]
    fn generated_rust_context_schedule_rejects_dropped_input_values() {
        let mut document = VisualGraphDocument::new("az.graph_builder.tests.generated");
        let mut node = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000024")),
            "az.graph_builder.tests.Print",
            1,
        );
        node.input_values.insert(
            NodePortId::new(1),
            ReflectedValueEnvelope::typed_ron("alloc::string::String", r#""hello""#),
        );
        document.nodes.push(node);
        let graph_type = test_generated_rust_graph_type();
        let composer = test_composer();
        let node_catalog = NodeTypeCatalog::compose(1, 0, composer.registries()).unwrap();

        let error = compile_graph_with_backends(
            GraphCompileRequest {
                source_path: "graphs/generated.azgraph.ron",
                source_uuid: uuid!("018f0c5a-0000-7000-8000-0000000000cf"),
                platform: "pc",
                document: &document,
                graph_type: &graph_type,
                node_catalog: &node_catalog,
                registries: composer.registries(),
            },
            &[&GeneratedRustAotCompiler],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            GraphCompileError::UnsupportedGeneratedRustInputValue {
                node_id,
                port_id,
                ..
            } if node_id == GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000024"))
                && port_id == NodePortId::new(1)
        ));
    }

    #[test]
    fn generated_rust_context_schedule_rejects_data_connections() {
        let mut document = VisualGraphDocument::new("az.graph_builder.tests.generated");
        let source = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000025")),
            "az.graph_builder.tests.StringValue",
            1,
        );
        let target = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000026")),
            "az.graph_builder.tests.Print",
            1,
        );
        document.connections.push(GraphConnection::new(
            GraphConnectionId::new(uuid!("018f0c5a-0000-7000-8000-000000000027")),
            GraphPortRef::new(source.id, NodePortId::new(1)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        ));
        document.nodes.push(source);
        document.nodes.push(target);
        let graph_type = test_generated_rust_graph_type();
        let composer = test_composer();
        let node_catalog = NodeTypeCatalog::compose(1, 0, composer.registries()).unwrap();

        let error = compile_graph_with_backends(
            GraphCompileRequest {
                source_path: "graphs/generated.azgraph.ron",
                source_uuid: uuid!("018f0c5a-0000-7000-8000-0000000000d0"),
                platform: "pc",
                document: &document,
                graph_type: &graph_type,
                node_catalog: &node_catalog,
                registries: composer.registries(),
            },
            &[&GeneratedRustAotCompiler],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            GraphCompileError::UnsupportedGeneratedRustDataConnection {
                connection_id,
                from_port_id,
                to_port_id,
                ..
            } if connection_id
                == GraphConnectionId::new(uuid!("018f0c5a-0000-7000-8000-000000000027"))
                && from_port_id == NodePortId::new(1)
                && to_port_id == NodePortId::new(1)
        ));
    }

    #[test]
    fn built_in_generated_rust_compiler_emits_typed_dataflow_source() {
        let mut document = VisualGraphDocument::new("az.graph_builder.tests.generated");
        let source_node = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000028")),
            "az.graph_builder.tests.DataflowStringValue",
            1,
        );
        let transform_node = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000029")),
            "az.graph_builder.tests.DataflowAppendSuffix",
            1,
        );
        document.graph_type = "az.graph_builder.tests.generated-dataflow".to_string();
        document.connections.push(GraphConnection::new(
            GraphConnectionId::new(uuid!("018f0c5a-0000-7000-8000-00000000002a")),
            GraphPortRef::new(source_node.id, NodePortId::new(1)),
            GraphPortRef::new(transform_node.id, NodePortId::new(1)),
        ));
        document.nodes.push(source_node);
        document.nodes.push(transform_node);
        let graph_type = test_generated_rust_dataflow_graph_type();
        let composer = test_composer();
        let node_catalog = NodeTypeCatalog::compose(1, 0, composer.registries()).unwrap();

        let output = compile_graph_with_backends(
            GraphCompileRequest {
                source_path: "graphs/dataflow.azgraph.ron",
                source_uuid: uuid!("018f0c5a-0000-7000-8000-0000000000d1"),
                platform: "pc",
                document: &document,
                graph_type: &graph_type,
                node_catalog: &node_catalog,
                registries: composer.registries(),
            },
            &[&GeneratedRustAotCompiler],
        )
        .unwrap();

        assert_eq!(output.products.len(), 2);
        let manifest_product = output.runtime_product().unwrap();
        let manifest = decode_aot_graph_manifest(&manifest_product.bytes).unwrap();
        assert_eq!(
            manifest.product_kind,
            "azoth.graph.generated-rust-dataflow-test"
        );
        assert_eq!(
            manifest.entry_symbol,
            "az_graph_builder_generated_tests_018f0c5a0000700080000000000000d1::execute_dataflow"
        );

        let source_product = output
            .products
            .iter()
            .find(|product| product.role == GraphCompileProductRole::GeneratedRustSource)
            .unwrap();
        let generated_source = std::str::from_utf8(&source_product.bytes).unwrap();
        assert!(
            generated_source.contains("pub fn execute_dataflow("),
            "{generated_source}"
        );
        assert!(
            generated_source
                .contains("let runtime_context = context.runtime_context_mut::<az_graph_builder_generated_tests::RuntimeContext>()?;"),
            "{generated_source}"
        );
        assert!(
            generated_source
                .contains("let node_018f0c5a_0000_7000_8000_000000000028_output_1: String = az_graph_builder_tests::dataflow_string_value()?;"),
            "{generated_source}"
        );
        assert!(
            generated_source
                .contains("let node_018f0c5a_0000_7000_8000_000000000029_output_2: String = az_graph_builder_tests::append_suffix(&*runtime_context, &node_018f0c5a_0000_7000_8000_000000000028_output_1)?;"),
            "{generated_source}"
        );
    }

    #[test]
    fn generated_rust_dataflow_rejects_hidden_by_value_fanout() {
        fn by_value_transform_node_type() -> NodeTypeDescriptor {
            NodeTypeDescriptor::new(
                "az.graph_builder.tests.DataflowConsumeString",
                1,
                "Dataflow Consume String",
            )
            .with_port(NodePortDescriptor::new(
                NodePortId::new(1),
                "value",
                NodePortDirection::Input,
                NodePortValue::Data {
                    schema_type: "core.string".to_string(),
                },
            ))
            .with_runtime_binding(NodeRuntimeBinding::rust_typed_dataflow(
                "az-graph-builder-tests",
                "az_graph_builder_tests::consume_string",
                RustTypedDataflowNodeCall::new(RustDataflowOutput::None).with_parameter(
                    RustDataflowParameter::input(
                        NodePortId::new(1),
                        "String",
                        RustValuePassing::ByValue,
                    ),
                ),
            ))
        }

        let source_node = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-00000000002b")),
            "az.graph_builder.tests.DataflowStringValue",
            1,
        );
        let left = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-00000000002c")),
            "az.graph_builder.tests.DataflowConsumeString",
            1,
        );
        let right = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-00000000002d")),
            "az.graph_builder.tests.DataflowConsumeString",
            1,
        );
        let mut document = VisualGraphDocument::new("az.graph_builder.tests.generated-dataflow");
        document.connections.push(GraphConnection::new(
            GraphConnectionId::new(uuid!("018f0c5a-0000-7000-8000-00000000002e")),
            GraphPortRef::new(source_node.id, NodePortId::new(1)),
            GraphPortRef::new(left.id, NodePortId::new(1)),
        ));
        document.connections.push(GraphConnection::new(
            GraphConnectionId::new(uuid!("018f0c5a-0000-7000-8000-00000000002f")),
            GraphPortRef::new(source_node.id, NodePortId::new(1)),
            GraphPortRef::new(right.id, NodePortId::new(1)),
        ));
        document.nodes.push(source_node);
        document.nodes.push(left);
        document.nodes.push(right);
        let graph_type = test_generated_rust_dataflow_graph_type();
        let composer = test_composer();
        let node_catalog = NodeTypeCatalog::try_new(
            1,
            0,
            [
                az_node_graph::node_types(composer.registries()),
                vec![by_value_transform_node_type()],
            ]
            .concat(),
        )
        .unwrap();

        let error = compile_graph_with_backends(
            GraphCompileRequest {
                source_path: "graphs/dataflow.azgraph.ron",
                source_uuid: uuid!("018f0c5a-0000-7000-8000-0000000000d2"),
                platform: "pc",
                document: &document,
                graph_type: &graph_type,
                node_catalog: &node_catalog,
                registries: composer.registries(),
            },
            &[&GeneratedRustAotCompiler],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            GraphCompileError::MovedGeneratedRustDataflowValueFanout {
                node_id,
                port_id,
                consumer_count,
            } if node_id == GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-00000000002b"))
                && port_id == NodePortId::new(1)
                && consumer_count == 2
        ));
    }

    #[test]
    fn graph_compile_rejects_backend_output_asset_type_mismatch() {
        struct WrongAssetTypeCompiler;

        impl GraphCompilerBackend for WrongAssetTypeCompiler {
            fn backend_id(&self) -> &'static str {
                "az.graph-builder.tests.wrong-asset-type"
            }

            fn supports(&self, backend: &GraphCompilerBackendDescriptor) -> bool {
                matches!(
                    &backend.kind,
                    GraphCompilerBackendKind::GeneratedRust { .. }
                )
            }

            fn compile(
                &self,
                _request: GraphCompileRequest<'_>,
            ) -> Result<GraphCompileOutput, GraphCompileError> {
                Ok(GraphCompileOutput::single(GraphCompileProduct {
                    role: GraphCompileProductRole::RuntimeProduct,
                    product_path: "graphs/wrong.azgaot".to_string(),
                    asset_type: PACKED_GRAPH_IR_ASSET_TYPE,
                    product_format: AOT_GRAPH_MANIFEST_FORMAT_ID,
                    product_format_version: AOT_GRAPH_MANIFEST_PRODUCT_FORMAT_VERSION,
                    sub_id: RUNTIME_GRAPH_PRODUCT_SUB_ID,
                    bytes: Vec::new(),
                }))
            }
        }

        let mut document = VisualGraphDocument::new("az.graph_builder.tests.generated");
        document.nodes.push(GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000023")),
            "az.graph_builder.tests.Print",
            1,
        ));
        let graph_type = test_generated_rust_graph_type();
        let composer = test_composer();
        let node_catalog = NodeTypeCatalog::compose(1, 0, composer.registries()).unwrap();

        let error = compile_graph_with_backends(
            GraphCompileRequest {
                source_path: "graphs/generated.azgraph.ron",
                source_uuid: uuid!("018f0c5a-0000-7000-8000-0000000000ce"),
                platform: "pc",
                document: &document,
                graph_type: &graph_type,
                node_catalog: &node_catalog,
                registries: composer.registries(),
            },
            &[&WrongAssetTypeCompiler],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            GraphCompileError::RuntimeAssetTypeMismatch { expected, actual, .. }
                if expected == GENERATED_GRAPH_ASSET_TYPE && actual == PACKED_GRAPH_IR_ASSET_TYPE
        ));
    }

    #[test]
    fn graph_compile_rejects_backend_runtime_strategy_mismatch() {
        let mut document = VisualGraphDocument::new("az.graph_builder.tests.generated");
        document.nodes.push(GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-000000000022")),
            "az.graph_builder.tests.Print",
            1,
        ));
        let graph_type = test_generated_rust_graph_type().with_runtime_product(
            RuntimeGraphProductDescriptor::new(
                GENERATED_GRAPH_ASSET_TYPE_NAME,
                "azoth.graph.generated-rust-test",
                RuntimeGraphExecutionStrategy::PackedIr,
            ),
        );
        let composer = test_composer();
        let node_catalog = NodeTypeCatalog::compose(1, 0, composer.registries()).unwrap();
        let compiler: &dyn GraphCompilerBackend = &GeneratedRustAotCompiler;

        let error = compile_graph_with_backends(
            GraphCompileRequest {
                source_path: "graphs/generated.azgraph.ron",
                source_uuid: uuid!("018f0c5a-0000-7000-8000-0000000000cd"),
                platform: "pc",
                document: &document,
                graph_type: &graph_type,
                node_catalog: &node_catalog,
                registries: composer.registries(),
            },
            &[compiler],
        )
        .unwrap_err();

        assert!(matches!(error, GraphCompileError::GraphTypeCatalog(_)));
    }

    #[test]
    fn composed_generated_rust_graphs_are_claimed_by_the_graph_compiler() {
        let composer = test_composer();
        let registries = composer.registries();

        assert!(is_compilable_graph_type(
            registries,
            "az.graph_builder.tests.generated"
        ));
        assert!(
            !is_compilable_graph_type(registries, "az.graph_builder.tests.unknown"),
            "an uncomposed graph type is not claimed"
        );
        assert!(
            compilable_graph_type_names(registries)
                .contains(&"az.graph_builder.tests.generated".to_string())
        );
    }

    /// The rule's claim is its host's composition, all the way through the
    /// job path: a composed compilable graph type is claimed, and a host that
    /// composed none claims nothing.
    #[test]
    fn the_rule_claims_exactly_what_its_host_composed() {
        fn plan(context: JobContext<'_>) -> CreateJobsResult {
            let rule = graph_compiler_build_rule(&context);
            (rule.create_jobs)(&CreateJobsRequest {
                builder_id: rule.id,
                source_path: SourcePath::new("graphs/generated.azgraph.ron"),
                source_root: Path::new("assets"),
                source_uuid: uuid!("018f0c5a-0000-7000-8000-0000000000dd"),
                source_schema_type: Some("az.graph_builder.tests.generated"),
                source_bytes: b"",
                platforms: &[az_asset_builder::PlatformId::new_static("pc")],
                context: &context,
            })
            .result
        }

        let composer = test_composer();
        let host_context = JobContext::new(composer.registries());
        let rule = graph_compiler_build_rule(&host_context);
        let SourceMatcher::Schemas { schemas, .. } = &rule.primary_source else {
            panic!("the graph compiler matches on source schemas");
        };
        assert!(
            schemas
                .iter()
                .any(|schema| schema.as_str() == "az.graph_builder.tests.generated"),
            "a composed compilable graph type must widen the rule's claim"
        );
        assert!(rule.matches_source(
            "graphs/generated.azgraph.ron",
            Some("az.graph_builder.tests.generated")
        ));
        assert_eq!(plan(host_context), CreateJobsResult::Success);

        let empty = Registries::new();
        assert_eq!(plan(JobContext::new(&empty)), CreateJobsResult::Skipped);
    }

    #[test]
    fn packed_ir_decoder_rejects_version_in_magic() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"AZGIR001");

        let error = decode_packed_graph_ir(&bytes).unwrap_err();
        assert!(matches!(error, PackedGraphIrError::BadMagic { .. }));
    }
}
