//! Cap'n Proto protocol types for the asset-processor service.

// Machine-generated Cap'n Proto output: written into OUT_DIR by build.rs and
// regenerated on every build, so it is not in git and has no per-site fix. The
// macro completes upstream's own `#![allow(clippy::all)]` for the pedantic and
// nursery groups this workspace denies.
az_proto_core::generated_schema!(pub mod asset_capnp, "azoth/asset_capnp.rs");

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    path::{Component, Path},
};

use az_proto_core::{self as core, Capability, SideChannelHandle, StagingFileSideChannelError};
use capnp::{Error, message, serialize_packed};
use thiserror::Error as ThisError;
use uuid::Uuid;

pub use az_proto_authoring::{
    ReflectedValueEncoding, ReflectedValueEnvelope, SourceFileCodecOperation,
    SourceFileEditDocument, SourceFileEditObject, SourceFileEditOperation,
};
pub use az_proto_core as core_proto;

pub const ASSET_PROCESSOR_NAMESPACE: &str = "azoth";
pub const ASSET_PROCESSOR_SERVICE_NAME: &str = "asset-processor";
pub const ASSET_WORKER_SERVICE_NAMESPACE: &str = "azoth";
pub const ASSET_WORKER_SERVICE_NAME: &str = "asset-worker";
pub const ASSET_PROCESSOR_AUDIENCE: &str = "asset-processor";
pub const ASSET_JOBS_PERMISSION: &str = "asset.jobs";
pub const ASSET_READ_PERMISSION: &str = "asset.read";
pub const ASSET_WRITE_PERMISSION: &str = "asset.write";
/// Maximum duration admitted for one renewable asset-job grant.
///
/// Workers renew long-running jobs in process. A larger initial duration would
/// only delay recovery after a lost worker and make unchecked `Instant`
/// arithmetic part of the protocol boundary.
pub const MAX_ASSET_JOB_LEASE_DURATION_MS: u64 = 5 * 60 * 1_000;
/// Asset-service product manifest schema version.
///
/// Pinned to 1 pre-release. The reader rejects a mismatched manifest outright
/// and keeps no migration path, so an incompatible shape change is answered by
/// changing the shape and reprocessing, not by incrementing this gate.
pub const PRODUCT_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const RELEASE_ASSET_CATALOG_FILE_NAME: &str = "assetcatalog.bin";
pub const PROJECT_SOURCE_ROOT: &str = "project:source-root";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum JobStatus {
    Queued = 0,
    Leased = 1,
    Succeeded = 2,
    Failed = 3,
}

impl JobStatus {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }

    #[must_use]
    pub const fn to_capnp(self) -> asset_capnp::JobStatus {
        match self {
            Self::Queued => asset_capnp::JobStatus::Queued,
            Self::Leased => asset_capnp::JobStatus::Leased,
            Self::Succeeded => asset_capnp::JobStatus::Succeeded,
            Self::Failed => asset_capnp::JobStatus::Failed,
        }
    }

    #[must_use]
    pub const fn from_capnp(status: asset_capnp::JobStatus) -> Self {
        match status {
            asset_capnp::JobStatus::Queued => Self::Queued,
            asset_capnp::JobStatus::Leased => Self::Leased,
            asset_capnp::JobStatus::Succeeded => Self::Succeeded,
            asset_capnp::JobStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum AttemptStatus {
    Leased = 1,
    Succeeded = 2,
    Failed = 3,
    Abandoned = 4,
}

impl AttemptStatus {
    #[must_use]
    pub const fn to_capnp(self) -> asset_capnp::AttemptStatus {
        match self {
            Self::Leased => asset_capnp::AttemptStatus::Leased,
            Self::Succeeded => asset_capnp::AttemptStatus::Succeeded,
            Self::Failed => asset_capnp::AttemptStatus::Failed,
            Self::Abandoned => asset_capnp::AttemptStatus::Abandoned,
        }
    }

    #[must_use]
    pub const fn from_capnp(status: asset_capnp::AttemptStatus) -> Self {
        match status {
            asset_capnp::AttemptStatus::Leased => Self::Leased,
            asset_capnp::AttemptStatus::Succeeded => Self::Succeeded,
            asset_capnp::AttemptStatus::Failed => Self::Failed,
            asset_capnp::AttemptStatus::Abandoned => Self::Abandoned,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum JobDependencyKind {
    Order = 0,
    Fingerprint = 1,
    OrderOnly = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum SourceRelation {
    SourceToSource = 0,
    JobToJob = 1,
    SourceLikeMatch = 2,
}

impl SourceRelation {
    #[must_use]
    pub const fn to_capnp(self) -> asset_capnp::SourceRelation {
        match self {
            Self::SourceToSource => asset_capnp::SourceRelation::SourceToSource,
            Self::JobToJob => asset_capnp::SourceRelation::JobToJob,
            Self::SourceLikeMatch => asset_capnp::SourceRelation::SourceLikeMatch,
        }
    }

    #[must_use]
    pub const fn from_capnp(relation: asset_capnp::SourceRelation) -> Self {
        match relation {
            asset_capnp::SourceRelation::SourceToSource => Self::SourceToSource,
            asset_capnp::SourceRelation::JobToJob => Self::JobToJob,
            asset_capnp::SourceRelation::SourceLikeMatch => Self::SourceLikeMatch,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i64)]
pub enum CatalogPathRegistration {
    #[default]
    Registered = 0,
    AssetIdOnly = 1,
}

impl CatalogPathRegistration {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }

    #[must_use]
    pub const fn to_capnp(self) -> asset_capnp::CatalogPathRegistration {
        match self {
            Self::Registered => asset_capnp::CatalogPathRegistration::Registered,
            Self::AssetIdOnly => asset_capnp::CatalogPathRegistration::AssetIdOnly,
        }
    }

    #[must_use]
    pub const fn from_capnp(value: asset_capnp::CatalogPathRegistration) -> Self {
        match value {
            asset_capnp::CatalogPathRegistration::Registered => Self::Registered,
            asset_capnp::CatalogPathRegistration::AssetIdOnly => Self::AssetIdOnly,
        }
    }
}

impl JobDependencyKind {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }

    #[must_use]
    pub const fn to_capnp(self) -> asset_capnp::JobDependencyKind {
        match self {
            Self::Order => asset_capnp::JobDependencyKind::Order,
            Self::Fingerprint => asset_capnp::JobDependencyKind::Fingerprint,
            Self::OrderOnly => asset_capnp::JobDependencyKind::OrderOnly,
        }
    }

    #[must_use]
    pub const fn from_capnp(kind: asset_capnp::JobDependencyKind) -> Self {
        match kind {
            asset_capnp::JobDependencyKind::Order => Self::Order,
            asset_capnp::JobDependencyKind::Fingerprint => Self::Fingerprint,
            asset_capnp::JobDependencyKind::OrderOnly => Self::OrderOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum WorkspaceEntryDiff {
    Clean = 0,
    Added = 1,
    Modified = 2,
    Deleted = 3,
    Conflicted = 4,
}

impl WorkspaceEntryDiff {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }

    #[must_use]
    pub const fn to_capnp(self) -> asset_capnp::WorkspaceEntryDiff {
        match self {
            Self::Clean => asset_capnp::WorkspaceEntryDiff::Clean,
            Self::Added => asset_capnp::WorkspaceEntryDiff::Added,
            Self::Modified => asset_capnp::WorkspaceEntryDiff::Modified,
            Self::Deleted => asset_capnp::WorkspaceEntryDiff::Deleted,
            Self::Conflicted => asset_capnp::WorkspaceEntryDiff::Conflicted,
        }
    }

    #[must_use]
    pub const fn from_capnp(diff: asset_capnp::WorkspaceEntryDiff) -> Self {
        match diff {
            asset_capnp::WorkspaceEntryDiff::Clean => Self::Clean,
            asset_capnp::WorkspaceEntryDiff::Added => Self::Added,
            asset_capnp::WorkspaceEntryDiff::Modified => Self::Modified,
            asset_capnp::WorkspaceEntryDiff::Deleted => Self::Deleted,
            asset_capnp::WorkspaceEntryDiff::Conflicted => Self::Conflicted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i64)]
pub enum AssetRootScope {
    #[default]
    BrowserAssets = 0,
    All = 1,
}

impl AssetRootScope {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self as i64
    }

    #[must_use]
    pub const fn to_capnp(self) -> asset_capnp::AssetRootScope {
        match self {
            Self::BrowserAssets => asset_capnp::AssetRootScope::BrowserAssets,
            Self::All => asset_capnp::AssetRootScope::All,
        }
    }

    #[must_use]
    pub const fn from_capnp(scope: asset_capnp::AssetRootScope) -> Self {
        match scope {
            asset_capnp::AssetRootScope::BrowserAssets => Self::BrowserAssets,
            asset_capnp::AssetRootScope::All => Self::All,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum AssetProcessorEventKind {
    SourceRecorded = 0,
    SourceCreated = 1,
    JobCompleted = 2,
    SourceDeleted = 3,
    SourceMoved = 4,
    SourceReprocessed = 5,
    SourceReconciled = 6,
}

impl AssetProcessorEventKind {
    #[must_use]
    pub const fn to_capnp(self) -> asset_capnp::AssetProcessorEventKind {
        match self {
            Self::SourceRecorded => asset_capnp::AssetProcessorEventKind::SourceRecorded,
            Self::SourceCreated => asset_capnp::AssetProcessorEventKind::SourceCreated,
            Self::JobCompleted => asset_capnp::AssetProcessorEventKind::JobCompleted,
            Self::SourceDeleted => asset_capnp::AssetProcessorEventKind::SourceDeleted,
            Self::SourceMoved => asset_capnp::AssetProcessorEventKind::SourceMoved,
            Self::SourceReprocessed => asset_capnp::AssetProcessorEventKind::SourceReprocessed,
            Self::SourceReconciled => asset_capnp::AssetProcessorEventKind::SourceReconciled,
        }
    }

    #[must_use]
    pub const fn from_capnp(kind: asset_capnp::AssetProcessorEventKind) -> Self {
        match kind {
            asset_capnp::AssetProcessorEventKind::SourceRecorded => Self::SourceRecorded,
            asset_capnp::AssetProcessorEventKind::SourceCreated => Self::SourceCreated,
            asset_capnp::AssetProcessorEventKind::JobCompleted => Self::JobCompleted,
            asset_capnp::AssetProcessorEventKind::SourceDeleted => Self::SourceDeleted,
            asset_capnp::AssetProcessorEventKind::SourceMoved => Self::SourceMoved,
            asset_capnp::AssetProcessorEventKind::SourceReprocessed => Self::SourceReprocessed,
            asset_capnp::AssetProcessorEventKind::SourceReconciled => Self::SourceReconciled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetBuilderPatternKind {
    Wildcard,
    Regex,
}

impl AssetBuilderPatternKind {
    #[must_use]
    pub const fn to_capnp(self) -> asset_capnp::AssetBuilderPatternKind {
        match self {
            Self::Wildcard => asset_capnp::AssetBuilderPatternKind::Wildcard,
            Self::Regex => asset_capnp::AssetBuilderPatternKind::Regex,
        }
    }

    #[must_use]
    pub const fn from_capnp(kind: asset_capnp::AssetBuilderPatternKind) -> Self {
        match kind {
            asset_capnp::AssetBuilderPatternKind::Wildcard => Self::Wildcard,
            asset_capnp::AssetBuilderPatternKind::Regex => Self::Regex,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectJobSelector {
    Job(i64),
    Attempt(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedAssetJob {
    pub attempt_id: i64,
    pub workspace_id: i64,
    pub owner: JobOwner,
    pub source_guid: Uuid,
    pub preserved_source_sub_id: Option<u32>,
    pub source_path: String,
    pub source_root: String,
    pub source_schema_type: Option<String>,
    pub job_key: String,
    pub platform: String,
    pub ordinal: i64,
    pub staging_root: String,
    pub source_payload: Option<SideChannelHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectJobRequest {
    pub capability: Capability,
    pub selector: InspectJobSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOwner {
    Plan,
    Build(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    pub job_id: i64,
    pub workspace_id: i64,
    pub source_guid: Uuid,
    pub source_path: String,
    pub source_root: String,
    pub source_schema_type: Option<String>,
    pub owner: JobOwner,
    pub key: String,
    pub platform: String,
    pub status: JobStatus,
    pub ready: bool,
    pub attempts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobAttemptRecord {
    pub attempt_id: i64,
    pub job_id: i64,
    pub ordinal: i64,
    pub status: AttemptStatus,
    pub owner: Option<String>,
    pub staging: Option<String>,
    pub finished_unix_ms: Option<i64>,
    pub error_count: i64,
    pub warning_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobActivity {
    pub job: JobRecord,
    pub attempt: Option<JobAttemptRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobProductEdgeRecord {
    pub product_edge_id: i64,
    pub product_id: i64,
    pub asset_guid: Uuid,
    pub sub_id: i64,
    pub flags: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobProductRecord {
    pub product_id: i64,
    pub job_id: i64,
    pub path: String,
    pub asset_type: Uuid,
    pub sub_id: i64,
    pub product_format: String,
    pub product_format_version: u32,
    pub catalog_path_registration: CatalogPathRegistration,
    pub content_hash: String,
    pub byte_length: i64,
    pub aliases: Vec<String>,
    pub edges: Vec<JobProductEdgeRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobDependencyTarget {
    Guid(Uuid),
    Path(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDependencyRecord {
    pub job_edge_id: i64,
    pub job_id: i64,
    pub target: JobDependencyTarget,
    pub key: String,
    pub platform: String,
    pub kind: JobDependencyKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobInspection {
    pub job: JobRecord,
    pub attempt: Option<JobAttemptRecord>,
    pub products: Vec<JobProductRecord>,
    pub dependencies: Vec<JobDependencyRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectJobResult {
    pub inspection: Option<JobInspection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBuilderPatternDescriptor {
    pub kind: AssetBuilderPatternKind,
    pub pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBuilderDescriptor {
    pub name: String,
    pub builder_guid: Uuid,
    pub version: u32,
    pub analysis_fingerprint: String,
    pub patterns: Vec<AssetBuilderPatternDescriptor>,
    pub source_schema_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSchemaDescriptor {
    pub schema_type: String,
    pub owner: String,
    pub label: String,
    pub category: String,
    pub authoring: SourceSchemaAuthoring,
    pub file_templates: Vec<SourceFileTemplateDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileTemplateDescriptor {
    pub owner: String,
    pub source_path: String,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileWorkflowDescriptor {
    pub source_root: String,
    pub default_path_prefix: String,
    pub extensions: Vec<String>,
    pub can_create: bool,
    pub can_edit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSchemaAuthoring {
    File {
        workflow: SourceFileWorkflowDescriptor,
    },
    ProjectDocument {
        schema_type: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBuilderCatalogRequest {
    pub capability: Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductFormatDescriptor {
    pub id: String,
    pub current_version: u32,
    pub owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBuilderCatalogResult {
    pub builders: Vec<AssetBuilderDescriptor>,
    pub source_schemas: Vec<SourceSchemaDescriptor>,
    pub product_formats: Vec<ProductFormatDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub entry_id: i64,
    pub workspace_id: i64,
    pub asset_guid: Uuid,
    pub root_id: i64,
    pub source_path: String,
    pub schema_type: Option<String>,
    pub content_hash: String,
    pub diff: WorkspaceEntryDiff,
    pub diagnostics_count: i64,
    pub updated_unix_ms: i64,
    pub jobs: Vec<JobActivity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntryPageRequest {
    pub capability: Capability,
    pub root_scope: AssetRootScope,
    pub after_entry_id: Option<i64>,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntryPageResult {
    pub entries: Vec<WorkspaceEntry>,
    pub next_after_entry_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetProcessorEvent {
    pub seq: u64,
    pub kind: AssetProcessorEventKind,
    pub event_unix_ms: i64,
    pub entry: WorkspaceEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetProcessorEventSubscriptionRequest {
    pub capability: Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetProcessorEventSubscriptionResult {
    pub subscribed: bool,
    pub initial_health: core::ServiceHealth,
}

// The four bools mirror four separately-numbered `Bool` fields in
// asset.capnp (`isRoot @8`, `recursive @11`, `watch @12`, `writable @13`);
// collapsing them into a flags type would break the field-for-field
// correspondence the write/read pair is audited against.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot {
    pub workspace_root_id: i64,
    pub workspace_id: i64,
    pub root_id: i64,
    pub declared_root_id: String,
    pub owner_id: String,
    pub source_root: String,
    pub display_name: String,
    pub portable_key: String,
    pub mount: String,
    pub recursive: bool,
    pub watch: bool,
    pub writable: bool,
    pub output_prefix: String,
    pub is_root: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub workspace_id: i64,
    pub project_id: String,
    pub workspace_root: String,
    pub branch: String,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
    pub roots: Vec<WorkspaceRoot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSnapshotRequest {
    pub capability: Capability,
    pub root_scope: AssetRootScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSnapshotResult {
    pub snapshot: Option<WorkspaceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProductEntry {
    pub job_id: i64,
    pub product_id: i64,
    pub asset_guid: Uuid,
    pub source_path: String,
    pub builder_guid: Uuid,
    pub job_key: String,
    pub platform: String,
    pub product_path: String,
    pub asset_type: Uuid,
    pub sub_id: i64,
    pub product_format: String,
    pub product_format_version: u32,
    pub content_hash: String,
    /// Additional catalog lookup paths for this product.
    pub catalog_aliases: Vec<String>,
    pub catalog_path_registration: CatalogPathRegistration,
    pub byte_length: i64,
    /// Runtime product dependencies for this product, in deterministic order.
    pub dependencies: Vec<CatalogProductDependency>,
}

/// A runtime product dependency edge carried with a current product.
///
/// `asset_guid`/`sub_id` are the target product identity; `asset_type` is the
/// target product's native asset type resolved from the same view/platform
/// product set (nil when the target is not present in that projection). `hint`
/// is a diagnostic `@assets@/...` address only, never a resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProductDependency {
    pub asset_guid: Uuid,
    pub sub_id: i64,
    pub asset_type: Option<Uuid>,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProductsRequest {
    pub capability: Capability,
    pub platform: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProductsResult {
    pub entries: Vec<CatalogProductEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseContentTarget {
    AssetCatalog,
    ProductAsset { asset_guid: Uuid, sub_id: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseContentReadRequest {
    pub capability: Capability,
    pub platform: String,
    pub target: ReleaseContentTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseContentProduct {
    pub asset_guid: Uuid,
    pub sub_id: u32,
    pub product_path: String,
    pub product_format: String,
    pub product_format_version: u32,
    pub byte_length: u64,
    pub content_hash: Vec<u8>,
    pub payload: SideChannelHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseContentReadResult {
    None,
    AssetCatalog(SideChannelHandle),
    Product(ReleaseContentProduct),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAssetRecordRequest {
    pub capability: Capability,
    pub session_id: String,
    pub workspace_root_id: i64,
    pub owner_id: String,
    pub source_path: String,
    pub schema_type: Option<String>,
    pub content_hash: Vec<u8>,
    pub changed_unix_ms: i64,
    pub diagnostics_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAssetRecordResult {
    pub asset_guid: Uuid,
    pub entry: WorkspaceEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFileCreateContent {
    DefaultTemplate,
    /// Boxed so the handle's size does not dominate every
    /// `SourceFileCreateContent`; the capnp encoding is unchanged.
    Payload(Box<SideChannelHandle>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileCreateRequest {
    pub capability: Capability,
    pub session_id: String,
    pub source_root: String,
    pub source_path: String,
    pub schema_type: String,
    pub changed_unix_ms: i64,
    pub content: SourceFileCreateContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileCreateResult {
    pub record: SourceAssetRecordResult,
}

/// Canonical workspace-file identity for structured source editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSourceFileRef {
    pub source_root_key: String,
    pub source_path: String,
    pub schema_type: String,
}

/// Optimistic workspace-file edit transaction.
///
/// The Asset Processor validates `expected_source_fingerprint` against the
/// authoritative file. Project-host retains its own source-session revision
/// and advances it only after this transaction succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileEditRequest {
    pub capability: Capability,
    pub session_id: String,
    pub source: WorkspaceSourceFileRef,
    pub expected_source_fingerprint: Vec<u8>,
    pub operation: SourceFileEditOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileOpenRequest {
    pub capability: Capability,
    pub session_id: String,
    pub source: WorkspaceSourceFileRef,
}

/// Trusted project-host replay of a previously returned canonical document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileRestoreRequest {
    pub capability: Capability,
    pub session_id: String,
    pub source: WorkspaceSourceFileRef,
    pub expected_source_fingerprint: Vec<u8>,
    pub document: SourceFileEditDocument,
}

/// Asset Processor request to execute a project-linked edit codec in the
/// active asset worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileCodecRequest {
    pub source: WorkspaceSourceFileRef,
    pub authoritative_source: SideChannelHandle,
    pub operation: SourceFileCodecOperation,
    pub output_destination: SourceFileCodecOutputDestination,
}

/// Asset Processor-owned destination for a worker-produced source file.
///
/// The destination deliberately omits content metadata: the worker supplies
/// the verified length and hash only after atomically writing this exact path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileCodecOutputDestination {
    pub locator: String,
    pub platform: String,
    pub capability: Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFileCodecReplacement {
    Unchanged,
    /// Boxed so the handle's size does not dominate every
    /// `SourceFileCodecReplacement`; the capnp encoding is unchanged.
    SavedSource(Box<SideChannelHandle>),
}

/// Worker-produced source bytes and the document reloaded from those bytes.
///
/// A saved-source replacement is internal to the processor/worker transaction.
/// Public open/edit results expose only the canonical document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileCodecResult {
    pub document: SourceFileEditDocument,
    pub replacement: SourceFileCodecReplacement,
}

/// Canonical post-transaction view reloaded from the authoritative file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileEditSnapshot {
    pub source: WorkspaceSourceFileRef,
    pub source_fingerprint: Vec<u8>,
    pub document: SourceFileEditDocument,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileEditResult {
    pub snapshot: SourceFileEditSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileOpenResult {
    pub snapshot: SourceFileEditSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileRestoreResult {
    pub snapshot: SourceFileEditSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileDeleteRequest {
    pub capability: Capability,
    pub session_id: String,
    pub source_root: String,
    pub source_path: String,
    pub changed_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileDeleteResult {
    pub record: SourceAssetRecordResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileMoveRequest {
    pub capability: Capability,
    pub session_id: String,
    pub source_root: String,
    pub from_source_path: String,
    pub to_source_path: String,
    pub changed_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileMoveResult {
    pub record: SourceAssetRecordResult,
    pub old_source_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForceReprocessAssetRequest {
    pub capability: Capability,
    pub session_id: String,
    pub source_root: String,
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForceReprocessAssetResult {
    pub record: SourceAssetRecordResult,
    pub enqueued_jobs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileAssetSourcesRequest {
    pub capability: Capability,
    pub session_id: String,
    pub root_scope: AssetRootScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileAssetSourcesResult {
    pub source_root_count: u32,
    pub recorded_source_asset_count: u32,
    pub deleted_source_asset_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetProcessingStatusRequest {
    pub capability: Capability,
    pub session_id: String,
    pub platform: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AssetProcessingStatusResult {
    pub queued: u32,
    pub leased: u32,
    pub failed: u32,
    pub in_flight_sweeps: u32,
}

impl AssetProcessingStatusResult {
    #[must_use]
    pub const fn active(self) -> u32 {
        self.queued + self.leased + self.in_flight_sweeps
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishAssetCatalogRequest {
    pub capability: Capability,
    pub session_id: String,
    pub platform: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishAssetCatalogResult {
    pub catalog_path: String,
    pub entry_count: u32,
    pub reused: bool,
}

/// Worker → host registration of the project-linked builder/schema catalog.
///
/// The engine-owned asset-processor host has zero project gem linkage, so it
/// cannot discover builders via inventory. Project asset-workers publish this
/// catalog after connecting (with an exact protocol-version handshake).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishBuilderCatalogRequest {
    pub capability: Capability,
    pub protocol: core::ProtocolVersion,
    pub catalog: AssetBuilderCatalogResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishBuilderCatalogResult {
    pub accepted: bool,
}

/// Reserved job key used by the engine host to enqueue `create_jobs` planner work
/// onto project workers. Real product jobs never use this key.
pub const ASSET_PLANNER_JOB_KEY: &str = "__azoth.create_jobs";

/// Product format written by planner jobs; host `complete_attempt` expands it into
/// real asset job plans instead of promoting products.
pub const ASSET_JOB_PLAN_PRODUCT_FORMAT: &str = "azoth.job-plan/v1";
/// Reserved cache-relative path for the planner control product.
pub const ASSET_JOB_PLAN_PRODUCT_PATH: &str = "azoth/job-plan.json";
/// Stable type identity for the planner control product.
///
/// Planner output is protocol metadata rather than a runtime asset, but giving
/// it an explicit identity keeps product manifests uniformly typed across the
/// worker/host boundary.
pub const ASSET_JOB_PLAN_ASSET_TYPE: Uuid =
    Uuid::from_u128(0x4BD1_D5F4_AA37_41B7_8F48_EA2E_9B6C_F8BD);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDependentSource {
    pub source_edge_id: i64,
    pub source_path: String,
    pub builder_guid: Uuid,
    pub relation: SourceRelation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDependentJob {
    pub job_edge_id: i64,
    pub job_id: i64,
    pub latest_attempt_id: Option<i64>,
    pub source_path: String,
    pub owner: JobOwner,
    pub job_key: String,
    pub platform: String,
    pub dependency_job_key: String,
    pub dependency_platform: String,
    pub kind: JobDependencyKind,
    pub product_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDependentsRequest {
    pub capability: Capability,
    pub session_id: String,
    pub source_root: String,
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDependentsResult {
    pub source_path: String,
    pub source_dependents: Vec<SourceDependentSource>,
    pub job_dependents: Vec<SourceDependentJob>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseAssetJobRequest {
    pub capability: Capability,
    pub lease_owner: String,
    pub lease_duration_ms: u64,
    pub staging_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseAssetJobResult {
    pub leased: LeasedAssetJob,
    pub grant_key: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenewAssetJobLeaseRequest {
    pub capability: Capability,
    pub asset_job_attempt_id: i64,
    pub lease_owner: String,
    pub grant_key: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteAssetJobAttemptRequest {
    pub capability: Capability,
    pub asset_job_attempt_id: i64,
    pub lease_owner: String,
    pub status: AttemptStatus,
    pub finished_unix_ms: i64,
    pub error_count: i64,
    pub warning_count: i64,
    pub product_manifest: Option<SideChannelHandle>,
    pub grant_key: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductManifest {
    pub schema_version: u32,
    pub generated_unix_ms: u64,
    pub products: Vec<ProductManifestProduct>,
}

impl ProductManifest {
    #[must_use]
    pub const fn new(generated_unix_ms: u64, products: Vec<ProductManifestProduct>) -> Self {
        Self {
            schema_version: PRODUCT_MANIFEST_SCHEMA_VERSION,
            generated_unix_ms,
            products,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductManifestProduct {
    pub product_path: String,
    /// Additional catalog lookup paths for this product.
    pub catalog_aliases: Vec<String>,
    pub catalog_path_registration: CatalogPathRegistration,
    pub asset_type: Uuid,
    pub sub_id: u32,
    pub product_format: String,
    pub product_format_version: u32,
    pub staged_path: String,
    pub content_hash: Vec<u8>,
    pub byte_length: u64,
    pub dependencies: Vec<ProductManifestProductDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductManifestProductDependency {
    pub asset_guid: Uuid,
    pub sub_id: u32,
    pub flags: u32,
}

#[derive(Debug, ThisError)]
pub enum ProductManifestSideChannelError {
    #[error("invalid product manifest side-channel handle: {0}")]
    InvalidHandle(#[from] StagingFileSideChannelError),

    #[error("failed to decode product manifest side-channel: {0}")]
    Decode(#[from] Error),

    #[error("unsupported product manifest schema version {version}")]
    UnsupportedVersion { version: u32 },
}

fn write_leased_asset_job(
    leased: &LeasedAssetJob,
    mut builder: asset_capnp::leased_asset_job::Builder<'_>,
) -> Result<(), Error> {
    validate_leased_asset_job_for_transport(leased)?;
    builder.set_attempt_id(leased.attempt_id);
    builder.set_workspace_id(leased.workspace_id);
    write_job_owner(&leased.owner, builder.reborrow().init_owner());
    builder.set_job_key(&leased.job_key);
    builder.set_platform(&leased.platform);
    builder.set_ordinal(leased.ordinal);
    builder.set_staging_root(&leased.staging_root);
    builder.set_source_guid(leased.source_guid.as_bytes());
    builder.set_source_path(&leased.source_path);
    builder.set_source_root(&leased.source_root);
    write_optional_text(
        leased.source_schema_type.as_deref(),
        builder.reborrow().init_source_schema_type(),
    );
    write_optional_u32(
        leased.preserved_source_sub_id,
        builder.reborrow().init_preserved_source_sub_id(),
    );
    match &leased.source_payload {
        Some(handle) => core::SideChannelHandle::to_capnp(handle, builder.init_source_payload())?,
        None => builder.set_no_source_payload(()),
    }
    Ok(())
}

fn read_leased_asset_job(
    reader: asset_capnp::leased_asset_job::Reader<'_>,
) -> Result<LeasedAssetJob, Error> {
    let leased = LeasedAssetJob {
        attempt_id: reader.get_attempt_id(),
        workspace_id: reader.get_workspace_id(),
        owner: read_job_owner(reader.get_owner()?)?,
        source_guid: read_uuid_data(reader.get_source_guid()?)?,
        preserved_source_sub_id: read_optional_u32(reader.get_preserved_source_sub_id()?)?,
        source_path: reader.get_source_path()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        source_schema_type: read_optional_text(reader.get_source_schema_type()?)?,
        job_key: reader.get_job_key()?.to_string()?,
        platform: reader.get_platform()?.to_string()?,
        ordinal: reader.get_ordinal(),
        staging_root: reader.get_staging_root()?.to_string()?,
        source_payload: match reader.which()? {
            asset_capnp::leased_asset_job::Which::NoSourcePayload(()) => None,
            asset_capnp::leased_asset_job::Which::SourcePayload(handle) => {
                Some(core::SideChannelHandle::from_capnp(handle?)?)
            }
        },
    };
    validate_leased_asset_job_for_transport(&leased)?;
    Ok(leased)
}

fn write_inspect_job_request(
    request: &InspectJobRequest,
    mut builder: asset_capnp::inspect_job_request::Builder<'_>,
) -> Result<(), Error> {
    validate_inspect_job_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    let mut selector = builder.init_selector();
    match request.selector {
        InspectJobSelector::Job(id) => selector.set_job_id(id),
        InspectJobSelector::Attempt(id) => selector.set_attempt_id(id),
    }
    Ok(())
}

fn read_inspect_job_request(
    reader: asset_capnp::inspect_job_request::Reader<'_>,
) -> Result<InspectJobRequest, Error> {
    let selector = match reader.get_selector().which()? {
        asset_capnp::inspect_job_request::selector::Which::JobId(id) => InspectJobSelector::Job(id),
        asset_capnp::inspect_job_request::selector::Which::AttemptId(id) => {
            InspectJobSelector::Attempt(id)
        }
    };
    let request = InspectJobRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        selector,
    };
    validate_inspect_job_request_for_transport(&request)?;
    Ok(request)
}

fn write_job_owner(owner: &JobOwner, mut builder: asset_capnp::job_owner::Builder<'_>) {
    match owner {
        JobOwner::Plan => builder.set_plan(()),
        JobOwner::Build(builder_guid) => builder.set_build(builder_guid.as_bytes()),
    }
}

fn read_job_owner(reader: asset_capnp::job_owner::Reader<'_>) -> Result<JobOwner, Error> {
    match reader.which()? {
        asset_capnp::job_owner::Which::Plan(()) => Ok(JobOwner::Plan),
        asset_capnp::job_owner::Which::Build(builder_guid) => {
            Ok(JobOwner::Build(read_uuid_data(builder_guid?)?))
        }
    }
}

fn write_job_record(
    job: &JobRecord,
    mut builder: asset_capnp::job_record::Builder<'_>,
) -> Result<(), Error> {
    validate_job_record_for_transport(job)?;
    builder.set_job_id(job.job_id);
    builder.set_workspace_id(job.workspace_id);
    builder.set_source_guid(job.source_guid.as_bytes());
    builder.set_source_path(&job.source_path);
    builder.set_source_root(&job.source_root);
    write_optional_text(
        job.source_schema_type.as_deref(),
        builder.reborrow().init_source_schema_type(),
    );
    write_job_owner(&job.owner, builder.reborrow().init_owner());
    builder.set_key(&job.key);
    builder.set_platform(&job.platform);
    builder.set_status(job.status.to_capnp());
    builder.set_ready(job.ready);
    builder.set_attempts(job.attempts);
    Ok(())
}

fn read_job_record(reader: asset_capnp::job_record::Reader<'_>) -> Result<JobRecord, Error> {
    let job = JobRecord {
        job_id: reader.get_job_id(),
        workspace_id: reader.get_workspace_id(),
        source_guid: read_uuid_data(reader.get_source_guid()?)?,
        source_path: reader.get_source_path()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        source_schema_type: read_optional_text(reader.get_source_schema_type()?)?,
        owner: read_job_owner(reader.get_owner()?)?,
        key: reader.get_key()?.to_string()?,
        platform: reader.get_platform()?.to_string()?,
        status: reader
            .get_status()
            .map(JobStatus::from_capnp)
            .map_err(|e| Error::failed(format!("unknown job status: {e:?}")))?,
        ready: reader.get_ready(),
        attempts: reader.get_attempts(),
    };
    validate_job_record_for_transport(&job)?;
    Ok(job)
}

fn write_job_attempt_record(
    attempt: &JobAttemptRecord,
    mut builder: asset_capnp::job_attempt_record::Builder<'_>,
) {
    builder.set_attempt_id(attempt.attempt_id);
    builder.set_job_id(attempt.job_id);
    builder.set_ordinal(attempt.ordinal);
    builder.set_status(attempt.status.to_capnp());
    write_optional_text(attempt.owner.as_deref(), builder.reborrow().init_owner());
    write_optional_text(
        attempt.staging.as_deref(),
        builder.reborrow().init_staging(),
    );
    write_optional_i64(
        attempt.finished_unix_ms,
        builder.reborrow().init_finished_unix_ms(),
    );
    builder.set_error_count(attempt.error_count);
    builder.set_warning_count(attempt.warning_count);
}

fn read_job_attempt_record(
    reader: asset_capnp::job_attempt_record::Reader<'_>,
) -> Result<JobAttemptRecord, Error> {
    Ok(JobAttemptRecord {
        attempt_id: reader.get_attempt_id(),
        job_id: reader.get_job_id(),
        ordinal: reader.get_ordinal(),
        status: reader
            .get_status()
            .map(AttemptStatus::from_capnp)
            .map_err(|e| Error::failed(format!("unknown attempt status: {e:?}")))?,
        owner: read_optional_text(reader.get_owner()?)?,
        staging: read_optional_text(reader.get_staging()?)?,
        finished_unix_ms: read_optional_i64(reader.get_finished_unix_ms()?)?,
        error_count: reader.get_error_count(),
        warning_count: reader.get_warning_count(),
    })
}

fn write_job_activity(
    activity: &JobActivity,
    mut builder: asset_capnp::job_activity::Builder<'_>,
) -> Result<(), Error> {
    write_job_record(&activity.job, builder.reborrow().init_job())?;
    if let Some(attempt) = &activity.attempt {
        write_job_attempt_record(attempt, builder.init_attempt());
    } else {
        builder.set_no_attempt(());
    }
    Ok(())
}
fn read_job_activity(reader: asset_capnp::job_activity::Reader<'_>) -> Result<JobActivity, Error> {
    Ok(JobActivity {
        job: read_job_record(reader.get_job()?)?,
        attempt: match reader.which()? {
            asset_capnp::job_activity::Which::NoAttempt(()) => None,
            asset_capnp::job_activity::Which::Attempt(attempt) => {
                Some(read_job_attempt_record(attempt?)?)
            }
        },
    })
}

fn write_job_product_edge_record(
    edge: &JobProductEdgeRecord,
    mut builder: asset_capnp::job_product_edge_record::Builder<'_>,
) {
    builder.set_product_edge_id(edge.product_edge_id);
    builder.set_product_id(edge.product_id);
    builder.set_asset_guid(edge.asset_guid.as_bytes());
    builder.set_sub_id(edge.sub_id);
    builder.set_flags(edge.flags);
}
fn read_job_product_edge_record(
    reader: asset_capnp::job_product_edge_record::Reader<'_>,
) -> Result<JobProductEdgeRecord, Error> {
    Ok(JobProductEdgeRecord {
        product_edge_id: reader.get_product_edge_id(),
        product_id: reader.get_product_id(),
        asset_guid: read_uuid_data(reader.get_asset_guid()?)?,
        sub_id: reader.get_sub_id(),
        flags: reader.get_flags(),
    })
}

fn write_job_product_record(
    product: &JobProductRecord,
    mut builder: asset_capnp::job_product_record::Builder<'_>,
) -> Result<(), Error> {
    builder.set_product_id(product.product_id);
    builder.set_job_id(product.job_id);
    builder.set_path(&product.path);
    builder.set_asset_type(product.asset_type.as_bytes());
    builder.set_sub_id(product.sub_id);
    builder.set_product_format(&product.product_format);
    builder.set_product_format_version(product.product_format_version);
    builder.set_catalog_path_registration(product.catalog_path_registration.to_capnp());
    builder.set_content_hash(&product.content_hash);
    builder.set_byte_length(product.byte_length);
    let mut aliases = builder
        .reborrow()
        .init_aliases(capnp_list_index(product.aliases.len())?);
    for (i, alias) in product.aliases.iter().enumerate() {
        aliases.set(capnp_list_index(i)?, alias);
    }
    let mut edges = builder
        .reborrow()
        .init_edges(capnp_list_index(product.edges.len())?);
    for (i, edge) in product.edges.iter().enumerate() {
        write_job_product_edge_record(edge, edges.reborrow().get(capnp_list_index(i)?));
    }
    Ok(())
}
fn read_job_product_record(
    reader: asset_capnp::job_product_record::Reader<'_>,
) -> Result<JobProductRecord, Error> {
    Ok(JobProductRecord {
        product_id: reader.get_product_id(),
        job_id: reader.get_job_id(),
        path: reader.get_path()?.to_string()?,
        asset_type: read_uuid_data(reader.get_asset_type()?)?,
        sub_id: reader.get_sub_id(),
        product_format: reader.get_product_format()?.to_string()?,
        product_format_version: reader.get_product_format_version(),
        catalog_path_registration: reader
            .get_catalog_path_registration()
            .map(CatalogPathRegistration::from_capnp)
            .map_err(|e| Error::failed(format!("unknown catalog registration: {e:?}")))?,
        content_hash: reader.get_content_hash()?.to_string()?,
        byte_length: reader.get_byte_length(),
        aliases: reader
            .get_aliases()?
            .iter()
            .map(|v| Ok(v?.to_string()?))
            .collect::<Result<_, Error>>()?,
        edges: reader
            .get_edges()?
            .iter()
            .map(read_job_product_edge_record)
            .collect::<Result<_, _>>()?,
    })
}

fn write_job_dependency_record(
    dependency: &JobDependencyRecord,
    mut builder: asset_capnp::job_dependency_record::Builder<'_>,
) {
    builder.set_job_edge_id(dependency.job_edge_id);
    builder.set_job_id(dependency.job_id);
    {
        let mut target = builder.reborrow().init_target();
        match &dependency.target {
            JobDependencyTarget::Guid(guid) => target.set_guid(guid.as_bytes()),
            JobDependencyTarget::Path(path) => target.set_path(path),
        }
    }
    builder.set_key(&dependency.key);
    builder.set_platform(&dependency.platform);
    builder.set_kind(dependency.kind.to_capnp());
}
fn read_job_dependency_record(
    reader: asset_capnp::job_dependency_record::Reader<'_>,
) -> Result<JobDependencyRecord, Error> {
    let target = reader.get_target()?;
    let target = match target.which()? {
        asset_capnp::job_dependency_target::Which::Guid(guid) => {
            JobDependencyTarget::Guid(read_uuid_data(guid?)?)
        }
        asset_capnp::job_dependency_target::Which::Path(path) => {
            JobDependencyTarget::Path(path?.to_string()?)
        }
    };
    Ok(JobDependencyRecord {
        job_edge_id: reader.get_job_edge_id(),
        job_id: reader.get_job_id(),
        target,
        key: reader.get_key()?.to_string()?,
        platform: reader.get_platform()?.to_string()?,
        kind: reader
            .get_kind()
            .map(JobDependencyKind::from_capnp)
            .map_err(|e| Error::failed(format!("unknown job dependency kind: {e:?}")))?,
    })
}

fn write_job_inspection(
    inspection: &JobInspection,
    mut builder: asset_capnp::job_inspection::Builder<'_>,
) -> Result<(), Error> {
    write_job_record(&inspection.job, builder.reborrow().init_job())?;
    match &inspection.attempt {
        Some(attempt) => write_job_attempt_record(attempt, builder.reborrow().init_attempt()),
        None => builder.set_no_attempt(()),
    }
    let mut products = builder
        .reborrow()
        .init_products(capnp_list_index(inspection.products.len())?);
    for (i, product) in inspection.products.iter().enumerate() {
        write_job_product_record(product, products.reborrow().get(capnp_list_index(i)?))?;
    }
    let mut dependencies = builder
        .reborrow()
        .init_dependencies(capnp_list_index(inspection.dependencies.len())?);
    for (i, dependency) in inspection.dependencies.iter().enumerate() {
        write_job_dependency_record(
            dependency,
            dependencies.reborrow().get(capnp_list_index(i)?),
        );
    }
    Ok(())
}
fn read_job_inspection(
    reader: asset_capnp::job_inspection::Reader<'_>,
) -> Result<JobInspection, Error> {
    Ok(JobInspection {
        job: read_job_record(reader.get_job()?)?,
        attempt: match reader.which()? {
            asset_capnp::job_inspection::Which::NoAttempt(()) => None,
            asset_capnp::job_inspection::Which::Attempt(attempt) => {
                Some(read_job_attempt_record(attempt?)?)
            }
        },
        products: reader
            .get_products()?
            .iter()
            .map(read_job_product_record)
            .collect::<Result<_, _>>()?,
        dependencies: reader
            .get_dependencies()?
            .iter()
            .map(read_job_dependency_record)
            .collect::<Result<_, _>>()?,
    })
}

fn write_inspect_job_result(
    result: &InspectJobResult,
    mut builder: asset_capnp::inspect_job_result::Builder<'_>,
) -> Result<(), Error> {
    if let Some(inspection) = &result.inspection {
        write_job_inspection(inspection, builder.init_inspection())
    } else {
        builder.set_none(());
        Ok(())
    }
}
fn read_inspect_job_result(
    reader: asset_capnp::inspect_job_result::Reader<'_>,
) -> Result<InspectJobResult, Error> {
    Ok(InspectJobResult {
        inspection: match reader.which()? {
            asset_capnp::inspect_job_result::Which::None(()) => None,
            asset_capnp::inspect_job_result::Which::Inspection(inspection) => {
                Some(read_job_inspection(inspection?)?)
            }
        },
    })
}

fn write_asset_builder_pattern_descriptor(
    descriptor: &AssetBuilderPatternDescriptor,
    mut builder: asset_capnp::asset_builder_pattern_descriptor::Builder<'_>,
) -> Result<(), Error> {
    validate_asset_builder_pattern_descriptor_for_transport(descriptor)?;
    builder.set_kind(descriptor.kind.to_capnp());
    builder.set_pattern(&descriptor.pattern);
    Ok(())
}

fn read_asset_builder_pattern_descriptor(
    reader: asset_capnp::asset_builder_pattern_descriptor::Reader<'_>,
) -> Result<AssetBuilderPatternDescriptor, Error> {
    let kind = reader
        .get_kind()
        .map(AssetBuilderPatternKind::from_capnp)
        .map_err(|error| Error::failed(format!("unknown asset builder pattern kind: {error:?}")))?;
    let descriptor = AssetBuilderPatternDescriptor {
        kind,
        pattern: reader.get_pattern()?.to_string()?,
    };
    validate_asset_builder_pattern_descriptor_for_transport(&descriptor)?;
    Ok(descriptor)
}

fn write_asset_builder_descriptor(
    descriptor: &AssetBuilderDescriptor,
    mut builder: asset_capnp::asset_builder_descriptor::Builder<'_>,
) -> Result<(), Error> {
    validate_asset_builder_descriptor_for_transport(descriptor)?;
    builder.set_name(&descriptor.name);
    builder.set_builder_guid(descriptor.builder_guid.as_bytes());
    builder.set_version(descriptor.version);
    builder.set_analysis_fingerprint(&descriptor.analysis_fingerprint);
    let mut patterns = builder
        .reborrow()
        .init_patterns(capnp_list_index(descriptor.patterns.len())?);
    for (index, pattern) in descriptor.patterns.iter().enumerate() {
        (pattern).to_capnp(patterns.reborrow().get(capnp_list_index(index)?))?;
    }
    let mut source_schema_types = builder
        .reborrow()
        .init_source_schema_types(capnp_list_index(descriptor.source_schema_types.len())?);
    for (index, source_schema_type) in descriptor.source_schema_types.iter().enumerate() {
        source_schema_types
            .reborrow()
            .set(capnp_list_index(index)?, source_schema_type);
    }
    Ok(())
}

fn read_asset_builder_descriptor(
    reader: asset_capnp::asset_builder_descriptor::Reader<'_>,
) -> Result<AssetBuilderDescriptor, Error> {
    let patterns = reader
        .get_patterns()?
        .iter()
        .map(read_asset_builder_pattern_descriptor)
        .collect::<Result<Vec<_>, Error>>()?;
    let descriptor = AssetBuilderDescriptor {
        name: reader.get_name()?.to_string()?,
        builder_guid: read_uuid_data(reader.get_builder_guid()?)?,
        version: reader.get_version(),
        analysis_fingerprint: reader.get_analysis_fingerprint()?.to_string()?,
        patterns,
        source_schema_types: reader
            .get_source_schema_types()?
            .iter()
            .map(|schema_type| Ok(schema_type?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_asset_builder_descriptor_for_transport(&descriptor)?;
    Ok(descriptor)
}

fn write_source_schema_descriptor(
    descriptor: &SourceSchemaDescriptor,
    mut builder: asset_capnp::source_schema_descriptor::Builder<'_>,
) -> Result<(), Error> {
    validate_source_schema_descriptor_for_transport(descriptor)?;
    builder.set_schema_type(&descriptor.schema_type);
    builder.set_owner(&descriptor.owner);
    builder.set_label(&descriptor.label);
    builder.set_category(&descriptor.category);
    write_source_schema_authoring(&descriptor.authoring, builder.reborrow().init_authoring())?;
    let mut templates = builder
        .reborrow()
        .init_file_templates(capnp_list_index(descriptor.file_templates.len())?);
    for (index, template) in descriptor.file_templates.iter().enumerate() {
        (template).to_capnp(templates.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_source_schema_descriptor(
    reader: asset_capnp::source_schema_descriptor::Reader<'_>,
) -> Result<SourceSchemaDescriptor, Error> {
    let descriptor = SourceSchemaDescriptor {
        schema_type: reader.get_schema_type()?.to_string()?,
        owner: reader.get_owner()?.to_string()?,
        label: reader.get_label()?.to_string()?,
        category: reader.get_category()?.to_string()?,
        authoring: read_source_schema_authoring(reader.get_authoring()?)?,
        file_templates: reader
            .get_file_templates()?
            .iter()
            .map(read_source_file_template_descriptor)
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_source_schema_descriptor_for_transport(&descriptor)?;
    Ok(descriptor)
}

fn write_product_format_descriptor(
    descriptor: &ProductFormatDescriptor,
    mut builder: asset_capnp::product_format_descriptor::Builder<'_>,
) -> Result<(), Error> {
    validate_product_format_descriptor_for_transport(descriptor)?;
    builder.set_id(&descriptor.id);
    builder.set_current_version(descriptor.current_version);
    builder.set_owner(&descriptor.owner);
    Ok(())
}

fn read_product_format_descriptor(
    reader: asset_capnp::product_format_descriptor::Reader<'_>,
) -> Result<ProductFormatDescriptor, Error> {
    let descriptor = ProductFormatDescriptor {
        id: reader.get_id()?.to_string()?,
        current_version: reader.get_current_version(),
        owner: reader.get_owner()?.to_string()?,
    };
    validate_product_format_descriptor_for_transport(&descriptor)?;
    Ok(descriptor)
}

fn write_source_file_template_descriptor(
    descriptor: &SourceFileTemplateDescriptor,
    mut builder: asset_capnp::source_file_template_descriptor::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_template_descriptor_for_transport(descriptor)?;
    builder.set_owner(&descriptor.owner);
    builder.set_source_path(&descriptor.source_path);
    builder.set_label(&descriptor.label);
    builder.set_description(&descriptor.description);
    Ok(())
}

fn read_source_file_template_descriptor(
    reader: asset_capnp::source_file_template_descriptor::Reader<'_>,
) -> Result<SourceFileTemplateDescriptor, Error> {
    let descriptor = SourceFileTemplateDescriptor {
        owner: reader.get_owner()?.to_string()?,
        source_path: reader.get_source_path()?.to_string()?,
        label: reader.get_label()?.to_string()?,
        description: reader.get_description()?.to_string()?,
    };
    validate_source_file_template_descriptor_for_transport(&descriptor)?;
    Ok(descriptor)
}

fn write_source_schema_authoring(
    authoring: &SourceSchemaAuthoring,
    mut builder: asset_capnp::source_schema_authoring::Builder<'_>,
) -> Result<(), Error> {
    validate_source_schema_authoring_for_transport(authoring)?;
    match authoring {
        SourceSchemaAuthoring::File { workflow } => {
            write_source_file_workflow_descriptor(workflow, builder.init_file())?;
        }
        SourceSchemaAuthoring::ProjectDocument { schema_type } => {
            builder.set_project_document(schema_type);
        }
    }
    Ok(())
}

fn write_source_file_workflow_descriptor(
    workflow: &SourceFileWorkflowDescriptor,
    mut builder: asset_capnp::source_file_workflow_descriptor::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_workflow_descriptor_for_transport(workflow)?;
    builder.set_source_root(&workflow.source_root);
    builder.set_default_path_prefix(&workflow.default_path_prefix);
    let mut extensions = builder
        .reborrow()
        .init_extensions(capnp_list_index(workflow.extensions.len())?);
    for (index, extension) in workflow.extensions.iter().enumerate() {
        extensions
            .reborrow()
            .set(capnp_list_index(index)?, extension);
    }
    builder.set_can_create(workflow.can_create);
    builder.set_can_edit(workflow.can_edit);
    Ok(())
}

fn read_source_schema_authoring(
    reader: asset_capnp::source_schema_authoring::Reader<'_>,
) -> Result<SourceSchemaAuthoring, Error> {
    let authoring = match reader.which()? {
        asset_capnp::source_schema_authoring::Which::File(workflow) => {
            SourceSchemaAuthoring::File {
                workflow: read_source_file_workflow_descriptor(workflow?)?,
            }
        }
        asset_capnp::source_schema_authoring::Which::ProjectDocument(schema_type) => {
            SourceSchemaAuthoring::ProjectDocument {
                schema_type: schema_type?.to_string()?,
            }
        }
    };
    validate_source_schema_authoring_for_transport(&authoring)?;
    Ok(authoring)
}

fn read_source_file_workflow_descriptor(
    reader: asset_capnp::source_file_workflow_descriptor::Reader<'_>,
) -> Result<SourceFileWorkflowDescriptor, Error> {
    let workflow = SourceFileWorkflowDescriptor {
        source_root: reader.get_source_root()?.to_string()?,
        default_path_prefix: reader.get_default_path_prefix()?.to_string()?,
        extensions: reader
            .get_extensions()?
            .iter()
            .map(|extension| Ok(extension?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
        can_create: reader.get_can_create(),
        can_edit: reader.get_can_edit(),
    };
    validate_source_file_workflow_descriptor_for_transport(&workflow)?;
    Ok(workflow)
}

fn write_asset_builder_catalog_request(
    request: &AssetBuilderCatalogRequest,
    mut builder: asset_capnp::asset_builder_catalog_request::Builder<'_>,
) -> Result<(), Error> {
    validate_asset_builder_catalog_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    Ok(())
}

fn read_asset_builder_catalog_request(
    reader: asset_capnp::asset_builder_catalog_request::Reader<'_>,
) -> Result<AssetBuilderCatalogRequest, Error> {
    let request = AssetBuilderCatalogRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_asset_builder_catalog_request_for_transport(&request)?;
    Ok(request)
}

fn write_asset_builder_catalog_result(
    result: &AssetBuilderCatalogResult,
    mut builder: asset_capnp::asset_builder_catalog_result::Builder<'_>,
) -> Result<(), Error> {
    validate_asset_builder_catalog_result_for_transport(result)?;
    let mut builders = builder
        .reborrow()
        .init_builders(capnp_list_index(result.builders.len())?);
    for (index, descriptor) in result.builders.iter().enumerate() {
        (descriptor).to_capnp(builders.reborrow().get(capnp_list_index(index)?))?;
    }
    let mut source_schemas = builder
        .reborrow()
        .init_source_schemas(capnp_list_index(result.source_schemas.len())?);
    for (index, descriptor) in result.source_schemas.iter().enumerate() {
        (descriptor).to_capnp(source_schemas.reborrow().get(capnp_list_index(index)?))?;
    }
    let mut product_formats = builder
        .reborrow()
        .init_product_formats(capnp_list_index(result.product_formats.len())?);
    for (index, descriptor) in result.product_formats.iter().enumerate() {
        (descriptor).to_capnp(product_formats.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_asset_builder_catalog_result(
    reader: asset_capnp::asset_builder_catalog_result::Reader<'_>,
) -> Result<AssetBuilderCatalogResult, Error> {
    let builders = reader
        .get_builders()?
        .iter()
        .map(read_asset_builder_descriptor)
        .collect::<Result<Vec<_>, Error>>()?;
    let source_schemas = reader
        .get_source_schemas()?
        .iter()
        .map(read_source_schema_descriptor)
        .collect::<Result<Vec<_>, Error>>()?;
    let product_formats = reader
        .get_product_formats()?
        .iter()
        .map(read_product_format_descriptor)
        .collect::<Result<Vec<_>, Error>>()?;
    let result = AssetBuilderCatalogResult {
        builders,
        source_schemas,
        product_formats,
    };
    validate_asset_builder_catalog_result_for_transport(&result)?;
    Ok(result)
}

fn write_workspace_entry(
    entry: &WorkspaceEntry,
    mut builder: asset_capnp::workspace_entry::Builder<'_>,
) -> Result<(), Error> {
    validate_workspace_entry_for_transport(entry)?;
    builder.set_entry_id(entry.entry_id);
    builder.set_workspace_id(entry.workspace_id);
    builder.set_asset_guid(entry.asset_guid.as_bytes());
    builder.set_root_id(entry.root_id);
    builder.set_source_path(&entry.source_path);
    write_optional_text(
        entry.schema_type.as_deref(),
        builder.reborrow().init_schema_type(),
    );
    builder.set_content_hash(&entry.content_hash);
    builder.set_diff(entry.diff.to_capnp());
    builder.set_diagnostics_count(entry.diagnostics_count);
    builder.set_updated_unix_ms(entry.updated_unix_ms);
    let mut jobs = builder
        .reborrow()
        .init_jobs(capnp_list_index(entry.jobs.len())?);
    for (index, activity) in entry.jobs.iter().enumerate() {
        write_job_activity(activity, jobs.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_workspace_entry(
    reader: asset_capnp::workspace_entry::Reader<'_>,
) -> Result<WorkspaceEntry, Error> {
    let diff = reader
        .get_diff()
        .map(WorkspaceEntryDiff::from_capnp)
        .map_err(|error| Error::failed(format!("unknown workspace entry diff: {error:?}")))?;
    let entry = WorkspaceEntry {
        entry_id: reader.get_entry_id(),
        workspace_id: reader.get_workspace_id(),
        asset_guid: read_uuid_data(reader.get_asset_guid()?)?,
        root_id: reader.get_root_id(),
        source_path: reader.get_source_path()?.to_string()?,
        schema_type: read_optional_text(reader.get_schema_type()?)?,
        content_hash: reader.get_content_hash()?.to_string()?,
        diff,
        diagnostics_count: reader.get_diagnostics_count(),
        updated_unix_ms: reader.get_updated_unix_ms(),
        jobs: reader
            .get_jobs()?
            .iter()
            .map(read_job_activity)
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_workspace_entry_for_transport(&entry)?;
    Ok(entry)
}

fn write_workspace_entry_page_request(
    request: &WorkspaceEntryPageRequest,
    mut builder: asset_capnp::workspace_entry_page_request::Builder<'_>,
) -> Result<(), Error> {
    validate_workspace_entry_page_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    write_optional_i64(
        request.after_entry_id,
        builder.reborrow().init_after_entry_id(),
    );
    builder.set_page_size(request.page_size);
    builder.set_root_scope(request.root_scope.to_capnp());
    Ok(())
}

fn read_workspace_entry_page_request(
    reader: asset_capnp::workspace_entry_page_request::Reader<'_>,
) -> Result<WorkspaceEntryPageRequest, Error> {
    let request = WorkspaceEntryPageRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        root_scope: reader
            .get_root_scope()
            .map(AssetRootScope::from_capnp)
            .map_err(|error| Error::failed(format!("unknown asset root scope: {error:?}")))?,
        after_entry_id: read_optional_i64(reader.get_after_entry_id()?)?,
        page_size: reader.get_page_size(),
    };
    validate_workspace_entry_page_request_for_transport(&request)?;
    Ok(request)
}

fn write_workspace_entry_page_result(
    result: &WorkspaceEntryPageResult,
    mut builder: asset_capnp::workspace_entry_page_result::Builder<'_>,
) -> Result<(), Error> {
    validate_workspace_entry_page_result_for_transport(result)?;
    let mut entries = builder
        .reborrow()
        .init_entries(capnp_list_index(result.entries.len())?);
    for (index, entry) in result.entries.iter().enumerate() {
        (entry).to_capnp(entries.reborrow().get(capnp_list_index(index)?))?;
    }
    write_optional_i64(
        result.next_after_entry_id,
        builder.reborrow().init_next_after_entry_id(),
    );
    Ok(())
}

fn read_workspace_entry_page_result(
    reader: asset_capnp::workspace_entry_page_result::Reader<'_>,
) -> Result<WorkspaceEntryPageResult, Error> {
    let entries = reader
        .get_entries()?
        .iter()
        .map(read_workspace_entry)
        .collect::<Result<Vec<_>, Error>>()?;
    let result = WorkspaceEntryPageResult {
        entries,
        next_after_entry_id: read_optional_i64(reader.get_next_after_entry_id()?)?,
    };
    validate_workspace_entry_page_result_for_transport(&result)?;
    Ok(result)
}

fn write_asset_processor_event(
    event: &AssetProcessorEvent,
    mut builder: asset_capnp::asset_processor_event::Builder<'_>,
) -> Result<(), Error> {
    validate_asset_processor_event_for_transport(event)?;
    builder.set_seq(event.seq);
    builder.set_kind(event.kind.to_capnp());
    builder.set_event_unix_ms(event.event_unix_ms);
    (event.entry).to_capnp(builder.reborrow().init_entry())?;
    Ok(())
}

fn read_asset_processor_event(
    reader: asset_capnp::asset_processor_event::Reader<'_>,
) -> Result<AssetProcessorEvent, Error> {
    let kind = reader
        .get_kind()
        .map(AssetProcessorEventKind::from_capnp)
        .map_err(|error| Error::failed(format!("unknown asset processor event kind: {error:?}")))?;
    let event = AssetProcessorEvent {
        seq: reader.get_seq(),
        kind,
        event_unix_ms: reader.get_event_unix_ms(),
        entry: WorkspaceEntry::from_capnp(reader.get_entry()?)?,
    };
    validate_asset_processor_event_for_transport(&event)?;
    Ok(event)
}

fn write_asset_processor_event_subscription_request(
    request: &AssetProcessorEventSubscriptionRequest,
    mut builder: asset_capnp::asset_processor_event_subscription_request::Builder<'_>,
) -> Result<(), Error> {
    validate_asset_processor_event_subscription_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    Ok(())
}

fn read_asset_processor_event_subscription_request(
    reader: asset_capnp::asset_processor_event_subscription_request::Reader<'_>,
) -> Result<AssetProcessorEventSubscriptionRequest, Error> {
    let request = AssetProcessorEventSubscriptionRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_asset_processor_event_subscription_request_for_transport(&request)?;
    Ok(request)
}

fn write_asset_processor_event_subscription_result(
    result: &AssetProcessorEventSubscriptionResult,
    mut builder: asset_capnp::asset_processor_event_subscription_result::Builder<'_>,
) {
    builder.set_subscribed(result.subscribed);
    result
        .initial_health
        .to_capnp(builder.reborrow().init_initial_health())
        .expect("validated asset processor subscription health");
}

fn read_asset_processor_event_subscription_result(
    reader: asset_capnp::asset_processor_event_subscription_result::Reader<'_>,
) -> Result<AssetProcessorEventSubscriptionResult, Error> {
    Ok(AssetProcessorEventSubscriptionResult {
        subscribed: reader.get_subscribed(),
        initial_health: core::ServiceHealth::from_capnp(reader.get_initial_health()?)?,
    })
}

fn write_workspace_snapshot(
    snapshot: &WorkspaceSnapshot,
    mut builder: asset_capnp::workspace_snapshot::Builder<'_>,
) -> Result<(), Error> {
    validate_workspace_snapshot_for_transport(snapshot)?;
    builder.set_workspace_id(snapshot.workspace_id);
    builder.set_project_id(&snapshot.project_id);
    builder.set_workspace_root(&snapshot.workspace_root);
    builder.set_branch(&snapshot.branch);
    builder.set_created_unix_ms(snapshot.created_unix_ms);
    builder.set_updated_unix_ms(snapshot.updated_unix_ms);
    let mut roots = builder
        .reborrow()
        .init_roots(capnp_list_index(snapshot.roots.len())?);
    for (index, root) in snapshot.roots.iter().enumerate() {
        (root).to_capnp(roots.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_workspace_snapshot(
    reader: asset_capnp::workspace_snapshot::Reader<'_>,
) -> Result<WorkspaceSnapshot, Error> {
    let roots = reader
        .get_roots()?
        .iter()
        .map(read_workspace_root)
        .collect::<Result<Vec<_>, Error>>()?;
    let snapshot = WorkspaceSnapshot {
        workspace_id: reader.get_workspace_id(),
        project_id: reader.get_project_id()?.to_string()?,
        workspace_root: reader.get_workspace_root()?.to_string()?,
        branch: reader.get_branch()?.to_string()?,
        created_unix_ms: reader.get_created_unix_ms(),
        updated_unix_ms: reader.get_updated_unix_ms(),
        roots,
    };
    validate_workspace_snapshot_for_transport(&snapshot)?;
    Ok(snapshot)
}

fn write_workspace_root(
    source_root: &WorkspaceRoot,
    mut builder: asset_capnp::workspace_root::Builder<'_>,
) -> Result<(), Error> {
    validate_workspace_root_for_transport(source_root)?;
    builder.set_workspace_root_id(source_root.workspace_root_id);
    builder.set_workspace_id(source_root.workspace_id);
    builder.set_root_id(source_root.root_id);
    builder.set_declared_root_id(&source_root.declared_root_id);
    builder.set_owner_id(&source_root.owner_id);
    builder.set_source_root(&source_root.source_root);
    builder.set_display_name(&source_root.display_name);
    builder.set_portable_key(&source_root.portable_key);
    builder.set_mount(&source_root.mount);
    builder.set_recursive(source_root.recursive);
    builder.set_watch(source_root.watch);
    builder.set_writable(source_root.writable);
    builder.set_output_prefix(&source_root.output_prefix);
    builder.set_is_root(source_root.is_root);
    Ok(())
}

fn read_workspace_root(
    reader: asset_capnp::workspace_root::Reader<'_>,
) -> Result<WorkspaceRoot, Error> {
    let source_root = WorkspaceRoot {
        workspace_root_id: reader.get_workspace_root_id(),
        workspace_id: reader.get_workspace_id(),
        root_id: reader.get_root_id(),
        declared_root_id: reader.get_declared_root_id()?.to_string()?,
        owner_id: reader.get_owner_id()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        display_name: reader.get_display_name()?.to_string()?,
        portable_key: reader.get_portable_key()?.to_string()?,
        mount: reader.get_mount()?.to_string()?,
        recursive: reader.get_recursive(),
        watch: reader.get_watch(),
        writable: reader.get_writable(),
        output_prefix: reader.get_output_prefix()?.to_string()?,
        is_root: reader.get_is_root(),
    };
    validate_workspace_root_for_transport(&source_root)?;
    Ok(source_root)
}

fn write_workspace_snapshot_request(
    request: &WorkspaceSnapshotRequest,
    mut builder: asset_capnp::workspace_snapshot_request::Builder<'_>,
) -> Result<(), Error> {
    validate_workspace_snapshot_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_root_scope(request.root_scope.to_capnp());
    Ok(())
}

fn read_workspace_snapshot_request(
    reader: asset_capnp::workspace_snapshot_request::Reader<'_>,
) -> Result<WorkspaceSnapshotRequest, Error> {
    let request = WorkspaceSnapshotRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        root_scope: reader
            .get_root_scope()
            .map(AssetRootScope::from_capnp)
            .map_err(|error| Error::failed(format!("unknown asset root scope: {error:?}")))?,
    };
    validate_workspace_snapshot_request_for_transport(&request)?;
    Ok(request)
}

fn write_workspace_snapshot_result(
    result: &WorkspaceSnapshotResult,
    mut builder: asset_capnp::workspace_snapshot_result::Builder<'_>,
) -> Result<(), Error> {
    match &result.snapshot {
        Some(snapshot) => (snapshot).to_capnp(builder.init_snapshot())?,
        None => builder.set_none(()),
    }
    Ok(())
}

fn read_workspace_snapshot_result(
    reader: asset_capnp::workspace_snapshot_result::Reader<'_>,
) -> Result<WorkspaceSnapshotResult, Error> {
    let snapshot = match reader.which()? {
        asset_capnp::workspace_snapshot_result::Which::None(()) => None,
        asset_capnp::workspace_snapshot_result::Which::Snapshot(snapshot) => {
            Some(WorkspaceSnapshot::from_capnp(snapshot?)?)
        }
    };
    Ok(WorkspaceSnapshotResult { snapshot })
}

fn write_catalog_product_entry(
    entry: &CatalogProductEntry,
    mut builder: asset_capnp::catalog_product_entry::Builder<'_>,
) -> Result<(), Error> {
    validate_catalog_product_entry_for_transport(entry)?;
    builder.set_job_id(entry.job_id);
    builder.set_product_id(entry.product_id);
    builder.set_asset_guid(entry.asset_guid.as_bytes());
    builder.set_source_path(&entry.source_path);
    builder.set_builder_guid(entry.builder_guid.as_bytes());
    builder.set_job_key(&entry.job_key);
    builder.set_platform(&entry.platform);
    builder.set_product_path(&entry.product_path);
    builder.set_asset_type(entry.asset_type.as_bytes());
    builder.set_sub_id(entry.sub_id);
    builder.set_content_hash(&entry.content_hash);
    builder.set_byte_length(entry.byte_length);
    builder.set_product_format(&entry.product_format);
    builder.set_product_format_version(entry.product_format_version);
    builder.set_catalog_path_registration(entry.catalog_path_registration.to_capnp());
    let mut aliases = builder
        .reborrow()
        .init_catalog_aliases(capnp_list_index(entry.catalog_aliases.len())?);
    for (index, alias) in entry.catalog_aliases.iter().enumerate() {
        aliases.set(capnp_list_index(index)?, alias);
    }
    let mut dependencies = builder.init_dependencies(capnp_list_index(entry.dependencies.len())?);
    for (index, dependency) in entry.dependencies.iter().enumerate() {
        let mut dependency_builder = dependencies.reborrow().get(capnp_list_index(index)?);
        dependency_builder.set_asset_guid(dependency.asset_guid.as_bytes());
        dependency_builder.set_sub_id(dependency.sub_id);
        match dependency.asset_type {
            Some(asset_type) => dependency_builder.set_asset_type(asset_type.as_bytes()),
            None => dependency_builder.set_no_asset_type(()),
        }
        write_optional_text(dependency.hint.as_deref(), dependency_builder.init_hint());
    }
    Ok(())
}

fn read_catalog_product_entry(
    reader: asset_capnp::catalog_product_entry::Reader<'_>,
) -> Result<CatalogProductEntry, Error> {
    let dependencies = reader
        .get_dependencies()?
        .iter()
        .map(read_catalog_product_dependency)
        .collect::<Result<Vec<_>, Error>>()?;
    let entry = CatalogProductEntry {
        job_id: reader.get_job_id(),
        product_id: reader.get_product_id(),
        asset_guid: read_uuid_data(reader.get_asset_guid()?)?,
        source_path: reader.get_source_path()?.to_string()?,
        builder_guid: read_uuid_data(reader.get_builder_guid()?)?,
        job_key: reader.get_job_key()?.to_string()?,
        platform: reader.get_platform()?.to_string()?,
        product_path: reader.get_product_path()?.to_string()?,
        asset_type: read_uuid_data(reader.get_asset_type()?)?,
        sub_id: reader.get_sub_id(),
        content_hash: reader.get_content_hash()?.to_string()?,
        catalog_aliases: reader
            .get_catalog_aliases()?
            .iter()
            .map(|alias| -> Result<String, Error> { Ok(alias?.to_string()?) })
            .collect::<Result<Vec<_>, _>>()?,
        catalog_path_registration: reader
            .get_catalog_path_registration()
            .map(CatalogPathRegistration::from_capnp)
            .map_err(|error| {
                Error::failed(format!("unknown catalog path registration: {error:?}"))
            })?,
        byte_length: reader.get_byte_length(),
        product_format: reader.get_product_format()?.to_string()?,
        product_format_version: reader.get_product_format_version(),
        dependencies,
    };
    validate_catalog_product_entry_for_transport(&entry)?;
    Ok(entry)
}

fn read_catalog_product_dependency(
    reader: asset_capnp::catalog_product_dependency::Reader<'_>,
) -> Result<CatalogProductDependency, Error> {
    Ok(CatalogProductDependency {
        asset_guid: read_uuid_data(reader.get_asset_guid()?)?,
        sub_id: reader.get_sub_id(),
        asset_type: match reader.which()? {
            asset_capnp::catalog_product_dependency::Which::NoAssetType(()) => None,
            asset_capnp::catalog_product_dependency::Which::AssetType(asset_type) => {
                Some(read_uuid_data(asset_type?)?)
            }
        },
        hint: read_optional_text(reader.get_hint()?)?,
    })
}

fn write_catalog_products_request(
    request: &CatalogProductsRequest,
    mut builder: asset_capnp::catalog_products_request::Builder<'_>,
) -> Result<(), Error> {
    validate_catalog_products_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_platform(&request.platform);
    Ok(())
}

fn read_catalog_products_request(
    reader: asset_capnp::catalog_products_request::Reader<'_>,
) -> Result<CatalogProductsRequest, Error> {
    let request = CatalogProductsRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        platform: reader.get_platform()?.to_string()?,
    };
    validate_catalog_products_request_for_transport(&request)?;
    Ok(request)
}

fn write_catalog_products_result(
    result: &CatalogProductsResult,
    mut builder: asset_capnp::catalog_products_result::Builder<'_>,
) -> Result<(), Error> {
    validate_catalog_products_result_for_transport(result)?;
    let mut entries = builder
        .reborrow()
        .init_entries(capnp_list_index(result.entries.len())?);
    for (index, entry) in result.entries.iter().enumerate() {
        (entry).to_capnp(entries.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_catalog_products_result(
    reader: asset_capnp::catalog_products_result::Reader<'_>,
) -> Result<CatalogProductsResult, Error> {
    let entries = reader
        .get_entries()?
        .iter()
        .map(read_catalog_product_entry)
        .collect::<Result<Vec<_>, Error>>()?;
    let result = CatalogProductsResult { entries };
    validate_catalog_products_result_for_transport(&result)?;
    Ok(result)
}

fn write_release_content_read_request(
    request: &ReleaseContentReadRequest,
    mut builder: asset_capnp::release_content_read_request::Builder<'_>,
) -> Result<(), Error> {
    validate_release_content_read_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_platform(&request.platform);
    write_release_content_target(&request.target, builder.reborrow().init_target());
    Ok(())
}

fn read_release_content_read_request(
    reader: asset_capnp::release_content_read_request::Reader<'_>,
) -> Result<ReleaseContentReadRequest, Error> {
    let request = ReleaseContentReadRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        platform: reader.get_platform()?.to_string()?,
        target: read_release_content_target(reader.get_target()?)?,
    };
    validate_release_content_read_request_for_transport(&request)?;
    Ok(request)
}

fn write_release_content_product(
    product: &ReleaseContentProduct,
    mut builder: asset_capnp::release_content_product::Builder<'_>,
) -> Result<(), Error> {
    validate_release_content_product_for_transport(product)?;
    builder.set_asset_guid(product.asset_guid.as_bytes());
    builder.set_sub_id(product.sub_id);
    builder.set_product_path(&product.product_path);
    builder.set_product_format(&product.product_format);
    builder.set_product_format_version(product.product_format_version);
    builder.set_byte_length(product.byte_length);
    builder.set_content_hash(&product.content_hash);
    core::SideChannelHandle::to_capnp(&product.payload, builder.reborrow().init_payload())?;
    Ok(())
}

fn read_release_content_product(
    reader: asset_capnp::release_content_product::Reader<'_>,
) -> Result<ReleaseContentProduct, Error> {
    let product = ReleaseContentProduct {
        asset_guid: read_uuid_data(reader.get_asset_guid()?)?,
        sub_id: reader.get_sub_id(),
        product_path: reader.get_product_path()?.to_string()?,
        product_format: reader.get_product_format()?.to_string()?,
        product_format_version: reader.get_product_format_version(),
        byte_length: reader.get_byte_length(),
        content_hash: reader.get_content_hash()?.to_vec(),
        payload: core::SideChannelHandle::from_capnp(reader.get_payload()?)?,
    };
    validate_release_content_product_for_transport(&product)?;
    Ok(product)
}

fn write_release_content_read_result(
    result: &ReleaseContentReadResult,
    mut builder: asset_capnp::release_content_read_result::Builder<'_>,
) -> Result<(), Error> {
    validate_release_content_read_result_for_transport(result)?;
    match result {
        ReleaseContentReadResult::None => builder.set_none(()),
        ReleaseContentReadResult::AssetCatalog(handle) => {
            core::SideChannelHandle::to_capnp(handle, builder.reborrow().init_asset_catalog())?;
        }
        ReleaseContentReadResult::Product(product) => {
            (product).to_capnp(builder.reborrow().init_product())?;
        }
    }
    Ok(())
}

fn read_release_content_read_result(
    reader: asset_capnp::release_content_read_result::Reader<'_>,
) -> Result<ReleaseContentReadResult, Error> {
    let result = match reader.which()? {
        asset_capnp::release_content_read_result::Which::None(()) => ReleaseContentReadResult::None,
        asset_capnp::release_content_read_result::Which::AssetCatalog(handle) => {
            ReleaseContentReadResult::AssetCatalog(core::SideChannelHandle::from_capnp(handle?)?)
        }
        asset_capnp::release_content_read_result::Which::Product(product) => {
            ReleaseContentReadResult::Product(ReleaseContentProduct::from_capnp(product?)?)
        }
    };
    validate_release_content_read_result_for_transport(&result)?;
    Ok(result)
}

fn write_release_content_target(
    target: &ReleaseContentTarget,
    mut builder: asset_capnp::release_content_target::Builder<'_>,
) {
    match target {
        ReleaseContentTarget::AssetCatalog => builder.set_asset_catalog(()),
        ReleaseContentTarget::ProductAsset { asset_guid, sub_id } => {
            let mut asset_id = builder.init_product_asset();
            asset_id.set_asset_guid(asset_guid.as_bytes());
            asset_id.set_sub_id(*sub_id);
        }
    }
}

fn read_release_content_target(
    reader: asset_capnp::release_content_target::Reader<'_>,
) -> Result<ReleaseContentTarget, Error> {
    match reader.which()? {
        asset_capnp::release_content_target::Which::AssetCatalog(()) => {
            Ok(ReleaseContentTarget::AssetCatalog)
        }
        asset_capnp::release_content_target::Which::ProductAsset(asset_id) => {
            let asset_id = asset_id?;
            Ok(ReleaseContentTarget::ProductAsset {
                asset_guid: read_uuid_data(asset_id.get_asset_guid()?)?,
                sub_id: asset_id.get_sub_id(),
            })
        }
    }
}

fn write_source_asset_record_request(
    request: &SourceAssetRecordRequest,
    mut builder: asset_capnp::source_asset_record_request::Builder<'_>,
) -> Result<(), Error> {
    validate_source_asset_record_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    builder.set_workspace_root_id(request.workspace_root_id);
    builder.set_owner_id(&request.owner_id);
    builder.set_source_path(&request.source_path);
    write_optional_text(
        request.schema_type.as_deref(),
        builder.reborrow().init_schema_type(),
    );
    builder.set_content_hash(&request.content_hash);
    builder.set_changed_unix_ms(request.changed_unix_ms);
    builder.set_diagnostics_count(request.diagnostics_count);
    Ok(())
}

fn read_source_asset_record_request(
    reader: asset_capnp::source_asset_record_request::Reader<'_>,
) -> Result<SourceAssetRecordRequest, Error> {
    let request = SourceAssetRecordRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        workspace_root_id: reader.get_workspace_root_id(),
        owner_id: reader.get_owner_id()?.to_string()?,
        source_path: reader.get_source_path()?.to_string()?,
        schema_type: read_optional_text(reader.get_schema_type()?)?,
        content_hash: reader.get_content_hash()?.to_vec(),
        changed_unix_ms: reader.get_changed_unix_ms(),
        diagnostics_count: reader.get_diagnostics_count(),
    };
    validate_source_asset_record_request_for_transport(&request)?;
    Ok(request)
}

fn write_source_asset_record_result(
    result: &SourceAssetRecordResult,
    mut builder: asset_capnp::source_asset_record_result::Builder<'_>,
) -> Result<(), Error> {
    validate_source_asset_record_result_for_transport(result)?;
    builder.set_asset_guid(result.asset_guid.as_bytes());
    (result.entry).to_capnp(builder.init_entry())?;
    Ok(())
}

fn read_source_asset_record_result(
    reader: asset_capnp::source_asset_record_result::Reader<'_>,
) -> Result<SourceAssetRecordResult, Error> {
    let result = SourceAssetRecordResult {
        asset_guid: read_uuid_data(reader.get_asset_guid()?)?,
        entry: WorkspaceEntry::from_capnp(reader.get_entry()?)?,
    };
    validate_source_asset_record_result_for_transport(&result)?;
    Ok(result)
}

fn write_source_file_create_request(
    request: &SourceFileCreateRequest,
    mut builder: asset_capnp::source_file_create_request::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_create_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    builder.set_source_root(&request.source_root);
    builder.set_source_path(&request.source_path);
    builder.set_schema_type(&request.schema_type);
    builder.set_changed_unix_ms(request.changed_unix_ms);
    match &request.content {
        SourceFileCreateContent::DefaultTemplate => builder.set_default_template(()),
        SourceFileCreateContent::Payload(handle) => {
            core::SideChannelHandle::to_capnp(handle, builder.reborrow().init_payload())?;
        }
    }
    Ok(())
}

fn read_source_file_create_request(
    reader: asset_capnp::source_file_create_request::Reader<'_>,
) -> Result<SourceFileCreateRequest, Error> {
    let content = match reader.which()? {
        asset_capnp::source_file_create_request::Which::DefaultTemplate(()) => {
            SourceFileCreateContent::DefaultTemplate
        }
        asset_capnp::source_file_create_request::Which::Payload(handle) => {
            SourceFileCreateContent::Payload(Box::new(core::SideChannelHandle::from_capnp(
                handle?,
            )?))
        }
    };
    let request = SourceFileCreateRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        source_path: reader.get_source_path()?.to_string()?,
        schema_type: reader.get_schema_type()?.to_string()?,
        changed_unix_ms: reader.get_changed_unix_ms(),
        content,
    };
    validate_source_file_create_request_for_transport(&request)?;
    Ok(request)
}

fn write_source_file_create_result(
    result: &SourceFileCreateResult,
    mut builder: asset_capnp::source_file_create_result::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_create_result_for_transport(result)?;
    (result.record).to_capnp(builder.reborrow().init_record())
}

fn read_source_file_create_result(
    reader: asset_capnp::source_file_create_result::Reader<'_>,
) -> Result<SourceFileCreateResult, Error> {
    let result = SourceFileCreateResult {
        record: SourceAssetRecordResult::from_capnp(reader.get_record()?)?,
    };
    validate_source_file_create_result_for_transport(&result)?;
    Ok(result)
}

fn write_workspace_source_file_ref(
    source: &WorkspaceSourceFileRef,
    mut builder: asset_capnp::workspace_source_file_ref::Builder<'_>,
) -> Result<(), Error> {
    validate_workspace_source_file_ref_for_transport(source)?;
    builder.set_source_root_key(&source.source_root_key);
    builder.set_source_path(&source.source_path);
    builder.set_schema_type(&source.schema_type);
    Ok(())
}

fn read_workspace_source_file_ref(
    reader: asset_capnp::workspace_source_file_ref::Reader<'_>,
) -> Result<WorkspaceSourceFileRef, Error> {
    let source = WorkspaceSourceFileRef {
        source_root_key: reader.get_source_root_key()?.to_string()?,
        source_path: reader.get_source_path()?.to_string()?,
        schema_type: reader.get_schema_type()?.to_string()?,
    };
    validate_workspace_source_file_ref_for_transport(&source)?;
    Ok(source)
}

fn write_source_file_edit_request(
    request: &SourceFileEditRequest,
    mut builder: asset_capnp::source_file_edit_request::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_edit_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    write_workspace_source_file_ref(&request.source, builder.reborrow().init_source())?;
    builder.set_expected_source_fingerprint(&request.expected_source_fingerprint);
    request
        .operation
        .to_capnp(builder.reborrow().init_operation())
}

fn read_source_file_edit_request(
    reader: asset_capnp::source_file_edit_request::Reader<'_>,
) -> Result<SourceFileEditRequest, Error> {
    let request = SourceFileEditRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        source: read_workspace_source_file_ref(reader.get_source()?)?,
        expected_source_fingerprint: reader.get_expected_source_fingerprint()?.to_vec(),
        operation: SourceFileEditOperation::from_capnp(reader.get_operation()?)?,
    };
    validate_source_file_edit_request_for_transport(&request)?;
    Ok(request)
}

fn write_source_file_open_request(
    request: &SourceFileOpenRequest,
    mut builder: asset_capnp::source_file_open_request::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_open_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    write_workspace_source_file_ref(&request.source, builder.reborrow().init_source())
}

fn read_source_file_open_request(
    reader: asset_capnp::source_file_open_request::Reader<'_>,
) -> Result<SourceFileOpenRequest, Error> {
    let request = SourceFileOpenRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        source: read_workspace_source_file_ref(reader.get_source()?)?,
    };
    validate_source_file_open_request_for_transport(&request)?;
    Ok(request)
}

fn write_source_file_restore_request(
    request: &SourceFileRestoreRequest,
    mut builder: asset_capnp::source_file_restore_request::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_restore_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    write_workspace_source_file_ref(&request.source, builder.reborrow().init_source())?;
    builder.set_expected_source_fingerprint(&request.expected_source_fingerprint);
    request
        .document
        .to_capnp(builder.reborrow().init_document())
}

fn read_source_file_restore_request(
    reader: asset_capnp::source_file_restore_request::Reader<'_>,
) -> Result<SourceFileRestoreRequest, Error> {
    let request = SourceFileRestoreRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        source: read_workspace_source_file_ref(reader.get_source()?)?,
        expected_source_fingerprint: reader.get_expected_source_fingerprint()?.to_vec(),
        document: SourceFileEditDocument::from_capnp(reader.get_document()?)?,
    };
    validate_source_file_restore_request_for_transport(&request)?;
    Ok(request)
}

fn write_source_file_codec_request(
    request: &SourceFileCodecRequest,
    mut builder: asset_capnp::source_file_codec_request::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_codec_request_for_transport(request)?;
    write_workspace_source_file_ref(&request.source, builder.reborrow().init_source())?;
    core::SideChannelHandle::to_capnp(
        &request.authoritative_source,
        builder.reborrow().init_authoritative_source(),
    )?;
    request
        .operation
        .to_capnp(builder.reborrow().init_operation())?;
    write_source_file_codec_output_destination(
        &request.output_destination,
        builder.reborrow().init_output_destination(),
    )
}

fn read_source_file_codec_request(
    reader: asset_capnp::source_file_codec_request::Reader<'_>,
) -> Result<SourceFileCodecRequest, Error> {
    let request = SourceFileCodecRequest {
        source: read_workspace_source_file_ref(reader.get_source()?)?,
        authoritative_source: core::SideChannelHandle::from_capnp(
            reader.get_authoritative_source()?,
        )?,
        operation: SourceFileCodecOperation::from_capnp(reader.get_operation()?)?,
        output_destination: read_source_file_codec_output_destination(
            reader.get_output_destination()?,
        )?,
    };
    validate_source_file_codec_request_for_transport(&request)?;
    Ok(request)
}

fn write_source_file_codec_output_destination(
    destination: &SourceFileCodecOutputDestination,
    mut builder: asset_capnp::source_file_codec_output_destination::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_codec_output_destination_for_transport(destination)?;
    builder.set_locator(&destination.locator);
    builder.set_platform(&destination.platform);
    destination
        .capability
        .to_capnp(builder.reborrow().init_capability())
}

fn read_source_file_codec_output_destination(
    reader: asset_capnp::source_file_codec_output_destination::Reader<'_>,
) -> Result<SourceFileCodecOutputDestination, Error> {
    let destination = SourceFileCodecOutputDestination {
        locator: reader.get_locator()?.to_string()?,
        platform: reader.get_platform()?.to_string()?,
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_source_file_codec_output_destination_for_transport(&destination)?;
    Ok(destination)
}

fn write_source_file_codec_result(
    result: &SourceFileCodecResult,
    mut builder: asset_capnp::source_file_codec_result::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_codec_result_for_transport(result)?;
    result
        .document
        .to_capnp(builder.reborrow().init_document())?;
    match &result.replacement {
        SourceFileCodecReplacement::Unchanged => builder.set_unchanged(()),
        SourceFileCodecReplacement::SavedSource(handle) => {
            core::SideChannelHandle::to_capnp(handle, builder.reborrow().init_saved_source())?;
        }
    }
    Ok(())
}

fn read_source_file_codec_result(
    reader: asset_capnp::source_file_codec_result::Reader<'_>,
) -> Result<SourceFileCodecResult, Error> {
    let replacement = match reader.which()? {
        asset_capnp::source_file_codec_result::Unchanged(()) => {
            SourceFileCodecReplacement::Unchanged
        }
        asset_capnp::source_file_codec_result::SavedSource(handle) => {
            SourceFileCodecReplacement::SavedSource(Box::new(core::SideChannelHandle::from_capnp(
                handle?,
            )?))
        }
    };
    let result = SourceFileCodecResult {
        document: SourceFileEditDocument::from_capnp(reader.get_document()?)?,
        replacement,
    };
    validate_source_file_codec_result_for_transport(&result)?;
    Ok(result)
}

fn write_source_file_edit_snapshot(
    snapshot: &SourceFileEditSnapshot,
    mut builder: asset_capnp::source_file_edit_snapshot::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_edit_snapshot_for_transport(snapshot)?;
    write_workspace_source_file_ref(&snapshot.source, builder.reborrow().init_source())?;
    builder.set_source_fingerprint(&snapshot.source_fingerprint);
    snapshot
        .document
        .to_capnp(builder.reborrow().init_document())
}

fn read_source_file_edit_snapshot(
    reader: asset_capnp::source_file_edit_snapshot::Reader<'_>,
) -> Result<SourceFileEditSnapshot, Error> {
    let snapshot = SourceFileEditSnapshot {
        source: read_workspace_source_file_ref(reader.get_source()?)?,
        source_fingerprint: reader.get_source_fingerprint()?.to_vec(),
        document: SourceFileEditDocument::from_capnp(reader.get_document()?)?,
    };
    validate_source_file_edit_snapshot_for_transport(&snapshot)?;
    Ok(snapshot)
}

fn write_source_file_edit_result(
    result: &SourceFileEditResult,
    mut builder: asset_capnp::source_file_edit_result::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_edit_result_for_transport(result)?;
    write_source_file_edit_snapshot(&result.snapshot, builder.reborrow().init_snapshot())
}

fn read_source_file_edit_result(
    reader: asset_capnp::source_file_edit_result::Reader<'_>,
) -> Result<SourceFileEditResult, Error> {
    let result = SourceFileEditResult {
        snapshot: read_source_file_edit_snapshot(reader.get_snapshot()?)?,
    };
    validate_source_file_edit_result_for_transport(&result)?;
    Ok(result)
}

fn write_source_file_open_result(
    result: &SourceFileOpenResult,
    mut builder: asset_capnp::source_file_open_result::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_open_result_for_transport(result)?;
    write_source_file_edit_snapshot(&result.snapshot, builder.reborrow().init_snapshot())
}

fn read_source_file_open_result(
    reader: asset_capnp::source_file_open_result::Reader<'_>,
) -> Result<SourceFileOpenResult, Error> {
    let result = SourceFileOpenResult {
        snapshot: read_source_file_edit_snapshot(reader.get_snapshot()?)?,
    };
    validate_source_file_open_result_for_transport(&result)?;
    Ok(result)
}

fn write_source_file_restore_result(
    result: &SourceFileRestoreResult,
    mut builder: asset_capnp::source_file_restore_result::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_restore_result_for_transport(result)?;
    write_source_file_edit_snapshot(&result.snapshot, builder.reborrow().init_snapshot())
}

fn read_source_file_restore_result(
    reader: asset_capnp::source_file_restore_result::Reader<'_>,
) -> Result<SourceFileRestoreResult, Error> {
    let result = SourceFileRestoreResult {
        snapshot: read_source_file_edit_snapshot(reader.get_snapshot()?)?,
    };
    validate_source_file_restore_result_for_transport(&result)?;
    Ok(result)
}

fn write_source_file_delete_request(
    request: &SourceFileDeleteRequest,
    mut builder: asset_capnp::source_file_delete_request::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_delete_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    builder.set_source_root(&request.source_root);
    builder.set_source_path(&request.source_path);
    builder.set_changed_unix_ms(request.changed_unix_ms);
    Ok(())
}

fn read_source_file_delete_request(
    reader: asset_capnp::source_file_delete_request::Reader<'_>,
) -> Result<SourceFileDeleteRequest, Error> {
    let request = SourceFileDeleteRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        source_path: reader.get_source_path()?.to_string()?,
        changed_unix_ms: reader.get_changed_unix_ms(),
    };
    validate_source_file_delete_request_for_transport(&request)?;
    Ok(request)
}

fn write_source_file_delete_result(
    result: &SourceFileDeleteResult,
    mut builder: asset_capnp::source_file_delete_result::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_delete_result_for_transport(result)?;
    (result.record).to_capnp(builder.reborrow().init_record())
}

fn read_source_file_delete_result(
    reader: asset_capnp::source_file_delete_result::Reader<'_>,
) -> Result<SourceFileDeleteResult, Error> {
    let result = SourceFileDeleteResult {
        record: SourceAssetRecordResult::from_capnp(reader.get_record()?)?,
    };
    validate_source_file_delete_result_for_transport(&result)?;
    Ok(result)
}

fn write_source_file_move_request(
    request: &SourceFileMoveRequest,
    mut builder: asset_capnp::source_file_move_request::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_move_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    builder.set_source_root(&request.source_root);
    builder.set_from_source_path(&request.from_source_path);
    builder.set_to_source_path(&request.to_source_path);
    builder.set_changed_unix_ms(request.changed_unix_ms);
    Ok(())
}

fn read_source_file_move_request(
    reader: asset_capnp::source_file_move_request::Reader<'_>,
) -> Result<SourceFileMoveRequest, Error> {
    let request = SourceFileMoveRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        from_source_path: reader.get_from_source_path()?.to_string()?,
        to_source_path: reader.get_to_source_path()?.to_string()?,
        changed_unix_ms: reader.get_changed_unix_ms(),
    };
    validate_source_file_move_request_for_transport(&request)?;
    Ok(request)
}

fn write_source_file_move_result(
    result: &SourceFileMoveResult,
    mut builder: asset_capnp::source_file_move_result::Builder<'_>,
) -> Result<(), Error> {
    validate_source_file_move_result_for_transport(result)?;
    (result.record).to_capnp(builder.reborrow().init_record())?;
    builder.set_old_source_path(&result.old_source_path);
    Ok(())
}

fn read_source_file_move_result(
    reader: asset_capnp::source_file_move_result::Reader<'_>,
) -> Result<SourceFileMoveResult, Error> {
    let result = SourceFileMoveResult {
        record: SourceAssetRecordResult::from_capnp(reader.get_record()?)?,
        old_source_path: reader.get_old_source_path()?.to_string()?,
    };
    validate_source_file_move_result_for_transport(&result)?;
    Ok(result)
}

fn write_force_reprocess_asset_request(
    request: &ForceReprocessAssetRequest,
    mut builder: asset_capnp::force_reprocess_asset_request::Builder<'_>,
) -> Result<(), Error> {
    validate_force_reprocess_asset_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    builder.set_source_root(&request.source_root);
    builder.set_source_path(&request.source_path);
    Ok(())
}

fn read_force_reprocess_asset_request(
    reader: asset_capnp::force_reprocess_asset_request::Reader<'_>,
) -> Result<ForceReprocessAssetRequest, Error> {
    let request = ForceReprocessAssetRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        source_path: reader.get_source_path()?.to_string()?,
    };
    validate_force_reprocess_asset_request_for_transport(&request)?;
    Ok(request)
}

fn write_force_reprocess_asset_result(
    result: &ForceReprocessAssetResult,
    mut builder: asset_capnp::force_reprocess_asset_result::Builder<'_>,
) -> Result<(), Error> {
    validate_force_reprocess_asset_result_for_transport(result)?;
    (result.record).to_capnp(builder.reborrow().init_record())?;
    builder.set_enqueued_jobs(result.enqueued_jobs);
    Ok(())
}

fn read_force_reprocess_asset_result(
    reader: asset_capnp::force_reprocess_asset_result::Reader<'_>,
) -> Result<ForceReprocessAssetResult, Error> {
    let result = ForceReprocessAssetResult {
        record: SourceAssetRecordResult::from_capnp(reader.get_record()?)?,
        enqueued_jobs: reader.get_enqueued_jobs(),
    };
    validate_force_reprocess_asset_result_for_transport(&result)?;
    Ok(result)
}

fn write_reconcile_asset_sources_request(
    request: &ReconcileAssetSourcesRequest,
    mut builder: asset_capnp::reconcile_asset_sources_request::Builder<'_>,
) -> Result<(), Error> {
    validate_reconcile_asset_sources_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    builder.set_root_scope(request.root_scope.to_capnp());
    Ok(())
}

fn read_reconcile_asset_sources_request(
    reader: asset_capnp::reconcile_asset_sources_request::Reader<'_>,
) -> Result<ReconcileAssetSourcesRequest, Error> {
    let request = ReconcileAssetSourcesRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        root_scope: AssetRootScope::from_capnp(reader.get_root_scope()?),
    };
    validate_reconcile_asset_sources_request_for_transport(&request)?;
    Ok(request)
}

fn write_reconcile_asset_sources_result(
    result: &ReconcileAssetSourcesResult,
    mut builder: asset_capnp::reconcile_asset_sources_result::Builder<'_>,
) {
    validate_reconcile_asset_sources_result_for_transport(result);
    builder.set_source_root_count(result.source_root_count);
    builder.set_recorded_source_asset_count(result.recorded_source_asset_count);
    builder.set_deleted_source_asset_count(result.deleted_source_asset_count);
}

fn read_reconcile_asset_sources_result(
    reader: asset_capnp::reconcile_asset_sources_result::Reader<'_>,
) -> ReconcileAssetSourcesResult {
    let result = ReconcileAssetSourcesResult {
        source_root_count: reader.get_source_root_count(),
        recorded_source_asset_count: reader.get_recorded_source_asset_count(),
        deleted_source_asset_count: reader.get_deleted_source_asset_count(),
    };
    validate_reconcile_asset_sources_result_for_transport(&result);
    result
}

fn write_asset_processing_status_request(
    request: &AssetProcessingStatusRequest,
    mut builder: asset_capnp::asset_processing_status_request::Builder<'_>,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "asset processing status request",
        &request.capability,
    )?;
    require_non_empty_text(
        "asset processing status request",
        "session id",
        &request.session_id,
    )?;
    require_non_empty_text(
        "asset processing status request",
        "platform",
        &request.platform,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    builder.set_platform(&request.platform);
    Ok(())
}

fn read_asset_processing_status_request(
    reader: asset_capnp::asset_processing_status_request::Reader<'_>,
) -> Result<AssetProcessingStatusRequest, Error> {
    let request = AssetProcessingStatusRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        platform: reader.get_platform()?.to_string()?,
    };
    let mut message = message::Builder::new_default();
    write_asset_processing_status_request(
        &request,
        message.init_root::<asset_capnp::asset_processing_status_request::Builder<'_>>(),
    )?;
    Ok(request)
}

fn write_asset_processing_status_result(
    result: &AssetProcessingStatusResult,
    mut builder: asset_capnp::asset_processing_status_result::Builder<'_>,
) {
    builder.set_queued(result.queued);
    builder.set_leased(result.leased);
    builder.set_failed(result.failed);
    builder.set_in_flight_sweeps(result.in_flight_sweeps);
}

fn read_asset_processing_status_result(
    reader: asset_capnp::asset_processing_status_result::Reader<'_>,
) -> AssetProcessingStatusResult {
    AssetProcessingStatusResult {
        queued: reader.get_queued(),
        leased: reader.get_leased(),
        failed: reader.get_failed(),
        in_flight_sweeps: reader.get_in_flight_sweeps(),
    }
}

fn write_publish_asset_catalog_request(
    request: &PublishAssetCatalogRequest,
    mut builder: asset_capnp::publish_asset_catalog_request::Builder<'_>,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "publish asset catalog request",
        &request.capability,
    )?;
    require_non_empty_text(
        "publish asset catalog request",
        "session id",
        &request.session_id,
    )?;
    require_non_empty_text(
        "publish asset catalog request",
        "platform",
        &request.platform,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    builder.set_platform(&request.platform);
    Ok(())
}

fn read_publish_asset_catalog_request(
    reader: asset_capnp::publish_asset_catalog_request::Reader<'_>,
) -> Result<PublishAssetCatalogRequest, Error> {
    let request = PublishAssetCatalogRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        platform: reader.get_platform()?.to_string()?,
    };
    let mut message = message::Builder::new_default();
    write_publish_asset_catalog_request(
        &request,
        message.init_root::<asset_capnp::publish_asset_catalog_request::Builder<'_>>(),
    )?;
    Ok(request)
}

fn write_publish_asset_catalog_result(
    result: &PublishAssetCatalogResult,
    mut builder: asset_capnp::publish_asset_catalog_result::Builder<'_>,
) -> Result<(), Error> {
    require_non_empty_text(
        "publish asset catalog result",
        "catalog path",
        &result.catalog_path,
    )?;
    builder.set_catalog_path(&result.catalog_path);
    builder.set_entry_count(result.entry_count);
    builder.set_reused(result.reused);
    Ok(())
}

fn read_publish_asset_catalog_result(
    reader: asset_capnp::publish_asset_catalog_result::Reader<'_>,
) -> Result<PublishAssetCatalogResult, Error> {
    let result = PublishAssetCatalogResult {
        catalog_path: reader.get_catalog_path()?.to_string()?,
        entry_count: reader.get_entry_count(),
        reused: reader.get_reused(),
    };
    require_non_empty_text(
        "publish asset catalog result",
        "catalog path",
        &result.catalog_path,
    )?;
    Ok(result)
}

fn write_publish_builder_catalog_request(
    request: &PublishBuilderCatalogRequest,
    mut builder: asset_capnp::publish_builder_catalog_request::Builder<'_>,
) -> Result<(), Error> {
    validate_asset_jobs_capability_request_for_transport(
        "publish builder catalog request",
        &request.capability,
    )?;
    validate_asset_builder_catalog_result_for_transport(&request.catalog)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    request
        .protocol
        .to_capnp(builder.reborrow().init_protocol());
    request
        .catalog
        .to_capnp(builder.reborrow().init_catalog())?;
    Ok(())
}

fn read_publish_builder_catalog_request(
    reader: asset_capnp::publish_builder_catalog_request::Reader<'_>,
) -> Result<PublishBuilderCatalogRequest, Error> {
    let request = PublishBuilderCatalogRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        protocol: core::ProtocolVersion::from_capnp(reader.get_protocol()?),
        catalog: AssetBuilderCatalogResult::from_capnp(reader.get_catalog()?)?,
    };
    let mut message = message::Builder::new_default();
    write_publish_builder_catalog_request(
        &request,
        message.init_root::<asset_capnp::publish_builder_catalog_request::Builder<'_>>(),
    )?;
    Ok(request)
}

fn write_publish_builder_catalog_result(
    result: &PublishBuilderCatalogResult,
    mut builder: asset_capnp::publish_builder_catalog_result::Builder<'_>,
) {
    builder.set_accepted(result.accepted);
}

fn read_publish_builder_catalog_result(
    reader: asset_capnp::publish_builder_catalog_result::Reader<'_>,
) -> PublishBuilderCatalogResult {
    PublishBuilderCatalogResult {
        accepted: reader.get_accepted(),
    }
}

fn write_source_dependent_source(
    dependent: &SourceDependentSource,
    mut builder: asset_capnp::source_dependent_source::Builder<'_>,
) -> Result<(), Error> {
    validate_source_dependent_source_for_transport(dependent)?;
    builder.set_source_edge_id(dependent.source_edge_id);
    builder.set_source_path(&dependent.source_path);
    builder.set_builder_guid(dependent.builder_guid.as_bytes());
    builder.set_relation(dependent.relation.to_capnp());
    Ok(())
}

fn read_source_dependent_source(
    reader: asset_capnp::source_dependent_source::Reader<'_>,
) -> Result<SourceDependentSource, Error> {
    let dependent = SourceDependentSource {
        source_edge_id: reader.get_source_edge_id(),
        source_path: reader.get_source_path()?.to_string()?,
        builder_guid: read_uuid_data(reader.get_builder_guid()?)?,
        relation: reader
            .get_relation()
            .map(SourceRelation::from_capnp)
            .map_err(|error| Error::failed(format!("unknown source relation: {error:?}")))?,
    };
    validate_source_dependent_source_for_transport(&dependent)?;
    Ok(dependent)
}

fn write_source_dependent_job(
    dependent: &SourceDependentJob,
    mut builder: asset_capnp::source_dependent_job::Builder<'_>,
) -> Result<(), Error> {
    validate_source_dependent_job_for_transport(dependent)?;
    builder.set_job_edge_id(dependent.job_edge_id);
    builder.set_job_id(dependent.job_id);
    write_optional_i64(
        dependent.latest_attempt_id,
        builder.reborrow().init_latest_attempt_id(),
    );
    builder.set_source_path(&dependent.source_path);
    write_job_owner(&dependent.owner, builder.reborrow().init_owner());
    builder.set_job_key(&dependent.job_key);
    builder.set_platform(&dependent.platform);
    builder.set_dependency_job_key(&dependent.dependency_job_key);
    builder.set_dependency_platform(&dependent.dependency_platform);
    builder.set_kind(dependent.kind.to_capnp());
    let mut product_paths = builder
        .reborrow()
        .init_product_paths(capnp_list_index(dependent.product_paths.len())?);
    for (index, product_path) in dependent.product_paths.iter().enumerate() {
        product_paths
            .reborrow()
            .set(capnp_list_index(index)?, product_path);
    }
    Ok(())
}

fn read_source_dependent_job(
    reader: asset_capnp::source_dependent_job::Reader<'_>,
) -> Result<SourceDependentJob, Error> {
    let dependent = SourceDependentJob {
        job_edge_id: reader.get_job_edge_id(),
        job_id: reader.get_job_id(),
        latest_attempt_id: read_optional_i64(reader.get_latest_attempt_id()?)?,
        source_path: reader.get_source_path()?.to_string()?,
        owner: read_job_owner(reader.get_owner()?)?,
        job_key: reader.get_job_key()?.to_string()?,
        platform: reader.get_platform()?.to_string()?,
        dependency_job_key: reader.get_dependency_job_key()?.to_string()?,
        dependency_platform: reader.get_dependency_platform()?.to_string()?,
        kind: reader
            .get_kind()
            .map(JobDependencyKind::from_capnp)
            .map_err(|error| Error::failed(format!("unknown job dependency kind: {error:?}")))?,
        product_paths: reader
            .get_product_paths()?
            .iter()
            .map(|path| Ok(path?.to_string()?))
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_source_dependent_job_for_transport(&dependent)?;
    Ok(dependent)
}

fn write_source_dependents_request(
    request: &SourceDependentsRequest,
    mut builder: asset_capnp::source_dependents_request::Builder<'_>,
) -> Result<(), Error> {
    validate_source_dependents_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_session_id(&request.session_id);
    builder.set_source_root(&request.source_root);
    builder.set_source_path(&request.source_path);
    Ok(())
}

fn read_source_dependents_request(
    reader: asset_capnp::source_dependents_request::Reader<'_>,
) -> Result<SourceDependentsRequest, Error> {
    let request = SourceDependentsRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        session_id: reader.get_session_id()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        source_path: reader.get_source_path()?.to_string()?,
    };
    validate_source_dependents_request_for_transport(&request)?;
    Ok(request)
}

fn write_source_dependents_result(
    result: &SourceDependentsResult,
    mut builder: asset_capnp::source_dependents_result::Builder<'_>,
) -> Result<(), Error> {
    validate_source_dependents_result_for_transport(result)?;
    builder.set_source_path(&result.source_path);
    let mut source_dependents = builder
        .reborrow()
        .init_source_dependents(capnp_list_index(result.source_dependents.len())?);
    for (index, dependent) in result.source_dependents.iter().enumerate() {
        (dependent).to_capnp(source_dependents.reborrow().get(capnp_list_index(index)?))?;
    }
    let mut job_dependents = builder
        .reborrow()
        .init_job_dependents(capnp_list_index(result.job_dependents.len())?);
    for (index, dependent) in result.job_dependents.iter().enumerate() {
        (dependent).to_capnp(job_dependents.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_source_dependents_result(
    reader: asset_capnp::source_dependents_result::Reader<'_>,
) -> Result<SourceDependentsResult, Error> {
    let result = SourceDependentsResult {
        source_path: reader.get_source_path()?.to_string()?,
        source_dependents: reader
            .get_source_dependents()?
            .iter()
            .map(read_source_dependent_source)
            .collect::<Result<Vec<_>, Error>>()?,
        job_dependents: reader
            .get_job_dependents()?
            .iter()
            .map(read_source_dependent_job)
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_source_dependents_result_for_transport(&result)?;
    Ok(result)
}

fn write_lease_asset_job_request(
    request: &LeaseAssetJobRequest,
    mut builder: asset_capnp::lease_asset_job_request::Builder<'_>,
) -> Result<(), Error> {
    validate_lease_asset_job_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_lease_owner(&request.lease_owner);
    builder.set_lease_duration_ms(request.lease_duration_ms);
    write_optional_text(
        request.staging_root.as_deref(),
        builder.reborrow().init_staging_root(),
    );
    Ok(())
}

fn read_lease_asset_job_request(
    reader: asset_capnp::lease_asset_job_request::Reader<'_>,
) -> Result<LeaseAssetJobRequest, Error> {
    let request = LeaseAssetJobRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        lease_owner: reader.get_lease_owner()?.to_string()?,
        lease_duration_ms: reader.get_lease_duration_ms(),
        staging_root: read_optional_text(reader.get_staging_root()?)?,
    };
    validate_lease_asset_job_request_for_transport(&request)?;
    Ok(request)
}

fn write_lease_asset_job_result(
    result: &LeaseAssetJobResult,
    mut builder: asset_capnp::lease_asset_job_result::Builder<'_>,
) -> Result<(), Error> {
    if result.grant_key.is_nil() {
        return Err(invalid_asset_protocol_value(
            "lease asset job result",
            "grant key must not be nil",
        ));
    }
    builder.set_grant_key(result.grant_key.as_bytes());
    result.leased.to_capnp(builder.init_leased())?;
    Ok(())
}

fn read_lease_asset_job_result(
    reader: asset_capnp::lease_asset_job_result::Reader<'_>,
) -> Result<LeaseAssetJobResult, Error> {
    let result = LeaseAssetJobResult {
        leased: LeasedAssetJob::from_capnp(reader.get_leased()?)?,
        grant_key: read_uuid_data(reader.get_grant_key()?)?,
    };
    if result.grant_key.is_nil() {
        return Err(invalid_asset_protocol_value(
            "lease asset job result",
            "grant key must not be nil",
        ));
    }
    Ok(result)
}

fn write_renew_asset_job_lease_request(
    request: &RenewAssetJobLeaseRequest,
    mut builder: asset_capnp::renew_asset_job_lease_request::Builder<'_>,
) -> Result<(), Error> {
    validate_renew_asset_job_lease_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_asset_job_attempt_id(request.asset_job_attempt_id);
    builder.set_lease_owner(&request.lease_owner);
    builder.set_grant_key(request.grant_key.as_bytes());
    Ok(())
}

fn read_renew_asset_job_lease_request(
    reader: asset_capnp::renew_asset_job_lease_request::Reader<'_>,
) -> Result<RenewAssetJobLeaseRequest, Error> {
    let request = RenewAssetJobLeaseRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        asset_job_attempt_id: reader.get_asset_job_attempt_id(),
        lease_owner: reader.get_lease_owner()?.to_string()?,
        grant_key: read_uuid_data(reader.get_grant_key()?)?,
    };
    validate_renew_asset_job_lease_request_for_transport(&request)?;
    Ok(request)
}

fn write_complete_asset_job_attempt_request(
    request: &CompleteAssetJobAttemptRequest,
    mut builder: asset_capnp::complete_asset_job_attempt_request::Builder<'_>,
) -> Result<(), Error> {
    validate_complete_asset_job_attempt_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    builder.set_asset_job_attempt_id(request.asset_job_attempt_id);
    builder.set_lease_owner(&request.lease_owner);
    builder.set_status(request.status.to_capnp());
    builder.set_finished_unix_ms(request.finished_unix_ms);
    builder.set_error_count(request.error_count);
    builder.set_warning_count(request.warning_count);
    builder.set_grant_key(request.grant_key.as_bytes());
    if let Some(handle) = &request.product_manifest {
        core::SideChannelHandle::to_capnp(handle, builder.init_product_manifest())?;
    } else {
        builder.set_no_product_manifest(());
    }
    Ok(())
}

fn read_complete_asset_job_attempt_request(
    reader: asset_capnp::complete_asset_job_attempt_request::Reader<'_>,
) -> Result<CompleteAssetJobAttemptRequest, Error> {
    let status = reader
        .get_status()
        .map(AttemptStatus::from_capnp)
        .map_err(|error| Error::failed(format!("unknown asset job attempt status: {error:?}")))?;
    let request = CompleteAssetJobAttemptRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        asset_job_attempt_id: reader.get_asset_job_attempt_id(),
        lease_owner: reader.get_lease_owner()?.to_string()?,
        status,
        finished_unix_ms: reader.get_finished_unix_ms(),
        error_count: reader.get_error_count(),
        warning_count: reader.get_warning_count(),
        grant_key: read_uuid_data(reader.get_grant_key()?)?,
        product_manifest: match reader.which()? {
            asset_capnp::complete_asset_job_attempt_request::Which::NoProductManifest(()) => None,
            asset_capnp::complete_asset_job_attempt_request::Which::ProductManifest(handle) => {
                Some(core::SideChannelHandle::from_capnp(handle?)?)
            }
        },
    };
    validate_complete_asset_job_attempt_request_for_transport(&request)?;
    Ok(request)
}

fn write_product_manifest(
    manifest: &ProductManifest,
    mut builder: asset_capnp::product_manifest::Builder<'_>,
) -> Result<(), Error> {
    builder.set_schema_version(manifest.schema_version);
    builder.set_generated_unix_ms(manifest.generated_unix_ms);
    let mut products = builder
        .reborrow()
        .init_products(capnp_list_index(manifest.products.len())?);
    for (index, product) in manifest.products.iter().enumerate() {
        let mut product_builder = products.reborrow().get(capnp_list_index(index)?);
        product_builder.set_product_path(&product.product_path);
        product_builder.set_asset_type(product.asset_type.as_bytes());
        product_builder.set_sub_id(product.sub_id);
        product_builder.set_product_format(&product.product_format);
        product_builder.set_product_format_version(product.product_format_version);
        product_builder.set_staged_path(&product.staged_path);
        product_builder.set_content_hash(&product.content_hash);
        product_builder.set_byte_length(product.byte_length);
        product_builder.set_catalog_path_registration(product.catalog_path_registration.to_capnp());
        let mut aliases = product_builder
            .reborrow()
            .init_catalog_aliases(capnp_list_index(product.catalog_aliases.len())?);
        for (alias_index, alias) in product.catalog_aliases.iter().enumerate() {
            aliases.set(capnp_list_index(alias_index)?, alias);
        }
        let mut dependencies = product_builder
            .reborrow()
            .init_dependencies(capnp_list_index(product.dependencies.len())?);
        for (dependency_index, dependency) in product.dependencies.iter().enumerate() {
            let mut dependency_builder = dependencies
                .reborrow()
                .get(capnp_list_index(dependency_index)?);
            dependency_builder.set_asset_guid(dependency.asset_guid.as_bytes());
            dependency_builder.set_sub_id(dependency.sub_id);
            dependency_builder.set_flags(dependency.flags);
        }
    }
    Ok(())
}

fn read_product_manifest(
    reader: asset_capnp::product_manifest::Reader<'_>,
) -> Result<ProductManifest, Error> {
    let products = reader
        .get_products()?
        .iter()
        .map(|product| {
            Ok(ProductManifestProduct {
                product_path: product.get_product_path()?.to_string()?,
                catalog_aliases: product
                    .get_catalog_aliases()?
                    .iter()
                    .map(|alias| -> Result<String, Error> { Ok(alias?.to_string()?) })
                    .collect::<Result<Vec<_>, _>>()?,
                catalog_path_registration: product
                    .get_catalog_path_registration()
                    .map(CatalogPathRegistration::from_capnp)
                    .map_err(|error| {
                        Error::failed(format!("unknown catalog path registration: {error:?}"))
                    })?,
                asset_type: read_uuid_data(product.get_asset_type()?)?,
                sub_id: product.get_sub_id(),
                product_format: product.get_product_format()?.to_string()?,
                product_format_version: product.get_product_format_version(),
                staged_path: product.get_staged_path()?.to_string()?,
                content_hash: product.get_content_hash()?.to_vec(),
                byte_length: product.get_byte_length(),
                dependencies: product
                    .get_dependencies()?
                    .iter()
                    .map(|dependency| {
                        Ok(ProductManifestProductDependency {
                            asset_guid: read_uuid_data(dependency.get_asset_guid()?)?,
                            sub_id: dependency.get_sub_id(),
                            flags: dependency.get_flags(),
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let manifest = ProductManifest {
        schema_version: reader.get_schema_version(),
        generated_unix_ms: reader.get_generated_unix_ms(),
        products,
    };
    validate_product_manifest_for_transport(&manifest)?;
    Ok(manifest)
}

/// Serializes the product-manifest side-channel blob payload.
/// # Errors
///
/// Returns an error if the manifest fails its transport validation, or if the
/// message runs out of space while writing the manifest.
pub fn encode_product_manifest(manifest: &ProductManifest) -> Result<Vec<u8>, Error> {
    validate_product_manifest_for_transport(manifest)?;

    let mut message = message::Builder::new_default();
    write_product_manifest(
        manifest,
        message.init_root::<asset_capnp::product_manifest::Builder<'_>>(),
    )?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// # Errors
///
/// Returns an error if the staging file behind `handle` cannot be read or fails
/// its hash verification, if the bytes are not a readable packed Cap'n Proto
/// message, or if a field of the manifest is absent from the message or is not
/// valid UTF-8.
pub fn load_product_manifest_side_channel(
    handle: &SideChannelHandle,
) -> Result<ProductManifest, ProductManifestSideChannelError> {
    let file = core::read_verified_staging_file(handle)?;
    let message = serialize_packed::read_message(
        &mut Cursor::new(file.bytes),
        message::ReaderOptions::new(),
    )?;
    let manifest = ProductManifest::from_capnp(
        message.get_root::<asset_capnp::product_manifest::Reader<'_>>()?,
    )?;
    Ok(manifest)
}

fn validate_asset_read_capability_request_for_transport(
    kind: &str,
    capability: &Capability,
) -> Result<(), Error> {
    validate_asset_capability_request_for_transport(
        kind,
        capability,
        ASSET_READ_PERMISSION,
        &[
            core::ServiceRole::Editor,
            core::ServiceRole::ProjectHost,
            core::ServiceRole::SessionSupervisor,
            core::ServiceRole::AssetProcessor,
            core::ServiceRole::Worker,
        ],
    )
}

fn validate_asset_write_capability_request_for_transport(
    kind: &str,
    capability: &Capability,
) -> Result<(), Error> {
    validate_asset_capability_request_for_transport(
        kind,
        capability,
        ASSET_WRITE_PERMISSION,
        &[
            core::ServiceRole::Editor,
            core::ServiceRole::ProjectHost,
            core::ServiceRole::SessionSupervisor,
            core::ServiceRole::AssetProcessor,
        ],
    )
}

fn validate_asset_jobs_capability_request_for_transport(
    kind: &str,
    capability: &Capability,
) -> Result<(), Error> {
    validate_asset_capability_request_for_transport(
        kind,
        capability,
        ASSET_JOBS_PERMISSION,
        &[core::ServiceRole::Worker],
    )?;
    if capability.service.namespace != ASSET_WORKER_SERVICE_NAMESPACE
        || capability.service.name != ASSET_WORKER_SERVICE_NAME
    {
        return Err(invalid_asset_protocol_value(
            kind,
            format!(
                "asset jobs capability service must be `{ASSET_WORKER_SERVICE_NAMESPACE}/{ASSET_WORKER_SERVICE_NAME}`, got `{}/{}`",
                capability.service.namespace, capability.service.name
            ),
        ));
    }
    Ok(())
}

fn validate_asset_capability_request_for_transport(
    kind: &str,
    capability: &Capability,
    required_permission: &'static str,
    allowed_roles: &[core::ServiceRole],
) -> Result<(), Error> {
    if capability.is_empty() {
        return Err(invalid_asset_protocol_value(
            kind,
            "capability cannot be empty",
        ));
    }
    if capability.audience != ASSET_PROCESSOR_AUDIENCE {
        return Err(invalid_asset_protocol_value(
            kind,
            format!(
                "capability audience must be `{ASSET_PROCESSOR_AUDIENCE}`, got `{}`",
                capability.audience
            ),
        ));
    }
    if !allowed_roles.iter().any(|role| role == &capability.role) {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("capability role {:?} is not allowed", capability.role),
        ));
    }
    if !capability.has_permissions(&[required_permission]) {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("capability missing `{required_permission}`"),
        ));
    }
    capability
        .validate_lifetime()
        .map_err(|error| invalid_asset_protocol_value(kind, error.to_string()))?;
    Ok(())
}

fn validate_asset_builder_pattern_descriptor_for_transport(
    descriptor: &AssetBuilderPatternDescriptor,
) -> Result<(), Error> {
    require_non_empty_text("asset builder pattern", "pattern", &descriptor.pattern)
}

fn validate_asset_builder_catalog_request_for_transport(
    request: &AssetBuilderCatalogRequest,
) -> Result<(), Error> {
    validate_asset_read_capability_request_for_transport(
        "asset builder catalog request",
        &request.capability,
    )
}

fn validate_asset_builder_descriptor_for_transport(
    descriptor: &AssetBuilderDescriptor,
) -> Result<(), Error> {
    require_non_empty_text("asset builder descriptor", "name", &descriptor.name)?;
    require_non_nil_uuid(
        "asset builder descriptor",
        "builder guid",
        0,
        descriptor.builder_guid,
    )?;
    if descriptor.version == 0 {
        return Err(invalid_asset_protocol_value(
            "asset builder descriptor",
            format!("builder `{}` version must be positive", descriptor.name),
        ));
    }
    if descriptor.patterns.is_empty() {
        return Err(invalid_asset_protocol_value(
            "asset builder descriptor",
            format!(
                "builder `{}` must declare at least one pattern",
                descriptor.name
            ),
        ));
    }

    let mut seen_patterns = BTreeSet::new();
    for pattern in &descriptor.patterns {
        let pattern_key = (
            asset_builder_pattern_kind_key(pattern.kind),
            pattern.pattern.as_str(),
        );
        if !seen_patterns.insert(pattern_key) {
            return Err(invalid_asset_protocol_value(
                "asset builder descriptor",
                format!(
                    "builder `{}` contains duplicate {:?} pattern `{}`",
                    descriptor.name, pattern.kind, pattern.pattern
                ),
            ));
        }
    }
    let mut seen_source_schema_types = BTreeSet::new();
    for source_schema_type in &descriptor.source_schema_types {
        require_non_empty_text(
            "asset builder descriptor",
            "source schema type",
            source_schema_type,
        )?;
        if !seen_source_schema_types.insert(source_schema_type.as_str()) {
            return Err(invalid_asset_protocol_value(
                "asset builder descriptor",
                format!(
                    "builder `{}` contains duplicate source schema type `{}`",
                    descriptor.name, source_schema_type
                ),
            ));
        }
    }
    Ok(())
}

fn validate_source_schema_descriptor_for_transport(
    descriptor: &SourceSchemaDescriptor,
) -> Result<(), Error> {
    require_non_empty_text(
        "source schema descriptor",
        "source schema type",
        &descriptor.schema_type,
    )?;
    if descriptor.owner.trim() != descriptor.owner {
        return Err(invalid_asset_protocol_value(
            "source schema descriptor",
            format!(
                "source schema `{}` owner must not have leading or trailing whitespace",
                descriptor.schema_type
            ),
        ));
    }
    if descriptor.label.trim() != descriptor.label {
        return Err(invalid_asset_protocol_value(
            "source schema descriptor",
            format!(
                "source schema `{}` label must not have leading or trailing whitespace",
                descriptor.schema_type
            ),
        ));
    }
    if descriptor.category.trim() != descriptor.category {
        return Err(invalid_asset_protocol_value(
            "source schema descriptor",
            format!(
                "source schema `{}` category must not have leading or trailing whitespace",
                descriptor.schema_type
            ),
        ));
    }
    validate_source_schema_authoring_for_transport(&descriptor.authoring)?;
    match &descriptor.authoring {
        SourceSchemaAuthoring::File { .. } | SourceSchemaAuthoring::ProjectDocument { .. }
            if descriptor.file_templates.is_empty() => {}
        SourceSchemaAuthoring::File { workflow } if workflow.can_create => {
            for template in &descriptor.file_templates {
                validate_source_file_template_matches_workflow(
                    &descriptor.schema_type,
                    template,
                    workflow,
                )?;
            }
        }
        SourceSchemaAuthoring::File { .. } => {
            return Err(invalid_asset_protocol_value(
                "source schema descriptor",
                format!(
                    "source schema `{}` exposes source file templates but is not default-creatable",
                    descriptor.schema_type
                ),
            ));
        }
        SourceSchemaAuthoring::ProjectDocument { .. } => {
            return Err(invalid_asset_protocol_value(
                "source schema descriptor",
                format!(
                    "source schema `{}` exposes source file templates but is project-document backed",
                    descriptor.schema_type
                ),
            ));
        }
    }
    let mut seen_template_paths = BTreeSet::new();
    for template in &descriptor.file_templates {
        validate_source_file_template_descriptor_for_transport(template)?;
        if !seen_template_paths.insert(template.source_path.as_str()) {
            return Err(invalid_asset_protocol_value(
                "source schema descriptor",
                format!(
                    "source schema `{}` repeats source file template path `{}`",
                    descriptor.schema_type, template.source_path
                ),
            ));
        }
    }
    Ok(())
}

fn validate_source_file_template_matches_workflow(
    schema_type: &str,
    template: &SourceFileTemplateDescriptor,
    workflow: &SourceFileWorkflowDescriptor,
) -> Result<(), Error> {
    if source_path_matches_source_file_extensions(&template.source_path, &workflow.extensions) {
        Ok(())
    } else {
        Err(invalid_asset_protocol_value(
            "source schema descriptor",
            format!(
                "source schema `{schema_type}` template `{}` does not match workflow extensions {:?}",
                template.source_path, workflow.extensions
            ),
        ))
    }
}

fn validate_source_file_template_descriptor_for_transport(
    descriptor: &SourceFileTemplateDescriptor,
) -> Result<(), Error> {
    validate_source_file_template_source_path(&descriptor.source_path)?;
    for (field, value) in [
        ("owner", descriptor.owner.as_str()),
        ("label", descriptor.label.as_str()),
        ("description", descriptor.description.as_str()),
    ] {
        if value.trim() != value {
            return Err(invalid_asset_protocol_value(
                "source file template",
                format!("{field} must not have leading or trailing whitespace"),
            ));
        }
    }
    Ok(())
}

fn source_path_matches_source_file_extensions(source_path: &str, extensions: &[String]) -> bool {
    let file_name = source_path
        .rsplit('/')
        .next()
        .unwrap_or(source_path)
        .to_ascii_lowercase();
    extensions.iter().any(|extension| {
        let extension = extension.to_ascii_lowercase();
        extension == "*" || file_name == extension || file_name.ends_with(&format!(".{extension}"))
    })
}

fn validate_source_schema_authoring_for_transport(
    authoring: &SourceSchemaAuthoring,
) -> Result<(), Error> {
    match authoring {
        SourceSchemaAuthoring::File { workflow } => {
            validate_source_file_workflow_descriptor_for_transport(workflow)
        }
        SourceSchemaAuthoring::ProjectDocument { schema_type } => require_non_empty_text(
            "source schema authoring",
            "project document schema type",
            schema_type,
        ),
    }
}

fn validate_source_file_workflow_descriptor_for_transport(
    workflow: &SourceFileWorkflowDescriptor,
) -> Result<(), Error> {
    require_non_empty_text("source file workflow", "source root", &workflow.source_root)?;
    if workflow.source_root.trim() != workflow.source_root {
        return Err(invalid_asset_protocol_value(
            "source file workflow",
            "source root must not have leading or trailing whitespace",
        ));
    }
    validate_source_file_default_path_prefix(&workflow.default_path_prefix)?;
    if workflow.extensions.is_empty() {
        return Err(invalid_asset_protocol_value(
            "source file workflow",
            "extensions cannot be empty",
        ));
    }

    let mut seen_extensions = BTreeSet::new();
    for extension in &workflow.extensions {
        validate_source_file_extension(extension)?;
        if !seen_extensions.insert(extension.as_str()) {
            return Err(invalid_asset_protocol_value(
                "source file workflow",
                format!("duplicate extension `{extension}`"),
            ));
        }
    }
    if workflow.extensions.iter().any(|extension| extension == "*") && workflow.extensions.len() > 1
    {
        return Err(invalid_asset_protocol_value(
            "source file workflow",
            "`*` catch-all extension must be the only extension",
        ));
    }
    if workflow.can_create && workflow.extensions.iter().any(|extension| extension == "*") {
        return Err(invalid_asset_protocol_value(
            "source file workflow",
            "creatable file workflows cannot use the `*` catch-all extension",
        ));
    }
    Ok(())
}

fn validate_source_file_default_path_prefix(value: &str) -> Result<(), Error> {
    if value.trim() != value {
        return Err(invalid_asset_protocol_value(
            "source file workflow",
            "default path prefix must be trimmed",
        ));
    }
    if value.is_empty() {
        return Ok(());
    }
    if value.starts_with('/') || value.contains('\\') || value.contains(':') || value.ends_with('/')
    {
        return Err(invalid_asset_protocol_value(
            "source file workflow",
            format!(
                "default path prefix `{value}` must be a relative asset-tree path using forward slashes"
            ),
        ));
    }
    for component in value.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(invalid_asset_protocol_value(
                "source file workflow",
                format!(
                    "default path prefix `{value}` must not contain empty, current, or parent path components"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_source_file_template_source_path(value: &str) -> Result<(), Error> {
    if value.trim() != value || value.is_empty() {
        return Err(invalid_asset_protocol_value(
            "source file template",
            "source path must be non-empty and trimmed",
        ));
    }
    if value.starts_with('/') || value.contains('\\') || value.contains(':') {
        return Err(invalid_asset_protocol_value(
            "source file template",
            format!("source path `{value}` must be a relative asset path using forward slashes"),
        ));
    }
    for component in value.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(invalid_asset_protocol_value(
                "source file template",
                format!(
                    "source path `{value}` must not contain empty, current, or parent path components"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_source_file_extension(value: &str) -> Result<(), Error> {
    if value.trim() != value || value.is_empty() {
        return Err(invalid_asset_protocol_value(
            "source file workflow",
            "extension must be non-empty and trimmed",
        ));
    }
    if value == "*" {
        return Ok(());
    }
    if value.starts_with('.')
        || value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || value.contains('*')
        || value.contains('?')
    {
        return Err(invalid_asset_protocol_value(
            "source file workflow",
            format!(
                "extension `{value}` must be written without a leading dot or path/wildcard characters"
            ),
        ));
    }
    Ok(())
}

fn validate_asset_builder_catalog_result_for_transport(
    result: &AssetBuilderCatalogResult,
) -> Result<(), Error> {
    let mut seen_builder_guids = BTreeSet::new();
    for builder in &result.builders {
        if !seen_builder_guids.insert(builder.builder_guid) {
            return Err(invalid_asset_protocol_value(
                "asset builder catalog result",
                format!("duplicate builder guid {}", builder.builder_guid),
            ));
        }
    }

    let mut seen_source_schemas = BTreeSet::new();
    for source_schema in &result.source_schemas {
        validate_source_schema_descriptor_for_transport(source_schema)?;
        if !seen_source_schemas.insert(source_schema.schema_type.as_str()) {
            return Err(invalid_asset_protocol_value(
                "asset builder catalog result",
                format!("duplicate source schema type {}", source_schema.schema_type),
            ));
        }
    }

    for builder in &result.builders {
        for source_schema_type in &builder.source_schema_types {
            if !seen_source_schemas.contains(source_schema_type.as_str()) {
                return Err(invalid_asset_protocol_value(
                    "asset builder catalog result",
                    format!(
                        "builder `{}` references source schema type `{}` without a source schema descriptor",
                        builder.name, source_schema_type
                    ),
                ));
            }
        }
    }

    let mut seen_product_formats = BTreeSet::new();
    for product_format in &result.product_formats {
        validate_product_format_descriptor_for_transport(product_format)?;
        if !seen_product_formats.insert(product_format.id.as_str()) {
            return Err(invalid_asset_protocol_value(
                "asset builder catalog result",
                format!("duplicate product format id {}", product_format.id),
            ));
        }
    }

    Ok(())
}

fn validate_product_format_descriptor_for_transport(
    descriptor: &ProductFormatDescriptor,
) -> Result<(), Error> {
    require_trimmed_non_empty_text("product format descriptor", "id", &descriptor.id)?;
    if descriptor.current_version == 0 {
        return Err(invalid_asset_protocol_value(
            "product format descriptor",
            format!(
                "product format `{}` current version must be positive",
                descriptor.id
            ),
        ));
    }
    if descriptor.owner.trim() != descriptor.owner {
        return Err(invalid_asset_protocol_value(
            "product format descriptor",
            format!("product format `{}` owner must be trimmed", descriptor.id),
        ));
    }
    Ok(())
}

fn validate_workspace_entry_page_request_for_transport(
    request: &WorkspaceEntryPageRequest,
) -> Result<(), Error> {
    validate_asset_read_capability_request_for_transport(
        "workspace entry page request",
        &request.capability,
    )?;
    if request.after_entry_id.is_some_and(|value| value <= 0) {
        return Err(invalid_asset_protocol_value(
            "workspace entry page request",
            "after cursor must be positive when present",
        ));
    }
    if request.page_size == 0 {
        return Err(invalid_asset_protocol_value(
            "workspace entry page request",
            "page size must be positive",
        ));
    }
    Ok(())
}

fn validate_workspace_entry_for_transport(entry: &WorkspaceEntry) -> Result<(), Error> {
    require_positive_i64("workspace entry", "entry id", entry.entry_id)?;
    require_positive_i64("workspace entry", "workspace id", entry.workspace_id)?;
    require_non_nil_uuid(
        "workspace entry",
        "asset guid",
        entry.entry_id,
        entry.asset_guid,
    )?;
    require_positive_i64("workspace entry", "root id", entry.root_id)?;
    require_non_empty_text("workspace entry", "source path", &entry.source_path)?;
    require_asset_db_relative_path("workspace entry", "source path", &entry.source_path)?;
    require_optional_non_empty_text(
        "workspace entry",
        "schema type",
        entry.schema_type.as_deref(),
    )?;
    require_64_hex_hash("workspace entry", "content hash", &entry.content_hash)?;
    require_non_negative_i64(
        "workspace entry",
        "diagnostics count",
        entry.diagnostics_count,
    )?;
    require_non_negative_i64(
        "workspace entry",
        "updated timestamp",
        entry.updated_unix_ms,
    )?;

    for activity in &entry.jobs {
        if let Some(attempt) = &activity.attempt
            && attempt.job_id != activity.job.job_id
        {
            return Err(invalid_asset_protocol_value(
                "workspace asset entry",
                format!(
                    "latest activity attempt {} does not match job {}",
                    attempt.attempt_id, activity.job.job_id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_workspace_entry_page_result_for_transport(
    result: &WorkspaceEntryPageResult,
) -> Result<(), Error> {
    let mut previous_entry_id = 0;
    let mut seen_entry_ids = BTreeSet::new();
    for entry in &result.entries {
        if entry.entry_id <= previous_entry_id {
            return Err(invalid_asset_protocol_value(
                "workspace entry page result",
                format!(
                    "entry id {} does not advance after {}",
                    entry.entry_id, previous_entry_id
                ),
            ));
        }
        if !seen_entry_ids.insert(entry.entry_id) {
            return Err(invalid_asset_protocol_value(
                "workspace entry page result",
                format!("duplicate entry id {}", entry.entry_id),
            ));
        }
        previous_entry_id = entry.entry_id;
    }

    if let Some(next_after) = result.next_after_entry_id {
        let Some(last) = result.entries.last() else {
            return Err(invalid_asset_protocol_value(
                "workspace entry page result",
                "next cursor cannot be present for an empty page",
            ));
        };
        if next_after != last.entry_id {
            return Err(invalid_asset_protocol_value(
                "workspace entry page result",
                format!(
                    "next cursor {} does not match last entry {}",
                    next_after, last.entry_id
                ),
            ));
        }
    }

    Ok(())
}

fn validate_asset_processor_event_for_transport(event: &AssetProcessorEvent) -> Result<(), Error> {
    if event.seq == 0 {
        return Err(invalid_asset_protocol_value(
            "asset processor event",
            "sequence must be positive",
        ));
    }
    require_non_negative_i64(
        "asset processor event",
        "event timestamp",
        event.event_unix_ms,
    )?;
    validate_workspace_entry_for_transport(&event.entry)?;
    Ok(())
}

fn validate_asset_processor_event_subscription_request_for_transport(
    request: &AssetProcessorEventSubscriptionRequest,
) -> Result<(), Error> {
    validate_asset_read_capability_request_for_transport(
        "asset processor event subscription request",
        &request.capability,
    )
}

fn validate_inspect_job_request_for_transport(request: &InspectJobRequest) -> Result<(), Error> {
    validate_asset_read_capability_request_for_transport("inspect job request", &request.capability)
}

fn validate_workspace_snapshot_request_for_transport(
    request: &WorkspaceSnapshotRequest,
) -> Result<(), Error> {
    validate_asset_read_capability_request_for_transport(
        "workspace snapshot request",
        &request.capability,
    )
}

fn validate_catalog_products_request_for_transport(
    request: &CatalogProductsRequest,
) -> Result<(), Error> {
    validate_asset_read_capability_request_for_transport(
        "catalog products request",
        &request.capability,
    )?;
    require_trimmed_non_empty_text("catalog products request", "platform", &request.platform)
}

fn validate_catalog_product_entry_for_transport(entry: &CatalogProductEntry) -> Result<(), Error> {
    require_positive_i64("current products entry", "job id", entry.job_id)?;
    require_positive_i64("current products entry", "product id", entry.product_id)?;
    require_non_nil_uuid(
        "current products entry",
        "asset guid",
        entry.product_id,
        entry.asset_guid,
    )?;
    require_non_empty_text("current products entry", "source path", &entry.source_path)?;
    require_asset_db_relative_path("current products entry", "source path", &entry.source_path)?;
    require_non_nil_uuid(
        "current products entry",
        "builder guid",
        entry.job_id,
        entry.builder_guid,
    )?;
    require_trimmed_non_empty_text("current products entry", "job key", &entry.job_key)?;
    require_trimmed_non_empty_text("current products entry", "platform", &entry.platform)?;
    require_non_empty_text(
        "current products entry",
        "product path",
        &entry.product_path,
    )?;
    require_asset_db_relative_path(
        "current products entry",
        "product path",
        &entry.product_path,
    )?;
    let mut catalog_paths = BTreeSet::from([entry.product_path.as_str()]);
    if entry.catalog_path_registration == CatalogPathRegistration::AssetIdOnly
        && !entry.catalog_aliases.is_empty()
    {
        return Err(invalid_asset_protocol_value(
            "current products entry",
            "asset-id-only product cannot declare catalog aliases",
        ));
    }
    for alias in &entry.catalog_aliases {
        require_non_empty_text("current products entry", "catalog alias", alias)?;
        require_asset_db_relative_path("current products entry", "catalog alias", alias)?;
        if !catalog_paths.insert(alias.as_str()) {
            return Err(invalid_asset_protocol_value(
                "current products entry",
                format!("duplicate catalog path `{alias}`"),
            ));
        }
    }
    require_non_nil_uuid(
        "current products entry",
        "asset type",
        entry.product_id,
        entry.asset_type,
    )?;
    require_non_negative_i64("current products entry", "sub id", entry.sub_id)?;
    require_trimmed_non_empty_text(
        "current products entry",
        "product format",
        &entry.product_format,
    )?;
    require_positive_u32(
        "current products entry",
        "product format version",
        entry.product_format_version,
    )?;
    require_64_hex_hash(
        "current products entry",
        "content hash",
        &entry.content_hash,
    )?;
    require_non_negative_i64("current products entry", "byte length", entry.byte_length)?;
    for dependency in &entry.dependencies {
        // The target's asset type is resolved best-effort from the shipping
        // product set and may legitimately be nil for a dangling edge, so only
        // the target identity is required here.
        require_non_nil_uuid(
            "current products entry",
            "dependency asset guid",
            entry.product_id,
            dependency.asset_guid,
        )?;
        require_non_negative_i64(
            "current products entry",
            "dependency sub id",
            dependency.sub_id,
        )?;
    }
    Ok(())
}

fn validate_catalog_products_result_for_transport(
    result: &CatalogProductsResult,
) -> Result<(), Error> {
    let mut seen_product_ids = BTreeSet::new();
    let mut previous_sort_key: Option<(String, Uuid, i64, i64)> = None;
    for entry in &result.entries {
        if !seen_product_ids.insert(entry.product_id) {
            return Err(invalid_asset_protocol_value(
                "current products result",
                format!("duplicate product id {}", entry.product_id),
            ));
        }
        let sort_key = (
            entry.product_path.clone(),
            entry.asset_guid,
            entry.sub_id,
            entry.product_id,
        );
        if let Some(previous) = previous_sort_key.as_ref()
            && previous > &sort_key
        {
            return Err(invalid_asset_protocol_value(
                "current products result",
                format!(
                    "entry product path `{}` is not in catalog order",
                    entry.product_path
                ),
            ));
        }
        previous_sort_key = Some(sort_key);
    }
    Ok(())
}

fn validate_release_content_read_request_for_transport(
    request: &ReleaseContentReadRequest,
) -> Result<(), Error> {
    validate_asset_read_capability_request_for_transport(
        "release content read request",
        &request.capability,
    )?;
    require_trimmed_non_empty_text(
        "release content read request",
        "platform",
        &request.platform,
    )?;
    match &request.target {
        ReleaseContentTarget::AssetCatalog => Ok(()),
        ReleaseContentTarget::ProductAsset { asset_guid, .. } => {
            require_non_nil_uuid("release content read request", "asset guid", 0, *asset_guid)
        }
    }
}

fn validate_release_content_product_for_transport(
    product: &ReleaseContentProduct,
) -> Result<(), Error> {
    require_non_nil_uuid(
        "release content product",
        "asset guid",
        0,
        product.asset_guid,
    )?;
    require_non_empty_text(
        "release content product",
        "product path",
        &product.product_path,
    )?;
    require_asset_db_relative_path(
        "release content product",
        "product path",
        &product.product_path,
    )?;
    require_trimmed_non_empty_text(
        "release content product",
        "product format",
        &product.product_format,
    )?;
    require_positive_u32(
        "release content product",
        "product format version",
        product.product_format_version,
    )?;
    require_positive_u64(
        "release content product",
        "byte length",
        product.byte_length,
    )?;
    require_blake3_hash_bytes(
        "release content product",
        "content hash",
        &product.content_hash,
    )?;
    require_bound_side_channel_handle(
        "release content product",
        "product payload",
        &product.payload,
    )?;
    if product.payload.byte_length != product.byte_length {
        return Err(invalid_asset_protocol_value(
            "release content product",
            format!(
                "payload byte length {} does not match product byte length {}",
                product.payload.byte_length, product.byte_length
            ),
        ));
    }
    if !product.payload.content_hash.is_empty()
        && product.payload.content_hash != product.content_hash
    {
        return Err(invalid_asset_protocol_value(
            "release content product",
            "payload content hash does not match product content hash",
        ));
    }
    Ok(())
}

fn validate_release_content_read_result_for_transport(
    result: &ReleaseContentReadResult,
) -> Result<(), Error> {
    match result {
        ReleaseContentReadResult::None => Ok(()),
        ReleaseContentReadResult::AssetCatalog(handle) => require_bound_side_channel_handle(
            "release content read result",
            "asset catalog payload",
            handle,
        ),
        ReleaseContentReadResult::Product(product) => {
            validate_release_content_product_for_transport(product)
        }
    }
}

fn validate_workspace_root_for_transport(source_root: &WorkspaceRoot) -> Result<(), Error> {
    require_positive_i64(
        "workspace root",
        "workspace root id",
        source_root.workspace_root_id,
    )?;
    require_positive_i64("workspace root", "workspace id", source_root.workspace_id)?;
    require_positive_i64("workspace root", "root id", source_root.root_id)?;
    require_non_empty_text("workspace root", "owner id", &source_root.owner_id)?;
    require_non_empty_text(
        "workspace root",
        "declared root id",
        &source_root.declared_root_id,
    )?;
    require_non_empty_text("workspace root", "source root", &source_root.source_root)?;
    require_non_empty_text("workspace root", "portable key", &source_root.portable_key)?;
    require_non_empty_text("workspace root", "mount", &source_root.mount)
}

fn validate_workspace_snapshot_for_transport(snapshot: &WorkspaceSnapshot) -> Result<(), Error> {
    require_positive_i64("workspace snapshot", "workspace id", snapshot.workspace_id)?;
    require_non_empty_text("workspace snapshot", "project id", &snapshot.project_id)?;
    require_non_empty_text(
        "workspace snapshot",
        "workspace root",
        &snapshot.workspace_root,
    )?;
    require_non_empty_text("workspace snapshot", "branch", &snapshot.branch)?;
    require_non_negative_i64(
        "workspace snapshot",
        "created timestamp",
        snapshot.created_unix_ms,
    )?;
    require_non_negative_i64(
        "workspace snapshot",
        "updated timestamp",
        snapshot.updated_unix_ms,
    )?;
    if snapshot.updated_unix_ms < snapshot.created_unix_ms {
        return Err(invalid_asset_protocol_value(
            "workspace snapshot",
            format!(
                "workspace {} was updated before it was created",
                snapshot.workspace_id
            ),
        ));
    }
    if snapshot.roots.is_empty() {
        return Err(invalid_asset_protocol_value(
            "workspace snapshot",
            format!("workspace {} must include roots", snapshot.workspace_id),
        ));
    }

    let mut seen_workspace_root_ids = BTreeSet::new();
    let mut seen_portable_keys = BTreeSet::new();
    for source_root in &snapshot.roots {
        if source_root.workspace_id != snapshot.workspace_id {
            return Err(invalid_asset_protocol_value(
                "workspace snapshot",
                format!(
                    "root {} belongs to workspace {}, expected {}",
                    source_root.workspace_root_id, source_root.workspace_id, snapshot.workspace_id
                ),
            ));
        }
        if !seen_workspace_root_ids.insert(source_root.workspace_root_id) {
            return Err(invalid_asset_protocol_value(
                "workspace snapshot",
                format!(
                    "duplicate workspace root id {}",
                    source_root.workspace_root_id
                ),
            ));
        }
        if !seen_portable_keys.insert(source_root.portable_key.as_str()) {
            return Err(invalid_asset_protocol_value(
                "workspace snapshot",
                format!("duplicate root key `{}`", source_root.portable_key),
            ));
        }
    }

    Ok(())
}

fn validate_source_asset_record_request_for_transport(
    request: &SourceAssetRecordRequest,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "source asset record request",
        &request.capability,
    )?;
    require_non_empty_text(
        "source asset record request",
        "session id",
        &request.session_id,
    )?;
    require_positive_i64(
        "source asset record request",
        "workspace root id",
        request.workspace_root_id,
    )?;
    require_non_empty_text("source asset record request", "owner id", &request.owner_id)?;
    require_non_empty_text(
        "source asset record request",
        "source path",
        &request.source_path,
    )?;
    require_asset_db_relative_path(
        "source asset record request",
        "source path",
        &request.source_path,
    )?;
    require_optional_non_empty_text(
        "source asset record request",
        "schema type",
        request.schema_type.as_deref(),
    )?;
    if request.content_hash.len() != 32 {
        return Err(invalid_asset_protocol_value(
            "source asset record request",
            format!(
                "content hash must be 32 bytes, got {}",
                request.content_hash.len()
            ),
        ));
    }
    require_non_negative_i64(
        "source asset record request",
        "changed timestamp",
        request.changed_unix_ms,
    )?;
    require_non_negative_i64(
        "source asset record request",
        "diagnostics count",
        request.diagnostics_count,
    )
}

fn validate_source_asset_record_result_for_transport(
    result: &SourceAssetRecordResult,
) -> Result<(), Error> {
    require_non_nil_uuid(
        "source asset record result",
        "asset guid",
        result.entry.entry_id,
        result.asset_guid,
    )
}

fn validate_source_file_create_request_for_transport(
    request: &SourceFileCreateRequest,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "source file create request",
        &request.capability,
    )?;
    require_non_empty_text(
        "source file create request",
        "session id",
        &request.session_id,
    )?;
    require_non_empty_text(
        "source file create request",
        "source root",
        &request.source_root,
    )?;
    if request.source_root.trim() != request.source_root {
        return Err(invalid_asset_protocol_value(
            "source file create request",
            "source root must not have leading or trailing whitespace",
        ));
    }
    require_non_empty_text(
        "source file create request",
        "source path",
        &request.source_path,
    )?;
    require_asset_db_relative_path(
        "source file create request",
        "source path",
        &request.source_path,
    )?;
    require_non_empty_text(
        "source file create request",
        "schema type",
        &request.schema_type,
    )?;
    require_non_negative_i64(
        "source file create request",
        "changed timestamp",
        request.changed_unix_ms,
    )?;
    if let SourceFileCreateContent::Payload(handle) = &request.content
        && handle.capability.is_none()
    {
        return Err(invalid_asset_protocol_value(
            "source file create request",
            "payload side channel must carry its brokered capability",
        ));
    }
    Ok(())
}

fn validate_source_file_create_result_for_transport(
    result: &SourceFileCreateResult,
) -> Result<(), Error> {
    validate_source_asset_record_result_for_transport(&result.record)
}

fn validate_workspace_source_file_ref_for_transport(
    source: &WorkspaceSourceFileRef,
) -> Result<(), Error> {
    require_non_empty_text(
        "workspace source file reference",
        "source root key",
        &source.source_root_key,
    )?;
    if source.source_root_key.trim() != source.source_root_key {
        return Err(invalid_asset_protocol_value(
            "workspace source file reference",
            "source root key must not have leading or trailing whitespace",
        ));
    }
    if source
        .source_root_key
        .chars()
        .any(|character| matches!(character, '/' | '\\'))
    {
        return Err(invalid_asset_protocol_value(
            "workspace source file reference",
            "source root key must be a portable key, not a filesystem path",
        ));
    }
    require_non_empty_text(
        "workspace source file reference",
        "source path",
        &source.source_path,
    )?;
    require_asset_db_relative_path(
        "workspace source file reference",
        "source path",
        &source.source_path,
    )?;
    require_non_empty_text(
        "workspace source file reference",
        "schema type",
        &source.schema_type,
    )
}

fn validate_source_fingerprint_for_transport(kind: &str, fingerprint: &[u8]) -> Result<(), Error> {
    if fingerprint.len() == 32 {
        return Ok(());
    }
    Err(invalid_asset_protocol_value(
        kind,
        format!(
            "source fingerprint must be 32 bytes, got {}",
            fingerprint.len()
        ),
    ))
}

fn validate_source_file_edit_request_for_transport(
    request: &SourceFileEditRequest,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "source file edit request",
        &request.capability,
    )?;
    require_non_empty_text(
        "source file edit request",
        "session id",
        &request.session_id,
    )?;
    validate_workspace_source_file_ref_for_transport(&request.source)?;
    validate_source_fingerprint_for_transport(
        "source file edit request",
        &request.expected_source_fingerprint,
    )?;
    az_proto_authoring::validate_source_file_edit_operation(&request.operation)
}

fn validate_source_file_open_request_for_transport(
    request: &SourceFileOpenRequest,
) -> Result<(), Error> {
    validate_asset_read_capability_request_for_transport(
        "source file open request",
        &request.capability,
    )?;
    require_non_empty_text(
        "source file open request",
        "session id",
        &request.session_id,
    )?;
    validate_workspace_source_file_ref_for_transport(&request.source)
}

fn validate_source_file_restore_request_for_transport(
    request: &SourceFileRestoreRequest,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "source file restore request",
        &request.capability,
    )?;
    if request.capability.role != core::ServiceRole::ProjectHost {
        return Err(invalid_asset_protocol_value(
            "source file restore request",
            "capability role must be ProjectHost",
        ));
    }
    require_non_empty_text(
        "source file restore request",
        "session id",
        &request.session_id,
    )?;
    validate_workspace_source_file_ref_for_transport(&request.source)?;
    validate_source_fingerprint_for_transport(
        "source file restore request",
        &request.expected_source_fingerprint,
    )?;
    az_proto_authoring::validate_source_file_edit_document(&request.document)
}

fn validate_source_file_codec_request_for_transport(
    request: &SourceFileCodecRequest,
) -> Result<(), Error> {
    validate_workspace_source_file_ref_for_transport(&request.source)?;
    if request.authoritative_source.capability.is_none() {
        return Err(invalid_asset_protocol_value(
            "source file edit codec request",
            "authoritative source side channel must carry its brokered capability",
        ));
    }
    validate_source_file_codec_output_destination_for_transport(&request.output_destination)?;
    if request.authoritative_source.capability.as_ref()
        != Some(&request.output_destination.capability)
    {
        return Err(invalid_asset_protocol_value(
            "source file codec request",
            "authoritative source and output destination must use the same worker capability",
        ));
    }
    match &request.operation {
        SourceFileCodecOperation::Load => Ok(()),
        SourceFileCodecOperation::Edit(operation) => {
            az_proto_authoring::validate_source_file_edit_operation(operation)
        }
        SourceFileCodecOperation::RestoreDocument(document) => {
            az_proto_authoring::validate_source_file_edit_document(document)
        }
    }
}

fn validate_source_file_codec_output_destination_for_transport(
    destination: &SourceFileCodecOutputDestination,
) -> Result<(), Error> {
    require_non_empty_text(
        "source file codec output destination",
        "locator",
        &destination.locator,
    )?;
    require_non_empty_text(
        "source file codec output destination",
        "platform",
        &destination.platform,
    )?;
    if destination.capability.is_empty() {
        return Err(invalid_asset_protocol_value(
            "source file codec output destination",
            "capability cannot be empty",
        ));
    }
    let path = Path::new(&destination.locator);
    if !path.is_absolute()
        || !path.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
    {
        return Err(invalid_asset_protocol_value(
            "source file codec output destination",
            "locator must be a normal absolute path",
        ));
    }
    Ok(())
}

fn validate_source_file_codec_result_for_transport(
    result: &SourceFileCodecResult,
) -> Result<(), Error> {
    if let SourceFileCodecReplacement::SavedSource(handle) = &result.replacement
        && handle.capability.is_none()
    {
        return Err(invalid_asset_protocol_value(
            "source file edit codec result",
            "saved source side channel must carry its brokered capability",
        ));
    }
    az_proto_authoring::validate_source_file_edit_document(&result.document)
}

fn validate_source_file_edit_snapshot_for_transport(
    snapshot: &SourceFileEditSnapshot,
) -> Result<(), Error> {
    validate_workspace_source_file_ref_for_transport(&snapshot.source)?;
    validate_source_fingerprint_for_transport(
        "source file edit snapshot",
        &snapshot.source_fingerprint,
    )?;
    az_proto_authoring::validate_source_file_edit_document(&snapshot.document)
}

fn validate_source_file_edit_result_for_transport(
    result: &SourceFileEditResult,
) -> Result<(), Error> {
    validate_source_file_edit_snapshot_for_transport(&result.snapshot)
}

fn validate_source_file_open_result_for_transport(
    result: &SourceFileOpenResult,
) -> Result<(), Error> {
    validate_source_file_edit_snapshot_for_transport(&result.snapshot)
}

fn validate_source_file_restore_result_for_transport(
    result: &SourceFileRestoreResult,
) -> Result<(), Error> {
    validate_source_file_edit_snapshot_for_transport(&result.snapshot)
}

fn validate_source_file_delete_request_for_transport(
    request: &SourceFileDeleteRequest,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "source file delete request",
        &request.capability,
    )?;
    require_non_empty_text(
        "source file delete request",
        "session id",
        &request.session_id,
    )?;
    require_non_empty_text(
        "source file delete request",
        "source root",
        &request.source_root,
    )?;
    if request.source_root.trim() != request.source_root {
        return Err(invalid_asset_protocol_value(
            "source file delete request",
            "source root must not have leading or trailing whitespace",
        ));
    }
    require_non_empty_text(
        "source file delete request",
        "source path",
        &request.source_path,
    )?;
    require_asset_db_relative_path(
        "source file delete request",
        "source path",
        &request.source_path,
    )?;
    require_non_negative_i64(
        "source file delete request",
        "changed timestamp",
        request.changed_unix_ms,
    )
}

fn validate_source_file_delete_result_for_transport(
    result: &SourceFileDeleteResult,
) -> Result<(), Error> {
    validate_source_asset_record_result_for_transport(&result.record)
}

fn validate_source_file_move_request_for_transport(
    request: &SourceFileMoveRequest,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "source file move request",
        &request.capability,
    )?;
    require_non_empty_text(
        "source file move request",
        "session id",
        &request.session_id,
    )?;
    require_non_empty_text(
        "source file move request",
        "source root",
        &request.source_root,
    )?;
    if request.source_root.trim() != request.source_root {
        return Err(invalid_asset_protocol_value(
            "source file move request",
            "source root must not have leading or trailing whitespace",
        ));
    }
    require_non_empty_text(
        "source file move request",
        "from source path",
        &request.from_source_path,
    )?;
    require_asset_db_relative_path(
        "source file move request",
        "from source path",
        &request.from_source_path,
    )?;
    require_non_empty_text(
        "source file move request",
        "to source path",
        &request.to_source_path,
    )?;
    require_asset_db_relative_path(
        "source file move request",
        "to source path",
        &request.to_source_path,
    )?;
    if request.from_source_path == request.to_source_path {
        return Err(invalid_asset_protocol_value(
            "source file move request",
            "from source path and to source path must differ",
        ));
    }
    require_non_negative_i64(
        "source file move request",
        "changed timestamp",
        request.changed_unix_ms,
    )
}

fn validate_source_file_move_result_for_transport(
    result: &SourceFileMoveResult,
) -> Result<(), Error> {
    validate_source_asset_record_result_for_transport(&result.record)?;
    require_non_empty_text(
        "source file move result",
        "old source path",
        &result.old_source_path,
    )?;
    require_asset_db_relative_path(
        "source file move result",
        "old source path",
        &result.old_source_path,
    )
}

fn validate_force_reprocess_asset_request_for_transport(
    request: &ForceReprocessAssetRequest,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "force reprocess asset request",
        &request.capability,
    )?;
    require_non_empty_text(
        "force reprocess asset request",
        "session id",
        &request.session_id,
    )?;
    require_non_empty_text(
        "force reprocess asset request",
        "source root",
        &request.source_root,
    )?;
    if request.source_root.trim() != request.source_root {
        return Err(invalid_asset_protocol_value(
            "force reprocess asset request",
            "source root must not have leading or trailing whitespace",
        ));
    }
    require_non_empty_text(
        "force reprocess asset request",
        "source path",
        &request.source_path,
    )?;
    require_asset_db_relative_path(
        "force reprocess asset request",
        "source path",
        &request.source_path,
    )
}

fn validate_force_reprocess_asset_result_for_transport(
    result: &ForceReprocessAssetResult,
) -> Result<(), Error> {
    validate_source_asset_record_result_for_transport(&result.record)
}

fn validate_reconcile_asset_sources_request_for_transport(
    request: &ReconcileAssetSourcesRequest,
) -> Result<(), Error> {
    validate_asset_write_capability_request_for_transport(
        "reconcile asset sources request",
        &request.capability,
    )?;
    require_non_empty_text(
        "reconcile asset sources request",
        "session id",
        &request.session_id,
    )
}

/// Transport validation hook for [`ReconcileAssetSourcesResult`].
///
/// The result carries three plain counters with no cross-field invariant, so
/// there is nothing to reject; the hook stays so the write/read pair keeps the
/// same shape as its siblings and gains a home if a constraint appears.
const fn validate_reconcile_asset_sources_result_for_transport(
    _result: &ReconcileAssetSourcesResult,
) {
}

fn validate_source_dependent_source_for_transport(
    dependent: &SourceDependentSource,
) -> Result<(), Error> {
    require_positive_i64(
        "source dependent source",
        "source edge id",
        dependent.source_edge_id,
    )?;
    require_asset_db_relative_path(
        "source dependent source",
        "source path",
        &dependent.source_path,
    )?;
    require_non_nil_uuid(
        "source dependent source",
        "builder guid",
        dependent.source_edge_id,
        dependent.builder_guid,
    )
}

fn validate_job_owner(owner: &JobOwner, context: &'static str, identity: i64) -> Result<(), Error> {
    if let JobOwner::Build(builder_guid) = owner {
        require_non_nil_uuid(context, "builder guid", identity, *builder_guid)?;
    }
    Ok(())
}

fn validate_leased_asset_job_for_transport(leased: &LeasedAssetJob) -> Result<(), Error> {
    require_positive_i64("leased asset job", "attempt id", leased.attempt_id)?;
    require_positive_i64("leased asset job", "workspace id", leased.workspace_id)?;
    validate_job_owner(&leased.owner, "leased asset job", leased.attempt_id)?;
    require_non_nil_uuid(
        "leased asset job",
        "source guid",
        leased.attempt_id,
        leased.source_guid,
    )?;
    require_asset_db_relative_path("leased asset job", "source path", &leased.source_path)?;
    require_non_empty_text("leased asset job", "source root", &leased.source_root)?;
    require_optional_non_empty_text(
        "leased asset job",
        "source schema type",
        leased.source_schema_type.as_deref(),
    )?;
    require_non_empty_text("leased asset job", "job key", &leased.job_key)?;
    require_non_empty_text("leased asset job", "platform", &leased.platform)?;
    require_positive_i64("leased asset job", "ordinal", leased.ordinal)?;
    require_non_empty_text("leased asset job", "staging root", &leased.staging_root)
}

fn validate_job_record_for_transport(job: &JobRecord) -> Result<(), Error> {
    require_positive_i64("job record", "job id", job.job_id)?;
    require_positive_i64("job record", "workspace id", job.workspace_id)?;
    require_non_nil_uuid("job record", "source guid", job.job_id, job.source_guid)?;
    require_asset_db_relative_path("job record", "source path", &job.source_path)?;
    require_non_empty_text("job record", "source root", &job.source_root)?;
    require_optional_non_empty_text(
        "job record",
        "source schema type",
        job.source_schema_type.as_deref(),
    )?;
    validate_job_owner(&job.owner, "job record", job.job_id)?;
    require_non_empty_text("job record", "key", &job.key)?;
    require_non_empty_text("job record", "platform", &job.platform)?;
    require_non_negative_i64("job record", "attempts", job.attempts)
}

fn validate_source_dependent_job_for_transport(
    dependent: &SourceDependentJob,
) -> Result<(), Error> {
    require_positive_i64("source dependent job", "job edge id", dependent.job_edge_id)?;
    require_positive_i64("source dependent job", "job id", dependent.job_id)?;
    if let Some(attempt_id) = dependent.latest_attempt_id {
        require_positive_i64("source dependent job", "latest attempt id", attempt_id)?;
    }
    require_asset_db_relative_path(
        "source dependent job",
        "source path",
        &dependent.source_path,
    )?;
    validate_job_owner(&dependent.owner, "source dependent job", dependent.job_id)?;
    require_non_empty_text("source dependent job", "job key", &dependent.job_key)?;
    require_non_empty_text("source dependent job", "platform", &dependent.platform)?;
    require_non_empty_text(
        "source dependent job",
        "dependency job key",
        &dependent.dependency_job_key,
    )?;
    require_non_empty_text(
        "source dependent job",
        "dependency platform",
        &dependent.dependency_platform,
    )?;
    for product_path in &dependent.product_paths {
        require_asset_db_relative_path("source dependent job", "product path", product_path)?;
    }
    Ok(())
}

fn validate_source_dependents_request_for_transport(
    request: &SourceDependentsRequest,
) -> Result<(), Error> {
    validate_asset_read_capability_request_for_transport(
        "source dependents request",
        &request.capability,
    )?;
    require_non_empty_text(
        "source dependents request",
        "session id",
        &request.session_id,
    )?;
    require_non_empty_text(
        "source dependents request",
        "source root",
        &request.source_root,
    )?;
    if request.source_root.trim() != request.source_root {
        return Err(invalid_asset_protocol_value(
            "source dependents request",
            "source root must not have leading or trailing whitespace",
        ));
    }
    require_non_empty_text(
        "source dependents request",
        "source path",
        &request.source_path,
    )?;
    require_asset_db_relative_path(
        "source dependents request",
        "source path",
        &request.source_path,
    )
}

fn validate_source_dependents_result_for_transport(
    result: &SourceDependentsResult,
) -> Result<(), Error> {
    require_non_empty_text(
        "source dependents result",
        "source path",
        &result.source_path,
    )?;
    require_asset_db_relative_path(
        "source dependents result",
        "source path",
        &result.source_path,
    )?;
    for dependent in &result.source_dependents {
        validate_source_dependent_source_for_transport(dependent)?;
    }
    for dependent in &result.job_dependents {
        validate_source_dependent_job_for_transport(dependent)?;
    }
    Ok(())
}

fn validate_lease_asset_job_request_for_transport(
    request: &LeaseAssetJobRequest,
) -> Result<(), Error> {
    validate_asset_jobs_capability_request_for_transport(
        "lease asset job request",
        &request.capability,
    )?;
    require_non_empty_text(
        "lease asset job request",
        "lease owner",
        &request.lease_owner,
    )?;
    if request.lease_duration_ms == 0 || request.lease_duration_ms > MAX_ASSET_JOB_LEASE_DURATION_MS
    {
        return Err(invalid_asset_protocol_value(
            "lease asset job request",
            format!(
                "lease duration must be between 1 and {MAX_ASSET_JOB_LEASE_DURATION_MS} milliseconds"
            ),
        ));
    }
    require_optional_non_empty_text(
        "lease asset job request",
        "staging root",
        request.staging_root.as_deref(),
    )
}

fn validate_renew_asset_job_lease_request_for_transport(
    request: &RenewAssetJobLeaseRequest,
) -> Result<(), Error> {
    validate_asset_jobs_capability_request_for_transport(
        "renew asset job lease request",
        &request.capability,
    )?;
    require_positive_i64(
        "renew asset job lease request",
        "asset job attempt id",
        request.asset_job_attempt_id,
    )?;
    require_non_empty_text(
        "renew asset job lease request",
        "lease owner",
        &request.lease_owner,
    )?;
    if request.grant_key.is_nil() {
        return Err(invalid_asset_protocol_value(
            "renew asset job lease request",
            "grant key must not be nil",
        ));
    }
    Ok(())
}

fn validate_complete_asset_job_attempt_request_for_transport(
    request: &CompleteAssetJobAttemptRequest,
) -> Result<(), Error> {
    validate_asset_jobs_capability_request_for_transport(
        "complete asset job attempt request",
        &request.capability,
    )?;
    require_positive_i64(
        "complete asset job attempt request",
        "asset job attempt id",
        request.asset_job_attempt_id,
    )?;
    require_non_empty_text(
        "complete asset job attempt request",
        "lease owner",
        &request.lease_owner,
    )?;
    if request.grant_key.is_nil() {
        return Err(invalid_asset_protocol_value(
            "complete asset job attempt request",
            "grant key must not be nil",
        ));
    }
    if !matches!(
        request.status,
        AttemptStatus::Succeeded | AttemptStatus::Failed | AttemptStatus::Abandoned
    ) {
        return Err(invalid_asset_protocol_value(
            "complete asset job attempt request",
            format!("status {:?} is not terminal", request.status),
        ));
    }
    require_non_negative_i64(
        "complete asset job attempt request",
        "finished timestamp",
        request.finished_unix_ms,
    )?;
    require_non_negative_i64(
        "complete asset job attempt request",
        "error count",
        request.error_count,
    )?;
    require_non_negative_i64(
        "complete asset job attempt request",
        "warning count",
        request.warning_count,
    )
}

fn validate_product_manifest_for_transport(manifest: &ProductManifest) -> Result<(), Error> {
    if manifest.schema_version != PRODUCT_MANIFEST_SCHEMA_VERSION {
        return Err(Error::failed(format!(
            "unsupported product manifest schema version {}; expected {PRODUCT_MANIFEST_SCHEMA_VERSION}",
            manifest.schema_version
        )));
    }

    let mut seen_catalog_paths = BTreeSet::new();
    let mut physical_products = BTreeMap::<&str, &ProductManifestProduct>::new();
    for product in &manifest.products {
        validate_manifest_product_catalog_paths(product, &mut seen_catalog_paths)?;
        validate_manifest_product_payload_fields(product)?;
        if let Some(first) = physical_products.get(product.product_path.as_str()) {
            if !same_manifest_product_backing(first, product) {
                return Err(invalid_product_manifest(format!(
                    "products with sub-IDs {} and {} share physical path `{}` but disagree on its backing payload",
                    first.sub_id, product.sub_id, product.product_path
                )));
            }
        } else {
            physical_products.insert(product.product_path.as_str(), product);
        }
        validate_manifest_product_dependencies(product)?;
    }

    Ok(())
}

/// Checks one product's own path and its catalog aliases, recording every path
/// it claims in `seen_catalog_paths` so later products collide against it.
fn validate_manifest_product_catalog_paths<'a>(
    product: &'a ProductManifestProduct,
    seen_catalog_paths: &mut BTreeSet<&'a str>,
) -> Result<(), Error> {
    if product.product_path.trim().is_empty() {
        return Err(invalid_product_manifest("product path cannot be empty"));
    }
    if !is_safe_asset_db_relative_path(&product.product_path) {
        return Err(invalid_product_manifest(format!(
            "product path `{}` must be relative and stay inside the asset cache",
            product.product_path
        )));
    }
    if product.catalog_path_registration == CatalogPathRegistration::AssetIdOnly
        && !product.catalog_aliases.is_empty()
    {
        return Err(invalid_product_manifest(format!(
            "asset-id-only product `{}` cannot declare catalog aliases",
            product.product_path
        )));
    }
    if product.catalog_path_registration == CatalogPathRegistration::Registered
        && !seen_catalog_paths.insert(product.product_path.as_str())
    {
        return Err(invalid_product_manifest(format!(
            "duplicate registered catalog path `{}`",
            product.product_path
        )));
    }
    for alias in &product.catalog_aliases {
        if alias.trim().is_empty() {
            return Err(invalid_product_manifest("catalog alias cannot be empty"));
        }
        if !is_safe_asset_db_relative_path(alias) {
            return Err(invalid_product_manifest(format!(
                "catalog alias `{alias}` must be relative and stay inside the asset cache"
            )));
        }
        if !seen_catalog_paths.insert(alias.as_str()) {
            return Err(invalid_product_manifest(format!(
                "duplicate product or catalog-alias path `{alias}`"
            )));
        }
    }
    Ok(())
}

/// Checks the scalar fields that describe one product's staged payload.
fn validate_manifest_product_payload_fields(product: &ProductManifestProduct) -> Result<(), Error> {
    if product.asset_type.is_nil() {
        return Err(invalid_product_manifest(format!(
            "product `{}` asset type cannot be nil",
            product.product_path
        )));
    }
    if product.staged_path.trim().is_empty() {
        return Err(invalid_product_manifest(format!(
            "staged path for `{}` cannot be empty",
            product.product_path
        )));
    }
    if product.product_format.trim().is_empty() {
        return Err(invalid_product_manifest(format!(
            "product format for `{}` cannot be empty",
            product.product_path
        )));
    }
    if product.product_format_version == 0 {
        return Err(invalid_product_manifest(format!(
            "product format version for `{}` must be positive",
            product.product_path
        )));
    }
    if product.content_hash.len() != 32 {
        return Err(invalid_product_manifest(format!(
            "product `{}` content hash must be 32 bytes, got {}",
            product.product_path,
            product.content_hash.len()
        )));
    }
    if product.byte_length > i64::MAX as u64 {
        return Err(invalid_product_manifest(format!(
            "product `{}` byte length exceeds DB range",
            product.product_path
        )));
    }
    Ok(())
}

/// Checks that one product's dependency edges are non-nil and unique.
fn validate_manifest_product_dependencies(product: &ProductManifestProduct) -> Result<(), Error> {
    let mut seen_dependencies = BTreeSet::new();
    for dependency in &product.dependencies {
        if dependency.asset_guid.is_nil() {
            return Err(invalid_product_manifest(format!(
                "product `{}` dependency asset guid cannot be nil",
                product.product_path
            )));
        }
        if !seen_dependencies.insert((dependency.asset_guid, dependency.sub_id)) {
            return Err(invalid_product_manifest(format!(
                "product `{}` has duplicate dependency {}:{}",
                product.product_path, dependency.asset_guid, dependency.sub_id
            )));
        }
    }
    Ok(())
}

fn same_manifest_product_backing(
    left: &ProductManifestProduct,
    right: &ProductManifestProduct,
) -> bool {
    left.asset_type == right.asset_type
        && left.product_format == right.product_format
        && left.product_format_version == right.product_format_version
        && left.staged_path == right.staged_path
        && left.content_hash == right.content_hash
        && left.byte_length == right.byte_length
        && left.dependencies == right.dependencies
}

/// Narrows a `usize` list length or element index to the `u32` Cap'n Proto
/// uses for both.
///
/// The casts this replaces were lossy on 64-bit targets: an oversized
/// collection would have silently written a wrapped length and produced a
/// message that decodes to the wrong number of elements. Failing the write is
/// the honest outcome, and every caller is already in a `Result`.
///
/// # Errors
///
/// Returns an error if `value` does not fit in a `u32`.
fn capnp_list_index(value: usize) -> Result<u32, Error> {
    u32::try_from(value)
        .map_err(|_| Error::failed("Cap'n Proto list length exceeds u32 range".to_string()))
}

fn invalid_product_manifest(reason: impl Into<String>) -> Error {
    Error::failed(format!("invalid product manifest: {}", reason.into()))
}

fn invalid_asset_protocol_value(kind: &str, reason: impl Into<String>) -> Error {
    Error::failed(format!("invalid {kind}: {}", reason.into()))
}

fn require_positive_i64(kind: &str, label: &str, value: i64) -> Result<(), Error> {
    if value <= 0 {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("{label} must be positive, got {value}"),
        ));
    }
    Ok(())
}

fn require_positive_u32(kind: &str, label: &str, value: u32) -> Result<(), Error> {
    if value == 0 {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("{label} must be positive, got {value}"),
        ));
    }
    Ok(())
}

fn require_positive_u64(kind: &str, label: &str, value: u64) -> Result<(), Error> {
    if value == 0 {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("{label} must be positive, got {value}"),
        ));
    }
    Ok(())
}

fn require_non_negative_i64(kind: &str, label: &str, value: i64) -> Result<(), Error> {
    if value < 0 {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("{label} cannot be negative, got {value}"),
        ));
    }
    Ok(())
}

fn require_bound_side_channel_handle(
    kind: &str,
    label: &str,
    handle: &SideChannelHandle,
) -> Result<(), Error> {
    core::require_side_channel_capability(handle, label)
        .map(|_| ())
        .map_err(|error| invalid_asset_protocol_value(kind, error.to_string()))
}

fn require_non_empty_text(kind: &str, label: &str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("{label} cannot be empty"),
        ));
    }
    Ok(())
}

fn require_trimmed_non_empty_text(kind: &str, label: &str, value: &str) -> Result<(), Error> {
    require_non_empty_text(kind, label, value)?;
    if value.trim() != value {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("{label} must be trimmed"),
        ));
    }
    Ok(())
}

fn require_optional_non_empty_text(
    kind: &str,
    label: &str,
    value: Option<&str>,
) -> Result<(), Error> {
    if let Some(value) = value {
        require_non_empty_text(kind, label, value)?;
    }
    Ok(())
}

fn require_non_nil_uuid(kind: &str, label: &str, record_id: i64, uuid: Uuid) -> Result<(), Error> {
    if uuid == Uuid::nil() {
        let owner = if record_id > 0 {
            format!(" for record {record_id}")
        } else {
            String::new()
        };
        return Err(invalid_asset_protocol_value(
            kind,
            format!("{label}{owner} cannot be nil"),
        ));
    }
    Ok(())
}

fn require_asset_db_relative_path(kind: &str, label: &str, path: &str) -> Result<(), Error> {
    if !is_safe_asset_db_relative_path(path) {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("{label} `{path}` must be an asset-db relative path"),
        ));
    }
    Ok(())
}

fn require_64_hex_hash(kind: &str, label: &str, value: &str) -> Result<(), Error> {
    if !is_64_hex_hash(value) {
        return Err(invalid_asset_protocol_value(
            kind,
            format!("{label} must be 64 hex characters"),
        ));
    }
    Ok(())
}

fn require_blake3_hash_bytes(kind: &str, label: &str, value: &[u8]) -> Result<(), Error> {
    if value.len() != core::SIDE_CHANNEL_BLAKE3_HASH_BYTES {
        return Err(invalid_asset_protocol_value(
            kind,
            format!(
                "{label} must be {} bytes, got {}",
                core::SIDE_CHANNEL_BLAKE3_HASH_BYTES,
                value.len()
            ),
        ));
    }
    Ok(())
}

fn is_64_hex_hash(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

const fn asset_builder_pattern_kind_key(kind: AssetBuilderPatternKind) -> &'static str {
    match kind {
        AssetBuilderPatternKind::Wildcard => "wildcard",
        AssetBuilderPatternKind::Regex => "regex",
    }
}

fn is_safe_asset_db_relative_path(path: &str) -> bool {
    if path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains(':')
        || path.trim() != path
    {
        return false;
    }

    let mut has_component = false;
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return false;
        }
        has_component = true;
    }

    has_component
}

fn write_optional_text(
    value: Option<&str>,
    mut builder: core::core_capnp::optional_text::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_text(
    reader: core::core_capnp::optional_text::Reader<'_>,
) -> Result<Option<String>, Error> {
    match reader.which()? {
        core::core_capnp::optional_text::Which::None(()) => Ok(None),
        core::core_capnp::optional_text::Which::Value(value) => Ok(Some(value?.to_string()?)),
    }
}

fn write_optional_i64(
    value: Option<i64>,
    mut builder: core::core_capnp::optional_i64::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_i64(
    reader: core::core_capnp::optional_i64::Reader<'_>,
) -> Result<Option<i64>, Error> {
    match reader.which()? {
        core::core_capnp::optional_i64::Which::None(()) => Ok(None),
        core::core_capnp::optional_i64::Which::Value(value) => Ok(Some(value)),
    }
}

fn write_optional_u32(
    value: Option<u32>,
    mut builder: core::core_capnp::optional_u32::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_u32(
    reader: core::core_capnp::optional_u32::Reader<'_>,
) -> Result<Option<u32>, Error> {
    match reader.which()? {
        core::core_capnp::optional_u32::Which::None(()) => Ok(None),
        core::core_capnp::optional_u32::Which::Value(value) => Ok(Some(value)),
    }
}

fn read_uuid_data(bytes: &[u8]) -> Result<Uuid, Error> {
    Uuid::from_slice(bytes).map_err(|error| Error::failed(format!("invalid UUID bytes: {error}")))
}

impl LeasedAssetJob {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the leased asset job.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::leased_asset_job::Builder<'_>,
    ) -> Result<(), Error> {
        write_leased_asset_job(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the leased asset job is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: asset_capnp::leased_asset_job::Reader<'_>) -> Result<Self, Error> {
        read_leased_asset_job(reader)
    }
}

impl InspectJobRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the inspect job request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::inspect_job_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_inspect_job_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the inspect job request is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: asset_capnp::inspect_job_request::Reader<'_>) -> Result<Self, Error> {
        read_inspect_job_request(reader)
    }
}

impl JobRecord {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the job record.
    pub fn to_capnp(&self, builder: asset_capnp::job_record::Builder<'_>) -> Result<(), Error> {
        write_job_record(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the job record is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(reader: asset_capnp::job_record::Reader<'_>) -> Result<Self, Error> {
        read_job_record(reader)
    }
}

impl JobAttemptRecord {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the job attempt record.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::job_attempt_record::Builder<'_>,
    ) -> Result<(), Error> {
        write_job_attempt_record(self, builder);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if a field of the job attempt record is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: asset_capnp::job_attempt_record::Reader<'_>) -> Result<Self, Error> {
        read_job_attempt_record(reader)
    }
}

impl JobActivity {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the job activity.
    pub fn to_capnp(&self, builder: asset_capnp::job_activity::Builder<'_>) -> Result<(), Error> {
        write_job_activity(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the job activity is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(reader: asset_capnp::job_activity::Reader<'_>) -> Result<Self, Error> {
        read_job_activity(reader)
    }
}

impl JobProductRecord {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the job product record.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::job_product_record::Builder<'_>,
    ) -> Result<(), Error> {
        write_job_product_record(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the job product record is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: asset_capnp::job_product_record::Reader<'_>) -> Result<Self, Error> {
        read_job_product_record(reader)
    }
}

impl JobDependencyRecord {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the job dependency record.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::job_dependency_record::Builder<'_>,
    ) -> Result<(), Error> {
        write_job_dependency_record(self, builder);
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if a field of the job dependency record is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::job_dependency_record::Reader<'_>,
    ) -> Result<Self, Error> {
        read_job_dependency_record(reader)
    }
}

impl JobInspection {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the job inspection.
    pub fn to_capnp(&self, builder: asset_capnp::job_inspection::Builder<'_>) -> Result<(), Error> {
        write_job_inspection(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the job inspection is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(reader: asset_capnp::job_inspection::Reader<'_>) -> Result<Self, Error> {
        read_job_inspection(reader)
    }
}

impl InspectJobResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the inspect job result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::inspect_job_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_inspect_job_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the inspect job result is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: asset_capnp::inspect_job_result::Reader<'_>) -> Result<Self, Error> {
        read_inspect_job_result(reader)
    }
}

impl AssetBuilderPatternDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the asset builder pattern
    /// descriptor.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::asset_builder_pattern_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_asset_builder_pattern_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the asset builder pattern descriptor is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::asset_builder_pattern_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_asset_builder_pattern_descriptor(reader)
    }
}

impl AssetBuilderDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the asset builder
    /// descriptor.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::asset_builder_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_asset_builder_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the asset builder descriptor is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::asset_builder_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_asset_builder_descriptor(reader)
    }
}

impl SourceSchemaDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source schema
    /// descriptor.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_schema_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_schema_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source schema descriptor is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_schema_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_schema_descriptor(reader)
    }
}

impl SourceFileTemplateDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file template
    /// descriptor.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_template_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_template_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file template descriptor is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_template_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_template_descriptor(reader)
    }
}

impl AssetBuilderCatalogRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the asset builder catalog
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::asset_builder_catalog_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_asset_builder_catalog_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the asset builder catalog request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::asset_builder_catalog_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_asset_builder_catalog_request(reader)
    }
}

impl ProductFormatDescriptor {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the product format
    /// descriptor.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::product_format_descriptor::Builder<'_>,
    ) -> Result<(), Error> {
        write_product_format_descriptor(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the product format descriptor is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::product_format_descriptor::Reader<'_>,
    ) -> Result<Self, Error> {
        read_product_format_descriptor(reader)
    }
}

impl AssetBuilderCatalogResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the asset builder catalog
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::asset_builder_catalog_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_asset_builder_catalog_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the asset builder catalog result is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::asset_builder_catalog_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_asset_builder_catalog_result(reader)
    }
}

impl WorkspaceEntry {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the workspace entry.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::workspace_entry::Builder<'_>,
    ) -> Result<(), Error> {
        write_workspace_entry(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the workspace entry is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: asset_capnp::workspace_entry::Reader<'_>) -> Result<Self, Error> {
        read_workspace_entry(reader)
    }
}

impl WorkspaceEntryPageRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the workspace entry page
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::workspace_entry_page_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_workspace_entry_page_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the workspace entry page request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::workspace_entry_page_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_workspace_entry_page_request(reader)
    }
}

impl WorkspaceEntryPageResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the workspace entry page
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::workspace_entry_page_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_workspace_entry_page_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the workspace entry page result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::workspace_entry_page_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_workspace_entry_page_result(reader)
    }
}

impl AssetProcessorEvent {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the asset processor event.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::asset_processor_event::Builder<'_>,
    ) -> Result<(), Error> {
        write_asset_processor_event(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the asset processor event is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::asset_processor_event::Reader<'_>,
    ) -> Result<Self, Error> {
        read_asset_processor_event(reader)
    }
}

impl AssetProcessorEventSubscriptionRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the asset processor event
    /// subscription request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::asset_processor_event_subscription_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_asset_processor_event_subscription_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the asset processor event subscription request is absent from
    /// the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::asset_processor_event_subscription_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_asset_processor_event_subscription_request(reader)
    }
}

impl AssetProcessorEventSubscriptionResult {
    pub fn to_capnp(
        &self,
        builder: asset_capnp::asset_processor_event_subscription_result::Builder<'_>,
    ) {
        write_asset_processor_event_subscription_result(self, builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the asset processor event subscription result is absent from
    /// the message or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::asset_processor_event_subscription_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_asset_processor_event_subscription_result(reader)
    }
}

impl WorkspaceSnapshot {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the workspace snapshot.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::workspace_snapshot::Builder<'_>,
    ) -> Result<(), Error> {
        write_workspace_snapshot(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the workspace snapshot is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: asset_capnp::workspace_snapshot::Reader<'_>) -> Result<Self, Error> {
        read_workspace_snapshot(reader)
    }
}

impl WorkspaceRoot {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the workspace root.
    pub fn to_capnp(&self, builder: asset_capnp::workspace_root::Builder<'_>) -> Result<(), Error> {
        write_workspace_root(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the workspace root is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(reader: asset_capnp::workspace_root::Reader<'_>) -> Result<Self, Error> {
        read_workspace_root(reader)
    }
}

impl WorkspaceSnapshotRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the workspace snapshot
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::workspace_snapshot_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_workspace_snapshot_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the workspace snapshot request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::workspace_snapshot_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_workspace_snapshot_request(reader)
    }
}

impl WorkspaceSnapshotResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the workspace snapshot
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::workspace_snapshot_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_workspace_snapshot_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the workspace snapshot result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::workspace_snapshot_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_workspace_snapshot_result(reader)
    }
}

impl CatalogProductEntry {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the catalog product entry.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::catalog_product_entry::Builder<'_>,
    ) -> Result<(), Error> {
        write_catalog_product_entry(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the catalog product entry is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::catalog_product_entry::Reader<'_>,
    ) -> Result<Self, Error> {
        read_catalog_product_entry(reader)
    }
}

impl CatalogProductsRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the catalog products
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::catalog_products_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_catalog_products_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the catalog products request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::catalog_products_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_catalog_products_request(reader)
    }
}

impl CatalogProductsResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the catalog products result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::catalog_products_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_catalog_products_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the catalog products result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::catalog_products_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_catalog_products_result(reader)
    }
}

impl ReleaseContentReadRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the release content read
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::release_content_read_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_release_content_read_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the release content read request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::release_content_read_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_release_content_read_request(reader)
    }
}

impl ReleaseContentProduct {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the release content product.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::release_content_product::Builder<'_>,
    ) -> Result<(), Error> {
        write_release_content_product(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the release content product is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::release_content_product::Reader<'_>,
    ) -> Result<Self, Error> {
        read_release_content_product(reader)
    }
}

impl ReleaseContentReadResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the release content read
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::release_content_read_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_release_content_read_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the release content read result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::release_content_read_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_release_content_read_result(reader)
    }
}

impl SourceAssetRecordRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source asset record
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_asset_record_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_asset_record_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source asset record request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_asset_record_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_asset_record_request(reader)
    }
}

impl SourceAssetRecordResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source asset record
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_asset_record_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_asset_record_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source asset record result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_asset_record_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_asset_record_result(reader)
    }
}

impl SourceFileCreateRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file create
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_create_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_create_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file create request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_create_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_create_request(reader)
    }
}

impl SourceFileCreateResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file create
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_create_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_create_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file create result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_create_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_create_result(reader)
    }
}

impl WorkspaceSourceFileRef {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the workspace source file
    /// ref.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::workspace_source_file_ref::Builder<'_>,
    ) -> Result<(), Error> {
        write_workspace_source_file_ref(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the workspace source file ref is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::workspace_source_file_ref::Reader<'_>,
    ) -> Result<Self, Error> {
        read_workspace_source_file_ref(reader)
    }
}

impl SourceFileEditRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file edit
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_edit_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_edit_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file edit request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_edit_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_edit_request(reader)
    }
}

impl SourceFileOpenRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file open
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_open_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_open_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file open request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_open_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_open_request(reader)
    }
}

impl SourceFileRestoreRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file restore
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_restore_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_restore_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file restore request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_restore_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_restore_request(reader)
    }
}

impl SourceFileCodecRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file codec
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_codec_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_codec_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file codec request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_codec_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_codec_request(reader)
    }
}

impl SourceFileCodecResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file codec
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_codec_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_codec_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file codec result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_codec_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_codec_result(reader)
    }
}

impl SourceFileEditSnapshot {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file edit
    /// snapshot.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_edit_snapshot::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_edit_snapshot(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file edit snapshot is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_edit_snapshot::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_edit_snapshot(reader)
    }
}

impl SourceFileEditResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file edit result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_edit_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_edit_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file edit result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_edit_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_edit_result(reader)
    }
}

impl SourceFileOpenResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file open result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_open_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_open_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file open result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_open_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_open_result(reader)
    }
}

impl SourceFileRestoreResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file restore
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_restore_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_restore_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file restore result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_restore_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_restore_result(reader)
    }
}

impl SourceFileDeleteRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file delete
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_delete_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_delete_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file delete request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_delete_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_delete_request(reader)
    }
}

impl SourceFileDeleteResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file delete
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_delete_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_delete_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file delete result is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_delete_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_delete_result(reader)
    }
}

impl SourceFileMoveRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file move
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_move_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_move_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file move request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_move_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_move_request(reader)
    }
}

impl SourceFileMoveResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source file move result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_file_move_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_file_move_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source file move result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_file_move_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_file_move_result(reader)
    }
}

impl ForceReprocessAssetRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the force reprocess asset
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::force_reprocess_asset_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_force_reprocess_asset_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the force reprocess asset request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::force_reprocess_asset_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_force_reprocess_asset_request(reader)
    }
}

impl ForceReprocessAssetResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the force reprocess asset
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::force_reprocess_asset_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_force_reprocess_asset_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the force reprocess asset result is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::force_reprocess_asset_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_force_reprocess_asset_result(reader)
    }
}

impl ReconcileAssetSourcesRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the reconcile asset sources
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::reconcile_asset_sources_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_reconcile_asset_sources_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the reconcile asset sources request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::reconcile_asset_sources_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_reconcile_asset_sources_request(reader)
    }
}

impl ReconcileAssetSourcesResult {
    /// # Errors
    ///
    /// Never returns an error today: the result is three fixed-width counters
    /// with no cross-field invariant, so neither validation nor the writes can
    /// fail. The `Result` is kept for signature parity with the other wire
    /// types, and so a future invariant has somewhere to report.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::reconcile_asset_sources_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_reconcile_asset_sources_result(self, builder);
        Ok(())
    }

    /// # Errors
    ///
    /// Never returns an error today: every field is a fixed-width counter that
    /// decodes without a fallible read. The `Result` is kept for signature
    /// parity with the other wire types.
    pub fn from_capnp(
        reader: asset_capnp::reconcile_asset_sources_result::Reader<'_>,
    ) -> Result<Self, Error> {
        Ok(read_reconcile_asset_sources_result(reader))
    }
}

impl AssetProcessingStatusRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the asset processing status
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::asset_processing_status_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_asset_processing_status_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the asset processing status request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::asset_processing_status_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_asset_processing_status_request(reader)
    }
}

impl AssetProcessingStatusResult {
    pub fn to_capnp(&self, builder: asset_capnp::asset_processing_status_result::Builder<'_>) {
        write_asset_processing_status_result(self, builder);
    }

    #[must_use]
    pub fn from_capnp(reader: asset_capnp::asset_processing_status_result::Reader<'_>) -> Self {
        read_asset_processing_status_result(reader)
    }
}

impl PublishAssetCatalogRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the publish asset catalog
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::publish_asset_catalog_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_publish_asset_catalog_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the publish asset catalog request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::publish_asset_catalog_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_publish_asset_catalog_request(reader)
    }
}

impl PublishAssetCatalogResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the publish asset catalog
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::publish_asset_catalog_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_publish_asset_catalog_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the publish asset catalog result is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::publish_asset_catalog_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_publish_asset_catalog_result(reader)
    }
}

impl PublishBuilderCatalogRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the publish builder catalog
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::publish_builder_catalog_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_publish_builder_catalog_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the publish builder catalog request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::publish_builder_catalog_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_publish_builder_catalog_request(reader)
    }
}

impl PublishBuilderCatalogResult {
    pub fn to_capnp(&self, builder: asset_capnp::publish_builder_catalog_result::Builder<'_>) {
        write_publish_builder_catalog_result(self, builder);
    }

    #[must_use]
    pub fn from_capnp(reader: asset_capnp::publish_builder_catalog_result::Reader<'_>) -> Self {
        read_publish_builder_catalog_result(reader)
    }
}

impl SourceDependentSource {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source dependent source.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_dependent_source::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_dependent_source(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source dependent source is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_dependent_source::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_dependent_source(reader)
    }
}

impl SourceDependentJob {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source dependent job.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_dependent_job::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_dependent_job(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source dependent job is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_dependent_job::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_dependent_job(reader)
    }
}

impl SourceDependentsRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source dependents
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_dependents_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_dependents_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source dependents request is absent from the message or
    /// is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_dependents_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_dependents_request(reader)
    }
}

impl SourceDependentsResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the source dependents
    /// result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::source_dependents_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_source_dependents_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the source dependents result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::source_dependents_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_source_dependents_result(reader)
    }
}

impl LeaseAssetJobRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the lease asset job request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::lease_asset_job_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_lease_asset_job_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the lease asset job request is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::lease_asset_job_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_lease_asset_job_request(reader)
    }
}

impl LeaseAssetJobResult {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the lease asset job result.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::lease_asset_job_result::Builder<'_>,
    ) -> Result<(), Error> {
        write_lease_asset_job_result(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the lease asset job result is absent from the message or is
    /// not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::lease_asset_job_result::Reader<'_>,
    ) -> Result<Self, Error> {
        read_lease_asset_job_result(reader)
    }
}

impl RenewAssetJobLeaseRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the renew asset job lease
    /// request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::renew_asset_job_lease_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_renew_asset_job_lease_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the renew asset job lease request is absent from the message
    /// or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::renew_asset_job_lease_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_renew_asset_job_lease_request(reader)
    }
}

impl CompleteAssetJobAttemptRequest {
    /// # Errors
    ///
    /// Returns an error if the message runs out of space while writing the complete asset job
    /// attempt request.
    pub fn to_capnp(
        &self,
        builder: asset_capnp::complete_asset_job_attempt_request::Builder<'_>,
    ) -> Result<(), Error> {
        write_complete_asset_job_attempt_request(self, builder)
    }

    /// # Errors
    ///
    /// Returns an error if a field of the complete asset job attempt request is absent from the
    /// message or is not valid UTF-8.
    pub fn from_capnp(
        reader: asset_capnp::complete_asset_job_attempt_request::Reader<'_>,
    ) -> Result<Self, Error> {
        read_complete_asset_job_attempt_request(reader)
    }
}

impl ProductManifest {
    /// # Errors
    ///
    /// Returns an error if a field of the product manifest is absent from the message or is not
    /// valid UTF-8.
    pub fn from_capnp(reader: asset_capnp::product_manifest::Reader<'_>) -> Result<Self, Error> {
        read_product_manifest(reader)
    }
}

#[cfg(test)]
mod tests {
    use az_proto_core::{Capability, ServiceId, ServiceRole};
    use capnp::message;

    use super::*;

    #[test]
    fn source_file_extension_matching_accepts_bare_and_prefixed_compound_names() {
        let extensions = vec!["mapsettings.json".to_string()];

        assert!(source_path_matches_source_file_extensions(
            "levels/tutorial/mapsettings.json",
            &extensions
        ));
        assert!(source_path_matches_source_file_extensions(
            "levels/tutorial/region.mapsettings.json",
            &extensions
        ));
        assert!(!source_path_matches_source_file_extensions(
            "levels/tutorial/settings.json",
            &extensions
        ));
    }

    fn test_session_id() -> Uuid {
        Uuid::from_bytes([0x42; 16])
    }

    fn read_capability() -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_session(test_session_id())
            .with_audience(ASSET_PROCESSOR_AUDIENCE)
            .with_permissions([ASSET_READ_PERMISSION])
    }

    fn write_capability() -> Capability {
        Capability::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
        )
        .with_session(test_session_id())
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_WRITE_PERMISSION])
    }

    fn jobs_capability() -> Capability {
        Capability::new(
            ServiceId::new(ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WORKER_SERVICE_NAME),
            ServiceRole::Worker,
        )
        .with_session(test_session_id())
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_JOBS_PERMISSION])
    }

    fn leased_job() -> LeasedAssetJob {
        LeasedAssetJob {
            attempt_id: 42,
            workspace_id: 2,
            owner: JobOwner::Build(Uuid::from_bytes([0xab; 16])),
            source_guid: Uuid::from_bytes([0x5a; 16]),
            preserved_source_sub_id: Some(2),
            source_path: "textures/rpc.png".to_string(),
            source_root: "project/assets".to_string(),
            source_schema_type: Some("az.test.Texture".to_string()),
            job_key: "default".to_string(),
            platform: "pc".to_string(),
            ordinal: 3,
            staging_root: "staging/42".to_string(),
            source_payload: None,
        }
    }

    fn valid_workspace_entry() -> WorkspaceEntry {
        let job = JobRecord {
            job_id: 7,
            workspace_id: 11,
            source_guid: Uuid::from_bytes([0x5a; 16]),
            source_path: "textures/rpc.png".to_string(),
            source_root: "project/assets".to_string(),
            source_schema_type: Some("az.test.Texture".to_string()),
            owner: JobOwner::Build(Uuid::from_bytes([0xab; 16])),
            key: "default".to_string(),
            platform: "pc".to_string(),
            status: JobStatus::Leased,
            ready: true,
            attempts: 3,
        };
        let attempt = JobAttemptRecord {
            attempt_id: 42,
            job_id: job.job_id,
            ordinal: 3,
            status: AttemptStatus::Leased,
            owner: Some("worker-a".to_string()),
            staging: Some("staging/42".to_string()),
            finished_unix_ms: None,
            error_count: 0,
            warning_count: 1,
        };
        WorkspaceEntry {
            entry_id: 55,
            workspace_id: 11,
            asset_guid: Uuid::from_bytes([0x5a; 16]),
            root_id: 3,
            source_path: "textures/rpc.png".to_string(),
            schema_type: Some("az.test.Texture".to_string()),
            content_hash: "ab".repeat(32),
            diff: WorkspaceEntryDiff::Modified,
            diagnostics_count: 2,
            updated_unix_ms: 500,
            jobs: vec![JobActivity {
                job,
                attempt: Some(attempt),
            }],
        }
    }

    fn valid_asset_builder_catalog_result() -> AssetBuilderCatalogResult {
        AssetBuilderCatalogResult {
            builders: vec![AssetBuilderDescriptor {
                name: "az.test.prefab".to_string(),
                builder_guid: Uuid::from_bytes([0xb1; 16]),
                version: 1,
                analysis_fingerprint: "prefab-analysis-v1".to_string(),
                patterns: vec![
                    AssetBuilderPatternDescriptor {
                        kind: AssetBuilderPatternKind::Wildcard,
                        pattern: "*.prefab.ron".to_string(),
                    },
                    AssetBuilderPatternDescriptor {
                        kind: AssetBuilderPatternKind::Regex,
                        pattern: r"^levels/.+\.scene\.ron$".to_string(),
                    },
                ],
                source_schema_types: vec!["az.test.Prefab".to_string()],
            }],
            source_schemas: vec![SourceSchemaDescriptor {
                schema_type: "az.test.Prefab".to_string(),
                owner: "az-test".to_string(),
                label: "Prefab".to_string(),
                category: "Authoring".to_string(),
                authoring: SourceSchemaAuthoring::ProjectDocument {
                    schema_type: "az.test.Prefab".to_string(),
                },
                file_templates: Vec::new(),
            }],
            product_formats: vec![ProductFormatDescriptor {
                id: "az.test.prefab".to_string(),
                current_version: 1,
                owner: "az-test".to_string(),
            }],
        }
    }

    fn valid_source_file_workflow() -> SourceFileWorkflowDescriptor {
        SourceFileWorkflowDescriptor {
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            default_path_prefix: "materials".to_string(),
            extensions: vec!["mtl".to_string()],
            can_create: false,
            can_edit: false,
        }
    }

    fn valid_workspace_snapshot_result() -> WorkspaceSnapshotResult {
        WorkspaceSnapshotResult {
            snapshot: Some(WorkspaceSnapshot {
                workspace_id: 11,
                project_id: "local.proto_asset_view".to_string(),
                workspace_root: "project".to_string(),
                branch: "main".to_string(),
                created_unix_ms: 100,
                updated_unix_ms: 200,
                roots: vec![WorkspaceRoot {
                    workspace_root_id: 22,
                    workspace_id: 11,
                    root_id: 33,
                    declared_root_id: "project.assets".to_string(),
                    owner_id: "local.proto_asset_view".to_string(),
                    source_root: "project/assets".to_string(),
                    display_name: "Project Assets".to_string(),
                    portable_key: "project:local.proto_asset_view:assets".to_string(),
                    mount: "@assets@".to_string(),
                    recursive: true,
                    watch: true,
                    writable: true,
                    output_prefix: String::new(),
                    is_root: true,
                }],
            }),
        }
    }

    fn valid_catalog_products_result() -> CatalogProductsResult {
        CatalogProductsResult {
            entries: vec![CatalogProductEntry {
                job_id: 42,
                product_id: 8,
                asset_guid: Uuid::from_bytes([0x5a; 16]),
                source_path: "textures/rpc.png".to_string(),
                builder_guid: Uuid::from_bytes([0xb1; 16]),
                job_key: "compile-texture".to_string(),
                platform: "pc".to_string(),
                product_path: "textures/rpc.dds".to_string(),
                asset_type: Uuid::from_bytes([0xcd; 16]),
                sub_id: 4,
                product_format: "az.test.raw".to_string(),
                product_format_version: 1,
                content_hash: "ef".repeat(32),
                catalog_aliases: vec!["textures/rpc.texture".to_string()],
                catalog_path_registration: CatalogPathRegistration::Registered,
                byte_length: 4096,
                dependencies: vec![
                    CatalogProductDependency {
                        asset_guid: Uuid::from_bytes([0x11; 16]),
                        sub_id: 0,
                        asset_type: Some(Uuid::from_bytes([0x22; 16])),
                        hint: Some("@assets@/textures/base.dds".to_string()),
                    },
                    CatalogProductDependency {
                        asset_guid: Uuid::from_bytes([0x33; 16]),
                        sub_id: 7,
                        asset_type: None,
                        hint: None,
                    },
                ],
            }],
        }
    }

    fn round_trip_asset_builder_catalog_result(
        result: &AssetBuilderCatalogResult,
    ) -> Result<AssetBuilderCatalogResult, Error> {
        let mut message = message::Builder::new_default();
        (result).to_capnp(
            message.init_root::<asset_capnp::asset_builder_catalog_result::Builder<'_>>(),
        )?;
        let reader = message
            .get_root_as_reader::<asset_capnp::asset_builder_catalog_result::Reader<'_>>()
            .unwrap();
        AssetBuilderCatalogResult::from_capnp(reader)
    }

    fn round_trip_workspace_entry_page_result(
        result: &WorkspaceEntryPageResult,
    ) -> Result<WorkspaceEntryPageResult, Error> {
        let mut message = message::Builder::new_default();
        (result).to_capnp(
            message.init_root::<asset_capnp::workspace_entry_page_result::Builder<'_>>(),
        )?;
        let reader = message
            .get_root_as_reader::<asset_capnp::workspace_entry_page_result::Reader<'_>>()
            .unwrap();
        WorkspaceEntryPageResult::from_capnp(reader)
    }

    fn round_trip_asset_processor_event(
        event: &AssetProcessorEvent,
    ) -> Result<AssetProcessorEvent, Error> {
        let mut message = message::Builder::new_default();
        (event).to_capnp(message.init_root::<asset_capnp::asset_processor_event::Builder<'_>>())?;
        let reader = message
            .get_root_as_reader::<asset_capnp::asset_processor_event::Reader<'_>>()
            .unwrap();
        AssetProcessorEvent::from_capnp(reader)
    }

    fn round_trip_workspace_snapshot_result(
        result: &WorkspaceSnapshotResult,
    ) -> Result<WorkspaceSnapshotResult, Error> {
        let mut message = message::Builder::new_default();
        (result)
            .to_capnp(message.init_root::<asset_capnp::workspace_snapshot_result::Builder<'_>>())?;
        let reader = message
            .get_root_as_reader::<asset_capnp::workspace_snapshot_result::Reader<'_>>()
            .unwrap();
        WorkspaceSnapshotResult::from_capnp(reader)
    }

    fn round_trip_catalog_products_result(
        result: &CatalogProductsResult,
    ) -> Result<CatalogProductsResult, Error> {
        let mut message = message::Builder::new_default();
        (result)
            .to_capnp(message.init_root::<asset_capnp::catalog_products_result::Builder<'_>>())?;
        let reader = message
            .get_root_as_reader::<asset_capnp::catalog_products_result::Reader<'_>>()
            .unwrap();
        CatalogProductsResult::from_capnp(reader)
    }

    fn round_trip_release_content_read_request(
        request: &ReleaseContentReadRequest,
    ) -> Result<ReleaseContentReadRequest, Error> {
        let mut message = message::Builder::new_default();
        (request).to_capnp(
            message.init_root::<asset_capnp::release_content_read_request::Builder<'_>>(),
        )?;
        let reader = message
            .get_root_as_reader::<asset_capnp::release_content_read_request::Reader<'_>>()
            .unwrap();
        ReleaseContentReadRequest::from_capnp(reader)
    }

    fn round_trip_release_content_read_result(
        result: &ReleaseContentReadResult,
    ) -> Result<ReleaseContentReadResult, Error> {
        let mut message = message::Builder::new_default();
        (result).to_capnp(
            message.init_root::<asset_capnp::release_content_read_result::Builder<'_>>(),
        )?;
        let reader = message
            .get_root_as_reader::<asset_capnp::release_content_read_result::Reader<'_>>()
            .unwrap();
        ReleaseContentReadResult::from_capnp(reader)
    }

    fn release_content_handle(
        root: &std::path::Path,
        file_name: &str,
        byte_length: u64,
        content_hash: Vec<u8>,
    ) -> SideChannelHandle {
        SideChannelHandle::staging_file(
            root.join(file_name).to_string_lossy().into_owned(),
            byte_length,
            content_hash,
            std::env::consts::OS,
        )
        .with_capability(read_capability())
    }

    fn valid_release_content_product(root: &std::path::Path) -> ReleaseContentProduct {
        let content_hash = vec![0x44; core::SIDE_CHANNEL_BLAKE3_HASH_BYTES];
        ReleaseContentProduct {
            asset_guid: Uuid::from_bytes([0x5a; 16]),
            sub_id: 4,
            product_path: "tables/archetypes.aztbl".to_string(),
            product_format: "aztbl".to_string(),
            product_format_version: 1,
            byte_length: 2048,
            content_hash: content_hash.clone(),
            payload: release_content_handle(root, "archetypes.aztbl", 2048, content_hash),
        }
    }

    #[test]
    fn lease_result_round_trips_attempt_and_grant() {
        let expected = LeaseAssetJobResult {
            leased: leased_job(),
            grant_key: Uuid::from_bytes([0x21; 16]),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::lease_asset_job_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::lease_asset_job_result::Reader<'_>>()
            .unwrap();
        let actual = LeaseAssetJobResult::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn lease_result_round_trips_planner_owner() {
        let mut attempt = leased_job();
        attempt.owner = JobOwner::Plan;
        let expected = LeaseAssetJobResult {
            leased: attempt,
            grant_key: Uuid::from_bytes([0x22; 16]),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::lease_asset_job_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::lease_asset_job_result::Reader<'_>>()
            .unwrap();
        let actual = LeaseAssetJobResult::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn lease_result_rejects_nil_build_owner() {
        let mut attempt = leased_job();
        attempt.owner = JobOwner::Build(Uuid::nil());
        let result = LeaseAssetJobResult {
            leased: attempt,
            grant_key: Uuid::from_bytes([0x23; 16]),
        };
        let mut message = message::Builder::new_default();
        let error = result
            .to_capnp(message.init_root::<asset_capnp::lease_asset_job_result::Builder<'_>>())
            .expect_err("nil build owner must not cross the transport boundary");
        assert!(error.to_string().contains("builder guid"), "{error}");
    }

    #[test]
    fn lease_result_round_trips_source_payload_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let mut attempt = leased_job();
        attempt.source_payload = Some(
            SideChannelHandle::staging_file(
                temp.path().join("source.ron").to_string_lossy(),
                128,
                vec![0x11; 32],
                std::env::consts::OS,
            )
            .with_capability(jobs_capability()),
        );
        let expected = LeaseAssetJobResult {
            leased: attempt,
            grant_key: Uuid::from_bytes([0x25; 16]),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::lease_asset_job_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::lease_asset_job_result::Reader<'_>>()
            .unwrap();
        let actual = LeaseAssetJobResult::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn asset_builder_catalog_round_trips_builders_and_patterns() {
        let expected_request = AssetBuilderCatalogRequest {
            capability: read_capability(),
        };
        let mut request_message = message::Builder::new_default();
        (expected_request)
            .to_capnp(
                request_message
                    .init_root::<asset_capnp::asset_builder_catalog_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<asset_capnp::asset_builder_catalog_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            AssetBuilderCatalogRequest::from_capnp(request_reader).unwrap(),
            expected_request
        );

        let expected_result = valid_asset_builder_catalog_result();
        let mut result_message = message::Builder::new_default();
        (expected_result)
            .to_capnp(
                result_message
                    .init_root::<asset_capnp::asset_builder_catalog_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<asset_capnp::asset_builder_catalog_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            AssetBuilderCatalogResult::from_capnp(result_reader).unwrap(),
            expected_result
        );

        let expected_publish = PublishBuilderCatalogRequest {
            capability: jobs_capability(),
            protocol: core::ProtocolVersion::CURRENT,
            catalog: valid_asset_builder_catalog_result(),
        };
        let mut publish_message = message::Builder::new_default();
        expected_publish
            .to_capnp(
                publish_message
                    .init_root::<asset_capnp::publish_builder_catalog_request::Builder<'_>>(),
            )
            .unwrap();
        let publish_reader = publish_message
            .get_root_as_reader::<asset_capnp::publish_builder_catalog_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            PublishBuilderCatalogRequest::from_capnp(publish_reader).unwrap(),
            expected_publish
        );
    }

    #[test]
    fn asset_builder_catalog_rejects_schema_filters_without_descriptors() {
        let mut result = valid_asset_builder_catalog_result();
        result.source_schemas.clear();

        let mut message = message::Builder::new_default();
        let error = (result)
            .to_capnp(message.init_root::<asset_capnp::asset_builder_catalog_result::Builder<'_>>())
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("without a source schema descriptor"),
            "{error}"
        );
    }

    #[test]
    fn asset_read_request_encode_rejects_wrong_permission() {
        let request = InspectJobRequest {
            capability: jobs_capability(),
            selector: InspectJobSelector::Attempt(42),
        };
        let mut message = message::Builder::new_default();
        let error = (request)
            .to_capnp(message.init_root::<asset_capnp::inspect_job_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains(ASSET_READ_PERMISSION), "{error}");
    }

    #[test]
    fn asset_read_request_round_trips_project_scoped_capability() {
        let mut capability = read_capability();
        capability.session = None;
        let request = AssetBuilderCatalogRequest {
            capability: capability.clone(),
        };
        let mut message = message::Builder::new_default();
        request
            .to_capnp(
                message.init_root::<asset_capnp::asset_builder_catalog_request::Builder<'_>>(),
            )
            .unwrap();
        let reader = message
            .get_root_as_reader::<asset_capnp::asset_builder_catalog_request::Reader<'_>>()
            .unwrap();
        let round_trip = AssetBuilderCatalogRequest::from_capnp(reader).unwrap();

        assert_eq!(round_trip.capability, capability);
    }

    #[test]
    fn workspace_entry_page_result_round_trips_entries_and_cursor() {
        let expected = WorkspaceEntryPageResult {
            entries: vec![valid_workspace_entry()],
            next_after_entry_id: Some(55),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::workspace_entry_page_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::workspace_entry_page_result::Reader<'_>>()
            .unwrap();
        let actual = WorkspaceEntryPageResult::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn workspace_entry_page_request_round_trips_attached_scope() {
        let expected = WorkspaceEntryPageRequest {
            capability: read_capability(),
            root_scope: AssetRootScope::All,
            after_entry_id: Some(123),
            page_size: 256,
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::workspace_entry_page_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::workspace_entry_page_request::Reader<'_>>()
            .unwrap();
        let actual = WorkspaceEntryPageRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn asset_processing_status_round_trips_request_and_result() {
        let expected_request = AssetProcessingStatusRequest {
            capability: write_capability(),
            session_id: "asset-session".to_string(),
            platform: "pc".to_string(),
        };
        let mut request_message = message::Builder::new_default();
        expected_request
            .to_capnp(
                request_message
                    .init_root::<asset_capnp::asset_processing_status_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<asset_capnp::asset_processing_status_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            AssetProcessingStatusRequest::from_capnp(request_reader).unwrap(),
            expected_request
        );

        let expected_result = AssetProcessingStatusResult {
            queued: 2,
            leased: 3,
            failed: 7,
            in_flight_sweeps: 1,
        };
        let mut result_message = message::Builder::new_default();
        expected_result.to_capnp(
            result_message.init_root::<asset_capnp::asset_processing_status_result::Builder<'_>>(),
        );
        let result_reader = result_message
            .get_root_as_reader::<asset_capnp::asset_processing_status_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            AssetProcessingStatusResult::from_capnp(result_reader),
            expected_result
        );
    }

    #[test]
    fn asset_processor_event_round_trips_entry_snapshot() {
        let expected = AssetProcessorEvent {
            seq: 7,
            kind: AssetProcessorEventKind::JobCompleted,
            event_unix_ms: 1_250,
            entry: valid_workspace_entry(),
        };

        let actual = round_trip_asset_processor_event(&expected).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn asset_processor_event_subscription_request_round_trips_attached_scope() {
        let expected = AssetProcessorEventSubscriptionRequest {
            capability: read_capability(),
        };
        let mut message = message::Builder::new_default();
        (expected).to_capnp(message
                .init_root::<asset_capnp::asset_processor_event_subscription_request::Builder<'_>>(
                ))
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                asset_capnp::asset_processor_event_subscription_request::Reader<'_>,
            >()
            .unwrap();
        let actual = AssetProcessorEventSubscriptionRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn workspace_snapshot_request_and_result_round_trip_attached_scope() {
        let expected_request = WorkspaceSnapshotRequest {
            capability: read_capability(),
            root_scope: AssetRootScope::All,
        };
        let mut request_message = message::Builder::new_default();
        (expected_request)
            .to_capnp(
                request_message.init_root::<asset_capnp::workspace_snapshot_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<asset_capnp::workspace_snapshot_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            WorkspaceSnapshotRequest::from_capnp(request_reader).unwrap(),
            expected_request
        );

        let expected_result = valid_workspace_snapshot_result();
        let mut result_message = message::Builder::new_default();
        (expected_result)
            .to_capnp(
                result_message.init_root::<asset_capnp::workspace_snapshot_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<asset_capnp::workspace_snapshot_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            WorkspaceSnapshotResult::from_capnp(result_reader).unwrap(),
            expected_result
        );
    }

    #[test]
    fn catalog_products_request_and_result_round_trip() {
        let expected_request = CatalogProductsRequest {
            capability: read_capability(),
            platform: "pc".to_string(),
        };
        let mut request_message = message::Builder::new_default();
        (expected_request)
            .to_capnp(
                request_message.init_root::<asset_capnp::catalog_products_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<asset_capnp::catalog_products_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            CatalogProductsRequest::from_capnp(request_reader).unwrap(),
            expected_request
        );

        let expected_result = valid_catalog_products_result();
        let actual = round_trip_catalog_products_result(&expected_result).unwrap();
        assert_eq!(actual, expected_result);
    }

    #[test]
    fn release_content_read_request_round_trips_catalog_and_product_targets() {
        let catalog_request = ReleaseContentReadRequest {
            capability: read_capability(),
            platform: "pc".to_string(),
            target: ReleaseContentTarget::AssetCatalog,
        };
        assert_eq!(
            round_trip_release_content_read_request(&catalog_request).unwrap(),
            catalog_request
        );

        let product_request = ReleaseContentReadRequest {
            capability: read_capability(),
            platform: "pc".to_string(),
            target: ReleaseContentTarget::ProductAsset {
                asset_guid: Uuid::from_bytes([0x5a; 16]),
                sub_id: 4,
            },
        };
        assert_eq!(
            round_trip_release_content_read_request(&product_request).unwrap(),
            product_request
        );
    }

    #[test]
    fn release_content_read_result_round_trips_side_channel_payloads() {
        let temp = tempfile::tempdir().unwrap();
        let catalog_hash = vec![0x33; core::SIDE_CHANNEL_BLAKE3_HASH_BYTES];
        let catalog = ReleaseContentReadResult::AssetCatalog(release_content_handle(
            temp.path(),
            "assetcatalog.bin",
            1024,
            catalog_hash,
        ));
        assert_eq!(
            round_trip_release_content_read_result(&catalog).unwrap(),
            catalog
        );

        let product = ReleaseContentReadResult::Product(valid_release_content_product(temp.path()));
        assert_eq!(
            round_trip_release_content_read_result(&product).unwrap(),
            product
        );
    }

    #[test]
    fn release_content_read_request_encode_rejects_wrong_permission() {
        let capability = read_capability().with_permissions([ASSET_WRITE_PERMISSION]);
        let request = ReleaseContentReadRequest {
            capability,
            platform: "pc".to_string(),
            target: ReleaseContentTarget::AssetCatalog,
        };
        let error = round_trip_release_content_read_request(&request).unwrap_err();
        assert!(error.to_string().contains(ASSET_READ_PERMISSION), "{error}");
    }

    #[test]
    fn release_content_result_encode_rejects_unbound_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let mut product = valid_release_content_product(temp.path());
        product.payload.capability = None;
        let error =
            round_trip_release_content_read_result(&ReleaseContentReadResult::Product(product))
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing its brokered capability"),
            "{error}"
        );
    }

    #[test]
    fn release_content_result_encode_rejects_mismatched_payload_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let mut product = valid_release_content_product(temp.path());
        product.payload.byte_length += 1;
        let error =
            round_trip_release_content_read_result(&ReleaseContentReadResult::Product(product))
                .unwrap_err();
        assert!(error.to_string().contains("byte length"), "{error}");

        let mut product = valid_release_content_product(temp.path());
        product.payload.content_hash = vec![0x99; core::SIDE_CHANNEL_BLAKE3_HASH_BYTES];
        let error =
            round_trip_release_content_read_result(&ReleaseContentReadResult::Product(product))
                .unwrap_err();
        assert!(error.to_string().contains("content hash"), "{error}");
    }

    #[test]
    fn source_asset_record_round_trips_saved_source_metadata() {
        let expected = SourceAssetRecordRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            workspace_root_id: 42,
            owner_id: "local.proto_asset".to_string(),
            source_path: "prefabs/door.prefab.ron".to_string(),
            schema_type: Some("az.test.Prefab".to_string()),
            content_hash: vec![0xab; 32],
            changed_unix_ms: 1_000,
            diagnostics_count: 2,
        };

        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::source_asset_record_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::source_asset_record_request::Reader<'_>>()
            .unwrap();
        let actual = SourceAssetRecordRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);

        let expected_result = SourceAssetRecordResult {
            asset_guid: Uuid::from_bytes([0x34; 16]),
            entry: WorkspaceEntry {
                entry_id: 77,
                workspace_id: 11,
                asset_guid: Uuid::from_bytes([0x34; 16]),
                root_id: 3,
                source_path: "prefabs/door.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash: "ab".repeat(32),
                diff: WorkspaceEntryDiff::Added,
                diagnostics_count: 2,
                updated_unix_ms: 1_000,
                jobs: Vec::new(),
            },
        };
        let mut message = message::Builder::new_default();
        (expected_result)
            .to_capnp(message.init_root::<asset_capnp::source_asset_record_result::Builder<'_>>())
            .unwrap();
        let reader = message
            .get_root_as_reader::<asset_capnp::source_asset_record_result::Reader<'_>>()
            .unwrap();
        let actual_result = SourceAssetRecordResult::from_capnp(reader).unwrap();
        assert_eq!(actual_result, expected_result);
    }

    #[test]
    fn publish_asset_catalog_result_round_trips_reuse_outcome() {
        let expected = PublishAssetCatalogResult {
            catalog_path: "Cache/pc/assetcatalog.bin".to_string(),
            entry_count: 42,
            reused: true,
        };
        let mut message = message::Builder::new_default();
        expected
            .to_capnp(message.init_root::<asset_capnp::publish_asset_catalog_result::Builder<'_>>())
            .unwrap();
        let reader = message
            .get_root_as_reader::<asset_capnp::publish_asset_catalog_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            PublishAssetCatalogResult::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn source_file_create_request_round_trips_default_template() {
        let expected = SourceFileCreateRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            source_path: "achievementdata/achievementdatatable.ron".to_string(),
            schema_type: "azoth.gamedata.TableSource".to_string(),
            changed_unix_ms: 1_000,
            content: SourceFileCreateContent::DefaultTemplate,
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::source_file_create_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::source_file_create_request::Reader<'_>>()
            .unwrap();
        let actual = SourceFileCreateRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn source_file_create_request_round_trips_payload_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let payload = SideChannelHandle::staging_file(
            temp.path().join("table.ron").to_string_lossy(),
            3,
            blake3::hash(b"[]\n").as_bytes().to_vec(),
            std::env::consts::OS,
        )
        .with_capability(write_capability());
        let expected = SourceFileCreateRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            source_path: "achievementdata/achievementdatatable.ron".to_string(),
            schema_type: "azoth.gamedata.TableSource".to_string(),
            changed_unix_ms: 1_000,
            content: SourceFileCreateContent::Payload(Box::new(payload)),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::source_file_create_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::source_file_create_request::Reader<'_>>()
            .unwrap();
        let actual = SourceFileCreateRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    fn workspace_source_file_ref() -> WorkspaceSourceFileRef {
        WorkspaceSourceFileRef {
            source_root_key: "project:local.proto_asset:assets".to_string(),
            source_path: "sharedassets/gamedata/achievementdata.ron".to_string(),
            schema_type: "azoth.gamedata.TableSource".to_string(),
        }
    }

    fn source_file_edit_document() -> SourceFileEditDocument {
        SourceFileEditDocument {
            root_object_id: Some("achievement:second".to_string()),
            root_schema: "sample.AchievementData".to_string(),
            value: ReflectedValueEnvelope::typed_ron("sample.AchievementData", "(id: \"second\")"),
            objects: vec![
                SourceFileEditObject {
                    object_id: "achievement:first".to_string(),
                    schema: "sample.AchievementData".to_string(),
                    value: ReflectedValueEnvelope::typed_ron(
                        "sample.AchievementData",
                        "(id: \"first\")",
                    ),
                },
                SourceFileEditObject {
                    object_id: "achievement:second".to_string(),
                    schema: "sample.AchievementData".to_string(),
                    value: ReflectedValueEnvelope::typed_ron(
                        "sample.AchievementData",
                        "(id: \"second\")",
                    ),
                },
            ],
            codec_state: b"stable-table-identity".to_vec(),
        }
    }

    fn source_file_edit_side_channel(
        root: &std::path::Path,
        name: &str,
        bytes: &[u8],
    ) -> SideChannelHandle {
        SideChannelHandle::staging_file(
            root.join(name).to_string_lossy(),
            bytes.len() as u64,
            blake3::hash(bytes).as_bytes().to_vec(),
            std::env::consts::OS,
        )
        .with_capability(jobs_capability())
    }

    fn source_file_codec_output_destination(
        root: &std::path::Path,
    ) -> SourceFileCodecOutputDestination {
        SourceFileCodecOutputDestination {
            locator: root.join("saved.ron").to_string_lossy().into_owned(),
            platform: std::env::consts::OS.to_string(),
            capability: jobs_capability(),
        }
    }

    #[test]
    fn source_file_edit_request_round_trips_canonical_source_and_operation() {
        let expected = SourceFileEditRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source: workspace_source_file_ref(),
            expected_source_fingerprint: vec![0x44; 32],
            operation: SourceFileEditOperation::DuplicateObject {
                object_id: "achievement:first".to_string(),
            },
        };
        let mut message = message::Builder::new_default();
        expected
            .to_capnp(message.init_root::<asset_capnp::source_file_edit_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::source_file_edit_request::Reader<'_>>()
            .unwrap();
        assert_eq!(SourceFileEditRequest::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn source_file_edit_operations_round_trip_without_format_vocabulary() {
        for expected in [
            SourceFileEditOperation::AppendDefault,
            SourceFileEditOperation::DuplicateObject {
                object_id: "row:a".to_string(),
            },
            SourceFileEditOperation::RemoveObject {
                object_id: "row:b".to_string(),
            },
        ] {
            let mut message = message::Builder::new_default();
            expected
                .to_capnp(
                    message.init_root::<
                        az_proto_authoring::authoring_capnp::source_file_edit_operation::Builder<
                            '_,
                        >,
                    >(),
                )
                .unwrap();
            let reader = message
                .get_root_as_reader::<
                    az_proto_authoring::authoring_capnp::source_file_edit_operation::Reader<'_>,
                >()
                .unwrap();
            assert_eq!(
                SourceFileEditOperation::from_capnp(reader).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn source_file_codec_round_trips_bytes_only_on_worker_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let expected_request = SourceFileCodecRequest {
            source: workspace_source_file_ref(),
            authoritative_source: source_file_edit_side_channel(
                temp.path(),
                "authoritative.ron",
                b"[]\n",
            ),
            operation: SourceFileCodecOperation::Edit(SourceFileEditOperation::RemoveObject {
                object_id: "achievement:first".to_string(),
            }),
            output_destination: source_file_codec_output_destination(temp.path()),
        };
        let mut request_message = message::Builder::new_default();
        expected_request
            .to_capnp(
                request_message.init_root::<asset_capnp::source_file_codec_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<asset_capnp::source_file_codec_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            SourceFileCodecRequest::from_capnp(request_reader).unwrap(),
            expected_request
        );

        let expected_result = SourceFileCodecResult {
            document: source_file_edit_document(),
            replacement: SourceFileCodecReplacement::SavedSource(Box::new(
                source_file_edit_side_channel(temp.path(), "saved.ron", b"[()]\n"),
            )),
        };
        let mut result_message = message::Builder::new_default();
        expected_result
            .to_capnp(
                result_message.init_root::<asset_capnp::source_file_codec_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<asset_capnp::source_file_codec_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            SourceFileCodecResult::from_capnp(result_reader).unwrap(),
            expected_result
        );
    }

    #[test]
    fn source_file_open_round_trips_load_without_replacement() {
        let request = SourceFileOpenRequest {
            capability: read_capability(),
            session_id: "editor-session".to_string(),
            source: workspace_source_file_ref(),
        };
        let mut request_message = message::Builder::new_default();
        request
            .to_capnp(
                request_message.init_root::<asset_capnp::source_file_open_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<asset_capnp::source_file_open_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            SourceFileOpenRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let codec_result = SourceFileCodecResult {
            document: source_file_edit_document(),
            replacement: SourceFileCodecReplacement::Unchanged,
        };
        let mut result_message = message::Builder::new_default();
        codec_result
            .to_capnp(
                result_message.init_root::<asset_capnp::source_file_codec_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<asset_capnp::source_file_codec_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            SourceFileCodecResult::from_capnp(result_reader).unwrap(),
            codec_result
        );
    }

    #[test]
    fn source_file_codec_round_trips_structured_history_restore() {
        let temp = tempfile::tempdir().unwrap();
        let expected = SourceFileCodecRequest {
            source: workspace_source_file_ref(),
            authoritative_source: source_file_edit_side_channel(
                temp.path(),
                "authoritative.ron",
                b"[]\n",
            ),
            operation: SourceFileCodecOperation::RestoreDocument(source_file_edit_document()),
            output_destination: source_file_codec_output_destination(temp.path()),
        };
        let mut message = message::Builder::new_default();
        expected
            .to_capnp(message.init_root::<asset_capnp::source_file_codec_request::Builder<'_>>())
            .unwrap();
        let reader = message
            .get_root_as_reader::<asset_capnp::source_file_codec_request::Reader<'_>>()
            .unwrap();

        assert_eq!(
            SourceFileCodecRequest::from_capnp(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn source_file_restore_round_trips_project_host_document() {
        let request = SourceFileRestoreRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source: workspace_source_file_ref(),
            expected_source_fingerprint: vec![0x71; 32],
            document: source_file_edit_document(),
        };
        let mut request_message = message::Builder::new_default();
        request
            .to_capnp(
                request_message
                    .init_root::<asset_capnp::source_file_restore_request::Builder<'_>>(),
            )
            .unwrap();
        let request_reader = request_message
            .get_root_as_reader::<asset_capnp::source_file_restore_request::Reader<'_>>()
            .unwrap();
        assert_eq!(
            SourceFileRestoreRequest::from_capnp(request_reader).unwrap(),
            request
        );

        let result = SourceFileRestoreResult {
            snapshot: SourceFileEditSnapshot {
                source: workspace_source_file_ref(),
                source_fingerprint: vec![0x72; 32],
                document: source_file_edit_document(),
            },
        };
        let mut result_message = message::Builder::new_default();
        result
            .to_capnp(
                result_message.init_root::<asset_capnp::source_file_restore_result::Builder<'_>>(),
            )
            .unwrap();
        let result_reader = result_message
            .get_root_as_reader::<asset_capnp::source_file_restore_result::Reader<'_>>()
            .unwrap();
        assert_eq!(
            SourceFileRestoreResult::from_capnp(result_reader).unwrap(),
            result
        );
    }

    #[test]
    fn source_file_restore_rejects_editor_role() {
        let mut capability = write_capability();
        capability.role = core::ServiceRole::Editor;
        let request = SourceFileRestoreRequest {
            capability,
            session_id: "editor-session".to_string(),
            source: workspace_source_file_ref(),
            expected_source_fingerprint: vec![0x71; 32],
            document: source_file_edit_document(),
        };
        let mut message = message::Builder::new_default();

        let error = request
            .to_capnp(message.init_root::<asset_capnp::source_file_restore_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("ProjectHost"));
    }

    #[test]
    fn source_file_edit_result_round_trips_canonical_reloaded_document() {
        let expected = SourceFileEditResult {
            snapshot: SourceFileEditSnapshot {
                source: workspace_source_file_ref(),
                source_fingerprint: vec![0xa5; 32],
                document: source_file_edit_document(),
            },
        };
        let mut message = message::Builder::new_default();
        expected
            .to_capnp(message.init_root::<asset_capnp::source_file_edit_result::Builder<'_>>())
            .unwrap();
        let reader = message
            .get_root_as_reader::<asset_capnp::source_file_edit_result::Reader<'_>>()
            .unwrap();
        assert_eq!(SourceFileEditResult::from_capnp(reader).unwrap(), expected);
    }

    #[test]
    fn source_file_edit_request_rejects_non_fingerprint_concurrency_token() {
        let request = SourceFileEditRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source: workspace_source_file_ref(),
            expected_source_fingerprint: vec![0x44; 31],
            operation: SourceFileEditOperation::AppendDefault,
        };
        let mut message = message::Builder::new_default();
        let error = request
            .to_capnp(message.init_root::<asset_capnp::source_file_edit_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("must be 32 bytes"), "{error}");
    }

    #[test]
    fn source_file_edit_request_rejects_path_as_source_root_key() {
        let mut invalid = SourceFileEditRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source: workspace_source_file_ref(),
            expected_source_fingerprint: vec![0x44; 32],
            operation: SourceFileEditOperation::AppendDefault,
        };
        invalid.source.source_root_key = "project/assets".to_string();
        let mut message = message::Builder::new_default();

        assert!(
            invalid
                .to_capnp(message.init_root::<asset_capnp::source_file_edit_request::Builder<'_>>())
                .is_err()
        );
    }

    #[test]
    fn source_file_create_request_encode_rejects_unbound_payload_side_channel() {
        let temp = tempfile::tempdir().unwrap();
        let payload = SideChannelHandle::staging_file(
            temp.path().join("table.ron").to_string_lossy(),
            3,
            blake3::hash(b"[]\n").as_bytes().to_vec(),
            std::env::consts::OS,
        );
        let request = SourceFileCreateRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            source_path: "achievementdata/achievementdatatable.ron".to_string(),
            schema_type: "azoth.gamedata.TableSource".to_string(),
            changed_unix_ms: 1_000,
            content: SourceFileCreateContent::Payload(Box::new(payload)),
        };
        let mut message = message::Builder::new_default();
        let error = (request)
            .to_capnp(message.init_root::<asset_capnp::source_file_create_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("brokered capability"), "{error}");
    }

    #[test]
    fn source_file_create_result_round_trips_record() {
        let expected = SourceFileCreateResult {
            record: SourceAssetRecordResult {
                asset_guid: Uuid::from_bytes([0x34; 16]),
                entry: WorkspaceEntry {
                    entry_id: 77,
                    workspace_id: 11,
                    asset_guid: Uuid::from_bytes([0x34; 16]),
                    root_id: 3,
                    source_path: "achievementdata/achievementdatatable.ron".to_string(),
                    schema_type: Some("azoth.gamedata.TableSource".to_string()),
                    content_hash: "ab".repeat(32),
                    diff: WorkspaceEntryDiff::Added,
                    diagnostics_count: 0,
                    updated_unix_ms: 1_000,
                    jobs: Vec::new(),
                },
            },
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::source_file_create_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::source_file_create_result::Reader<'_>>()
            .unwrap();
        let actual = SourceFileCreateResult::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn source_file_delete_request_round_trips() {
        let expected = SourceFileDeleteRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            source_path: "materials/delete.material.ron".to_string(),
            changed_unix_ms: 2_000,
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::source_file_delete_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::source_file_delete_request::Reader<'_>>()
            .unwrap();
        let actual = SourceFileDeleteRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn source_file_move_request_rejects_unsafe_target_path() {
        let request = SourceFileMoveRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            from_source_path: "materials/old.material.ron".to_string(),
            to_source_path: "../outside.material.ron".to_string(),
            changed_unix_ms: 2_000,
        };
        let mut message = message::Builder::new_default();
        let error = (request)
            .to_capnp(message.init_root::<asset_capnp::source_file_move_request::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("asset-db relative path"));
    }

    #[test]
    fn source_file_move_result_round_trips_record() {
        let expected = SourceFileMoveResult {
            old_source_path: "materials/old.material.ron".to_string(),
            record: SourceAssetRecordResult {
                asset_guid: Uuid::from_bytes([0x44; 16]),
                entry: WorkspaceEntry {
                    entry_id: 88,
                    workspace_id: 12,
                    asset_guid: Uuid::from_bytes([0x44; 16]),
                    root_id: 4,
                    source_path: "materials/new.material.ron".to_string(),
                    schema_type: Some("az.test.Material".to_string()),
                    content_hash: "cd".repeat(32),
                    diff: WorkspaceEntryDiff::Modified,
                    diagnostics_count: 0,
                    updated_unix_ms: 2_000,
                    jobs: Vec::new(),
                },
            },
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::source_file_move_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::source_file_move_result::Reader<'_>>()
            .unwrap();
        let actual = SourceFileMoveResult::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn force_reprocess_asset_request_round_trips() {
        let expected = ForceReprocessAssetRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            source_path: "materials/rebuild.material.ron".to_string(),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<asset_capnp::force_reprocess_asset_request::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::force_reprocess_asset_request::Reader<'_>>()
            .unwrap();
        let actual = ForceReprocessAssetRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn force_reprocess_asset_request_rejects_read_capability() {
        let request = ForceReprocessAssetRequest {
            capability: write_capability().with_permissions([ASSET_READ_PERMISSION]),
            session_id: "editor-session".to_string(),
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            source_path: "materials/rebuild.material.ron".to_string(),
        };
        let mut message = message::Builder::new_default();
        let error = (request)
            .to_capnp(
                message.init_root::<asset_capnp::force_reprocess_asset_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains(ASSET_WRITE_PERMISSION),
            "{error}"
        );
    }

    #[test]
    fn force_reprocess_asset_request_accepts_editor_write_capability() {
        let expected = ForceReprocessAssetRequest {
            capability: read_capability().with_permissions([ASSET_WRITE_PERMISSION]),
            session_id: "editor-session".to_string(),
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            source_path: "materials/rebuild.material.ron".to_string(),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<asset_capnp::force_reprocess_asset_request::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::force_reprocess_asset_request::Reader<'_>>()
            .unwrap();
        let actual = ForceReprocessAssetRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn force_reprocess_asset_result_round_trips_record_and_job_count() {
        let expected = ForceReprocessAssetResult {
            record: SourceAssetRecordResult {
                asset_guid: Uuid::from_bytes([0x45; 16]),
                entry: WorkspaceEntry {
                    entry_id: 89,
                    workspace_id: 13,
                    asset_guid: Uuid::from_bytes([0x45; 16]),
                    root_id: 5,
                    source_path: "materials/rebuild.material.ron".to_string(),
                    schema_type: Some("az.test.Material".to_string()),
                    content_hash: "ef".repeat(32),
                    diff: WorkspaceEntryDiff::Clean,
                    diagnostics_count: 0,
                    updated_unix_ms: 3_000,
                    jobs: Vec::new(),
                },
            },
            enqueued_jobs: 3,
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::force_reprocess_asset_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::force_reprocess_asset_result::Reader<'_>>()
            .unwrap();
        let actual = ForceReprocessAssetResult::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn reconcile_asset_sources_request_round_trips() {
        let expected = ReconcileAssetSourcesRequest {
            capability: read_capability().with_permissions([ASSET_WRITE_PERMISSION]),
            session_id: "editor-session".to_string(),
            root_scope: AssetRootScope::All,
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<asset_capnp::reconcile_asset_sources_request::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::reconcile_asset_sources_request::Reader<'_>>()
            .unwrap();
        let actual = ReconcileAssetSourcesRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn reconcile_asset_sources_result_round_trips() {
        let expected = ReconcileAssetSourcesResult {
            source_root_count: 3,
            recorded_source_asset_count: 21,
            deleted_source_asset_count: 2,
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<asset_capnp::reconcile_asset_sources_result::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::reconcile_asset_sources_result::Reader<'_>>()
            .unwrap();
        let actual = ReconcileAssetSourcesResult::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn source_dependents_result_round_trips_sources_and_jobs() {
        let expected = SourceDependentsResult {
            source_path: "materials/base.material.ron".to_string(),
            source_dependents: vec![SourceDependentSource {
                source_edge_id: 1,
                source_path: "prefabs/door.prefab.ron".to_string(),
                builder_guid: Uuid::from_bytes([0x55; 16]),
                relation: SourceRelation::SourceToSource,
            }],
            job_dependents: vec![SourceDependentJob {
                job_edge_id: 2,
                job_id: 3,
                latest_attempt_id: Some(4),
                source_path: "prefabs/door.prefab.ron".to_string(),
                owner: JobOwner::Build(Uuid::from_bytes([0x56; 16])),
                job_key: "compile-prefab".to_string(),
                platform: "pc".to_string(),
                dependency_job_key: "compile-material".to_string(),
                dependency_platform: "pc".to_string(),
                kind: JobDependencyKind::Order,
                product_paths: vec!["prefabs/door.runtime".to_string()],
            }],
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::source_dependents_result::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::source_dependents_result::Reader<'_>>()
            .unwrap();
        let actual = SourceDependentsResult::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn source_asset_record_request_encode_rejects_read_capability() {
        let read_only_project_host = Capability::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
        )
        .with_session(test_session_id())
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_READ_PERMISSION]);
        let request = SourceAssetRecordRequest {
            capability: read_only_project_host,
            session_id: "editor-session".to_string(),
            workspace_root_id: 42,
            owner_id: "local.proto_asset".to_string(),
            source_path: "prefabs/door.prefab.ron".to_string(),
            schema_type: Some("az.test.Prefab".to_string()),
            content_hash: vec![0xab; 32],
            changed_unix_ms: 1_000,
            diagnostics_count: 2,
        };
        let mut message = message::Builder::new_default();
        let error = (request)
            .to_capnp(message.init_root::<asset_capnp::source_asset_record_request::Builder<'_>>())
            .unwrap_err();

        assert!(
            error.to_string().contains(ASSET_WRITE_PERMISSION),
            "{error}"
        );
    }

    #[test]
    fn lease_request_round_trips_capability_and_staging_root() {
        let expected = LeaseAssetJobRequest {
            capability: jobs_capability(),
            lease_owner: "worker-b".to_string(),
            lease_duration_ms: 400,
            staging_root: Some("staging/worker-b".to_string()),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(message.init_root::<asset_capnp::lease_asset_job_request::Builder<'_>>())
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::lease_asset_job_request::Reader<'_>>()
            .unwrap();
        let actual = LeaseAssetJobRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn lease_request_rejects_duration_beyond_recovery_bound() {
        let request = LeaseAssetJobRequest {
            capability: jobs_capability(),
            lease_owner: "worker-b".to_string(),
            lease_duration_ms: MAX_ASSET_JOB_LEASE_DURATION_MS + 1,
            staging_root: Some("staging/worker-b".to_string()),
        };
        let mut message = message::Builder::new_default();

        let error = request
            .to_capnp(message.init_root::<asset_capnp::lease_asset_job_request::Builder<'_>>())
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains(&MAX_ASSET_JOB_LEASE_DURATION_MS.to_string()),
            "{error}"
        );
    }

    #[test]
    fn lease_request_encode_rejects_non_worker_jobs_service() {
        let wrong_service = Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Worker)
            .with_session(test_session_id())
            .with_audience(ASSET_PROCESSOR_AUDIENCE)
            .with_permissions([ASSET_JOBS_PERMISSION]);
        let request = LeaseAssetJobRequest {
            capability: wrong_service,
            lease_owner: "worker-b".to_string(),
            lease_duration_ms: 400,
            staging_root: Some("staging/worker-b".to_string()),
        };
        let mut message = message::Builder::new_default();
        let error = (request)
            .to_capnp(message.init_root::<asset_capnp::lease_asset_job_request::Builder<'_>>())
            .unwrap_err();

        assert!(
            error.to_string().contains(ASSET_WORKER_SERVICE_NAME),
            "{error}"
        );
    }

    #[test]
    fn complete_request_round_trips_terminal_status() {
        let handle = SideChannelHandle::staging_file(
            std::env::current_dir()
                .unwrap()
                .join("staging/manifest.capnp.packed")
                .to_string_lossy(),
            128,
            vec![0x11; 32],
            std::env::consts::OS,
        );
        let expected = CompleteAssetJobAttemptRequest {
            capability: jobs_capability(),
            asset_job_attempt_id: 42,
            lease_owner: "worker-a".to_string(),
            grant_key: Uuid::from_bytes([0x26; 16]),
            status: AttemptStatus::Succeeded,
            finished_unix_ms: 900,
            error_count: 0,
            warning_count: 3,
            product_manifest: Some(handle),
        };
        let mut message = message::Builder::new_default();
        (expected)
            .to_capnp(
                message.init_root::<asset_capnp::complete_asset_job_attempt_request::Builder<'_>>(),
            )
            .unwrap();

        let reader = message
            .get_root_as_reader::<asset_capnp::complete_asset_job_attempt_request::Reader<'_>>()
            .unwrap();
        let actual = CompleteAssetJobAttemptRequest::from_capnp(reader).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn asset_builder_catalog_decode_rejects_duplicate_identity() {
        let mut catalog = valid_asset_builder_catalog_result();
        catalog.builders.push(catalog.builders[0].clone());

        let error = round_trip_asset_builder_catalog_result(&catalog).unwrap_err();

        assert!(
            error.to_string().contains("duplicate builder guid"),
            "{error}"
        );
    }

    #[test]
    fn asset_builder_catalog_decode_rejects_duplicate_patterns() {
        let mut catalog = valid_asset_builder_catalog_result();
        let duplicate = catalog.builders[0].patterns[0].clone();
        catalog.builders[0].patterns.push(duplicate);

        let error = round_trip_asset_builder_catalog_result(&catalog).unwrap_err();

        assert!(
            error.to_string().contains("duplicate Wildcard pattern"),
            "{error}"
        );
    }

    #[test]
    fn asset_builder_catalog_decode_rejects_duplicate_source_schema_types() {
        let mut catalog = valid_asset_builder_catalog_result();
        catalog.builders[0]
            .source_schema_types
            .push("az.test.Prefab".to_string());

        let error = round_trip_asset_builder_catalog_result(&catalog).unwrap_err();

        assert!(
            error.to_string().contains("duplicate source schema type"),
            "{error}"
        );
    }

    #[test]
    fn asset_builder_catalog_decode_rejects_duplicate_source_schema_descriptors() {
        let mut catalog = valid_asset_builder_catalog_result();
        catalog
            .source_schemas
            .push(catalog.source_schemas[0].clone());

        let error = round_trip_asset_builder_catalog_result(&catalog).unwrap_err();

        assert!(
            error.to_string().contains("duplicate source schema type"),
            "{error}"
        );
    }

    #[test]
    fn asset_builder_catalog_round_trips_product_formats() {
        let catalog = valid_asset_builder_catalog_result();

        let decoded = round_trip_asset_builder_catalog_result(&catalog).unwrap();

        assert_eq!(decoded.product_formats, catalog.product_formats);
    }

    #[test]
    fn asset_builder_catalog_decode_rejects_duplicate_product_format_ids() {
        let mut catalog = valid_asset_builder_catalog_result();
        catalog
            .product_formats
            .push(catalog.product_formats[0].clone());

        let error = round_trip_asset_builder_catalog_result(&catalog).unwrap_err();

        assert!(
            error.to_string().contains("duplicate product format id"),
            "{error}"
        );
    }

    #[test]
    fn asset_builder_catalog_decode_rejects_zero_product_format_version() {
        let mut catalog = valid_asset_builder_catalog_result();
        catalog.product_formats[0].current_version = 0;

        let error = round_trip_asset_builder_catalog_result(&catalog).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("current version must be positive"),
            "{error}"
        );
    }

    #[test]
    fn asset_builder_catalog_round_trips_source_file_workflow() {
        let mut catalog = valid_asset_builder_catalog_result();
        let mut workflow = valid_source_file_workflow();
        workflow.can_create = true;
        workflow.can_edit = true;
        catalog.source_schemas.push(SourceSchemaDescriptor {
            schema_type: "az.test.LegacyMaterialSource".to_string(),
            owner: "legacy-materials".to_string(),
            label: "Legacy Material".to_string(),
            category: "Compatibility".to_string(),
            authoring: SourceSchemaAuthoring::File { workflow },
            file_templates: vec![SourceFileTemplateDescriptor {
                owner: "legacy-materials".to_string(),
                source_path: "materials/default.mtl".to_string(),
                label: "Default Material".to_string(),
                description: "Empty material source".to_string(),
            }],
        });

        let decoded = round_trip_asset_builder_catalog_result(&catalog).unwrap();

        assert_eq!(
            decoded.source_schemas.last().unwrap(),
            catalog.source_schemas.last().unwrap()
        );
    }

    #[test]
    fn asset_builder_catalog_decode_rejects_templates_for_non_creatable_workflows() {
        let mut catalog = valid_asset_builder_catalog_result();
        catalog.source_schemas.push(SourceSchemaDescriptor {
            schema_type: "az.test.ImportOnlyMaterialSource".to_string(),
            owner: "legacy-materials".to_string(),
            label: "Legacy Material".to_string(),
            category: "Compatibility".to_string(),
            authoring: SourceSchemaAuthoring::File {
                workflow: valid_source_file_workflow(),
            },
            file_templates: vec![SourceFileTemplateDescriptor {
                owner: "legacy-materials".to_string(),
                source_path: "materials/default.mtl".to_string(),
                label: "Default Material".to_string(),
                description: "Empty material source".to_string(),
            }],
        });

        let error = round_trip_asset_builder_catalog_result(&catalog).unwrap_err();

        assert!(
            error.to_string().contains("not default-creatable"),
            "{error}"
        );
    }

    #[test]
    fn asset_builder_catalog_decode_rejects_templates_for_project_document_workflows() {
        let mut catalog = valid_asset_builder_catalog_result();
        catalog.source_schemas[0]
            .file_templates
            .push(SourceFileTemplateDescriptor {
                owner: "prefab-documents".to_string(),
                source_path: "prefabs/default.prefab.ron".to_string(),
                label: "Default Prefab".to_string(),
                description: "Default prefab document".to_string(),
            });

        let error = round_trip_asset_builder_catalog_result(&catalog).unwrap_err();

        assert!(
            error.to_string().contains("project-document backed"),
            "{error}"
        );
    }

    #[test]
    fn asset_builder_catalog_decode_rejects_template_paths_outside_workflow_extensions() {
        let mut catalog = valid_asset_builder_catalog_result();
        let mut workflow = valid_source_file_workflow();
        workflow.can_create = true;
        catalog.source_schemas.push(SourceSchemaDescriptor {
            schema_type: "az.test.LegacyMaterialSource".to_string(),
            owner: "legacy-materials".to_string(),
            label: "Legacy Material".to_string(),
            category: "Compatibility".to_string(),
            authoring: SourceSchemaAuthoring::File { workflow },
            file_templates: vec![SourceFileTemplateDescriptor {
                owner: "legacy-materials".to_string(),
                source_path: "materials/default.txt".to_string(),
                label: "Default Material".to_string(),
                description: "Empty material source".to_string(),
            }],
        });

        let error = round_trip_asset_builder_catalog_result(&catalog).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("does not match workflow extensions"),
            "{error}"
        );
    }

    #[test]
    fn workspace_entry_page_decode_rejects_nonadvancing_entries() {
        let entry = valid_workspace_entry();
        let result = WorkspaceEntryPageResult {
            entries: vec![entry.clone(), entry],
            next_after_entry_id: Some(55),
        };

        let error = round_trip_workspace_entry_page_result(&result).unwrap_err();

        assert!(error.to_string().contains("does not advance"), "{error}");
    }

    #[test]
    fn workspace_entry_page_decode_rejects_bad_cursor() {
        let result = WorkspaceEntryPageResult {
            entries: Vec::new(),
            next_after_entry_id: Some(55),
        };

        let error = round_trip_workspace_entry_page_result(&result).unwrap_err();

        assert!(error.to_string().contains("empty page"), "{error}");
    }

    #[test]
    fn workspace_entry_page_decode_rejects_mismatched_job_attempt() {
        let mut entry = valid_workspace_entry();
        entry.jobs[0].attempt.as_mut().unwrap().job_id += 1;
        let result = WorkspaceEntryPageResult {
            entries: vec![entry],
            next_after_entry_id: Some(55),
        };

        let error = round_trip_workspace_entry_page_result(&result).unwrap_err();

        assert!(error.to_string().contains("does not match job"), "{error}");
    }

    #[test]
    fn workspace_snapshot_decode_rejects_duplicate_roots() {
        let mut result = valid_workspace_snapshot_result();
        let snapshot = result.snapshot.as_mut().unwrap();
        snapshot.roots.push(snapshot.roots[0].clone());

        let error = round_trip_workspace_snapshot_result(&result).unwrap_err();

        assert!(
            error.to_string().contains("duplicate workspace root id"),
            "{error}"
        );
    }

    #[test]
    fn workspace_snapshot_decode_allows_asset_root_only_scope() {
        let mut browser_snapshot = valid_workspace_snapshot_result();
        let snapshot = browser_snapshot.snapshot.as_mut().unwrap();
        snapshot.roots[0].source_root =
            "project/.azoth/workspaces/editor-session/assets".to_string();
        snapshot.roots[0].display_name = "Project Assets".to_string();
        snapshot.roots[0].portable_key = "project:local.proto_asset_view:assets".to_string();
        snapshot.roots[0].output_prefix = "assets".to_string();
        snapshot.roots[0].is_root = false;

        let actual = round_trip_workspace_snapshot_result(&browser_snapshot).unwrap();

        assert_eq!(actual, browser_snapshot);
    }

    #[test]
    fn catalog_products_decode_rejects_duplicates_and_unsorted_entries() {
        let result = valid_catalog_products_result();
        let mut duplicate = result.clone();
        duplicate.entries.push(duplicate.entries[0].clone());
        let error = round_trip_catalog_products_result(&duplicate).unwrap_err();
        assert!(
            error.to_string().contains("duplicate product id"),
            "{error}"
        );

        let mut unsorted = result;
        let mut first = unsorted.entries[0].clone();
        first.product_id = 9;
        first.product_path = "z/textures/rpc.dds".to_string();
        let mut second = first.clone();
        second.product_id = 10;
        second.product_path = "a/textures/rpc.dds".to_string();
        unsorted.entries = vec![first, second];
        let error = round_trip_catalog_products_result(&unsorted).unwrap_err();
        assert!(
            error.to_string().contains("not in catalog order"),
            "{error}"
        );
    }

    #[test]
    fn catalog_products_decode_rejects_bad_paths_and_nil_types() {
        let mut bad_source = valid_catalog_products_result();
        bad_source.entries[0].source_path = "../textures/rpc.png".to_string();
        let error = round_trip_catalog_products_result(&bad_source).unwrap_err();
        assert!(
            error.to_string().contains("asset-db relative path"),
            "{error}"
        );

        let mut bad_type = valid_catalog_products_result();
        bad_type.entries[0].asset_type = Uuid::nil();
        let error = round_trip_catalog_products_result(&bad_type).unwrap_err();
        assert!(error.to_string().contains("cannot be nil"), "{error}");
    }

    #[test]
    fn source_asset_record_result_encode_rejects_nil_asset_guid() {
        let result = SourceAssetRecordResult {
            asset_guid: Uuid::nil(),
            entry: valid_workspace_entry(),
        };
        let mut message = message::Builder::new_default();
        let error = (result)
            .to_capnp(message.init_root::<asset_capnp::source_asset_record_result::Builder<'_>>())
            .unwrap_err();

        assert!(error.to_string().contains("cannot be nil"), "{error}");
    }

    #[test]
    fn source_asset_record_request_encode_rejects_bad_hash_length() {
        let request = SourceAssetRecordRequest {
            capability: write_capability(),
            session_id: "editor-session".to_string(),
            workspace_root_id: 42,
            owner_id: "local.proto_asset".to_string(),
            source_path: "prefabs/door.prefab.ron".to_string(),
            schema_type: Some("az.test.Prefab".to_string()),
            content_hash: vec![0xab; 31],
            changed_unix_ms: 1_000,
            diagnostics_count: 2,
        };
        let mut message = message::Builder::new_default();
        let error = (request)
            .to_capnp(message.init_root::<asset_capnp::source_asset_record_request::Builder<'_>>())
            .unwrap_err();

        assert!(
            error.to_string().contains("content hash must be 32 bytes"),
            "{error}"
        );
    }

    #[test]
    fn complete_request_encode_rejects_non_terminal_status() {
        let expected = CompleteAssetJobAttemptRequest {
            capability: jobs_capability(),
            asset_job_attempt_id: 42,
            lease_owner: "worker-a".to_string(),
            grant_key: Uuid::from_bytes([0x27; 16]),
            status: AttemptStatus::Leased,
            finished_unix_ms: 900,
            error_count: 0,
            warning_count: 3,
            product_manifest: None,
        };
        let mut message = message::Builder::new_default();
        let error = (expected)
            .to_capnp(
                message.init_root::<asset_capnp::complete_asset_job_attempt_request::Builder<'_>>(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("not terminal"), "{error}");
    }

    #[test]
    fn product_manifest_side_channel_validates_hash_and_decodes_products() {
        let manifest = ProductManifest::new(
            1_000,
            vec![ProductManifestProduct {
                product_path: "cache/textures/rpc.dds".to_string(),
                catalog_aliases: vec!["textures/rpc.texture".to_string()],
                catalog_path_registration: CatalogPathRegistration::Registered,
                asset_type: Uuid::from_bytes([0xcd; 16]),
                sub_id: 7,
                product_format: "az.test.raw".to_string(),
                product_format_version: 1,
                staged_path: "textures/rpc.dds".to_string(),
                content_hash: vec![0xef; 32],
                byte_length: 4096,
                dependencies: vec![ProductManifestProductDependency {
                    asset_guid: Uuid::from_bytes([0xab; 16]),
                    sub_id: 2,
                    flags: 1,
                }],
            }],
        );
        let bytes = encode_product_manifest(&manifest).unwrap();
        let hash = blake3::hash(&bytes).as_bytes().to_vec();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("manifest.capnp.packed");
        std::fs::write(&path, &bytes).unwrap();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            bytes.len() as u64,
            hash,
            std::env::consts::OS,
        );

        let decoded = load_product_manifest_side_channel(&handle).unwrap();
        assert_eq!(decoded, manifest);

        let mut corrupted = handle;
        corrupted.content_hash = vec![0; 32];
        assert!(matches!(
            load_product_manifest_side_channel(&corrupted),
            Err(ProductManifestSideChannelError::InvalidHandle(
                StagingFileSideChannelError::HashMismatch { .. }
            ))
        ));
    }

    #[test]
    fn product_manifest_round_trips_shared_path_logical_products() {
        let product = |sub_id, catalog_path_registration| ProductManifestProduct {
            product_path: "slices/shared.slice.meta".to_string(),
            catalog_aliases: Vec::new(),
            catalog_path_registration,
            asset_type: Uuid::from_bytes([0xcd; 16]),
            sub_id,
            product_format: "az.test.raw".to_string(),
            product_format_version: 1,
            staged_path: "slices/shared.slice.meta".to_string(),
            content_hash: vec![0xef; 32],
            byte_length: 4096,
            dependencies: Vec::new(),
        };
        let manifest = ProductManifest::new(
            1_000,
            vec![
                product(3, CatalogPathRegistration::AssetIdOnly),
                product(7, CatalogPathRegistration::Registered),
            ],
        );

        let bytes = encode_product_manifest(&manifest).unwrap();
        let message =
            serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())
                .unwrap();
        let reader = message
            .get_root::<asset_capnp::product_manifest::Reader<'_>>()
            .unwrap();

        assert_eq!(read_product_manifest(reader).unwrap(), manifest);
    }

    #[test]
    fn product_manifest_round_trips_reserved_planner_control_product() {
        let manifest = ProductManifest::new(
            1_000,
            vec![ProductManifestProduct {
                product_path: ASSET_JOB_PLAN_PRODUCT_PATH.to_string(),
                catalog_aliases: Vec::new(),
                catalog_path_registration: CatalogPathRegistration::Registered,
                asset_type: ASSET_JOB_PLAN_ASSET_TYPE,
                sub_id: 0,
                product_format: ASSET_JOB_PLAN_PRODUCT_FORMAT.to_string(),
                product_format_version: 1,
                staged_path: ASSET_JOB_PLAN_PRODUCT_PATH.to_string(),
                content_hash: vec![0xef; 32],
                byte_length: 4096,
                dependencies: Vec::new(),
            }],
        );

        let bytes = encode_product_manifest(&manifest).unwrap();
        let message =
            serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())
                .unwrap();
        let reader = message
            .get_root::<asset_capnp::product_manifest::Reader<'_>>()
            .unwrap();

        assert_eq!(read_product_manifest(reader).unwrap(), manifest);
    }

    #[test]
    fn product_manifest_encoding_rejects_unsafe_product_path() {
        let manifest = ProductManifest::new(
            1_000,
            vec![ProductManifestProduct {
                product_path: "../cache/textures/rpc.dds".to_string(),
                catalog_aliases: Vec::new(),
                catalog_path_registration: CatalogPathRegistration::Registered,
                asset_type: Uuid::from_bytes([0xcd; 16]),
                sub_id: 7,
                product_format: "az.test.raw".to_string(),
                product_format_version: 1,
                staged_path: "textures/rpc.dds".to_string(),
                content_hash: vec![0xef; 32],
                byte_length: 4096,
                dependencies: Vec::new(),
            }],
        );

        let error = encode_product_manifest(&manifest).unwrap_err();

        assert!(error.to_string().contains("must be relative"), "{error}");
    }

    #[test]
    fn product_manifest_encoding_rejects_bad_hash_length() {
        let manifest = ProductManifest::new(
            1_000,
            vec![ProductManifestProduct {
                product_path: "cache/textures/rpc.dds".to_string(),
                catalog_aliases: Vec::new(),
                catalog_path_registration: CatalogPathRegistration::Registered,
                asset_type: Uuid::from_bytes([0xcd; 16]),
                sub_id: 7,
                product_format: "az.test.raw".to_string(),
                product_format_version: 1,
                staged_path: "textures/rpc.dds".to_string(),
                content_hash: vec![0xef; 31],
                byte_length: 4096,
                dependencies: Vec::new(),
            }],
        );

        let error = encode_product_manifest(&manifest).unwrap_err();

        assert!(
            error.to_string().contains("content hash must be 32 bytes"),
            "{error}"
        );
    }

    #[test]
    fn product_manifest_encoding_rejects_invalid_dependency_identity() {
        let manifest = ProductManifest::new(
            1_000,
            vec![ProductManifestProduct {
                product_path: "cache/textures/rpc.dds".to_string(),
                catalog_aliases: Vec::new(),
                catalog_path_registration: CatalogPathRegistration::Registered,
                asset_type: Uuid::from_bytes([0xcd; 16]),
                sub_id: 7,
                product_format: "az.test.raw".to_string(),
                product_format_version: 1,
                staged_path: "textures/rpc.dds".to_string(),
                content_hash: vec![0xef; 32],
                byte_length: 4096,
                dependencies: vec![ProductManifestProductDependency {
                    asset_guid: Uuid::nil(),
                    sub_id: 2,
                    flags: 0,
                }],
            }],
        );

        let error = encode_product_manifest(&manifest).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("dependency asset guid cannot be nil"),
            "{error}"
        );
    }

    #[test]
    fn product_manifest_raw_writer_stays_private_to_checked_encoder() {
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
            .expect("read az-proto-asset src/lib.rs");
        let public_raw_writers = source
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub fn write_product_manifest"))
            .collect::<Vec<_>>();

        assert!(
            public_raw_writers.is_empty(),
            "raw product-manifest writer must stay private to encode_product_manifest so transport validation runs: {public_raw_writers:?}"
        );
    }
}
