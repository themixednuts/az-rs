use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use az_asset_builder::{
    AssetBuilderPattern, BuildProduct, BuildRule, BuildRuleRegistration, BuildRuleRegistry,
    BuilderId, CreateJobsRequest, CreateJobsResponse, JobContext as BuilderJobContext,
    JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProductDependency, ProductFormat,
    ProductFormatId, ProductFormatRegistration, SourceFormat, SourceSchemaRegistration,
    SourceSchemaType,
};
use az_assetdb::{
    AssetDb, Exclusions, RegisterWorkspace, RegisterWorkspaceRoot, RepoError, WorkspaceKey,
};
use az_core::{AssetData, AssetTypeRegistration, AzRtti, AzTypeInfo};
use az_filesystem::{AzothDataHome, SourcePath, normalize, safe_join};
use az_gem_contract::{
    Composer, Contribution, ContributionDescriptor, ContributionId, GemId, GemTargetRole,
    ProductActivation, Registries,
};
use az_project::PortableKey;
use az_proto_asset::{
    AssetRootScope, ReconcileAssetSourcesRequest, ReconcileAssetSourcesResult, WorkspaceEntry,
    WorkspaceEntryPageRequest, WorkspaceSnapshot, WorkspaceSnapshotRequest,
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ASSET_JOBS_PERMISSION, ASSET_PROCESSOR_AUDIENCE, ASSET_READ_PERMISSION,
    ASSET_WORKER_SERVICE_NAME, ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WRITE_PERMISSION,
    AssetProcessor, AssetProcessorError, AssetProcessorRpc, RegisteredSourceRoot, SourceAssetMeta,
    SourceRootRole, resolve_source_asset_guid, source_meta_sidecar_path,
};

pub use az_proto_asset::{
    AssetBuilderCatalogRequest, AttemptStatus, SourceAssetRecordRequest, SourceDependentsRequest,
    SourceFileCreateContent, SourceFileCreateRequest, SourceFileDeleteRequest,
    SourceFileMoveRequest, SourceSchemaAuthoring, WorkspaceEntryDiff, asset_capnp,
};
pub use az_proto_core::{
    Capability, CapabilityGrantSet, ServiceId, ServiceRole, SideChannelHandle,
};

const SYNTHETIC_SESSION_ID: &str = "2f706be0-6d33-4b24-83e2-ec27e29c597e";
const SYNTHETIC_PROJECT_DOCUMENT_SCHEMA: SourceSchemaType =
    SourceSchemaType::from_dynamic_parts("az.synthetic.ProjectDocument");
const SYNTHETIC_RON_SCHEMA: SourceSchemaType =
    SourceSchemaType::from_dynamic_parts("az.synthetic.EditableRon");
const SYNTHETIC_JSON_SCHEMA: SourceSchemaType =
    SourceSchemaType::from_dynamic_parts("az.synthetic.EditableJson");
const SYNTHETIC_PNG_SCHEMA: SourceSchemaType =
    SourceSchemaType::from_dynamic_parts("az.synthetic.EditablePng");
const SYNTHETIC_BINARY_SCHEMA: SourceSchemaType =
    SourceSchemaType::from_dynamic_parts("az.synthetic.EditableBinary");

struct SyntheticProjectDocumentFormat;

impl SourceFormat for SyntheticProjectDocumentFormat {
    const SCHEMA: Option<SourceSchemaType> = Some(SYNTHETIC_PROJECT_DOCUMENT_SCHEMA);
    const SCHEMA_TYPES: &'static [SourceSchemaType] = &[SYNTHETIC_PROJECT_DOCUMENT_SCHEMA];
    const PATTERNS: &'static [AssetBuilderPattern] =
        &[AssetBuilderPattern::static_wildcard("*.prefab.ron")];
}

macro_rules! synthetic_file_format {
    ($name:ident, $schema:ident, $pattern:literal) => {
        struct $name;

        impl SourceFormat for $name {
            const SCHEMA: Option<SourceSchemaType> = Some($schema);
            const SCHEMA_TYPES: &'static [SourceSchemaType] = &[$schema];
            const PATTERNS: &'static [AssetBuilderPattern] =
                &[AssetBuilderPattern::static_wildcard($pattern)];
        }
    };
}

synthetic_file_format!(SyntheticRonFormat, SYNTHETIC_RON_SCHEMA, "*.ron");
synthetic_file_format!(SyntheticJsonFormat, SYNTHETIC_JSON_SCHEMA, "*.json");
synthetic_file_format!(SyntheticPngFormat, SYNTHETIC_PNG_SCHEMA, "*.png");
synthetic_file_format!(SyntheticBinaryFormat, SYNTHETIC_BINARY_SCHEMA, "*.bin");

struct SyntheticProductAsset;

impl AzTypeInfo for SyntheticProductAsset {
    const NAME: &'static str = "Azoth::SyntheticProductAsset";
    const TYPE_ID: Uuid = Uuid::from_u128(0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0a01);
}

impl AzRtti for SyntheticProductAsset {}

impl AssetData for SyntheticProductAsset {
    const STABLE_NAME: &'static str = "azoth.synthetic.product";
}

struct SyntheticProductFormat;

impl ProductFormat for SyntheticProductFormat {
    const ID: ProductFormatId = ProductFormatId::from_dynamic_parts("azoth.synthetic.product/v1");
    const VERSION: u32 = 1;
    type Asset = SyntheticProductAsset;
}

fn synthetic_create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
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

fn synthetic_process_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
    let stem = request.source_uuid.as_hyphenated();
    let primary_path = format!("synthetic/{stem}.product");
    let auxiliary_path = format!("synthetic/{stem}.aux.product");
    let primary = BuildProduct::for_primary_source_format::<SyntheticProductFormat>(
        request,
        primary_path,
        0,
        blake3::hash(request.source_bytes).as_bytes().to_vec(),
    )
    .expect("synthetic product path is asset-relative");
    let auxiliary = BuildProduct::for_format::<SyntheticProductFormat>(
        auxiliary_path,
        1,
        request.source_uuid.as_bytes().to_vec(),
    )
    .expect("synthetic auxiliary product path is asset-relative");
    ProcessJobResponse {
        products: vec![primary, auxiliary],
        product_dependencies: vec![(
            0,
            ProductDependency::new(az_asset::AssetId::new(request.source_uuid, 1)),
        )],
        ..ProcessJobResponse::default()
    }
}

fn synthetic_rule<S: SourceFormat>(name: &'static str, id: Uuid) -> BuildRule {
    BuildRule::for_source::<S>()
        .named(name)
        .id(BuilderId::new(id))
        .version(1)
        .produces::<SyntheticProductFormat>()
        .create_jobs(synthetic_create_jobs)
        .process(synthetic_process_job)
}

macro_rules! synthetic_rule_factory {
    ($function:ident, $format:ty, $name:literal, $id:expr) => {
        fn $function(_: &BuilderJobContext<'_>) -> BuildRule {
            synthetic_rule::<$format>($name, Uuid::from_u128($id))
        }
    };
}

synthetic_rule_factory!(
    synthetic_project_document_rule,
    SyntheticProjectDocumentFormat,
    "azoth.synthetic.project-document",
    0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b01
);
synthetic_rule_factory!(
    synthetic_ron_rule,
    SyntheticRonFormat,
    "azoth.synthetic.ron",
    0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b02
);
synthetic_rule_factory!(
    synthetic_json_rule,
    SyntheticJsonFormat,
    "azoth.synthetic.json",
    0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b03
);
synthetic_rule_factory!(
    synthetic_png_rule,
    SyntheticPngFormat,
    "azoth.synthetic.png",
    0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b04
);
synthetic_rule_factory!(
    synthetic_binary_rule,
    SyntheticBinaryFormat,
    "azoth.synthetic.binary",
    0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b05
);

az_gem_contract::declare_caps!(SyntheticCorpusCaps:);

const SYNTHETIC_CORPUS_CONTRIBUTION: ContributionDescriptor = ContributionDescriptor {
    gem: GemId::new("azoth.synthetic-asset-corpus"),
    contribution: ContributionId::new("source-composition"),
    roles: &[],
};

struct SyntheticCorpusContribution;

impl Contribution for SyntheticCorpusContribution {
    type Caps = SyntheticCorpusCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        SYNTHETIC_CORPUS_CONTRIBUTION
    }

    fn register(&self, context: &mut az_gem_contract::GemContext<'_, SyntheticCorpusCaps>) {
        context.registrar::<AssetTypeRegistration>().register(
            AssetTypeRegistration::for_asset::<SyntheticProductAsset>()
                .with_owner("az-asset-processor::test-support"),
        );
        context.registrar::<ProductFormatRegistration>().register(
            ProductFormatRegistration::for_format::<SyntheticProductFormat>(),
        );
        context
            .registrar::<SourceSchemaRegistration>()
            .register_many([
                SourceSchemaRegistration::for_source::<SyntheticProjectDocumentFormat>()
                    .with_label("Synthetic Project Document")
                    .with_category("Synthetic Corpus")
                    .with_creatable_document_schema("az.synthetic.ProjectDocument"),
                SourceSchemaRegistration::for_source::<SyntheticRonFormat>()
                    .with_label("Synthetic RON")
                    .with_category("Synthetic Corpus")
                    .with_editable_file("ron", &["ron"]),
                SourceSchemaRegistration::for_source::<SyntheticJsonFormat>()
                    .with_label("Synthetic JSON")
                    .with_category("Synthetic Corpus")
                    .with_editable_file("json", &["json"]),
                SourceSchemaRegistration::for_source::<SyntheticPngFormat>()
                    .with_label("Synthetic PNG")
                    .with_category("Synthetic Corpus")
                    .with_editable_file("images", &["png"]),
                SourceSchemaRegistration::for_source::<SyntheticBinaryFormat>()
                    .with_label("Synthetic Binary")
                    .with_category("Synthetic Corpus")
                    .with_editable_file("binary", &["bin"]),
            ]);
        context.registrar::<BuildRuleRegistration>().register_many([
            BuildRuleRegistration::new(
                "azoth.synthetic.project-document",
                BuilderId::new(Uuid::from_u128(0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b01)),
                synthetic_project_document_rule,
            ),
            BuildRuleRegistration::new(
                "azoth.synthetic.ron",
                BuilderId::new(Uuid::from_u128(0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b02)),
                synthetic_ron_rule,
            ),
            BuildRuleRegistration::new(
                "azoth.synthetic.json",
                BuilderId::new(Uuid::from_u128(0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b03)),
                synthetic_json_rule,
            ),
            BuildRuleRegistration::new(
                "azoth.synthetic.png",
                BuilderId::new(Uuid::from_u128(0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b04)),
                synthetic_png_rule,
            ),
            BuildRuleRegistration::new(
                "azoth.synthetic.binary",
                BuilderId::new(Uuid::from_u128(0x0c01_2a7e_e158_4cc4_916c_60b6_c84c_0b05)),
                synthetic_binary_rule,
            ),
        ]);
    }
}

fn synthetic_corpus_registries() -> &'static Registries {
    static REGISTRIES: OnceLock<&'static Registries> = OnceLock::new();
    REGISTRIES.get_or_init(|| {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(SyntheticCorpusContribution, ProductActivation::default())
            .expect("the synthetic corpus contribution has no capability floor");
        composer
            .finalize()
            .expect("the synthetic corpus composition is valid");
        Box::leak(Box::new(composer)).registries()
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticSourceSpec {
    pub source_path: String,
    pub bytes: Vec<u8>,
    pub preserved_guid: Option<Uuid>,
}

impl SyntheticSourceSpec {
    #[must_use]
    pub fn new(source_path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            source_path: source_path.into(),
            bytes: bytes.into(),
            preserved_guid: None,
        }
    }

    /// Preserve a caller-selected identity through the real source sidecar.
    #[must_use]
    pub const fn with_guid(mut self, guid: Uuid) -> Self {
        self.preserved_guid = Some(guid);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticSource {
    pub entry_id: i64,
    pub guid: Uuid,
    pub source_path: String,
    pub schema_type: Option<String>,
    pub content_hash: String,
    pub byte_length: u64,
}

struct PreparedProcessor {
    rpc: Rc<AssetProcessorRpc>,
    read_capability: Capability,
    write_capability: Capability,
    /// Only [`SyntheticFixture::jobs_capability_for_test`] reads this, and that
    /// accessor is `cfg(test)`: the `test-support` build hands out read and
    /// write grants only, so the field is genuinely unread there.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read only by the cfg(test) jobs_capability_for_test accessor"
        )
    )]
    jobs_capability: Capability,
    byte_lengths: BTreeMap<String, u64>,
    source_root: PathBuf,
}

enum SyntheticStorage {
    Memory,
    File(PathBuf),
}

/// Materializes every declared source (and its identity sidecar) under
/// `source_root`, returning each source's byte length by canonical path.
///
/// Two specs claiming the same canonical path are rejected rather than letting
/// the second silently overwrite the first.
fn write_synthetic_sources(
    source_root: &Path,
    sources: impl IntoIterator<Item = SyntheticSourceSpec>,
) -> Result<BTreeMap<String, u64>, SyntheticCorpusError> {
    let mut byte_lengths = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for source in sources {
        let source_path = SourcePath::non_empty(&source.source_path)
            .ok_or_else(|| SyntheticCorpusError::UnsafeSourcePath(source.source_path.clone()))?;
        let source_path_text = source_path.into_string();
        if !seen.insert(source_path_text.clone()) {
            return Err(SyntheticCorpusError::DuplicateSourcePath(source_path_text));
        }
        let absolute = safe_join(source_root, &source_path_text)
            .map_err(|_| SyntheticCorpusError::UnsafeSourcePath(source_path_text.clone()))?;
        if let Some(parent) = absolute.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&absolute, &source.bytes)?;
        let byte_length =
            u64::try_from(source.bytes.len()).map_err(|_| SyntheticCorpusError::CountOverflow {
                field: "source byte length",
                value: source.bytes.len(),
            })?;
        byte_lengths.insert(source_path_text.clone(), byte_length);
        if let Some(guid) = source.preserved_guid {
            let sidecar = SourceAssetMeta::preserving_source_guid(guid);
            std::fs::write(
                source_meta_sidecar_path(&absolute),
                serde_json::to_vec(&sidecar)?,
            )?;
        }
    }
    Ok(byte_lengths)
}

impl PreparedProcessor {
    fn prepare(
        workspace_root: impl AsRef<Path>,
        project_id: impl Into<String>,
        sources: impl IntoIterator<Item = SyntheticSourceSpec>,
        storage: SyntheticStorage,
    ) -> Result<Self, SyntheticCorpusError> {
        let workspace_root = normalize(workspace_root.as_ref());
        let project_id = project_id.into();
        let source_root = workspace_root.join("assets");
        std::fs::create_dir_all(&source_root)?;

        let byte_lengths = write_synthetic_sources(&source_root, sources)?;

        let db = match storage {
            SyntheticStorage::Memory => AssetDb::open_in_memory()?,
            SyntheticStorage::File(path) => {
                if path.exists() {
                    return Err(SyntheticCorpusError::ExistingDatabase { path });
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                AssetDb::open(&path)?
            }
        };
        let writer = db.writer()?;
        let workspace = writer.register_workspace(RegisterWorkspace {
            key: WorkspaceKey {
                project: project_id.clone(),
                root: workspace_root.to_string_lossy().into_owned(),
                branch: "synthetic".into(),
            },
            now: 1,
        });
        let workspace = futures::executor::block_on(workspace)?;
        let portable_key = String::from(PortableKey::project_assets(&project_id));
        let root = writer.register_workspace_root(RegisterWorkspaceRoot {
            workspace_pk: workspace.workspace_id,
            key: portable_key.clone(),
            owner: project_id.clone(),
            path: source_root.to_string_lossy().into_owned(),
            exclusions: Exclusions::default(),
        });
        let (root, policy) = futures::executor::block_on(root)?;
        let registered_root = RegisteredSourceRoot {
            workspace_pk: workspace.workspace_id,
            workspace_root_pk: policy.workspace_root_id,
            root_pk: root.root_id,
            id: portable_key.clone(),
            owner: project_id.clone(),
            path: policy.path,
            display_name: "Project Assets".into(),
            portable_key,
            mount: "@assets@".into(),
            recursive: true,
            watch: true,
            writable: true,
            exclusions: policy.exclusions,
            output_prefix: String::new(),
            role: SourceRootRole::ProjectAssets,
        };

        let read_capability = synthetic_read_capability();
        let write_capability = synthetic_write_capability();
        let jobs_capability = synthetic_jobs_capability();
        let grants = CapabilityGrantSet::from_grants(vec![
            read_capability.clone(),
            write_capability.clone(),
            jobs_capability.clone(),
        ]);
        let registries = synthetic_corpus_registries();
        let builders = BuildRuleRegistry::compose(&BuilderJobContext::new(registries));
        let project_data_paths = AzothDataHome::new(workspace_root.join(".azoth-test-home"))
            .project(&project_id, &workspace_root);
        let processor = AssetProcessor::with_builder_registry_and_catalog_with_asset_db_writer(
            db,
            builders,
            grants,
            registries,
            Some(workspace.workspace_id),
            Some(project_data_paths),
            None,
            writer,
        )
        .with_source_roots(vec![registered_root]);
        Ok(Self {
            rpc: Rc::new(AssetProcessorRpc::new(processor)),
            read_capability,
            write_capability,
            jobs_capability,
            byte_lengths,
            source_root,
        })
    }

    fn sweep(&self) -> Result<ReconcileAssetSourcesResult, AssetProcessorError> {
        self.rpc
            .processor
            .reconcile_asset_sources(&ReconcileAssetSourcesRequest {
                capability: self.write_capability.clone(),
                session_id: SYNTHETIC_SESSION_ID.into(),
                root_scope: AssetRootScope::All,
            })
    }

    fn snapshot(&self) -> Result<WorkspaceSnapshot, SyntheticCorpusError> {
        self.rpc
            .processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: self.read_capability.clone(),
                root_scope: AssetRootScope::All,
            })?
            .snapshot
            .ok_or(SyntheticCorpusError::MissingWorkspaceSnapshot)
    }

    fn sources(&self) -> Result<Vec<SyntheticSource>, SyntheticCorpusError> {
        let entries = workspace_entries(&self.rpc.processor, &self.read_capability)?;
        let mut projected_sources = Vec::with_capacity(entries.len());
        let mut projected_paths = BTreeSet::new();
        for entry in entries {
            let byte_length = self
                .byte_lengths
                .get(&entry.source_path)
                .copied()
                .ok_or_else(|| SyntheticCorpusError::MissingSeed(entry.source_path.clone()))?;
            projected_paths.insert(entry.source_path.clone());
            projected_sources.push(projected_source(entry, byte_length));
        }
        if let Some(source_path) = self
            .byte_lengths
            .keys()
            .find(|source_path| !projected_paths.contains(*source_path))
        {
            return Err(SyntheticCorpusError::MissingProjection(source_path.clone()));
        }
        Ok(projected_sources)
    }

    fn client(&self) -> asset_capnp::asset_processor::Client {
        AssetProcessorRpc::client_from_rc(&self.rpc)
    }
}

/// A small caller-shaped AP/RPC fixture.
///
/// It writes native sources and lets the shipping reconcile path derive every
/// identity, schema, digest, diff, and planner job. Storage keys and writer
/// commands do not escape this owner.
pub struct SyntheticFixture {
    prepared: PreparedProcessor,
    pub snapshot: WorkspaceSnapshot,
    pub sources: Vec<SyntheticSource>,
    pub reconcile: ReconcileAssetSourcesResult,
}

impl SyntheticFixture {
    /// # Errors
    ///
    /// Returns [`SyntheticCorpusError::ExistingDatabase`] if a corpus database is already present,
    /// [`SyntheticCorpusError::UnsafeSourcePath`] or [`SyntheticCorpusError::DuplicateSourcePath`] if a seed's path is
    /// not safe-relative or repeats another, [`SyntheticCorpusError::Io`] or [`SyntheticCorpusError::Json`] if the
    /// tree or a sidecar cannot be written, [`SyntheticCorpusError::Open`] or [`SyntheticCorpusError::Repo`] if the
    /// database cannot be opened or written, and [`SyntheticCorpusError::Processor`] if the processor
    /// refuses the corpus.
    pub fn build(
        workspace_root: impl AsRef<Path>,
        project_id: impl Into<String>,
        sources: impl IntoIterator<Item = SyntheticSourceSpec>,
    ) -> Result<Self, SyntheticCorpusError> {
        let prepared = PreparedProcessor::prepare(
            workspace_root,
            project_id,
            sources,
            SyntheticStorage::Memory,
        )?;
        let reconcile = prepared.sweep()?;
        let snapshot = prepared.snapshot()?;
        let sources = prepared.sources()?;
        Ok(Self {
            prepared,
            snapshot,
            sources,
            reconcile,
        })
    }

    #[cfg(test)]
    pub(crate) fn rpc_for_test(&self) -> Rc<AssetProcessorRpc> {
        Rc::clone(&self.prepared.rpc)
    }

    #[cfg(test)]
    pub(crate) fn jobs_capability_for_test(&self) -> Capability {
        self.prepared.jobs_capability.clone()
    }

    #[must_use]
    pub fn client(&self) -> asset_capnp::asset_processor::Client {
        self.prepared.client()
    }

    #[must_use]
    pub fn read_capability(&self) -> Capability {
        self.prepared.read_capability.clone()
    }

    #[must_use]
    pub fn write_capability(&self) -> Capability {
        self.prepared.write_capability.clone()
    }
}

fn workspace_entries(
    processor: &AssetProcessor,
    capability: &Capability,
) -> Result<Vec<WorkspaceEntry>, AssetProcessorError> {
    let mut entries = Vec::new();
    let mut after_entry_id = None;
    loop {
        let page = processor.workspace_entry_page(&WorkspaceEntryPageRequest {
            capability: capability.clone(),
            root_scope: AssetRootScope::All,
            after_entry_id,
            page_size: 4_096,
        })?;
        entries.extend(page.entries);
        let Some(next) = page.next_after_entry_id else {
            break;
        };
        after_entry_id = Some(next);
    }
    Ok(entries)
}

fn projected_source(entry: WorkspaceEntry, byte_length: u64) -> SyntheticSource {
    SyntheticSource {
        entry_id: entry.entry_id,
        guid: entry.asset_guid,
        source_path: entry.source_path,
        schema_type: entry.schema_type,
        content_hash: entry.content_hash,
        byte_length,
    }
}

fn synthetic_read_capability() -> Capability {
    Capability::new(
        ServiceId::new("azoth", "synthetic-corpus"),
        ServiceRole::Editor,
    )
    .with_audience(ASSET_PROCESSOR_AUDIENCE)
    .with_permissions([ASSET_READ_PERMISSION])
    .with_token_hash([0x73, 0x79, 0x6e, 0x72])
}

fn synthetic_write_capability() -> Capability {
    Capability::new(
        ServiceId::new("azoth", "synthetic-corpus"),
        ServiceRole::Editor,
    )
    .with_audience(ASSET_PROCESSOR_AUDIENCE)
    .with_permissions([ASSET_WRITE_PERMISSION])
    .with_token_hash([0x73, 0x79, 0x6e, 0x77])
}

fn synthetic_jobs_capability() -> Capability {
    Capability::new(
        ServiceId::new(ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WORKER_SERVICE_NAME),
        ServiceRole::Worker,
    )
    .with_audience(ASSET_PROCESSOR_AUDIENCE)
    .with_permissions([ASSET_JOBS_PERMISSION])
    .with_token_hash([0x73, 0x79, 0x6e, 0x6a])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusScale {
    Small,
    Dev,
    Release,
}

const GIB: u64 = 1_024 * 1_024 * 1_024;
const RELEASE_FILE_COUNT: usize = 1_151_840;
const MEASURED_RELEASE_TOTAL_BYTES: u64 = 153 * GIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorpusShape {
    pub regular_file_count: usize,
    pub total_bytes: u64,
    pub directory_count: usize,
    pub max_depth: usize,
    pub mean_path_hundredths: usize,
    pub max_path_bytes: usize,
    pub prefab_per_ten_thousand: usize,
    pub ron_per_ten_thousand: usize,
    pub sidecar_count: usize,
    pub prefab_bytes: usize,
}

impl CorpusScale {
    #[must_use]
    pub const fn shape(self) -> CorpusShape {
        match self {
            Self::Small => CorpusShape {
                regular_file_count: 64,
                total_bytes: scaled_total_bytes(64),
                directory_count: 16,
                max_depth: 12,
                mean_path_hundredths: 9_097,
                max_path_bytes: 165,
                prefab_per_ten_thousand: 2_220,
                ron_per_ten_thousand: 4_320,
                sidecar_count: 1,
                prefab_bytes: 37_683,
            },
            Self::Dev => CorpusShape {
                regular_file_count: 10_000,
                total_bytes: scaled_total_bytes(10_000),
                directory_count: 512,
                max_depth: 12,
                mean_path_hundredths: 9_097,
                max_path_bytes: 165,
                prefab_per_ten_thousand: 2_220,
                ron_per_ten_thousand: 4_320,
                sidecar_count: 1,
                prefab_bytes: 37_683,
            },
            Self::Release => CorpusShape {
                regular_file_count: RELEASE_FILE_COUNT,
                total_bytes: MEASURED_RELEASE_TOTAL_BYTES,
                directory_count: 33_018,
                max_depth: 12,
                mean_path_hundredths: 9_097,
                max_path_bytes: 165,
                prefab_per_ten_thousand: 2_220,
                ron_per_ten_thousand: 4_320,
                sidecar_count: 55,
                prefab_bytes: 37_683,
            },
        }
    }
}

/// Scales the measured release corpus byte total down to `regular_file_count`
/// files, keeping the multiply exact by widening it to `u128` first.
// A `const fn` cannot call `TryFrom`, so the narrowing back to `u64` has to be
// a cast. It cannot lose bits at any scale defined here: the widest product is
// the measured 153 GiB times the release file count, divided straight back
// down by that same count.
#[allow(clippy::cast_possible_truncation)]
const fn scaled_total_bytes(regular_file_count: usize) -> u64 {
    ((MEASURED_RELEASE_TOTAL_BYTES as u128 * regular_file_count as u128)
        / RELEASE_FILE_COUNT as u128) as u64
}

/// A deterministic Ticket-012 file tree and registered file-backed processor.
/// Construction is deliberately outside the timed sweep phase.
pub struct PreparedCorpus {
    pub scale: CorpusScale,
    pub shape: CorpusShape,
    prepared: PreparedProcessor,
    database_path: PathBuf,
}

impl PreparedCorpus {
    /// Invoke only the shipping scan/classify/diff reconcile path.
    ///
    /// # Errors
    ///
    /// Returns [`SyntheticCorpusError::Processor`] wrapping any error the processor's asset-source
    /// reconcile returns.
    pub fn sweep(&self) -> Result<ReconcileAssetSourcesResult, SyntheticCorpusError> {
        Ok(self.prepared.sweep()?)
    }

    #[must_use]
    pub fn client(&self) -> asset_capnp::asset_processor::Client {
        self.prepared.client()
    }

    #[must_use]
    pub fn read_capability(&self) -> Capability {
        self.prepared.read_capability.clone()
    }

    #[must_use]
    pub fn write_capability(&self) -> Capability {
        self.prepared.write_capability.clone()
    }

    #[must_use]
    pub fn source_root(&self) -> &Path {
        &self.prepared.source_root
    }

    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// # Errors
    ///
    /// Returns [`SyntheticCorpusError::MissingWorkspaceSnapshot`] if the processor did not project
    /// its attached workspace, and [`SyntheticCorpusError::Processor`] if the snapshot query fails.
    pub fn snapshot(&self) -> Result<WorkspaceSnapshot, SyntheticCorpusError> {
        self.prepared.snapshot()
    }

    /// # Errors
    ///
    /// Returns [`SyntheticCorpusError::MissingProjection`] if a seed was not projected,
    /// [`SyntheticCorpusError::MissingSeed`] if the projection names a seed the corpus does not have,
    /// and [`SyntheticCorpusError::Processor`] if the underlying query fails.
    pub fn sources(&self) -> Result<Vec<SyntheticSource>, SyntheticCorpusError> {
        self.prepared.sources()
    }
}

/// Where each extension's run ends in the generated source index space.
///
/// Sources are laid out in one pass: prefabs first, then plain `.ron`, then
/// `.json`, `.png`, and finally `.bin`, so a boundary is the exclusive end of
/// the run before it.
struct ExtensionPartition {
    prefab_count: usize,
    ron_count: usize,
    json_end: usize,
    png_end: usize,
}

/// How many bytes each non-prefab source gets.
///
/// The measured total is divided evenly across them; the first
/// `other_remainder` sources take one extra byte so the tree hits the measured
/// total exactly.
struct ByteBudget {
    other_base: usize,
    other_remainder: usize,
}

/// Splits `source_count` sources into the per-extension runs `shape` calls for.
fn synthetic_extension_partition(
    shape: CorpusShape,
    source_count: usize,
) -> Result<ExtensionPartition, SyntheticCorpusError> {
    let prefab_count = scaled_count(source_count, shape.prefab_per_ten_thousand)?;
    let ron_count = scaled_count(source_count, shape.ron_per_ten_thousand)?;
    let plain_ron_count = ron_count.saturating_sub(prefab_count);
    let json_end = prefab_count
        .checked_add(plain_ron_count)
        .and_then(|value| value.checked_add((source_count - ron_count) / 3))
        .ok_or(SyntheticCorpusError::CountOverflow {
            field: "synthetic extension partition",
            value: source_count,
        })?;
    let png_end = json_end.checked_add((source_count - ron_count) / 3).ok_or(
        SyntheticCorpusError::CountOverflow {
            field: "synthetic extension partition",
            value: source_count,
        },
    )?;
    Ok(ExtensionPartition {
        prefab_count,
        ron_count,
        json_end,
        png_end,
    })
}

/// Spreads the shape's measured byte total across the non-prefab sources.
///
/// Prefab and sidecar bytes are fixed per file, so they come off the total
/// first; whatever is left is what the remaining sources have to divide.
fn synthetic_byte_budget(
    shape: CorpusShape,
    source_count: usize,
    prefab_count: usize,
) -> Result<ByteBudget, SyntheticCorpusError> {
    let sidecar_bytes = synthetic_sidecar_byte_length()?;
    let sidecar_total = sidecar_bytes
        .checked_mul(u64::try_from(shape.sidecar_count).map_err(|_| {
            SyntheticCorpusError::CountOverflow {
                field: "sidecar count",
                value: shape.sidecar_count,
            }
        })?)
        .ok_or(SyntheticCorpusError::CountOverflow {
            field: "sidecar bytes",
            value: shape.sidecar_count,
        })?;
    let prefab_byte_length =
        u64::try_from(shape.prefab_bytes).map_err(|_| SyntheticCorpusError::CountOverflow {
            field: "prefab byte length",
            value: shape.prefab_bytes,
        })?;
    let prefab_total = u64::try_from(prefab_count)
        .ok()
        .and_then(|count| count.checked_mul(prefab_byte_length))
        .ok_or(SyntheticCorpusError::CountOverflow {
            field: "prefab bytes",
            value: prefab_count,
        })?;
    let other_count = source_count - prefab_count;
    let other_total = shape
        .total_bytes
        .checked_sub(sidecar_total)
        .and_then(|bytes| bytes.checked_sub(prefab_total))
        .ok_or(SyntheticCorpusError::InvalidShape(
            "measured total bytes do not cover prefab and sidecar bytes",
        ))?;
    let other_count_u64 =
        u64::try_from(other_count).map_err(|_| SyntheticCorpusError::CountOverflow {
            field: "other source count",
            value: other_count,
        })?;
    let other_base = usize::try_from(other_total / other_count_u64).map_err(|_| {
        SyntheticCorpusError::CountOverflow {
            field: "other source byte length",
            value: other_count,
        }
    })?;
    let other_remainder = usize::try_from(other_total % other_count_u64).map_err(|_| {
        SyntheticCorpusError::CountOverflow {
            field: "other source byte remainder",
            value: other_count,
        }
    })?;
    Ok(ByteBudget {
        other_base,
        other_remainder,
    })
}

pub struct CorpusTree;

impl CorpusTree {
    /// Write a shaped tree and prepare its file-backed AP owner without
    /// sweeping. Criterion setup calls this outside its timed closure.
    ///
    /// # Errors
    ///
    /// Returns [`SyntheticCorpusError::InvalidShape`] if the scale's shape is inconsistent,
    /// [`SyntheticCorpusError::CountOverflow`] if a generated count exceeds the supported integer
    /// range, [`SyntheticCorpusError::UnsafeSourcePath`] or [`SyntheticCorpusError::DuplicateSourcePath`] for a
    /// generated path that is not safe-relative or repeats another, and
    /// [`SyntheticCorpusError::Io`] or [`SyntheticCorpusError::Json`] if the tree or a sidecar cannot be written.
    pub fn write(
        workspace_root: impl AsRef<Path>,
        project_id: impl Into<String>,
        scale: CorpusScale,
    ) -> Result<PreparedCorpus, SyntheticCorpusError> {
        let shape = scale.shape();
        let source_count = shape
            .regular_file_count
            .checked_sub(shape.sidecar_count)
            .ok_or(SyntheticCorpusError::InvalidShape(
                "sidecars exceed regular files",
            ))?;
        if source_count < shape.directory_count || shape.max_depth < 2 {
            return Err(SyntheticCorpusError::InvalidShape(
                "every shaped directory needs one source and max depth must include a directory",
            ));
        }
        let directories = synthetic_directories(shape);
        let ExtensionPartition {
            prefab_count,
            ron_count,
            json_end,
            png_end,
        } = synthetic_extension_partition(shape, source_count)?;
        let ByteBudget {
            other_base,
            other_remainder,
        } = synthetic_byte_budget(shape, source_count, prefab_count)?;
        let path_targets = synthetic_path_targets(shape, source_count)?;
        let workspace_root = normalize(workspace_root.as_ref());
        let database_path = workspace_root.join(".azoth-corpus/assetdb.sqlite");
        let sources = (0..source_count).map(|index| {
            let (suffix, byte_length) = if index < prefab_count {
                (".prefab.ron", shape.prefab_bytes)
            } else if index < ron_count {
                (
                    ".ron",
                    other_base + usize::from(index - prefab_count < other_remainder),
                )
            } else if index < json_end {
                (
                    ".json",
                    other_base + usize::from(index - prefab_count < other_remainder),
                )
            } else if index < png_end {
                (
                    ".png",
                    other_base + usize::from(index - prefab_count < other_remainder),
                )
            } else {
                (
                    ".bin",
                    other_base + usize::from(index - prefab_count < other_remainder),
                )
            };
            let directory = &directories[index % directories.len()];
            let source_path = synthetic_source_path(directory, index, suffix, path_targets[index]);
            let mut source = SyntheticSourceSpec::new(source_path.clone(), vec![b's'; byte_length]);
            if index > 0 && index <= shape.sidecar_count {
                source = source.with_guid(resolve_source_asset_guid(&source_path, None));
            }
            source
        });
        let prepared = PreparedProcessor::prepare(
            &workspace_root,
            project_id,
            sources,
            SyntheticStorage::File(database_path.clone()),
        )?;
        Ok(PreparedCorpus {
            scale,
            shape,
            prepared,
            database_path,
        })
    }
}

/// End-to-end convenience for release-style runs. Benches should use
/// [`CorpusTree::write`] and time [`PreparedCorpus::sweep`] directly.
pub struct SyntheticCorpus {
    pub prepared: PreparedCorpus,
    pub reconcile: ReconcileAssetSourcesResult,
}

impl SyntheticCorpus {
    /// # Errors
    ///
    /// Returns any error [`CorpusTree::write`] returns while laying down the tree,
    /// then any error the corpus sweep returns.
    pub fn generate(
        workspace_root: impl AsRef<Path>,
        project_id: impl Into<String>,
        scale: CorpusScale,
    ) -> Result<Self, SyntheticCorpusError> {
        let prepared = CorpusTree::write(workspace_root, project_id, scale)?;
        let reconcile = prepared.sweep()?;
        Ok(Self {
            prepared,
            reconcile,
        })
    }
}

fn synthetic_sidecar_byte_length() -> Result<u64, SyntheticCorpusError> {
    let bytes = serde_json::to_vec(&SourceAssetMeta::preserving_source_guid(Uuid::nil()))?;
    u64::try_from(bytes.len()).map_err(|_| SyntheticCorpusError::CountOverflow {
        field: "sidecar byte length",
        value: bytes.len(),
    })
}

fn synthetic_path_targets(
    shape: CorpusShape,
    source_count: usize,
) -> Result<Vec<usize>, SyntheticCorpusError> {
    let aggregate = shape
        .regular_file_count
        .checked_mul(shape.mean_path_hundredths)
        .and_then(|value| value.checked_add(50))
        .map(|value| value / 100)
        .ok_or(SyntheticCorpusError::CountOverflow {
            field: "aggregate source path bytes",
            value: shape.regular_file_count,
        })?;
    let sidecar_suffix_bytes = shape
        .sidecar_count
        .checked_mul(crate::SOURCE_META_SIDECAR_SUFFIX.len())
        .ok_or(SyntheticCorpusError::CountOverflow {
            field: "aggregate sidecar path bytes",
            value: shape.sidecar_count,
        })?;
    let source_aggregate =
        aggregate
            .checked_sub(sidecar_suffix_bytes)
            .ok_or(SyntheticCorpusError::InvalidShape(
                "sidecar suffixes exceed the aggregate path budget",
            ))?;
    let remaining_bytes = source_aggregate.checked_sub(shape.max_path_bytes).ok_or(
        SyntheticCorpusError::InvalidShape("maximum path exceeds the aggregate path budget"),
    )?;
    let weights = (1..source_count)
        .map(|index| 1 + usize::from(index <= shape.sidecar_count))
        .collect::<Vec<_>>();
    let total_weight = weights.iter().sum::<usize>();
    let base = remaining_bytes / total_weight;
    let mut remainder = remaining_bytes % total_weight;
    let mut targets = Vec::with_capacity(source_count);
    targets.push(shape.max_path_bytes);
    for weight in weights {
        let extra = usize::from(remainder >= weight);
        remainder = remainder.saturating_sub(weight * extra);
        targets.push(base + extra);
    }
    debug_assert_eq!(remainder, 0);
    Ok(targets)
}

fn scaled_count(total: usize, per_ten_thousand: usize) -> Result<usize, SyntheticCorpusError> {
    total
        .checked_mul(per_ten_thousand)
        .and_then(|value| value.checked_add(5_000))
        .map(|value| value / 10_000)
        .ok_or(SyntheticCorpusError::CountOverflow {
            field: "synthetic distribution",
            value: total,
        })
}

fn synthetic_directories(shape: CorpusShape) -> Vec<String> {
    let chain_depth = shape.max_depth - 1;
    let mut directories = Vec::with_capacity(shape.directory_count);
    let mut chain = String::new();
    for depth in 0..chain_depth.min(shape.directory_count) {
        if !chain.is_empty() {
            chain.push('/');
        }
        chain.push(char::from(b'a' + u8::try_from(depth % 26).unwrap_or(0)));
        directories.push(chain.clone());
    }
    while directories.len() < shape.directory_count {
        directories.push(format!("bucket-{:05}", directories.len()));
    }
    directories
}

fn synthetic_source_path(directory: &str, index: usize, suffix: &str, target_len: usize) -> String {
    let stem = format!("source-{index:07}");
    let fixed_len = directory.len() + 1 + stem.len() + suffix.len();
    let padding = target_len.saturating_sub(fixed_len);
    format!("{directory}/{stem}{}{suffix}", "x".repeat(padding))
}

#[derive(Debug, Error)]
pub enum SyntheticCorpusError {
    #[error("synthetic corpus database open failed: {0}")]
    Open(#[from] az_assetdb::OpenError),
    #[error("synthetic corpus database operation failed: {0}")]
    Repo(#[from] RepoError),
    // Boxed: inlining `AssetProcessorError` here made every
    // `Result<_, SyntheticCorpusError>` oversized. The hand-written `From`
    // below keeps `?` working, since `#[from]` on a boxed field would only
    // yield `From<Box<..>>`.
    #[error("synthetic corpus processor operation failed: {0}")]
    Processor(#[source] Box<AssetProcessorError>),
    #[error("synthetic corpus filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("synthetic corpus database path already exists: {}", path.display())]
    ExistingDatabase { path: PathBuf },
    #[error("synthetic corpus sidecar serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("synthetic corpus source path `{0}` is not safe relative")]
    UnsafeSourcePath(String),
    #[error("synthetic corpus contains duplicate canonical source path `{0}`")]
    DuplicateSourcePath(String),
    #[error("synthetic corpus seed `{0}` was not projected")]
    MissingProjection(String),
    #[error("synthetic corpus projected unknown seed `{0}`")]
    MissingSeed(String),
    #[error("synthetic corpus processor did not project its attached workspace")]
    MissingWorkspaceSnapshot,
    #[error("synthetic corpus {field} count {value} exceeds the supported integer range")]
    CountOverflow { field: &'static str, value: usize },
    #[error("synthetic corpus shape is invalid: {0}")]
    InvalidShape(&'static str),
}

/// Boxes the processor error on the way in, so `?` still converts an
/// [`AssetProcessorError`] into the corpus error without widening it.
impl From<AssetProcessorError> for SyntheticCorpusError {
    fn from(error: AssetProcessorError) -> Self {
        Self::Processor(Box::new(error))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[derive(Default)]
    struct TreeFacts {
        files: usize,
        directories: usize,
        sidecars: usize,
        prefab_sources: usize,
        ron_sources: usize,
        total_bytes: u64,
        total_path_bytes: usize,
        max_path_bytes: usize,
        max_depth: usize,
    }

    #[test]
    fn release_shape_preserves_ticket_012_measurements() {
        let shape = CorpusScale::Release.shape();
        assert_eq!(shape.regular_file_count, 1_151_840);
        assert_eq!(shape.total_bytes, 153 * GIB);
        assert_eq!(shape.directory_count, 33_018);
        assert_eq!(shape.max_depth, 12);
        assert_eq!(shape.mean_path_hundredths, 9_097);
        assert_eq!(shape.max_path_bytes, 165);
        assert_eq!(shape.prefab_per_ten_thousand, 2_220);
        assert_eq!(shape.ron_per_ten_thousand, 4_320);
        assert_eq!(shape.sidecar_count, 55);
        assert_eq!(shape.prefab_bytes, 37_683);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn small_corpus_tree_matches_declared_shape_and_sweeps_through_shipping_path() {
        let temp = tempfile::tempdir().unwrap();
        let prepared = CorpusTree::write(temp.path(), "synthetic-shape", CorpusScale::Small)
            .expect("write prepared corpus");
        let shape = prepared.shape;
        let facts = tree_facts(prepared.source_root());

        assert_eq!(facts.files, shape.regular_file_count);
        assert_eq!(facts.directories, shape.directory_count);
        assert_eq!(facts.sidecars, shape.sidecar_count);
        assert_eq!(facts.total_bytes, shape.total_bytes);
        assert_eq!(facts.max_path_bytes, shape.max_path_bytes);
        assert_eq!(facts.max_depth, shape.max_depth);
        let mean_hundredths = (facts.total_path_bytes * 100 + facts.files / 2) / facts.files;
        assert_eq!(mean_hundredths, shape.mean_path_hundredths);

        let source_count = facts.files - facts.sidecars;
        assert_eq!(
            facts.prefab_sources,
            scaled_count(source_count, shape.prefab_per_ten_thousand).unwrap()
        );
        assert_eq!(
            facts.ron_sources,
            scaled_count(source_count, shape.ron_per_ten_thousand).unwrap()
        );
        assert!(prepared.database_path().is_file());

        let reconcile = prepared.sweep().expect("sweep prepared corpus");
        assert_eq!(reconcile.recorded_source_asset_count as usize, source_count);
        assert_eq!(prepared.sources().unwrap().len(), source_count);
        let status = prepared
            .prepared
            .rpc
            .processor()
            .processing_status(&az_proto_asset::AssetProcessingStatusRequest {
                capability: prepared.write_capability(),
                session_id: SYNTHETIC_SESSION_ID.to_string(),
                platform: crate::DEFAULT_PLATFORM.to_string(),
            })
            .expect("synthetic planning status");
        assert!(
            status.queued > 0,
            "synthetic corpus must produce ready work"
        );
    }

    fn tree_facts(root: &Path) -> TreeFacts {
        let mut facts = TreeFacts::default();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap();
                let depth = relative.components().count();
                facts.max_depth = facts.max_depth.max(depth);
                if entry.file_type().unwrap().is_dir() {
                    facts.directories += 1;
                    pending.push(path);
                    continue;
                }
                facts.files += 1;
                facts.total_bytes += entry.metadata().unwrap().len();
                let relative = relative.to_string_lossy().replace('\\', "/");
                facts.total_path_bytes += relative.len();
                facts.max_path_bytes = facts.max_path_bytes.max(relative.len());
                if relative.ends_with(crate::SOURCE_META_SIDECAR_SUFFIX) {
                    facts.sidecars += 1;
                } else {
                    let extension = Path::new(&relative)
                        .extension()
                        .and_then(std::ffi::OsStr::to_str);
                    facts.prefab_sources += usize::from(relative.ends_with(".prefab.ron"));
                    facts.ron_sources += usize::from(extension == Some("ron"));
                }
            }
        }
        facts
    }
}
