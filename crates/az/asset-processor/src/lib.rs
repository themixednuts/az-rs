//! Asset-processor service adapter.
//!
//! This crate is the Cap'n Proto boundary around the durable asset queue in
//! `az-assetdb`. Workers and session services should speak the protocol
//! surface here; direct DB access stays inside the asset-processor authority.

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::fs;
#[cfg(test)]
use std::future::Future;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(test)]
use az_asset::{ASSET_CATALOG_FILE_NAME, AssetId};
use az_asset_builder::{
    AssetBuilderPattern, BuildRule, BuildRuleRegistry, CreateJobsRequest, CreateJobsResponse,
    CreateJobsResult, DEFAULT_PLATFORM_ID, JobContext as BuilderJobContext, JobDependencyType,
    PROJECT_SOURCE_ROOT, SourceFileDependency, SourceFileTemplateError,
    SourceFileTemplateRegistration, SourceFileTemplateRequest,
    SourceSchemaAuthoring as BuilderSourceSchemaAuthoring, SourceSchemaRegistration,
    SourceSchemaType, composed_product_format_by_id, composed_product_formats,
    composed_source_file_template, composed_source_file_templates, composed_source_schemas,
};
#[cfg(test)]
use az_asset_builder::{DEFAULT_PLATFORM, ProductFormatPolicy, SourceMatcher};
use az_assetdb::{
    AbandonAttempts, Aliases, ApplyPlanDelta, AssetDb, AssetDbWriter, AssetProcessorQueue,
    AttemptFence, BuilderCatalogReplaceOutcome, BuilderDescriptor, CheckpointWrite, ClaimReadyJob,
    ClaimReadyJobResult, ClaimedJobContext, CompleteAttempt, CompleteAttemptResult,
    Coupling as DbCoupling, DeleteSource, DeleteSourceResult, Diff as DbDiff, Digest, Encoding,
    Exclusions, JobActivitySnapshot, JobEdgeInput, JobInspection as DbJobInspection,
    JobInspectionSelector as DbJobInspectionSelector, MoveSource, MoveSourceResult, OpenError,
    PlanDelta, PlannedJob, ProcessingStatus, ProductEdgeInput, ProductInput, PublishAuthoredSource,
    PublishAuthoredSourceResult, RegisterWorkspace, Registration, Relation as DbRelation,
    ReplaceBuilderCatalog, ReplaceWorkspaceRoots, RepoError, SelectAssets, SelectAttempts,
    SelectCatalog, SelectEntries, SelectJobs, SelectWorkspaceRoots, SelectWorkspaces,
    SourceDependentsInput as DbSourceDependentsInput, SourceEdgeInput, SourceStateToken,
    Status as DbStatus, SweepEntry, Target as DbTarget, Work as DbWork, WorkspaceEntrySnapshot,
    WorkspaceKey, WorkspaceRootRegistration, WriteSourcePayload,
};
#[cfg(test)]
use az_assetdb::{
    ApplySweepDelta, CatalogProductEdge, CatalogTarget, RegisterWorkspaceRoot, SweepPlannerJob,
    SweepRecord, WriteSourcePayloadResult,
};
#[cfg(any(test, feature = "test-support"))]
use az_filesystem::AzothDataHome;
use az_filesystem::{
    FileTransaction, FileWrite, ProjectDataPaths, SourcePath, canonical, normalize,
    normalize_lexical,
};
use az_gem_contract::{Attributed, Composer, GemTargetRole, ProductActivation, Registries};
use az_graph_builder::{
    GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID, RegisteredGraphSourceAuthoring,
    RegisteredGraphSourceSchema, graph_source_schemas,
};
use az_project::{
    ASSET_NAMESPACE_MOUNT, AssetMount, NativeAssetPath, PortableKey, ProjectAssetOverride,
    ProjectManifestError, ResolvedProjectGraph, load_resolved_project_graph,
};
#[cfg(test)]
use az_proto_asset::JobActivity;
use az_proto_asset::{
    ASSET_JOB_PLAN_ASSET_TYPE, ASSET_JOB_PLAN_PRODUCT_FORMAT, ASSET_JOB_PLAN_PRODUCT_PATH,
    ASSET_PLANNER_JOB_KEY, AssetBuilderCatalogRequest, AssetBuilderCatalogResult,
    AssetBuilderDescriptor, AssetBuilderPatternDescriptor, AssetBuilderPatternKind,
    AssetProcessingStatusRequest, AssetProcessingStatusResult, AssetProcessorEvent,
    AssetProcessorEventKind, AssetProcessorEventSubscriptionRequest,
    AssetProcessorEventSubscriptionResult, AssetRootScope, AttemptStatus, CatalogPathRegistration,
    CatalogProductsRequest, CatalogProductsResult, CompleteAssetJobAttemptRequest,
    ForceReprocessAssetRequest, ForceReprocessAssetResult, InspectJobRequest, InspectJobResult,
    InspectJobSelector, JobAttemptRecord, JobDependencyKind, JobDependencyRecord,
    JobDependencyTarget, JobInspection as ProtoJobInspection, JobOwner, JobProductEdgeRecord,
    JobProductRecord, JobRecord, JobStatus, LeaseAssetJobRequest, LeaseAssetJobResult,
    LeasedAssetJob, MAX_ASSET_JOB_LEASE_DURATION_MS, ProductFormatDescriptor, ProductManifest,
    ProductManifestProduct, ProductManifestSideChannelError, PublishAssetCatalogRequest,
    PublishAssetCatalogResult, PublishBuilderCatalogRequest, PublishBuilderCatalogResult,
    RELEASE_ASSET_CATALOG_FILE_NAME, ReconcileAssetSourcesRequest, ReconcileAssetSourcesResult,
    ReleaseContentProduct, ReleaseContentReadRequest, ReleaseContentReadResult,
    ReleaseContentTarget, RenewAssetJobLeaseRequest, SourceAssetRecordRequest,
    SourceAssetRecordResult, SourceDependentJob, SourceDependentSource, SourceDependentsRequest,
    SourceDependentsResult, SourceFileCodecOperation, SourceFileCodecOutputDestination,
    SourceFileCodecReplacement, SourceFileCodecRequest, SourceFileCodecResult,
    SourceFileCreateContent, SourceFileCreateRequest, SourceFileCreateResult,
    SourceFileDeleteRequest, SourceFileDeleteResult, SourceFileEditRequest, SourceFileEditResult,
    SourceFileEditSnapshot, SourceFileMoveRequest, SourceFileMoveResult, SourceFileOpenRequest,
    SourceFileOpenResult, SourceFileRestoreRequest, SourceFileRestoreResult,
    SourceFileTemplateDescriptor, SourceFileWorkflowDescriptor, SourceRelation,
    SourceSchemaAuthoring, SourceSchemaDescriptor, WorkspaceEntry, WorkspaceEntryDiff,
    WorkspaceEntryPageRequest, WorkspaceEntryPageResult, WorkspaceRoot, WorkspaceSnapshot,
    WorkspaceSnapshotRequest, WorkspaceSnapshotResult, WorkspaceSourceFileRef, asset_capnp,
    load_product_manifest_side_channel,
};
use az_proto_core::ProtocolVersion;
use az_proto_core::{
    Capability, CapabilityGrantSet, ServiceHealth, ServiceHealthState, ServiceId, ServiceRole,
    SideChannelCapabilityError, SideChannelHandle, StagingFileSideChannelError,
    read_verified_staging_file, validate_side_channel_capability_matches,
    validated_staging_file_path, write_named_staging_file_atomic,
};
use az_work::CancellationToken;
use capnp::Error;
use futures::StreamExt;
use thiserror::Error;
use tracing::{info, instrument, trace, warn};
use uuid::Uuid;

mod catalog;
mod dispatcher;
pub mod source_meta;
mod sweep;
mod transport;
mod watcher;

use catalog::{CatalogPublisher, CatalogPublisherOwner, CatalogScope};
use dispatcher::{
    AssetJobDispatcher, AssetJobDispatcherOwner, GrantIdentity, LeaseEnvelope, LeaseRequest,
    PayloadAuthority,
};
use sweep::{SweepHandle, SweepProvenance, SweepRequest, SweepRoot, SweepScope};

pub use source_meta::{
    SOURCE_META_SIDECAR_SUFFIX, SOURCE_META_SPEC, SourceAssetMeta, SourceMetaError,
    read_source_asset_meta, resolve_source_asset_guid, source_meta_sidecar_path,
};
pub use transport::*;

pub use az_proto_asset::{
    ASSET_JOBS_PERMISSION, ASSET_PROCESSOR_AUDIENCE, ASSET_PROCESSOR_NAMESPACE,
    ASSET_PROCESSOR_SERVICE_NAME, ASSET_READ_PERMISSION, ASSET_WORKER_SERVICE_NAME,
    ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WRITE_PERMISSION,
};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

/// The source roots one workspace registered, in precedence order.
///
/// `source_roots` keeps the order `az_project` resolved: home tier first, and
/// lock-closure order inside a tier (ADR 0034). A root's precedence is its
/// position here, so project content precedes gem content and gem content
/// precedes engine content without any root carrying a number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceAssetSourceRootsRegistration {
    pub workspace_id: i64,
    pub project_id: String,
    pub workspace_root: PathBuf,
    pub branch: String,
    pub source_roots: Vec<RegisteredAssetSourceRoot>,
}

/// An open asset database whose source roots are registered for one workspace.
///
/// This value carries the database-owner lifetime established while registering
/// the session. Production startup should consume it when starting the RPC
/// service so registration, launch validation, the listener, and the source
/// watcher all share one process-local Turso database owner.
#[derive(Debug)]
pub struct RegisteredWorkspaceAssetDb {
    db_path: PathBuf,
    database: AssetDb,
    /// One composition-owned writer for startup reconciliation, RPCs, and
    /// filesystem reconciliation throughout this processor lifetime.
    asset_db_writer: AssetDbWriter,
    source_roots: Vec<RegisteredSourceRoot>,
    registration: WorkspaceAssetSourceRootsRegistration,
}

impl RegisteredWorkspaceAssetDb {
    #[must_use]
    pub const fn registration(&self) -> &WorkspaceAssetSourceRootsRegistration {
        &self.registration
    }

    fn into_parts(
        self,
    ) -> (
        PathBuf,
        AssetDb,
        AssetDbWriter,
        Vec<RegisteredSourceRoot>,
        WorkspaceAssetSourceRootsRegistration,
    ) {
        (
            self.db_path,
            self.database,
            self.asset_db_writer,
            self.source_roots,
            self.registration,
        )
    }

    fn into_registration(self) -> WorkspaceAssetSourceRootsRegistration {
        self.registration
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredAssetSourceRoot {
    pub workspace_root_id: i64,
    pub scan_folder_id: i64,
    pub id: String,
    pub owner_id: String,
    pub root: PathBuf,
    pub display_name: String,
    pub portable_key: String,
    pub mount: String,
    pub recursive: bool,
    pub watch: bool,
    pub writable: bool,
    pub output_prefix: String,
}

/// What a registered source root is to its project.
///
/// The project's own assets root has to exist -- it is where the project's
/// sources live. Every other root is declared by a gem or a manifest override
/// and may legitimately have no directory yet, so a missing one is not an
/// error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceRootRole {
    ProjectAssets,
    Auxiliary,
}

impl SourceRootRole {
    /// Whether the root's directory must exist for registration to succeed.
    const fn is_required(self) -> bool {
        matches!(self, Self::ProjectAssets)
    }

    /// The role a root with `is_project_assets` set claims.
    const fn from_is_project_assets(is_project_assets: bool) -> Self {
        if is_project_assets {
            Self::ProjectAssets
        } else {
            Self::Auxiliary
        }
    }
}

/// One resolved source root on its way into the asset database.
///
/// The portable key and mount stay in their validated newtypes for as long as
/// they are in memory, so a malformed one cannot reach the database. Nothing
/// here ranks the root: the spec vector is already in precedence order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AssetSourceRootSpec {
    id: String,
    owner_id: String,
    root: PathBuf,
    display_name: String,
    portable_key: PortableKey,
    mount: AssetMount,
    recursive: bool,
    watch: bool,
    writable: bool,
    excluded_paths: Exclusions,
    output_prefix: String,
    role: SourceRootRole,
}

/// Processor-owned runtime root policy. `AssetDB` owns the normalized
/// Workspace/Root rows; watcher and transport retain only the manifest facts
/// they actually execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisteredSourceRoot {
    workspace_pk: i64,
    workspace_root_pk: i64,
    root_pk: i64,
    id: String,
    owner: String,
    path: String,
    display_name: String,
    portable_key: String,
    mount: String,
    recursive: bool,
    watch: bool,
    writable: bool,
    exclusions: Exclusions,
    output_prefix: String,
    role: SourceRootRole,
}

#[derive(Debug, Clone)]
struct SourceAssetClassifiers {
    project_documents: Vec<ProjectDocumentSourceClassifier>,
    file_sources: Vec<FileSourceClassifier>,
    /// Built on every publish so the claim set stays in step with the catalog,
    /// but the sweep decides planning from [`BuilderCatalogDiff`] rather than by
    /// asking a source whether some builder claims it; the only reader left is
    /// the `cfg(test)` accessor below.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read only by the cfg(test) source_has_builder_claim accessor"
        )
    )]
    builder_claims: Vec<SourceBuilderClassifier>,
}

impl SourceAssetClassifiers {
    #[cfg(test)]
    fn source_has_builder_claim(&self, source_path: &str, source_schema_type: &str) -> bool {
        self.builder_claims
            .iter()
            .any(|claim| claim.claims(source_path, Some(source_schema_type)))
    }
}

#[derive(Debug, Clone)]
struct SourceBuilderClassifier {
    source_schema_types: Vec<String>,
    source_patterns: Vec<AssetBuilderPattern>,
}

impl SourceBuilderClassifier {
    fn claims(&self, source_path: &str, source_schema_type: Option<&str>) -> bool {
        (self.source_schema_types.is_empty()
            || source_schema_type.is_some_and(|source_schema_type| {
                self.source_schema_types
                    .iter()
                    .any(|candidate| candidate == source_schema_type)
            }))
            && self
                .source_patterns
                .iter()
                .any(|pattern| pattern.matches(source_path))
    }
}

// Keep startup reconciliation transactions bounded so interactive worker and
// authoring RPCs can acquire the same WAL writer between chunks. Large source
// roots still hash concurrently; this only limits one database lock hold.
const SOURCE_ASSET_RECONCILE_MAX_BATCH_RECORDS: usize = 1_024;
const SOURCE_ASSET_SCAN_ACTIVITY_INTERVAL: usize = 512;
const SOURCE_ASSET_SCAN_LOG_INTERVAL: usize = 10_000;
/// Bound the explicit-priority identity seeks performed by one scheduler
/// phase. Bulk authoring actions may legitimately prioritize thousands of
/// sources; walking that entire set before every lease makes priority work
/// slower than the release backlog it is meant to bypass.
const PRIORITIZED_ASSET_LEASE_WINDOW: usize = 64;
/// Aggregate worker-RPC phase timings and emit one log line per this many
/// operations, so per-job overhead is attributable without per-op log spam.
const WORKER_RPC_STATS_SAMPLE: u64 = 512;

/// Rolling per-phase totals (microseconds) for one worker RPC kind.
#[derive(Default)]
struct WorkerRpcPhaseStats {
    count: Cell<u64>,
    phase_a_us: Cell<u64>,
    phase_b_us: Cell<u64>,
    phase_c_us: Cell<u64>,
    max_total_us: Cell<u64>,
}

impl WorkerRpcPhaseStats {
    /// Record one op's phase durations; returns averages when a sample
    /// window completes.
    fn record(&self, a: Duration, b: Duration, c: Duration) -> Option<(u64, u64, u64, u64)> {
        let (a, b, c) = (
            u64::try_from(a.as_micros()).unwrap_or(u64::MAX),
            u64::try_from(b.as_micros()).unwrap_or(u64::MAX),
            u64::try_from(c.as_micros()).unwrap_or(u64::MAX),
        );
        self.phase_a_us.set(self.phase_a_us.get() + a);
        self.phase_b_us.set(self.phase_b_us.get() + b);
        self.phase_c_us.set(self.phase_c_us.get() + c);
        self.max_total_us
            .set(self.max_total_us.get().max(a + b + c));
        let count = self.count.get() + 1;
        self.count.set(count);
        if count < WORKER_RPC_STATS_SAMPLE {
            return None;
        }
        let averages = (
            self.phase_a_us.get() / count,
            self.phase_b_us.get() / count,
            self.phase_c_us.get() / count,
            self.max_total_us.get(),
        );
        self.count.set(0);
        self.phase_a_us.set(0);
        self.phase_b_us.set(0);
        self.phase_c_us.set(0);
        self.max_total_us.set(0);
        Some(averages)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SourceReconcileActivity {
    active: bool,
    source_root: String,
    portable_key: String,
    current_path: String,
    source_root_index: usize,
    source_root_count: usize,
    visited_entry_count: usize,
    discovered_source_asset_count: usize,
    recorded_source_asset_count: usize,
    observed_source_asset_count: usize,
    deleted_source_asset_count: usize,
    adopted_external_source_asset_count: usize,
    conflicted_source_asset_count: usize,
    preserved_identity_rebind_count: usize,
    planned_job_count: usize,
    started_unix_ms: u64,
    updated_unix_ms: u64,
    message: String,
}

static SOURCE_RECONCILE_ACTIVITY: OnceLock<Mutex<SourceReconcileActivity>> = OnceLock::new();

fn source_reconcile_activity() -> &'static Mutex<SourceReconcileActivity> {
    SOURCE_RECONCILE_ACTIVITY.get_or_init(|| Mutex::new(SourceReconcileActivity::default()))
}

fn source_reconcile_now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn current_unix_ms_i64() -> Result<i64, std::time::SystemTimeError> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(i64::try_from(millis).unwrap_or(i64::MAX))
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn source_reconcile_activity_begin(source_root_count: usize) {
    let now = source_reconcile_now_unix_ms();
    let mut activity = source_reconcile_activity()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *activity = SourceReconcileActivity {
        active: true,
        source_root_count,
        started_unix_ms: now,
        updated_unix_ms: now,
        message: format!("scanning {source_root_count} asset source roots"),
        ..SourceReconcileActivity::default()
    };
}

fn source_reconcile_activity_source_root_started(
    source_root: &RegisteredSourceRoot,
    source_root_index: usize,
    source_root_count: usize,
) {
    let mut activity = source_reconcile_activity()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    activity.active = true;
    activity.source_root.clone_from(&source_root.path);
    activity.portable_key.clone_from(&source_root.portable_key);
    activity.current_path.clear();
    activity.source_root_index = source_root_index;
    activity.source_root_count = source_root_count;
    activity.visited_entry_count = 0;
    activity.discovered_source_asset_count = 0;
    activity.updated_unix_ms = source_reconcile_now_unix_ms();
    activity.message = format!(
        "scanning source root {source_root_index} of {source_root_count}: {}",
        source_root.display_name
    );
}

fn source_reconcile_activity_scan_progress(
    source_root: &RegisteredSourceRoot,
    path: &Path,
    visited_entry_count: usize,
) {
    let mut activity = source_reconcile_activity()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    activity.active = true;
    activity.source_root.clone_from(&source_root.path);
    activity.portable_key.clone_from(&source_root.portable_key);
    activity.current_path = path.display().to_string();
    activity.visited_entry_count = visited_entry_count;
    activity.updated_unix_ms = source_reconcile_now_unix_ms();
    activity.message = format!(
        "scanned {visited_entry_count} entries in {}",
        source_root.display_name
    );
}

fn source_reconcile_activity_recorded(recorded_source_asset_count: usize) {
    let mut activity = source_reconcile_activity()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    activity.recorded_source_asset_count = recorded_source_asset_count;
    activity.updated_unix_ms = source_reconcile_now_unix_ms();
    activity.message = format!("recorded {recorded_source_asset_count} source assets");
}

fn source_reconcile_activity_complete(summary: RegisteredSourceAssetsReconcileSummary) {
    let mut activity = source_reconcile_activity()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    activity.active = false;
    activity.updated_unix_ms = source_reconcile_now_unix_ms();
    activity.recorded_source_asset_count = summary.recorded;
    activity.observed_source_asset_count = summary.observed;
    activity.deleted_source_asset_count = summary.deleted;
    activity.adopted_external_source_asset_count = summary.adopted_external;
    activity.conflicted_source_asset_count = summary.conflicted;
    activity.preserved_identity_rebind_count = summary.preserved_identity_rebinds;
    activity.planned_job_count = summary.planned_jobs;
    activity.message = format!(
        "source scan complete: {} recorded, {} observed unchanged, {} deleted, {} external checkpoints adopted, {} preserved identities rebound, {} conflicts, {} queued jobs",
        summary.recorded,
        summary.observed,
        summary.deleted,
        summary.adopted_external,
        summary.preserved_identity_rebinds,
        summary.conflicted,
        summary.planned_jobs
    );
}

fn source_reconcile_activity_failed(error: impl fmt::Display) {
    let mut activity = source_reconcile_activity()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    activity.active = false;
    activity.updated_unix_ms = source_reconcile_now_unix_ms();
    activity.message = format!("source scan failed: {error}");
}

fn source_reconcile_activity_snapshot() -> SourceReconcileActivity {
    source_reconcile_activity()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

#[derive(Clone, Copy)]
enum StartupReconcileMode<'a> {
    DeferredToService,
    #[allow(dead_code)]
    Blocking {
        changed_by_session: Option<&'a str>,
        /// The composition whose source schemas and build rules classify what
        /// this reconcile finds. Only the blocking mode needs one: the
        /// deferred mode reconciles nothing at startup.
        registries: &'a Registries,
    },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RegisteredSourceAssetsReconcileSummary {
    recorded: usize,
    observed: usize,
    deleted: usize,
    adopted_external: usize,
    conflicted: usize,
    preserved_identity_rebinds: usize,
    planned_jobs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSourceAssetRecord {
    source_path: String,
    schema_type: String,
    content_hash: Digest,
    changed_unix_ms: i64,
    diagnostics_count: i64,
    observation: SourceFileObservation,
    /// Stable identity from the native metadata sidecar, or the canonical
    /// path-derived identity for a first observation.
    asset_guid: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceRootScanCandidate {
    entry_path: PathBuf,
    source_path: String,
    file_source_schema: Option<String>,
    has_project_document_candidates: bool,
    observation: SourceFileObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceFileObservation {
    source_file_byte_length: i64,
    source_file_modified_unix_ns: i64,
    source_meta_byte_length: i64,
    source_meta_modified_unix_ns: i64,
    last_observed_unix_ms: i64,
}

#[derive(Debug, Clone)]
struct ProjectDocumentSourceClassifier {
    source_schema_type: String,
    source_patterns: Vec<AssetBuilderPattern>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSourceClassifier {
    source_schema_type: String,
    source_root: String,
    default_path_prefix: String,
    source_patterns: Vec<AssetBuilderPattern>,
    extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedJobDependency {
    dependency_source: String,
    dependency_job_key: String,
    dependency_platform: String,
    kind: DbCoupling,
}

/// JSON body of the side-channel product emitted by planner jobs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WorkerCreateJobsPlan {
    builders: Vec<WorkerCreateJobsBuilderPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WorkerCreateJobsBuilderPlan {
    builder_guid: uuid::Uuid,
    jobs: Vec<WorkerCreateJobsJobPlan>,
    source_dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WorkerCreateJobsJobPlan {
    job_key: String,
    platform: String,
}

/// Register workspace source roots and return their durable metadata.
///
/// This metadata-only form closes its database handle before returning. The
/// production host should use [`open_registered_workspace_asset_db`] so the same
/// database owner remains alive through service startup.
///
/// # Errors
///
/// Returns [`AssetProcessorError::Open`] if the `AssetDB` at `db_path` cannot be
/// opened, [`AssetProcessorError::ProjectManifest`] if the project manifest
/// cannot be read, [`AssetProcessorError::ProjectManifestIdMismatch`] if it
/// disagrees with `project_id`, the `WorkspaceRoot*` variants if
/// `workspace_root` is not a usable absolute directory, the `SourceRoot*`
/// variants for a declared source root, and [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
#[instrument(skip(db_path, workspace_root), fields(project_id = %project_id, workspace_root = %workspace_root.as_ref().display(), branch = %branch))]
pub fn register_workspace_asset_source_roots(
    db_path: impl AsRef<Path>,
    project_id: &str,
    workspace_root: impl AsRef<Path>,
    branch: &str,
    now_unix_ms: i64,
) -> Result<WorkspaceAssetSourceRootsRegistration, AssetProcessorError> {
    Ok(open_registered_workspace_asset_db(
        db_path,
        project_id,
        workspace_root,
        branch,
        now_unix_ms,
    )?
    .into_registration())
}

/// Open the asset database and register the source roots required by a
/// production asset-processor project instance.
///
/// The returned value deliberately retains the database owner. Consume it with
/// [`start_asset_processor_rpc_server_with_registered_workspace_db`] rather than
/// reopening the database between startup phases.
///
/// # Errors
///
/// Returns any error [`register_workspace_asset_source_roots`] returns; this is
/// the same open-and-register path, handing back the live handle instead of just
/// the registration.
#[instrument(skip(db_path, workspace_root), fields(project_id = %project_id, workspace_root = %workspace_root.as_ref().display(), branch = %branch))]
pub fn open_registered_workspace_asset_db(
    db_path: impl AsRef<Path>,
    project_id: &str,
    workspace_root: impl AsRef<Path>,
    branch: &str,
    now_unix_ms: i64,
) -> Result<RegisteredWorkspaceAssetDb, AssetProcessorError> {
    open_registered_workspace_asset_db_with_mode(
        db_path,
        project_id,
        workspace_root,
        branch,
        now_unix_ms,
        StartupReconcileMode::DeferredToService,
    )
}

/// # Errors
///
/// Returns any error [`register_workspace_asset_source_roots`] returns. This
/// variant also resolves source roots from `registries`, so it reports the
/// `SourceRoot*` variants for a registry-declared root as well.
#[cfg(any(test, feature = "test-support"))]
#[instrument(skip(db_path, workspace_root, registries), fields(project_id = %project_id, changed_by_session, workspace_root = %workspace_root.as_ref().display(), branch = %branch))]
pub fn register_workspace_asset_source_roots_blocking(
    db_path: impl AsRef<Path>,
    project_id: &str,
    changed_by_session: Option<&str>,
    workspace_root: impl AsRef<Path>,
    branch: &str,
    now_unix_ms: i64,
    registries: &Registries,
) -> Result<WorkspaceAssetSourceRootsRegistration, AssetProcessorError> {
    Ok(open_registered_workspace_asset_db_with_mode(
        db_path,
        project_id,
        workspace_root,
        branch,
        now_unix_ms,
        StartupReconcileMode::Blocking {
            changed_by_session,
            registries,
        },
    )?
    .into_registration())
}

fn open_registered_workspace_asset_db_with_mode(
    db_path: impl AsRef<Path>,
    project_id: &str,
    workspace_root: impl AsRef<Path>,
    branch: &str,
    now_unix_ms: i64,
    reconcile_mode: StartupReconcileMode<'_>,
) -> Result<RegisteredWorkspaceAssetDb, AssetProcessorError> {
    let registration_started = Instant::now();
    validate_workspace_identity(project_id, branch)?;
    let db_path = db_path.as_ref().to_path_buf();
    let workspace_root = normalize_workspace_root(workspace_root.as_ref())?;
    let graph_started = Instant::now();
    let source_root_specs = workspace_asset_source_root_specs(project_id, &workspace_root)?;
    let graph_ms = duration_millis_u64(graph_started.elapsed());
    let database_started = Instant::now();
    let db = AssetDb::open(&db_path)?;
    let database_ms = duration_millis_u64(database_started.elapsed());
    let roots_started = Instant::now();
    let asset_db_writer = db.writer()?;
    let workspace = asset_db_writer
        .register_workspace(RegisterWorkspace {
            key: WorkspaceKey {
                project: project_id.to_owned(),
                root: workspace_root.to_string_lossy().into_owned(),
                branch: branch.to_owned(),
            },
            now: now_unix_ms,
        })
        .wait_blocking()?;
    let workspace_id = workspace.workspace_id;
    let source_root_rows =
        register_workspace_source_roots(&asset_db_writer, workspace_id, source_root_specs)?;
    let roots_ms = duration_millis_u64(roots_started.elapsed());
    let reconcile_started = Instant::now();
    let reconcile_summary = match reconcile_mode {
        StartupReconcileMode::DeferredToService => {
            RegisteredSourceAssetsReconcileSummary::default()
        }
        StartupReconcileMode::Blocking {
            changed_by_session,
            registries,
        } => {
            if let Some(changed_by_session) = changed_by_session {
                validate_session_id(changed_by_session)?;
            }
            let classifiers = source_asset_classifiers(None, registries);
            reconcile_registered_source_assets(
                ReconcilePass {
                    db: &db,
                    writer: &asset_db_writer,
                    changed_by_session,
                    classifiers: &classifiers,
                    now_unix_ms,
                },
                &source_root_rows,
            )?
        }
    };
    let reconcile_ms = duration_millis_u64(reconcile_started.elapsed());
    let source_roots = source_root_rows
        .iter()
        .cloned()
        .map(registered_source_root_from_row)
        .collect::<Vec<_>>();

    info!(
        project_id,
        workspace_id,
        graph_ms,
        database_ms,
        roots_ms,
        reconcile_ms,
        total_ms = duration_millis_u64(registration_started.elapsed()),
        source_root_count = source_roots.len(),
        recorded_source_asset_count = reconcile_summary.recorded,
        deleted_source_asset_count = reconcile_summary.deleted,
        startup_reconcile = match reconcile_mode {
            StartupReconcileMode::DeferredToService => "deferred_to_service",
            StartupReconcileMode::Blocking { .. } => "blocking",
        },
        "asset processor registered workspace asset source roots"
    );
    Ok(RegisteredWorkspaceAssetDb {
        db_path,
        database: db,
        asset_db_writer,
        source_roots: source_root_rows,
        registration: WorkspaceAssetSourceRootsRegistration {
            workspace_id,
            project_id: project_id.to_string(),
            workspace_root,
            branch: branch.to_string(),
            source_roots,
        },
    })
}

/// Writes the source-root topology for a workspace and returns the runtime rows.
///
/// This is the one post-open bootstrap write. It establishes the immutable
/// runtime root topology before the composition publishes its single writer;
/// every later processor mutation enters through that writer.
fn register_workspace_source_roots(
    asset_db_writer: &AssetDbWriter,
    workspace_id: i64,
    source_root_specs: Vec<AssetSourceRootSpec>,
) -> Result<Vec<RegisteredSourceRoot>, AssetProcessorError> {
    let desired_roots = source_root_specs
        .iter()
        .map(|spec| WorkspaceRootRegistration {
            key: spec.portable_key.as_str().to_owned(),
            owner: spec.owner_id.clone(),
            path: spec.root.to_string_lossy().into_owned(),
            exclusions: spec.excluded_paths.clone(),
        })
        .collect();
    let bindings = asset_db_writer
        .replace_workspace_roots(ReplaceWorkspaceRoots {
            workspace_pk: workspace_id,
            roots: desired_roots,
        })
        .wait_blocking()?;
    let mut source_root_rows = Vec::with_capacity(source_root_specs.len());
    for (spec, binding) in source_root_specs.into_iter().zip(bindings) {
        let path = spec.root.to_string_lossy().into_owned();
        source_root_rows.push(RegisteredSourceRoot {
            workspace_pk: workspace_id,
            workspace_root_pk: binding.policy.workspace_root_id,
            root_pk: binding.root.root_id,
            id: spec.id,
            owner: spec.owner_id,
            path,
            display_name: spec.display_name,
            portable_key: spec.portable_key.as_str().to_owned(),
            mount: spec.mount.as_str().to_owned(),
            recursive: spec.recursive,
            watch: spec.watch,
            writable: spec.writable,
            exclusions: spec.excluded_paths,
            output_prefix: spec.output_prefix,
            role: spec.role,
        });
    }
    Ok(source_root_rows)
}

fn validate_workspace_identity(project_id: &str, branch: &str) -> Result<(), AssetProcessorError> {
    if project_id.trim().is_empty() {
        return Err(AssetProcessorError::ProjectIdRequired);
    }
    if branch.trim().is_empty() {
        return Err(AssetProcessorError::WorkspaceBranchRequired);
    }
    Ok(())
}

fn validate_session_id(session_id: &str) -> Result<(), AssetProcessorError> {
    if session_id.trim().is_empty() {
        return Err(AssetProcessorError::SessionIdRequired);
    }
    let session = uuid::Uuid::parse_str(session_id).map_err(|source| {
        AssetProcessorError::InvalidSessionId {
            session_id: session_id.to_string(),
            source,
        }
    })?;
    if session.is_nil() {
        return Err(AssetProcessorError::NilSessionId {
            session_id: session_id.to_string(),
        });
    }
    Ok(())
}

/// Resolve one workspace's source roots in precedence order.
///
/// `az_project` already returns roots ordered by home tier and, inside a tier,
/// by lock-closure order (ADR 0034). That order is the whole ordering model —
/// it is preserved here, through registration, and back out of the database —
/// so no root needs a priority number to rank it.
fn workspace_asset_source_root_specs(
    project_id: &str,
    workspace_root: &Path,
) -> Result<Vec<AssetSourceRootSpec>, AssetProcessorError> {
    let graph = load_workspace_project_graph(project_id, workspace_root)?;

    let project_assets_key = PortableKey::project_assets(project_id);
    let mut source_roots = graph
        .source_roots
        .into_iter()
        .map(|source_root| AssetSourceRootSpec {
            role: SourceRootRole::from_is_project_assets(
                source_root.portable_key == project_assets_key,
            ),
            id: source_root.id,
            owner_id: source_root.owner_id,
            root: source_root.root,
            display_name: source_root.display_name,
            portable_key: source_root.portable_key,
            mount: source_root.mount,
            recursive: source_root.recursive,
            watch: source_root.watch,
            writable: source_root.writable,
            excluded_paths: Exclusions::default(),
            output_prefix: source_root.output_prefix,
        })
        .collect::<Vec<_>>();

    for source_root in &mut source_roots {
        source_root.root = normalize_asset_source_root(source_root)?;
    }

    apply_asset_namespace_policy(&mut source_roots, &graph.asset_overrides)?;

    Ok(source_roots)
}

fn load_workspace_project_graph(
    project_id: &str,
    workspace_root: &Path,
) -> Result<ResolvedProjectGraph, AssetProcessorError> {
    let graph = load_resolved_project_graph(workspace_root)?;
    if graph.manifest.project.id != project_id {
        return Err(AssetProcessorError::ProjectManifestIdMismatch {
            workspace_root: workspace_root.to_path_buf(),
            expected: project_id.to_string(),
            actual: graph.manifest.project.id,
        });
    }

    Ok(graph)
}

fn normalize_workspace_root(workspace_root: &Path) -> Result<PathBuf, AssetProcessorError> {
    if !workspace_root.is_absolute() {
        return Err(AssetProcessorError::WorkspaceRootNotAbsolute {
            workspace_root: workspace_root.to_path_buf(),
        });
    }
    let metadata = std::fs::metadata(workspace_root).map_err(|source| {
        AssetProcessorError::WorkspaceRootRead {
            workspace_root: workspace_root.to_path_buf(),
            source,
        }
    })?;
    if !metadata.is_dir() {
        return Err(AssetProcessorError::WorkspaceRootNotDirectory {
            workspace_root: workspace_root.to_path_buf(),
        });
    }
    canonical(workspace_root).map_err(|source| AssetProcessorError::WorkspaceRootCanonicalize {
        workspace_root: workspace_root.to_path_buf(),
        source,
    })
}

fn normalize_asset_source_root(
    source_root: &AssetSourceRootSpec,
) -> Result<PathBuf, AssetProcessorError> {
    if !source_root.root.is_absolute() {
        return Err(AssetProcessorError::SourceRootNotAbsolute {
            owner_id: source_root.owner_id.clone(),
            display_name: source_root.display_name.clone(),
            root: source_root.root.clone(),
        });
    }
    let metadata = match std::fs::metadata(&source_root.root) {
        Ok(metadata) => metadata,
        Err(source) if !source_root.role.is_required() && source.kind() == ErrorKind::NotFound => {
            return Ok(auxiliary_source_root_lexical_path(source_root));
        }
        Err(source) => {
            return Err(AssetProcessorError::SourceRootRead {
                owner_id: source_root.owner_id.clone(),
                display_name: source_root.display_name.clone(),
                root: source_root.root.clone(),
                source,
            });
        }
    };
    if !metadata.is_dir() {
        return Err(AssetProcessorError::SourceRootNotDirectory {
            owner_id: source_root.owner_id.clone(),
            display_name: source_root.display_name.clone(),
            root: source_root.root.clone(),
        });
    }
    match canonical(&source_root.root) {
        Ok(root) => Ok(root),
        Err(source) if !source_root.role.is_required() && source.kind() == ErrorKind::NotFound => {
            Ok(auxiliary_source_root_lexical_path(source_root))
        }
        Err(source) => Err(AssetProcessorError::SourceRootCanonicalize {
            owner_id: source_root.owner_id.clone(),
            display_name: source_root.display_name.clone(),
            root: source_root.root.clone(),
            source,
        }),
    }
}

fn auxiliary_source_root_lexical_path(source_root: &AssetSourceRootSpec) -> PathBuf {
    normalize_lexical(&source_root.root)
}

#[derive(Debug)]
struct AssetSourceClaim {
    root_id: String,
    physical_path: PathBuf,
}

/// The part of a source root the namespace claim walk reads.
///
/// The walk runs over freshly resolved specs at registration and over
/// registered database rows afterwards; borrowing the four fields it needs
/// lets both call it without either shape having to fabricate the other.
#[derive(Debug, Clone, Copy)]
struct Scan<'a> {
    id: &'a str,
    root: &'a Path,
    recursive: bool,
    /// The project's default asset root must exist; every other root may be
    /// declared before its directory does.
    required: bool,
}

impl<'a> Scan<'a> {
    fn spec(spec: &'a AssetSourceRootSpec) -> Self {
        Self {
            id: &spec.id,
            root: &spec.root,
            recursive: spec.recursive,
            required: spec.role.is_required(),
        }
    }

    fn row(row: &'a RegisteredSourceRoot) -> Self {
        Self {
            id: &row.id,
            root: Path::new(&row.path),
            recursive: row.recursive,
            required: row.role.is_required(),
        }
    }
}

fn apply_asset_namespace_policy(
    roots: &mut [AssetSourceRootSpec],
    overrides: &[ProjectAssetOverride],
) -> Result<(), AssetProcessorError> {
    let mut claims: BTreeMap<String, Vec<AssetSourceClaim>> = BTreeMap::new();
    for root in roots.iter() {
        collect_asset_source_claims(Scan::spec(root), &mut claims)?;
    }

    let normalized_overrides = overrides
        .iter()
        .map(|declaration| {
            Ok((
                declaration.normalized_path()?,
                declaration.winning_root.as_str(),
                declaration.replaced_root.as_str(),
            ))
        })
        .collect::<Result<Vec<_>, ProjectManifestError>>()?;
    let mut used_overrides = BTreeSet::new();
    let mut excluded_by_root: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();

    for (path, path_claims) in claims.iter().filter(|(_, claims)| claims.len() > 1) {
        let claim_roots = path_claims
            .iter()
            .map(|claim| claim.root_id.as_str())
            .collect::<BTreeSet<_>>();
        let winners = normalized_overrides
            .iter()
            .filter(|(override_path, winning, replaced)| {
                override_path == path
                    && claim_roots.contains(winning)
                    && claim_roots.contains(replaced)
            })
            .map(|(_, winning, _)| *winning)
            .collect::<BTreeSet<_>>();
        let Some(winning_root) = winners
            .iter()
            .next()
            .copied()
            .filter(|_| winners.len() == 1)
        else {
            return Err(asset_source_collision(path, path_claims));
        };

        for claim in path_claims {
            if claim.root_id == winning_root {
                continue;
            }
            let declaration =
                normalized_overrides
                    .iter()
                    .find(|(override_path, winning, replaced)| {
                        override_path == path
                            && *winning == winning_root
                            && *replaced == claim.root_id
                    });
            let Some((override_path, winning, replaced)) = declaration else {
                return Err(asset_source_collision(path, path_claims));
            };
            used_overrides.insert((
                override_path.clone(),
                (*winning).to_string(),
                (*replaced).to_string(),
            ));
            excluded_by_root
                .entry(&claim.root_id)
                .or_default()
                .insert(path.clone());
        }
    }

    if let Some((path, winning_root, replaced_root)) =
        normalized_overrides
            .iter()
            .find(|(path, winning, replaced)| {
                !used_overrides.contains(&(
                    path.clone(),
                    (*winning).to_string(),
                    (*replaced).to_string(),
                ))
            })
    {
        return Err(AssetProcessorError::AssetOverrideDoesNotMatchCollision {
            virtual_path: format!("{ASSET_NAMESPACE_MOUNT}/{path}"),
            winning_root: (*winning_root).to_string(),
            replaced_root: (*replaced_root).to_string(),
        });
    }

    for root in roots {
        root.excluded_paths = Exclusions::from(
            excluded_by_root
                .remove(root.id.as_str())
                .unwrap_or_default(),
        );
    }
    Ok(())
}

fn collect_asset_source_claims(
    root: Scan<'_>,
    claims: &mut BTreeMap<String, Vec<AssetSourceClaim>>,
) -> Result<(), AssetProcessorError> {
    let entries = match fs::read_dir(root.root) {
        Ok(entries) => entries,
        Err(source) if !root.required && source.kind() == ErrorKind::NotFound => {
            return Ok(());
        }
        Err(source) => {
            return Err(AssetProcessorError::SourceRootReconcileDir {
                path: root.root.to_path_buf(),
                source,
            });
        }
    };
    let mut stack = vec![SourceRootDirFrame {
        path: root.root.to_path_buf(),
        entries,
    }];

    while let Some(frame) = stack.last_mut() {
        let entry = match frame.entries.next() {
            Some(Ok(entry)) => entry,
            Some(Err(source)) => {
                return Err(AssetProcessorError::SourceRootReconcileDir {
                    path: frame.path.clone(),
                    source,
                });
            }
            None => {
                stack.pop();
                continue;
            }
        };
        let physical_path = entry.path();
        let file_type =
            entry
                .file_type()
                .map_err(|source| AssetProcessorError::SourceRootReconcileEntry {
                    path: physical_path.clone(),
                    source,
                })?;
        if file_type.is_dir() {
            if root.recursive {
                stack.push(SourceRootDirFrame {
                    entries: fs::read_dir(&physical_path).map_err(|source| {
                        AssetProcessorError::SourceRootReconcileDir {
                            path: physical_path.clone(),
                            source,
                        }
                    })?,
                    path: physical_path,
                });
            }
            continue;
        }
        if !file_type.is_file() || is_asset_root_scaffold_marker(&physical_path) {
            continue;
        }
        let Some(relative) = physical_path.strip_prefix(root.root).ok() else {
            continue;
        };
        let relative =
            relative
                .to_str()
                .ok_or_else(|| AssetProcessorError::InvalidNativeSourcePath {
                    path: physical_path.clone(),
                    reason: "path is not valid Unicode".to_string(),
                })?;
        let path = NativeAssetPath::new(relative).map_err(|error| {
            AssetProcessorError::InvalidNativeSourcePath {
                path: physical_path.clone(),
                reason: error.to_string(),
            }
        })?;
        claims
            .entry(path.as_str().to_string())
            .or_default()
            .push(AssetSourceClaim {
                root_id: root.id.to_string(),
                physical_path,
            });
    }
    Ok(())
}

fn is_asset_root_scaffold_marker(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.eq_ignore_ascii_case("README.md") || name.eq_ignore_ascii_case(".gitkeep")
    })
}

fn asset_source_collision(path: &str, claims: &[AssetSourceClaim]) -> AssetProcessorError {
    debug_assert!(claims.len() >= 2);
    let mut claims = claims.iter();
    let first = claims.next().expect("collision has a first claim");
    let second = claims.next().expect("collision has a second claim");
    AssetProcessorError::AssetSourceCollision(Box::new(AssetSourceCollisionDetail {
        virtual_path: format!("{ASSET_NAMESPACE_MOUNT}/{path}"),
        first_root: first.root_id.clone(),
        first_path: first.physical_path.clone(),
        second_root: second.root_id.clone(),
        second_path: second.physical_path.clone(),
    }))
}

fn registered_source_root_from_row(row: RegisteredSourceRoot) -> RegisteredAssetSourceRoot {
    RegisteredAssetSourceRoot {
        workspace_root_id: row.workspace_root_pk,
        scan_folder_id: row.root_pk,
        id: row.id,
        owner_id: row.owner,
        root: PathBuf::from(row.path),
        display_name: row.display_name,
        portable_key: row.portable_key,
        mount: row.mount,
        recursive: row.recursive,
        watch: row.watch,
        writable: row.writable,
        output_prefix: row.output_prefix,
    }
}

fn is_browser_asset_source_root(root: &RegisteredSourceRoot) -> bool {
    !root.path.trim().is_empty()
}

/// The ambient facts one source-reconcile pass runs under.
///
/// Every root in a pass must see exactly these five: the same read handle and
/// the same writer, the same provenance, the same classifier set, and one
/// timestamp. The timestamp in particular is why they travel together -- a pass
/// that re-read the clock per root would stamp one reconcile across several
/// instants, and observation fast-paths compare against that stamp. The pass
/// entry point, its scoped form, and each root's sweep all forward this value
/// unchanged.
#[derive(Clone, Copy)]
struct ReconcilePass<'a> {
    db: &'a AssetDb,
    writer: &'a AssetDbWriter,
    changed_by_session: Option<&'a str>,
    classifiers: &'a SourceAssetClassifiers,
    now_unix_ms: i64,
}

fn reconcile_registered_source_assets(
    pass: ReconcilePass<'_>,
    source_roots: &[RegisteredSourceRoot],
) -> Result<RegisteredSourceAssetsReconcileSummary, AssetProcessorError> {
    reconcile_registered_source_assets_scoped(pass, source_roots, source_roots, None)
}

fn reconcile_registered_source_assets_scoped(
    pass: ReconcilePass<'_>,
    namespace_roots: &[RegisteredSourceRoot],
    roots_to_reconcile: &[RegisteredSourceRoot],
    scopes: Option<&BTreeMap<i64, SweepScope>>,
) -> Result<RegisteredSourceAssetsReconcileSummary, AssetProcessorError> {
    validate_registered_asset_namespace(namespace_roots)?;
    source_reconcile_activity_begin(roots_to_reconcile.len());
    let started = Instant::now();
    let result = (|| {
        let mut summary = RegisteredSourceAssetsReconcileSummary::default();
        for (index, source_root) in roots_to_reconcile.iter().enumerate() {
            let scope = match scopes {
                None => &SweepScope::Root,
                Some(scopes) => scopes.get(&source_root.workspace_root_pk).ok_or(
                    AssetProcessorError::MissingSweepScope {
                        workspace_root_id: source_root.workspace_root_pk,
                    },
                )?,
            };
            let source_root_summary = reconcile_registered_source_root_assets(
                pass,
                source_root,
                index + 1,
                roots_to_reconcile.len(),
                scope,
            )?;
            summary.recorded += source_root_summary.recorded;
            summary.observed += source_root_summary.observed;
            summary.deleted += source_root_summary.deleted;
            summary.adopted_external += source_root_summary.adopted_external;
            summary.conflicted += source_root_summary.conflicted;
            summary.preserved_identity_rebinds += source_root_summary.preserved_identity_rebinds;
            summary.planned_jobs += source_root_summary.planned_jobs;
        }
        Ok(summary)
    })();
    match &result {
        Ok(summary) => {
            source_reconcile_activity_complete(*summary);
            info!(
                source_root_count = roots_to_reconcile.len(),
                namespace_root_count = namespace_roots.len(),
                recorded_source_asset_count = summary.recorded,
                observed_source_asset_count = summary.observed,
                deleted_source_asset_count = summary.deleted,
                adopted_external_source_asset_count = summary.adopted_external,
                conflicted_source_asset_count = summary.conflicted,
                preserved_identity_rebind_count = summary.preserved_identity_rebinds,
                elapsed_ms = started.elapsed().as_millis(),
                "asset source startup reconcile completed"
            );
        }
        Err(error) => {
            source_reconcile_activity_failed(error);
        }
    }
    result
}

/// What changed between the catalog the processor holds and the one being
/// published.
struct BuilderCatalogDiff {
    /// Set when the previous catalog could not be trusted, so every source has
    /// to be replanned instead of only the ones a changed builder claims.
    full_replan: bool,
    /// Builders whose descriptor digest moved between the two catalogs.
    changed_guids: HashSet<Uuid>,
    /// What those builders claim, on both the old and the new side, so a source
    /// that stops being claimed is invalidated as well as one that starts.
    changed_claims: Vec<SourceBuilderClassifier>,
}

/// Diffs the published catalog against the one the caller says it replaces.
///
/// The previous catalog is only trusted when its digest matches what the
/// publisher expected; otherwise this process missed a replacement and cannot
/// tell which builders moved, so everything is treated as changed.
fn builder_catalog_diff(
    expected: Option<Digest>,
    previous: Option<&AssetBuilderCatalogResult>,
    replacement: &AssetBuilderCatalogResult,
) -> BuilderCatalogDiff {
    let trusted_previous = previous.filter(|catalog| {
        expected.is_some_and(|digest| worker_builder_catalog_digest(catalog) == digest)
    });
    let next_by_guid = replacement
        .builders
        .iter()
        .map(|builder| {
            (
                builder.builder_guid,
                asset_builder_descriptor_digest(builder),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let previous_by_guid = trusted_previous.map(|catalog| {
        catalog
            .builders
            .iter()
            .map(|builder| {
                (
                    builder.builder_guid,
                    asset_builder_descriptor_digest(builder),
                )
            })
            .collect::<BTreeMap<_, _>>()
    });
    let full_replan = previous_by_guid.is_none();
    let changed_guids = previous_by_guid
        .as_ref()
        .map_or_else(HashSet::new, |previous| {
            previous
                .keys()
                .chain(next_by_guid.keys())
                .copied()
                .filter(|guid| previous.get(guid) != next_by_guid.get(guid))
                .collect::<HashSet<_>>()
        });
    let changed_claims = trusted_previous
        .into_iter()
        .flat_map(|catalog| &catalog.builders)
        .chain(&replacement.builders)
        .filter(|builder| full_replan || changed_guids.contains(&builder.builder_guid))
        .map(|builder| SourceBuilderClassifier {
            source_schema_types: builder.source_schema_types.clone(),
            source_patterns: catalog_patterns(&builder.patterns),
        })
        .collect::<Vec<_>>();
    BuilderCatalogDiff {
        full_replan,
        changed_guids,
        changed_claims,
    }
}

/// What planning needs to know about a source before it can replan it.
struct SourcePlanFacts {
    source_root: PathBuf,
    source_bytes: Vec<u8>,
    retire_job_ids: Vec<i64>,
    retire_source_edge_ids: Vec<i64>,
    /// Set when a live job of the kind this host plans already covers the
    /// source, so a clean re-observation does not have to replan it.
    has_current_execution_job: bool,
}

/// The durable rows a source-asset record publishes against.
struct SourceAssetRecord {
    workspace: SelectWorkspaces,
    policy: az_assetdb::SelectWorkspaceRoots,
    root: az_assetdb::SelectRoots,
    payload: az_assetdb::SelectPayloads,
    /// The already-registered asset and entry, when this source has been
    /// recorded before. Absent means this record introduces it.
    existing: Option<(SelectAssets, SelectEntries)>,
}

fn validate_registered_asset_namespace(
    roots: &[RegisteredSourceRoot],
) -> Result<(), AssetProcessorError> {
    let mut claims = BTreeMap::new();
    let mut exclusions = BTreeMap::new();
    for root in roots {
        let root_exclusions = root.exclusions.as_set().clone();
        exclusions.insert(root.id.as_str(), root_exclusions);
        collect_asset_source_claims(Scan::row(root), &mut claims)?;
    }

    for (path, path_claims) in claims.iter().filter(|(_, claims)| claims.len() > 1) {
        let visible_claims = path_claims
            .iter()
            .filter(|claim| {
                !exclusions
                    .get(claim.root_id.as_str())
                    .is_some_and(|paths| paths.contains(path))
            })
            .count();
        if visible_claims != 1 {
            return Err(asset_source_collision(path, path_claims));
        }
    }
    Ok(())
}

fn source_asset_classifiers(
    published_catalog: Option<&AssetBuilderCatalogResult>,
    registries: &Registries,
) -> SourceAssetClassifiers {
    // Prefer the catalog published by a connected project asset-worker. The
    // engine-owned host composes no project contributions; its own
    // composition is only meaningful for tests/harnesses that compose
    // builders in-process.
    if let Some(catalog) = published_catalog {
        return source_asset_classifiers_from_catalog(catalog);
    }
    source_asset_classifiers_from_composition(registries)
}

fn source_asset_classifiers_from_composition(registries: &Registries) -> SourceAssetClassifiers {
    let mut project_documents = Vec::new();
    let mut file_sources = Vec::new();
    let builders = BuildRuleRegistry::compose(&BuilderJobContext::new(registries));

    for attributed in composed_source_schemas(registries) {
        let registration = attributed.entry;
        let source_schema_type = registration.schema_type().as_str().to_string();
        match registration.authoring() {
            BuilderSourceSchemaAuthoring::ProjectDocument { schema_type } => {
                let source_patterns = registration.source_patterns().to_vec();
                if source_patterns.is_empty() {
                    warn!(
                        source_schema_type,
                        document_schema_type = schema_type,
                        "asset processor project-document source has no registered source-format patterns"
                    );
                    continue;
                }
                project_documents.push(ProjectDocumentSourceClassifier {
                    source_schema_type,
                    source_patterns,
                });
            }
            BuilderSourceSchemaAuthoring::File { workflow } => {
                file_sources.push(FileSourceClassifier {
                    source_schema_type,
                    source_root: workflow.source_root().to_string(),
                    default_path_prefix: workflow.default_path_prefix().to_string(),
                    source_patterns: registration.source_patterns().to_vec(),
                    extensions: workflow
                        .extensions()
                        .iter()
                        .map(|extension| (*extension).to_string())
                        .collect(),
                });
            }
        }
    }

    SourceAssetClassifiers {
        project_documents,
        file_sources,
        builder_claims: builders
            .iter()
            .map(|builder| SourceBuilderClassifier {
                source_schema_types: builder
                    .primary_source
                    .schema_types()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                source_patterns: builder.primary_source.patterns().to_vec(),
            })
            .collect(),
    }
}

fn catalog_source_patterns_for_schema(
    catalog: &AssetBuilderCatalogResult,
    source_schema_type: &str,
) -> Vec<AssetBuilderPattern> {
    catalog
        .builders
        .iter()
        .filter(|builder| {
            builder
                .source_schema_types
                .iter()
                .any(|candidate| candidate == source_schema_type)
        })
        .flat_map(|builder| catalog_patterns(&builder.patterns))
        .collect()
}

fn catalog_patterns(patterns: &[AssetBuilderPatternDescriptor]) -> Vec<AssetBuilderPattern> {
    patterns
        .iter()
        .filter_map(|pattern| match pattern.kind {
            AssetBuilderPatternKind::Wildcard => {
                Some(AssetBuilderPattern::wildcard(&pattern.pattern))
            }
            AssetBuilderPatternKind::Regex => AssetBuilderPattern::regex(&pattern.pattern).ok(),
        })
        .collect()
}

fn hash_builder_catalog_text(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

const ASSET_BUILDER_DESCRIPTOR_DIGEST_DOMAIN: &str = "azoth.asset-builder-descriptor/v1";
const ASSET_BUILDER_CATALOG_DIGEST_DOMAIN: &str = "azoth.asset-builder-catalog/v1";

fn asset_builder_descriptor_digest(builder: &AssetBuilderDescriptor) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hash_builder_catalog_text(&mut hasher, ASSET_BUILDER_DESCRIPTOR_DIGEST_DOMAIN);
    hash_builder_catalog_text(&mut hasher, &builder.name);
    hasher.update(builder.builder_guid.as_bytes());
    hasher.update(&builder.version.to_le_bytes());
    hash_builder_catalog_text(&mut hasher, &builder.analysis_fingerprint);

    let mut patterns = builder.patterns.iter().collect::<Vec<_>>();
    patterns.sort_by(|left, right| {
        (left.kind as u8, left.pattern.as_str()).cmp(&(right.kind as u8, right.pattern.as_str()))
    });
    for pattern in patterns {
        hasher.update(&[pattern.kind as u8]);
        hash_builder_catalog_text(&mut hasher, &pattern.pattern);
    }

    let mut source_schema_types = builder.source_schema_types.iter().collect::<Vec<_>>();
    source_schema_types.sort();
    for source_schema_type in source_schema_types {
        hash_builder_catalog_text(&mut hasher, source_schema_type);
    }
    Digest::from(hasher.finalize())
}

fn worker_builder_catalog_descriptors(
    catalog: &AssetBuilderCatalogResult,
) -> Vec<BuilderDescriptor> {
    let mut builders = catalog.builders.iter().collect::<Vec<_>>();
    builders.sort_by_key(|builder| builder.builder_guid);
    builders
        .into_iter()
        .map(|builder| BuilderDescriptor {
            guid: builder.builder_guid,
            name: builder.name.clone(),
            version: i64::from(builder.version),
            digest: asset_builder_descriptor_digest(builder),
        })
        .collect()
}

fn worker_builder_catalog_digest(catalog: &AssetBuilderCatalogResult) -> Digest {
    let descriptors = worker_builder_catalog_descriptors(catalog);
    let mut hasher = blake3::Hasher::new();
    hash_builder_catalog_text(&mut hasher, ASSET_BUILDER_CATALOG_DIGEST_DOMAIN);
    hasher.update(&(descriptors.len() as u64).to_le_bytes());
    for descriptor in descriptors {
        hasher.update(descriptor.guid.as_bytes());
        hasher.update(descriptor.digest.as_bytes());
    }
    Digest::from(hasher.finalize())
}

fn source_asset_classifiers_from_catalog(
    catalog: &AssetBuilderCatalogResult,
) -> SourceAssetClassifiers {
    let mut project_documents = Vec::new();
    let mut file_sources = Vec::new();

    for schema in &catalog.source_schemas {
        match &schema.authoring {
            SourceSchemaAuthoring::ProjectDocument { .. } => {
                let source_patterns =
                    catalog_source_patterns_for_schema(catalog, &schema.schema_type);
                if source_patterns.is_empty() {
                    continue;
                }
                project_documents.push(ProjectDocumentSourceClassifier {
                    source_schema_type: schema.schema_type.clone(),
                    source_patterns,
                });
            }
            SourceSchemaAuthoring::File { workflow } => {
                file_sources.push(FileSourceClassifier {
                    source_schema_type: schema.schema_type.clone(),
                    source_root: workflow.source_root.clone(),
                    default_path_prefix: workflow.default_path_prefix.clone(),
                    source_patterns: catalog_source_patterns_for_schema(
                        catalog,
                        &schema.schema_type,
                    ),
                    extensions: workflow.extensions.clone(),
                });
            }
        }
    }

    SourceAssetClassifiers {
        project_documents,
        file_sources,
        builder_claims: catalog
            .builders
            .iter()
            .map(|builder| SourceBuilderClassifier {
                source_schema_types: builder.source_schema_types.clone(),
                source_patterns: catalog_patterns(&builder.patterns),
            })
            .collect(),
    }
}

fn reconcile_registered_source_root_assets(
    pass: ReconcilePass<'_>,
    source_root: &RegisteredSourceRoot,
    source_root_index: usize,
    source_root_count: usize,
    scope: &SweepScope,
) -> Result<RegisteredSourceAssetsReconcileSummary, AssetProcessorError> {
    source_reconcile_activity_source_root_started(
        source_root,
        source_root_index,
        source_root_count,
    );
    let entries = pass.db.ordered_entries(source_root.workspace_root_pk)?;
    let provenance = pass
        .changed_by_session
        .map_or(sweep::SweepProvenance::Startup, |session| {
            sweep::SweepProvenance::Explicit {
                session: session.to_owned(),
            }
        });
    let effect = sweep::execute_sweep(
        pass.writer,
        source_root,
        entries,
        pass.classifiers,
        scope,
        &provenance,
        pass.now_unix_ms,
    )?;
    source_reconcile_activity_recorded(effect.summary.recorded);
    Ok(effect.summary)
}

fn source_candidate_facts_match(
    existing: Option<&ExistingSourceFacts>,
    candidate: &SourceRootScanCandidate,
) -> bool {
    existing.is_some_and(|entry| {
        entry.src_bytes == candidate.observation.source_file_byte_length
            && entry.src_mtime == candidate.observation.source_file_modified_unix_ns
            && entry.meta_bytes == candidate.observation.source_meta_byte_length
            && entry.meta_mtime == candidate.observation.source_meta_modified_unix_ns
            && (candidate.has_project_document_candidates
                || candidate.file_source_schema.as_deref() == entry.schema.as_deref())
    })
}

struct ExistingSourceFacts {
    digest: Digest,
    schema: Option<String>,
    src_bytes: i64,
    src_mtime: i64,
    meta_bytes: i64,
    meta_mtime: i64,
}

impl From<&WorkspaceEntrySnapshot> for ExistingSourceFacts {
    fn from(entry: &WorkspaceEntrySnapshot) -> Self {
        Self {
            digest: entry.digest,
            schema: entry.schema.clone(),
            src_bytes: entry.src_bytes,
            src_mtime: entry.src_mtime,
            meta_bytes: entry.meta_bytes,
            meta_mtime: entry.meta_mtime,
        }
    }
}

impl From<SelectEntries> for ExistingSourceFacts {
    fn from(entry: SelectEntries) -> Self {
        Self {
            digest: entry.digest,
            schema: entry.schema,
            src_bytes: entry.src_bytes,
            src_mtime: entry.src_mtime,
            meta_bytes: entry.meta_bytes,
            meta_mtime: entry.meta_mtime,
        }
    }
}
#[derive(Debug)]
struct SourceRootDirFrame {
    path: PathBuf,
    entries: fs::ReadDir,
}

#[derive(Debug)]
struct SourceRootScanInputs {
    source_root: RegisteredSourceRoot,
    source_root_path: PathBuf,
    classifiers: SourceAssetClassifiers,
    observed_unix_ms: i64,
    excluded_paths: BTreeSet<String>,
    stack: Vec<SourceScanDirFrame>,
    progress: SourceRootScanProgress,
}

impl SourceRootScanInputs {
    fn open(
        source_root: RegisteredSourceRoot,
        source_root_path: PathBuf,
        classifiers: SourceAssetClassifiers,
        observed_unix_ms: i64,
    ) -> Result<Option<Self>, AssetProcessorError> {
        Self::open_at(
            source_root,
            source_root_path.clone(),
            source_root_path,
            classifiers,
            observed_unix_ms,
        )
    }

    fn open_at(
        source_root: RegisteredSourceRoot,
        source_root_path: PathBuf,
        scan_path: PathBuf,
        classifiers: SourceAssetClassifiers,
        observed_unix_ms: i64,
    ) -> Result<Option<Self>, AssetProcessorError> {
        let frame = match SourceScanDirFrame::read(&scan_path) {
            Ok(frame) => frame,
            Err(source)
                if !source_root.role.is_required() && source.kind() == ErrorKind::NotFound =>
            {
                warn!(
                    owner_id = %source_root.owner,
                    display_name = %source_root.display_name,
                    portable_key = %source_root.portable_key,
                    source_root = %scan_path.display(),
                    "asset source root directory is missing; registering as an empty source root"
                );
                return Ok(None);
            }
            Err(source) => {
                return Err(AssetProcessorError::SourceRootReconcileDir {
                    path: scan_path,
                    source,
                });
            }
        };
        Ok(Some(Self {
            excluded_paths: source_root.exclusions.as_set().clone(),
            source_root,
            source_root_path,
            classifiers,
            observed_unix_ms,
            stack: vec![frame],
            progress: SourceRootScanProgress::default(),
        }))
    }
}

impl Iterator for SourceRootScanInputs {
    type Item = Result<SourceRootScanCandidate, AssetProcessorError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let frame = self.stack.last_mut()?;
            let Some(entry) = frame.entries.next() else {
                self.stack.pop();
                continue;
            };
            let entry_path = entry.path;
            let file_type = entry.file_type;
            let metadata = entry.metadata;
            self.progress.record_entry(&self.source_root, &entry_path);
            if file_type.is_dir() {
                if !self.source_root.recursive {
                    continue;
                }
                match SourceScanDirFrame::read(&entry_path) {
                    Ok(frame) => self.stack.push(frame),
                    Err(source) => {
                        return Some(Err(AssetProcessorError::SourceRootReconcileDir {
                            path: entry_path,
                            source,
                        }));
                    }
                }
                continue;
            }
            if !file_type.is_file() || is_asset_root_scaffold_marker(&entry_path) {
                continue;
            }

            let source_path =
                match source_root_relative_asset_path(&self.source_root_path, &entry_path) {
                    Ok(Some(source_path)) => source_path,
                    Ok(None) => continue,
                    Err(error) => return Some(Err(error)),
                };
            if self.excluded_paths.contains(&source_path) {
                continue;
            }
            // The source-metadata sidecar carries a source's identity out of
            // band; it is not itself a source asset.
            if source_path.ends_with(source_meta::SOURCE_META_SIDECAR_SUFFIX) {
                continue;
            }
            let file_source_schema =
                classify_file_source_asset(&self.source_root, &source_path, &self.classifiers);
            let has_project_document_candidates = project_document_source_path_has_candidates(
                &self.source_root,
                &source_path,
                &self.classifiers,
            );
            if file_source_schema.is_none() && !has_project_document_candidates {
                continue;
            }
            let sidecar_path = source_meta::source_meta_sidecar_path(&entry_path);
            let sidecar = frame.sidecars.get(&sidecar_path).copied();
            let observation =
                source_asset_observation_from_evidence(&metadata, sidecar, self.observed_unix_ms);
            return Some(Ok(SourceRootScanCandidate {
                entry_path,
                source_path,
                file_source_schema,
                has_project_document_candidates,
                observation,
            }));
        }
    }
}

#[derive(Debug)]
struct SourceScanDirEntry {
    path: PathBuf,
    file_type: fs::FileType,
    metadata: fs::Metadata,
}

#[derive(Debug)]
struct SourceScanDirFrame {
    entries: std::vec::IntoIter<SourceScanDirEntry>,
    sidecars: BTreeMap<PathBuf, (i64, i64)>,
}

impl SourceScanDirFrame {
    fn read(path: &Path) -> Result<Self, std::io::Error> {
        let mut entries = Vec::new();
        let mut sidecars = BTreeMap::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let file_type = entry.file_type()?;
            let metadata = entry.metadata()?;
            if file_type.is_file()
                && entry_path
                    .to_string_lossy()
                    .ends_with(source_meta::SOURCE_META_SIDECAR_SUFFIX)
            {
                sidecars.insert(
                    entry_path.clone(),
                    (
                        metadata_len_i64(&metadata),
                        metadata_modified_unix_ns(&metadata),
                    ),
                );
            }
            entries.push(SourceScanDirEntry {
                path: entry_path,
                file_type,
                metadata,
            });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self {
            entries: entries.into_iter(),
            sidecars,
        })
    }
}

fn metadata_len_i64(metadata: &fs::Metadata) -> i64 {
    i64::try_from(metadata.len()).unwrap_or(i64::MAX)
}

fn system_time_unix_ns(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_nanos()).ok())
        .unwrap_or(0)
}

fn metadata_modified_unix_ns(metadata: &fs::Metadata) -> i64 {
    metadata.modified().map_or(0, system_time_unix_ns)
}

fn source_asset_observation(
    entry_path: &Path,
    metadata: &fs::Metadata,
    observed_unix_ms: i64,
) -> Result<SourceFileObservation, AssetProcessorError> {
    let sidecar_path = source_meta::source_meta_sidecar_path(entry_path);
    let (source_meta_byte_length, source_meta_modified_unix_ns) = match fs::metadata(&sidecar_path)
    {
        Ok(metadata) => (
            metadata_len_i64(&metadata),
            metadata_modified_unix_ns(&metadata),
        ),
        Err(source) if source.kind() == ErrorKind::NotFound => (0, 0),
        Err(source) => {
            return Err(AssetProcessorError::SourceRootReconcileEntry {
                path: sidecar_path,
                source,
            });
        }
    };
    Ok(SourceFileObservation {
        source_file_byte_length: metadata_len_i64(metadata),
        source_file_modified_unix_ns: metadata_modified_unix_ns(metadata),
        source_meta_byte_length,
        source_meta_modified_unix_ns,
        last_observed_unix_ms: observed_unix_ms,
    })
}

fn source_asset_observation_from_evidence(
    metadata: &fs::Metadata,
    sidecar: Option<(i64, i64)>,
    observed_unix_ms: i64,
) -> SourceFileObservation {
    let (source_meta_byte_length, source_meta_modified_unix_ns) = sidecar.unwrap_or((0, 0));
    SourceFileObservation {
        source_file_byte_length: metadata_len_i64(metadata),
        source_file_modified_unix_ns: metadata_modified_unix_ns(metadata),
        source_meta_byte_length,
        source_meta_modified_unix_ns,
        last_observed_unix_ms: observed_unix_ms,
    }
}

fn source_meta_error_reason(error: SourceMetaError) -> String {
    match error {
        SourceMetaError::Read(source) => format!("read failed: {source}"),
        SourceMetaError::Parse(reason) => reason,
        error @ SourceMetaError::UnsafeSourcePath { .. } => error.to_string(),
    }
}

fn source_root_scan_candidate_to_record(
    source_root: &RegisteredSourceRoot,
    classifiers: &SourceAssetClassifiers,
    candidate: SourceRootScanCandidate,
    now_unix_ms: i64,
    cancel: &CancellationToken,
) -> Result<Option<PendingSourceAssetRecord>, AssetProcessorError> {
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let (schema_type, content_hash) = if candidate.has_project_document_candidates {
        let bytes = fs::read(&candidate.entry_path).map_err(|source| {
            AssetProcessorError::SourceRootReconcileFile {
                path: candidate.entry_path.clone(),
                source,
            }
        })?;
        let Some(schema_type) = classify_project_document_source_asset(
            source_root,
            &candidate.source_path,
            classifiers,
        )
        .or(candidate.file_source_schema) else {
            return Ok(None);
        };
        (schema_type, Digest::from(blake3::hash(&bytes)))
    } else {
        let Some(schema_type) = candidate.file_source_schema else {
            return Ok(None);
        };
        (
            schema_type,
            hash_source_file_streaming(&candidate.entry_path)?,
        )
    };
    let changed_unix_ms = candidate
        .observation
        .source_file_modified_unix_ns
        .checked_div(1_000_000)
        .filter(|value| *value > 0)
        .unwrap_or(now_unix_ms);
    let meta = source_meta::read_source_asset_meta(&candidate.entry_path).map_err(|error| {
        let reason = source_meta_error_reason(error);
        AssetProcessorError::SourceMetaSidecar {
            path: source_meta::source_meta_sidecar_path(&candidate.entry_path),
            reason,
        }
    })?;
    let asset_guid = meta
        .as_ref()
        .and_then(source_meta::SourceAssetMeta::preserved_guid)
        .unwrap_or_else(|| source_meta::resolve_source_asset_guid(&candidate.source_path, None));
    Ok(Some(PendingSourceAssetRecord {
        source_path: candidate.source_path,
        schema_type,
        content_hash,
        changed_unix_ms,
        diagnostics_count: 0,
        observation: candidate.observation,
        asset_guid,
    }))
}

/// How much of a source file is read per hashing round.
const SOURCE_HASH_BUFFER_BYTES: usize = 128 * 1024;

/// How much of a staged product is read per hashing round.
const STAGED_PRODUCT_HASH_BUFFER_BYTES: usize = 64 * 1024;

fn hash_source_file_streaming(path: &Path) -> Result<Digest, AssetProcessorError> {
    let mut file =
        fs::File::open(path).map_err(|source| AssetProcessorError::SourceRootReconcileFile {
            path: path.to_path_buf(),
            source,
        })?;
    let mut hasher = blake3::Hasher::new();
    // Heap, not stack: a 128 KiB frame is far more than a recursive directory
    // scan can afford to hold, and the buffer never outlives this call.
    let mut buffer = vec![0u8; SOURCE_HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).map_err(|source| {
            AssetProcessorError::SourceRootReconcileFile {
                path: path.to_path_buf(),
                source,
            }
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Digest::from(hasher.finalize()))
}

#[derive(Debug, Default)]
struct SourceRootScanProgress {
    /// Entries seen so far in this root.
    visited: usize,
    /// The `visited` value at which the next activity update fires.
    next_activity_at: usize,
    /// The `visited` value at which the next log line fires.
    next_log_at: usize,
}

impl SourceRootScanProgress {
    fn record_entry(&mut self, source_root: &RegisteredSourceRoot, path: &Path) {
        self.visited += 1;
        if self.next_activity_at == 0 {
            self.next_activity_at = 1;
        }
        if self.visited >= self.next_activity_at {
            source_reconcile_activity_scan_progress(source_root, path, self.visited);
            self.next_activity_at = self
                .visited
                .saturating_add(SOURCE_ASSET_SCAN_ACTIVITY_INTERVAL);
        }
        if self.next_log_at == 0 {
            self.next_log_at = SOURCE_ASSET_SCAN_LOG_INTERVAL;
        }
        if self.visited < self.next_log_at {
            return;
        }
        info!(
            workspace_id = source_root.workspace_pk,
            workspace_source_root_id = source_root.workspace_root_pk,
            scan_folder_id = source_root.root_pk,
            portable_key = %source_root.portable_key,
            source_root = %source_root.path,
            visited_entry_count = self.visited,
            current_path = %path.display(),
            "asset source root startup reconcile scan progress"
        );
        self.next_log_at = self.visited.saturating_add(SOURCE_ASSET_SCAN_LOG_INTERVAL);
    }
}

fn source_root_relative_asset_path(
    source_root: &Path,
    path: &Path,
) -> Result<Option<String>, AssetProcessorError> {
    let Some(relative) = path.strip_prefix(source_root).ok() else {
        return Ok(None);
    };
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                let Some(part) = part.to_str() else {
                    return Err(AssetProcessorError::InvalidNativeSourcePath {
                        path: path.to_path_buf(),
                        reason: "path is not valid Unicode".to_string(),
                    });
                };
                parts.push(part);
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => return Ok(None),
        }
    }
    NativeAssetPath::new(&parts.join("/"))
        .map(|path| Some(path.as_str().to_string()))
        .map_err(|error| AssetProcessorError::InvalidNativeSourcePath {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })
}

fn classify_project_document_source_asset(
    source_root: &RegisteredSourceRoot,
    source_path: &str,
    classifiers: &SourceAssetClassifiers,
) -> Option<String> {
    let mut candidates = classifiers
        .project_documents
        .iter()
        .filter(|_| source_root_matches_selector(source_root, PROJECT_SOURCE_ROOT))
        .filter(|classifier| {
            classifier
                .source_patterns
                .iter()
                .any(|pattern| pattern.matches(source_path))
        });
    let first = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    Some(first.source_schema_type.clone())
}

fn project_document_source_path_has_candidates(
    source_root: &RegisteredSourceRoot,
    source_path: &str,
    classifiers: &SourceAssetClassifiers,
) -> bool {
    classifiers.project_documents.iter().any(|classifier| {
        source_root_matches_selector(source_root, PROJECT_SOURCE_ROOT)
            && classifier
                .source_patterns
                .iter()
                .any(|pattern| pattern.matches(source_path))
    })
}

fn classify_file_source_asset(
    source_root: &RegisteredSourceRoot,
    source_path: &str,
    classifiers: &SourceAssetClassifiers,
) -> Option<String> {
    let candidates = classifiers
        .file_sources
        .iter()
        .filter(|classifier| source_root_matches_selector(source_root, &classifier.source_root))
        .filter(|classifier| {
            classifier.source_patterns.is_empty()
                || classifier
                    .source_patterns
                    .iter()
                    .any(|pattern| pattern.matches(source_path))
        })
        .filter_map(|classifier| {
            matching_source_extension_specificity(source_path, &classifier.extensions)
                .map(|extension_specificity| (classifier, extension_specificity))
        })
        .collect::<Vec<_>>();
    if candidates.len() == 1 {
        return Some(candidates[0].0.source_schema_type.clone());
    }

    // Prefer a concrete extension over a generic catch-all and a longer
    // compound extension over a shorter suffix. Compound extensions such as
    // `settings.ron` are intentionally reusable across gems, so use the
    // authored workflow's placement prefix as the secondary discriminator.
    // True ties remain ambiguous instead of depending on inventory order.
    let mut best: Option<(&FileSourceClassifier, (usize, bool, usize))> = None;
    let mut best_is_ambiguous = false;
    for (candidate, extension_specificity) in candidates {
        let prefix = candidate.default_path_prefix.trim_matches('/');
        let prefix_matches = source_path_matches_default_prefix(source_path, prefix);
        let specificity = (
            extension_specificity,
            prefix_matches,
            if prefix_matches { prefix.len() } else { 0 },
        );
        match best {
            Some((_, best_specificity)) if specificity < best_specificity => {}
            Some((_, best_specificity)) if specificity == best_specificity => {
                best_is_ambiguous = true;
            }
            _ => {
                best = Some((candidate, specificity));
                best_is_ambiguous = false;
            }
        }
    }

    (!best_is_ambiguous)
        .then(|| best.map(|(candidate, _)| candidate.source_schema_type.clone()))
        .flatten()
}

fn source_path_matches_default_prefix(source_path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || source_path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
}

fn source_root_matches_selector(source_root: &RegisteredSourceRoot, selector: &str) -> bool {
    if selector == PROJECT_SOURCE_ROOT {
        return source_root.role.is_required()
            && source_root.output_prefix.is_empty()
            && source_root.portable_key
                == PortableKey::project_assets(&source_root.owner).as_str();
    }

    source_root.portable_key == selector
}

/// Construction-owned database handles, split by one long-lived responsibility.
///
/// Query work stays on the RPC `LocalSet`, dispatch work stays on the dispatcher,
/// and catalog publication transfers its handle to the catalog owner thread.
/// No request path opens or replaces a database handle.
pub(crate) struct AssetProcessorDatabaseHandles {
    query: RefCell<AssetDb>,
    dispatch: Rc<AssetDb>,
    catalog: RefCell<Option<AssetDb>>,
}

impl AssetProcessorDatabaseHandles {
    pub(crate) fn open(bootstrap: AssetDb) -> Result<Self, OpenError> {
        let query = bootstrap.new_runtime_handle()?;
        let dispatch = Rc::new(bootstrap.new_runtime_handle()?);
        let catalog = bootstrap.new_runtime_handle()?;
        // The three runtime handles are the only ones the processor keeps. The
        // bootstrap handle has done its job by now, so release it here rather
        // than leaving a fourth connection open for the caller to forget.
        drop(bootstrap);
        Ok(Self {
            query: RefCell::new(query),
            dispatch,
            catalog: RefCell::new(Some(catalog)),
        })
    }

    fn query(&self) -> Ref<'_, AssetDb> {
        self.query.borrow()
    }

    fn query_mut(&self) -> RefMut<'_, AssetDb> {
        self.query.borrow_mut()
    }

    fn dispatch(&self) -> Rc<AssetDb> {
        Rc::clone(&self.dispatch)
    }

    fn take_catalog(&self) -> AssetDb {
        self.catalog
            .borrow_mut()
            .take()
            .expect("catalog database handle is consumed exactly once by its owner")
    }
}

pub struct AssetProcessor {
    databases: AssetProcessorDatabaseHandles,
    /// Durable project-instance workspace selected once during service
    /// startup. Session capabilities authorize calls but never select this
    /// database scope.
    workspace_id: Option<i64>,
    /// Manifest-owned runtime policy for the registered Roots. `AssetDB` owns
    /// durable root identity and exclusions; watcher behavior is not rebuilt
    /// from storage rows after startup.
    source_roots: Vec<RegisteredSourceRoot>,
    /// Project-instance filesystem authority injected by the service owner.
    /// Database rows identify the workspace; they never resolve or create a
    /// machine data home on their own.
    project_data_paths: Option<ProjectDataPaths>,
    /// Optional in-process builders used only by tests/harnesses that still
    /// construct a registry. Production engine host leaves this empty and
    /// dispatches `create_jobs/process_job` exclusively through project workers.
    builders: BuildRuleRegistry,
    /// The composition this processor resolves source schemas, product
    /// formats and in-process build rules against.
    ///
    /// The production engine host composes almost nothing here — it serves the
    /// catalog a project worker published. This is what the fallback path and
    /// the in-process harnesses read.
    registries: &'static Registries,
    /// Catalog published by a connected project asset-worker.
    published_catalog: RefCell<Option<AssetBuilderCatalogResult>>,
    /// Explicit force-reprocess requests serviced ahead of authored/schema
    /// lanes until the requested identity's active job chain drains.
    prioritized_asset_identities: RefCell<BTreeSet<i64>>,
    /// Composition-owned writer for every currently migrated live job write.
    /// The server and watcher hold clones of this exact handle; no runtime
    /// path creates a second writer connection.
    asset_db_writer: AssetDbWriter,
    /// One process-local owner for every runtime catalog projection this
    /// processor serves. RPC connections share the processor, never a
    /// per-connection freshness cache.
    catalog_publisher_owner: RefCell<Option<CatalogPublisherOwner>>,
    /// Sticky process consequence health shared by the dispatcher and catalog
    /// owner. The catalog owner runs on its own thread, so this surface is
    /// synchronized rather than tied to the RPC `LocalSet`.
    consequence_health: AssetProcessorConsequenceHealth,
    /// Phase timing for completions: a=manifest load, b=product promotion,
    /// c=durable group submit.
    complete_rpc_stats: WorkerRpcPhaseStats,
    /// Holds one product-cache preimage from promotion until the durable
    /// completion either commits and finalizes it or fails and compensates it.
    product_promotion_gate: Rc<tokio::sync::Mutex<()>>,
    capability_grants: CapabilityGrantSet,
}

impl fmt::Debug for AssetProcessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetProcessor").finish_non_exhaustive()
    }
}

/// Compose the engine's own contributions into a host's composer.
///
/// First, unconditionally, ahead of any lock closure: the engine floor is not
/// selectable, so every bundle takes [`ProductActivation::default`] and no
/// `enabled` switch, and whatever the host adds afterwards composes behind it
/// (asset-contract ticket 014, D6). Role is the only filter — a bundle that
/// does not name the composer's role is not this host's.
pub fn compose_engine(composer: &mut Composer) {
    engine(composer, az_engine_types::types_contribution());
    engine(composer, az_engine_assets::assets_contribution());
    engine(composer, az_engine_builders::builders_contribution());
}

/// One engine bundle, if this host's role names it.
///
/// The `expect` is not defensive: the engine floor declares no host-capability
/// floor at all, so the containment test it would fail cannot fail. A bundle
/// that grows one and a role that cannot provide it is a manifest error, and
/// this is where it surfaces.
fn engine<C: az_gem_contract::Contribution>(composer: &mut Composer, contribution: C) {
    if contribution
        .descriptor()
        .applies_to_role(composer.host().role())
    {
        composer
            .add(contribution, ProductActivation::default())
            .expect("the engine floor declares no host-capability floor");
    }
}

/// The engine-owned host's own composition.
///
/// This host links no *project or gem* code — that is still the whole point of
/// the worker split, and it answers `builderCatalog` from what a connected
/// project worker published. But engine built-ins are neither project nor gem
/// code, and the host has to classify engine-owned source families at startup,
/// before any worker connects: source-root registration resolves its
/// classifiers from a composition, and an empty one fails
/// [`AssetProcessorError::BuilderCatalogUnavailable`] (asset-contract ticket
/// 014, D7).
///
/// Composed once for the life of the process, because the borrow is
/// `'static`: the composer owns the contributions for lifecycle and reload, so
/// it outlives the registries it lent out.
///
/// # Panics
///
/// Panics if the engine composition does not finalize. The engine contributions
/// are compiled in, so a composition failure is a build-time defect in this
/// crate rather than anything a caller can cause or recover from.
#[must_use]
pub fn engine_host_registries() -> &'static Registries {
    static REGISTRIES: OnceLock<&'static Registries> = OnceLock::new();
    REGISTRIES.get_or_init(|| {
        let mut composer = Composer::new(GemTargetRole::AssetProcessor);
        compose_engine(&mut composer);
        composer
            .finalize()
            .expect("the engine composition is valid");
        Box::leak(Box::new(composer)).registries()
    })
}

#[cfg(test)]
fn block_on_test_runtime<F: Future>(future: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("asset-processor test runtime");
    tokio::task::LocalSet::new().block_on(&runtime, future)
}

impl AssetProcessor {
    fn sweep_root_for_selector(&self, selector: &str) -> Result<SweepRoot, AssetProcessorError> {
        self.source_roots
            .iter()
            .find(|root| source_root_matches_selector(root, selector))
            .map(SweepRoot::registered)
            .ok_or_else(|| AssetProcessorError::UnknownSourceRootSelector {
                selector: selector.to_owned(),
            })
    }

    /// Production host constructor: composes nothing of its own.
    #[must_use = "database-handle construction errors must be handled"]
    pub(crate) fn new(
        db: AssetDb,
        workspace_id: i64,
        project_data_paths: ProjectDataPaths,
        capability_grants: CapabilityGrantSet,
    ) -> Result<Self, OpenError> {
        let asset_db_writer = db
            .writer()
            .expect("asset processor requires an asset database writer");
        Self::new_with_asset_db_writer(
            db,
            workspace_id,
            project_data_paths,
            capability_grants,
            asset_db_writer,
        )
    }

    pub(crate) fn new_with_asset_db_writer(
        db: AssetDb,
        workspace_id: i64,
        project_data_paths: ProjectDataPaths,
        capability_grants: CapabilityGrantSet,
        asset_db_writer: AssetDbWriter,
    ) -> Result<Self, OpenError> {
        let registries = engine_host_registries();
        let databases = AssetProcessorDatabaseHandles::open(db)?;
        Ok(Self::with_database_handles_and_asset_db_writer(
            databases,
            BuildRuleRegistry::new(),
            capability_grants,
            registries,
            Some(workspace_id),
            Some(project_data_paths),
            None,
            asset_db_writer,
        ))
    }

    /// Harness constructor: an in-process processor that dispatches the build
    /// rules the given composition contributed.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_composed_builders(
        db: AssetDb,
        capability_grants: CapabilityGrantSet,
        registries: &'static Registries,
    ) -> Self {
        let builders = BuildRuleRegistry::compose(&BuilderJobContext::new(registries));
        Self::with_builder_registry(db, builders, capability_grants, registries)
    }

    #[must_use]
    pub(crate) fn with_builder_registry(
        db: AssetDb,
        builders: BuildRuleRegistry,
        capability_grants: CapabilityGrantSet,
        registries: &'static Registries,
    ) -> Self {
        Self::with_builder_registry_and_catalog(
            db,
            builders,
            capability_grants,
            registries,
            None,
            None,
            None,
        )
    }

    fn with_builder_registry_and_catalog(
        db: AssetDb,
        builders: BuildRuleRegistry,
        capability_grants: CapabilityGrantSet,
        registries: &'static Registries,
        workspace_id: Option<i64>,
        project_data_paths: Option<ProjectDataPaths>,
        published_catalog: Option<AssetBuilderCatalogResult>,
    ) -> Self {
        let asset_db_writer = db
            .writer()
            .expect("asset processor requires an asset database writer");
        Self::with_builder_registry_and_catalog_with_asset_db_writer(
            db,
            builders,
            capability_grants,
            registries,
            workspace_id,
            project_data_paths,
            published_catalog,
            asset_db_writer,
        )
    }

    // Middle link of the `with_*` constructor chain: it opens the database
    // handles and forwards everything else untouched. Its arguments are
    // `AssetProcessor`'s own independent fields, so a parameter struct would
    // restate that struct minus the parts each constructor derives, and all
    // three overloads would then translate into and out of it.
    #[expect(
        clippy::too_many_arguments,
        reason = "forwards AssetProcessor's own independent fields down the with_* constructor chain"
    )]
    fn with_builder_registry_and_catalog_with_asset_db_writer(
        db: AssetDb,
        builders: BuildRuleRegistry,
        capability_grants: CapabilityGrantSet,
        registries: &'static Registries,
        workspace_id: Option<i64>,
        project_data_paths: Option<ProjectDataPaths>,
        published_catalog: Option<AssetBuilderCatalogResult>,
        asset_db_writer: AssetDbWriter,
    ) -> Self {
        let databases = AssetProcessorDatabaseHandles::open(db)
            .expect("asset processor test database handles open");
        Self::with_database_handles_and_asset_db_writer(
            databases,
            builders,
            capability_grants,
            registries,
            workspace_id,
            project_data_paths,
            published_catalog,
            asset_db_writer,
        )
    }

    // The canonical constructor every other `with_*` form funnels into. These
    // eight are exactly the fields a caller gets to choose; the remaining
    // `AssetProcessor` fields are defaulted below. A parameter struct here
    // would exist only to be unpacked one line later into the struct it
    // mirrors field for field.
    #[expect(
        clippy::too_many_arguments,
        reason = "canonical constructor: each argument is a caller-chosen field of the AssetProcessor it returns"
    )]
    pub(crate) fn with_database_handles_and_asset_db_writer(
        databases: AssetProcessorDatabaseHandles,
        builders: BuildRuleRegistry,
        capability_grants: CapabilityGrantSet,
        registries: &'static Registries,
        workspace_id: Option<i64>,
        project_data_paths: Option<ProjectDataPaths>,
        published_catalog: Option<AssetBuilderCatalogResult>,
        asset_db_writer: AssetDbWriter,
    ) -> Self {
        Self {
            databases,
            workspace_id,
            source_roots: Vec::new(),
            project_data_paths,
            builders,
            registries,
            published_catalog: RefCell::new(published_catalog),
            prioritized_asset_identities: RefCell::new(BTreeSet::new()),
            asset_db_writer,
            catalog_publisher_owner: RefCell::new(None),
            consequence_health: AssetProcessorConsequenceHealth::default(),
            complete_rpc_stats: WorkerRpcPhaseStats::default(),
            product_promotion_gate: Rc::new(tokio::sync::Mutex::new(())),
            capability_grants,
        }
    }

    fn attached_workspace_id(&self) -> Result<i64, AssetProcessorError> {
        self.workspace_id
            .ok_or(AssetProcessorError::MissingAttachedWorkspace)
    }

    fn with_source_roots(mut self, source_roots: Vec<RegisteredSourceRoot>) -> Self {
        self.source_roots = source_roots;
        self
    }

    fn project_data_paths(&self) -> Result<&ProjectDataPaths, AssetProcessorError> {
        self.project_data_paths
            .as_ref()
            .ok_or(AssetProcessorError::MissingProjectDataPaths)
    }

    const fn asset_db_writer(&self) -> &AssetDbWriter {
        &self.asset_db_writer
    }

    fn catalog_publisher(&self) -> Result<CatalogPublisher, AssetProcessorError> {
        // Bind first so the `RefCell` guard is released at the end of this
        // statement instead of living across the whole `if let`.
        let existing = self
            .catalog_publisher_owner
            .borrow()
            .as_ref()
            .map(CatalogPublisherOwner::publisher);
        if let Some(publisher) = existing {
            return Ok(publisher);
        }

        let workspace_id = self.attached_workspace_id()?;
        let workspace = self
            .databases
            .query()
            .workspace_by_id(workspace_id)?
            .ok_or(AssetProcessorError::MissingAttachedWorkspace)?;
        let scope = CatalogScope::validated(&workspace, self.project_data_paths()?.clone())?;
        let owner = CatalogPublisherOwner::start(
            self.databases.take_catalog(),
            &self.asset_db_writer,
            scope,
            self.published_catalog.borrow().clone(),
            self.consequence_health.clone(),
        )?;
        let publisher = owner.publisher();
        *self.catalog_publisher_owner.borrow_mut() = Some(owner);
        Ok(publisher)
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn shutdown_catalog_publisher(&self) -> Result<(), AssetProcessorError> {
        let Some(owner) = self.catalog_publisher_owner.borrow_mut().take() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || owner.shutdown())
            .await
            .map_err(|error| AssetProcessorError::AssetCatalogWorker { error })?
    }

    fn dispatch_db(&self) -> Rc<AssetDb> {
        self.databases.dispatch()
    }

    #[must_use]
    pub const fn capability_grants(&self) -> &CapabilityGrantSet {
        &self.capability_grants
    }

    #[cfg(test)]
    #[must_use]
    pub fn db(&self) -> Ref<'_, AssetDb> {
        self.databases.query()
    }

    #[cfg(test)]
    #[must_use]
    pub const fn builders(&self) -> &BuildRuleRegistry {
        &self.builders
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the capability is not a
    /// valid jobs capability, [`AssetProcessorError::ProtocolVersionMismatch`] if the
    /// request's protocol version is not current,
    /// [`AssetProcessorError::InvalidBuilderCatalog`] if the catalog repeats a source
    /// schema or leaves a builder's source schema uncovered,
    /// [`AssetProcessorError::BuilderCatalogSnapshotConflict`] if the stored catalog
    /// changed before this replacement committed, and [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[instrument(skip(self, request))]
    pub async fn publish_builder_catalog(
        &self,
        request: &PublishBuilderCatalogRequest,
    ) -> Result<PublishBuilderCatalogResult, AssetProcessorError> {
        validate_jobs_capability(&request.capability, self.capability_grants())?;
        request
            .protocol
            .require(ProtocolVersion::CURRENT)
            .map_err(AssetProcessorError::ProtocolVersionMismatch)?;
        ensure_unique_source_schema_descriptors(&request.catalog.source_schemas)?;
        ensure_builder_catalog_source_schema_coverage(
            &request.catalog.builders,
            &request.catalog.source_schemas,
        )?;
        let (changed_builder_count, invalidated_source_count, enqueued_job_count) = self
            .invalidate_changed_builder_catalog_sources(request)
            .await?;
        *self.published_catalog.borrow_mut() = Some(request.catalog.clone());
        {
            // Bind first so the `RefCell` guard is released at the end of this
            // statement instead of living across the whole `if let`.
            let existing = self
                .catalog_publisher_owner
                .borrow()
                .as_ref()
                .map(CatalogPublisherOwner::publisher);
            if let Some(publisher) = existing {
                publisher.replace_builder_catalog(Some(request.catalog.clone()));
            }
        }
        info!(
            builder_count = request.catalog.builders.len(),
            source_schema_count = request.catalog.source_schemas.len(),
            product_format_count = request.catalog.product_formats.len(),
            changed_builder_count,
            invalidated_source_count,
            enqueued_job_count,
            "asset processor accepted published worker builder catalog"
        );
        // Subsequent reconcile/force-reprocess/watch paths use this catalog for
        // source classification; the engine host has no project inventory of its
        // own. Production source reconciliation remains closed until the worker
        // publishes this catalog.
        Ok(PublishBuilderCatalogResult { accepted: true })
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn invalidate_changed_builder_catalog_sources(
        &self,
        request: &PublishBuilderCatalogRequest,
    ) -> Result<(usize, usize, usize), AssetProcessorError> {
        let workspace_id = self.attached_workspace_id()?;
        let writer = self.asset_db_writer();
        let replacement = worker_builder_catalog_digest(&request.catalog);
        let descriptors = worker_builder_catalog_descriptors(&request.catalog);
        let previous_catalog = self.published_catalog.borrow().clone();
        let mut expected = self
            .databases
            .query()
            .workspace_by_id(workspace_id)?
            .ok_or(AssetProcessorError::MissingAttachedWorkspace)?
            .builders;

        for attempt in 0..=1 {
            let (plan_delta, changed_builder_count, invalidated_source_count) = self
                .prepare_builder_catalog_plan(
                    workspace_id,
                    expected,
                    previous_catalog.as_ref(),
                    &request.catalog,
                    replacement,
                )?;
            let enqueued_job_count = plan_delta.replacements.len();
            let outcome = writer
                .replace_builder_catalog(ReplaceBuilderCatalog {
                    workspace_pk: workspace_id,
                    expected,
                    replacement,
                    builders: descriptors.clone(),
                    plan_delta,
                    updated: current_unix_ms_i64()?,
                })
                .await?;
            match outcome {
                BuilderCatalogReplaceOutcome::Unchanged => return Ok((0, 0, 0)),
                BuilderCatalogReplaceOutcome::Replaced => {
                    return Ok((
                        changed_builder_count,
                        invalidated_source_count,
                        enqueued_job_count,
                    ));
                }
                BuilderCatalogReplaceOutcome::Conflict { actual } if attempt == 0 => {
                    expected = actual;
                }
                BuilderCatalogReplaceOutcome::Conflict { .. } => {
                    return Err(AssetProcessorError::BuilderCatalogSnapshotConflict {
                        workspace_id,
                    });
                }
            }
        }
        unreachable!("builder catalog CAS retry loop has a fixed two-attempt bound")
    }

    fn prepare_builder_catalog_plan(
        &self,
        workspace_pk: i64,
        expected: Option<Digest>,
        previous: Option<&AssetBuilderCatalogResult>,
        replacement: &AssetBuilderCatalogResult,
        replacement_digest: Digest,
    ) -> Result<(PlanDelta, usize, usize), AssetProcessorError> {
        if expected == Some(replacement_digest) {
            return Ok((PlanDelta::default(), 0, 0));
        }
        let BuilderCatalogDiff {
            full_replan,
            changed_guids,
            changed_claims,
        } = builder_catalog_diff(expected, previous, replacement);

        let db = self.databases.query();
        let mut delta = PlanDelta::default();
        let mut retire_job_ids = BTreeSet::new();
        let mut retire_source_edge_ids = BTreeSet::new();
        let mut after_entry_id = 0;
        let mut invalidated_source_count = 0;
        loop {
            let page = db.workspace_entry_page(
                workspace_pk,
                None,
                after_entry_id,
                u32::try_from(SOURCE_ASSET_RECONCILE_MAX_BATCH_RECORDS).unwrap_or(u32::MAX),
            )?;
            let Some(last) = page.last() else {
                break;
            };
            after_entry_id = last.entry_id;
            for entry in page {
                let affected = full_replan
                    || changed_claims
                        .iter()
                        .any(|claim| claim.claims(&entry.source_path, entry.schema.as_deref()));
                for job in db.jobs_for_asset(workspace_pk, entry.asset_pk)? {
                    if (affected && job.kind == DbWork::Plan)
                        || (job.kind == DbWork::Build
                            && (full_replan
                                || job
                                    .builder
                                    .is_some_and(|guid| changed_guids.contains(&guid))))
                    {
                        retire_job_ids.insert(job.job_id);
                    }
                }
                for edge in db.source_edges_for_asset(workspace_pk, entry.asset_pk)? {
                    if full_replan || changed_guids.contains(&edge.builder) {
                        retire_source_edge_ids.insert(edge.source_edge_id);
                    }
                }
                if affected {
                    invalidated_source_count += 1;
                    delta.replacements.push(PlannedJob::plan(
                        entry.asset_pk,
                        ASSET_PLANNER_JOB_KEY,
                        DEFAULT_PLATFORM_ID.as_str(),
                        Vec::new(),
                    ));
                }
            }
        }
        drop(db);
        delta.retire_job_ids = retire_job_ids.into_iter().collect();
        delta.retire_source_edge_ids = retire_source_edge_ids.into_iter().collect();
        let changed_builder_count = if full_replan {
            replacement.builders.len()
        } else {
            changed_guids.len()
        };
        Ok((delta, changed_builder_count, invalidated_source_count))
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation, and
    /// [`AssetProcessorError::BuilderCatalogUnavailable`] if no project asset-worker
    /// has published a catalog yet.
    #[instrument(skip(self, request))]
    pub fn builder_catalog(
        &self,
        request: &AssetBuilderCatalogRequest,
    ) -> Result<AssetBuilderCatalogResult, AssetProcessorError> {
        validate_read_capability(&request.capability, self.capability_grants())?;
        if let Some(catalog) = self.published_catalog.borrow().clone() {
            info!(
                builder_count = catalog.builders.len(),
                source_schema_count = catalog.source_schemas.len(),
                product_format_count = catalog.product_formats.len(),
                "asset processor builder catalog served from published worker catalog"
            );
            return Ok(catalog);
        }

        // Fallback for test/harness processes that still link builders in-process.
        if self.builders.iter().next().is_some() {
            let builders = self
                .builders
                .iter()
                .map(asset_builder_to_proto)
                .collect::<Vec<_>>();
            let source_schema_registrations = composed_source_schemas(self.registries);
            let source_file_template_registrations =
                composed_source_file_templates(self.registries);
            ensure_source_file_template_registrations_match_schemas(
                &source_schema_registrations,
                &source_file_template_registrations,
            )?;
            let mut source_schemas = source_schema_registrations
                .into_iter()
                .map(|attributed| {
                    source_schema_to_proto(&attributed, &source_file_template_registrations)
                })
                .collect::<Result<Vec<_>, _>>()?;
            source_schemas.extend(graph_source_schemas_to_proto(self.registries)?);
            source_schemas.sort_by(|left, right| {
                left.schema_type
                    .cmp(&right.schema_type)
                    .then_with(|| left.owner.cmp(&right.owner))
            });
            ensure_unique_source_schema_descriptors(&source_schemas)?;
            ensure_builder_catalog_source_schema_coverage(&builders, &source_schemas)?;
            return Ok(AssetBuilderCatalogResult {
                builders,
                source_schemas,
                product_formats: composed_product_formats_to_proto(self.registries),
            });
        }

        Err(AssetProcessorError::BuilderCatalogUnavailable)
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached,
    /// [`AssetProcessorError::Repo`] if an `AssetDB` query fails, and
    /// [`AssetProcessorError::InvalidJobInspection`] if the inspected job's rows are
    /// inconsistent.
    #[instrument(skip(self, request))]
    pub fn inspect_job(
        &self,
        request: &InspectJobRequest,
    ) -> Result<InspectJobResult, AssetProcessorError> {
        validate_read_capability(&request.capability, self.capability_grants())?;
        let db = self.databases.query();
        let workspace_id = self.attached_workspace_id()?;
        let selector = match request.selector {
            InspectJobSelector::Job(id) => DbJobInspectionSelector::Job(id),
            InspectJobSelector::Attempt(id) => DbJobInspectionSelector::Attempt(id),
        };
        let inspection = db.inspect_job(workspace_id, selector)?;
        drop(db);
        Ok(InspectJobResult {
            inspection: inspection.map(db_job_inspection_to_proto).transpose()?,
        })
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::InvalidWorkspaceEntryPageRequest`] if the page request
    /// is malformed, [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached, and
    /// [`AssetProcessorError::Repo`] if an `AssetDB` query fails. A workspace that does not exist
    /// yields an empty page rather than an error.
    #[instrument(skip(self, request), fields(page_size = request.page_size))]
    pub fn workspace_entry_page(
        &self,
        request: &WorkspaceEntryPageRequest,
    ) -> Result<WorkspaceEntryPageResult, AssetProcessorError> {
        validate_read_capability(&request.capability, self.capability_grants())?;
        let page_size = validate_workspace_entry_page_request(request)?;
        let db = self.databases.query();
        let workspace_id = self.attached_workspace_id()?;
        if db.workspace_by_id(workspace_id)?.is_none() {
            info!("asset processor workspace entry query found no workspace");
            return Ok(WorkspaceEntryPageResult {
                entries: Vec::new(),
                next_after_entry_id: None,
            });
        }

        let fetch_limit = page_size.saturating_add(1);
        let root_pks = match request.root_scope {
            AssetRootScope::BrowserAssets => {
                Some(browser_asset_scan_folder_ids(&db, workspace_id)?)
            }
            AssetRootScope::All => None,
        };
        let mut rows = db.workspace_entry_page(
            workspace_id,
            root_pks.as_deref(),
            request.after_entry_id.unwrap_or(0),
            u32::try_from(fetch_limit).unwrap_or(u32::MAX),
        )?;
        drop(db);
        let next_after_entry_id = if page_size > 0 && rows.len() > page_size {
            rows.truncate(page_size);
            rows.last().map(|row| row.entry_id)
        } else {
            None
        };

        let entries: Vec<WorkspaceEntry> = rows
            .into_iter()
            .map(workspace_entry_snapshot_to_proto)
            .collect::<Result<Vec<_>, _>>()?;

        info!(
            workspace_id,
            entry_count = entries.len(),
            has_next_page = next_after_entry_id.is_some(),
            "asset processor workspace entry query completed"
        );

        Ok(WorkspaceEntryPageResult {
            entries,
            next_after_entry_id,
        })
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::InvalidAssetSourceReconcileRequest`] if the request is
    /// malformed, [`AssetProcessorError::AssetSourceReconcileMissingWorkspace`] if
    /// the session has no workspace, the `SourceRoot*` variants if a registered root
    /// cannot be walked, [`AssetProcessorError::AssetSourceCollision`] if two roots
    /// claim one virtual path, and [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
    #[instrument(skip(self, request), fields(session_id = %request.session_id, root_scope = ?request.root_scope))]
    pub fn reconcile_asset_sources(
        &self,
        request: &ReconcileAssetSourcesRequest,
    ) -> Result<ReconcileAssetSourcesResult, AssetProcessorError> {
        validate_reconcile_asset_sources_request(request, self.capability_grants())?;
        self.reconcile_asset_sources_for_session(&request.session_id, request.root_scope)
    }

    #[instrument(skip(self), fields(session_id = %session_id, root_scope = ?root_scope))]
    pub(crate) fn reconcile_asset_sources_for_session(
        &self,
        session_id: &str,
        root_scope: AssetRootScope,
    ) -> Result<ReconcileAssetSourcesResult, AssetProcessorError> {
        let now_unix_ms = current_unix_ms_i64()?;
        let workspace_id = self.attached_workspace_id()?;
        // The composition owns the writer before this processor is exposed;
        // resolve its clone-free handle before borrowing the read connection.
        let group = self.asset_db_writer();
        let (workspace_id, source_roots, summary) = {
            let db = self.databases.query_mut();
            let mut source_roots = self.source_roots.clone();
            if root_scope == AssetRootScope::BrowserAssets {
                source_roots.retain(is_browser_asset_source_root);
            }
            let classifiers =
                source_asset_classifiers(self.published_catalog.borrow().as_ref(), self.registries);
            let summary = reconcile_registered_source_assets(
                ReconcilePass {
                    db: &db,
                    writer: group,
                    changed_by_session: Some(session_id),
                    classifiers: &classifiers,
                    now_unix_ms,
                },
                &source_roots,
            )?;
            drop(db);
            (workspace_id, source_roots, summary)
        };

        let source_root_count = u32::try_from(source_roots.len()).map_err(|_| {
            AssetProcessorError::AssetSourceReconcileCountOverflow {
                field: "source_root_count",
                count: source_roots.len(),
            }
        })?;
        let recorded_source_asset_count = u32::try_from(summary.recorded).map_err(|_| {
            AssetProcessorError::AssetSourceReconcileCountOverflow {
                field: "recorded_source_asset_count",
                count: summary.recorded,
            }
        })?;
        let deleted_source_asset_count = u32::try_from(summary.deleted).map_err(|_| {
            AssetProcessorError::AssetSourceReconcileCountOverflow {
                field: "deleted_source_asset_count",
                count: summary.deleted,
            }
        })?;

        info!(
            workspace_id,
            source_root_count,
            recorded_source_asset_count,
            deleted_source_asset_count,
            adopted_external_source_asset_count = summary.adopted_external,
            conflicted_source_asset_count = summary.conflicted,
            preserved_identity_rebind_count = summary.preserved_identity_rebinds,
            enqueued_job_count = summary.planned_jobs,
            "asset processor reconcile asset sources completed"
        );
        Ok(ReconcileAssetSourcesResult {
            source_root_count,
            recorded_source_asset_count,
            deleted_source_asset_count,
        })
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached, and
    /// [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
    #[instrument(skip(self, request), fields(root_scope = ?request.root_scope))]
    pub fn workspace_snapshot(
        &self,
        request: &WorkspaceSnapshotRequest,
    ) -> Result<WorkspaceSnapshotResult, AssetProcessorError> {
        validate_read_capability(&request.capability, self.capability_grants())?;
        let db = self.databases.query();
        let workspace_id = self.attached_workspace_id()?;
        let snapshot = db.workspace_by_id(workspace_id)?.map(|row| {
            let mut source_roots = self.source_roots.clone();
            if request.root_scope == AssetRootScope::BrowserAssets {
                source_roots.retain(is_browser_asset_source_root);
            }
            workspace_snapshot_to_proto(row, source_roots)
        });
        drop(db);

        info!(
            found = snapshot.is_some(),
            "asset processor workspace snapshot query completed"
        );
        Ok(WorkspaceSnapshotResult { snapshot })
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation, and
    /// [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached.
    #[instrument(skip(self, request))]
    pub fn validate_event_subscription(
        &self,
        request: &AssetProcessorEventSubscriptionRequest,
    ) -> Result<(), AssetProcessorError> {
        validate_read_capability(&request.capability, self.capability_grants())?;
        self.attached_workspace_id()?;
        info!("asset processor event subscription accepted");
        Ok(())
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::InvalidCatalogProductsRequest`] if the request is
    /// malformed, [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached, and
    /// [`AssetProcessorError::Repo`] if an `AssetDB` query fails. A workspace that does not exist
    /// yields an empty result rather than an error.
    #[instrument(skip(self, request), fields(platform = %request.platform))]
    pub fn catalog_products(
        &self,
        request: &CatalogProductsRequest,
    ) -> Result<CatalogProductsResult, AssetProcessorError> {
        validate_read_capability(&request.capability, self.capability_grants())?;
        validate_catalog_products_request(request)?;
        let db = self.databases.query();
        let workspace_id = self.attached_workspace_id()?;
        if db.workspace_by_id(workspace_id)?.is_none() {
            info!("asset processor catalog-products query found no workspace");
            return Ok(CatalogProductsResult {
                entries: Vec::new(),
            });
        }

        let entries = catalog::catalog_product_entries(
            &db,
            workspace_id,
            &request.platform,
            self.published_catalog.borrow().as_ref(),
        )?;
        drop(db);

        info!(
            workspace_id,
            platform = %request.platform,
            entry_count = entries.len(),
            "asset processor catalog-products query completed"
        );
        Ok(CatalogProductsResult { entries })
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the capability is not a
    /// valid editor asset-write capability, [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached,
    /// [`AssetProcessorError::Repo`] if an `AssetDB` query fails, and
    /// [`AssetProcessorError::AssetProcessingStatusCountOverflow`] if a count exceeds
    /// the protocol's `UInt32` range.
    #[instrument(level = "trace", skip(self, request), fields(session_id = %request.session_id, platform = %request.platform))]
    pub fn processing_status(
        &self,
        request: &AssetProcessingStatusRequest,
    ) -> Result<AssetProcessingStatusResult, AssetProcessorError> {
        validate_editor_asset_write_capability(&request.capability, self.capability_grants())?;
        let db = self.databases.query();
        let workspace_id = self.attached_workspace_id()?;
        let status = db.processing_status(workspace_id, Some(&request.platform))?;
        drop(db);
        let result = asset_processing_status_to_proto(status)?;
        trace!(
            workspace_id,
            queued = result.queued,
            leased = result.leased,
            failed = result.failed,
            "asset processor processing status queried"
        );
        Ok(result)
    }

    /// Waits for the next committed processing-status transition. The caller
    /// must subscribe before taking its source-graph snapshot, then re-derive
    /// the aggregate after each wake; the revision is a causal wake source,
    /// not a replacement for the database authority.
    fn subscribe_processing_status_changes(&self) -> az_assetdb::AssetProcessingStatusSubscription {
        self.databases.query().subscribe_asset_processing_status()
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the capability is not a
    /// valid editor asset-write capability, [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached, the
    /// `CatalogPublisher*` variants if the publisher cannot be started or has
    /// stopped, and [`AssetProcessorError::CatalogPublicationFailed`] if the
    /// publication itself fails.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[instrument(skip(self, request), fields(session_id = %request.session_id, platform = %request.platform))]
    pub async fn publish_asset_catalog(
        &self,
        request: &PublishAssetCatalogRequest,
    ) -> Result<PublishAssetCatalogResult, AssetProcessorError> {
        validate_editor_asset_write_capability(&request.capability, self.capability_grants())?;
        self.catalog_publisher()?
            .publish(request.platform.clone())
            .await
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::InvalidReleaseContentRequest`] if the request is
    /// malformed, [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached,
    /// [`AssetProcessorError::InvalidReleaseContentCacheRoot`] or
    /// [`AssetProcessorError::InvalidReleaseContentPlatform`] if the workspace's
    /// release-content configuration is invalid, the `ReleaseContent*` variants if a
    /// catalog or product file is missing, unreadable or the wrong length, and
    /// [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
    #[instrument(skip(self, request), fields(platform = %request.platform, target = ?request.target))]
    pub fn release_content(
        &self,
        request: &ReleaseContentReadRequest,
    ) -> Result<ReleaseContentReadResult, AssetProcessorError> {
        validate_read_capability(&request.capability, self.capability_grants())?;
        validate_release_content_request(request)?;
        let db = self.databases.query();
        let workspace_id = self.attached_workspace_id()?;
        let Some(workspace) = db.workspace_by_id(workspace_id)? else {
            info!("asset processor release-content read found no workspace");
            return Ok(ReleaseContentReadResult::None);
        };

        let cache_root =
            release_content_cache_root(self.project_data_paths()?, &workspace, &request.platform)?;
        let (result, product_count) = match &request.target {
            ReleaseContentTarget::AssetCatalog => {
                let result = release_asset_catalog_side_channel(
                    &request.capability,
                    &request.platform,
                    &cache_root,
                )?
                .map_or(
                    ReleaseContentReadResult::None,
                    ReleaseContentReadResult::AssetCatalog,
                );
                (result, None)
            }
            ReleaseContentTarget::ProductAsset { asset_guid, sub_id } => {
                let product = release_content_product_side_channel(
                    &db,
                    &request.capability,
                    workspace.workspace_id,
                    &request.platform,
                    &cache_root,
                    self.published_catalog.borrow().as_ref(),
                    *asset_guid,
                    *sub_id,
                )?;
                drop(db);
                let result = product.map_or(
                    ReleaseContentReadResult::None,
                    ReleaseContentReadResult::Product,
                );
                (result, None)
            }
        };
        let result_kind = match &result {
            ReleaseContentReadResult::None => "none",
            ReleaseContentReadResult::AssetCatalog(_) => "asset_catalog",
            ReleaseContentReadResult::Product(_) => "product",
        };

        info!(
            workspace_id = workspace.workspace_id,
            platform = %request.platform,
            product_count = product_count.unwrap_or(0),
            result = result_kind,
            "asset processor release-content read completed"
        );
        Ok(result)
    }

    /// What the durable side already holds for the source a record request names.
    ///
    /// The saved payload is the authority: recording only publishes what the
    /// editor already checkpointed, so the checkpoint hash has to match the hash
    /// the request carries before anything is written.
    fn source_asset_record(
        &self,
        request: &SourceAssetRecordRequest,
        workspace_id: i64,
        source_path: &str,
        content_hash: Digest,
    ) -> Result<SourceAssetRecord, AssetProcessorError> {
        let db = self.databases.query();
        let workspace = db.workspace_by_id(workspace_id)?.ok_or_else(|| {
            AssetProcessorError::InvalidSourceAssetRecord {
                reason: "attached workspace does not exist".to_owned(),
            }
        })?;
        let policy = db
            .workspace_root_by_id(request.workspace_root_id)?
            .filter(|policy| {
                policy.workspace_pk == workspace_id && policy.owner == request.owner_id
            })
            .ok_or_else(|| AssetProcessorError::InvalidSourceAssetRecord {
                reason: "workspace source root is outside the attached workspace authority"
                    .to_owned(),
            })?;
        let root = db.root_by_id(policy.root_pk)?.ok_or_else(|| {
            AssetProcessorError::InvalidSourceAssetRecord {
                reason: "workspace source root references a missing root".to_owned(),
            }
        })?;
        let payload = db
            .payload_for_source(workspace_id, root.root_id, source_path)?
            .ok_or_else(|| AssetProcessorError::AuthoredAssetMissingSavedPayload {
                workspace_id,
                source_path: source_path.to_owned(),
            })?;
        let checkpoint = payload.checkpoint.as_ref().ok_or_else(|| {
            AssetProcessorError::AuthoredAssetMissingSavedPayload {
                workspace_id,
                source_path: source_path.to_owned(),
            }
        })?;
        let checkpoint_hash = Digest::from(blake3::hash(checkpoint));
        if payload.saved != Some(payload.revision) || checkpoint_hash != content_hash {
            return Err(
                AssetProcessorError::AuthoredAssetRecordPayloadHashMismatch {
                    workspace_id,
                    source_path: source_path.to_owned(),
                    expected: content_hash.to_string(),
                    actual: checkpoint_hash.to_string(),
                },
            );
        }
        let existing = db.source_asset(workspace_id, root.root_id, source_path)?;
        drop(db);
        Ok(SourceAssetRecord {
            workspace,
            policy,
            root,
            payload,
            existing,
        })
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::InvalidSourceAssetRecord`] if the request is malformed,
    /// [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached,
    /// [`AssetProcessorError::AuthoredSourcePublicationRejected`] if the publication
    /// is refused, and [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
    ///
    /// # Panics
    ///
    /// Panics if the validated content hash is not digest-sized. Validation
    /// already rejected every other length, so reaching this point with the
    /// wrong one means the validator and the digest type disagree.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[instrument(skip(self, request), fields(session_id = %request.session_id, source_path = %request.source_path))]
    pub async fn record_source_asset(
        &self,
        request: &SourceAssetRecordRequest,
    ) -> Result<SourceAssetRecordResult, AssetProcessorError> {
        validate_source_asset_record_capability(&request.capability, self.capability_grants())?;
        let source_path = validate_source_asset_record_request(request)?;
        let workspace_id = self.attached_workspace_id()?;

        let content_hash = Digest::from_bytes(
            request
                .content_hash
                .as_slice()
                .try_into()
                .expect("validated source content hash length"),
        );
        let SourceAssetRecord {
            workspace,
            policy,
            root,
            payload,
            existing,
        } = self.source_asset_record(request, workspace_id, &source_path, content_hash)?;
        let guid = existing.as_ref().map_or_else(
            || resolve_source_asset_guid(&source_path, None),
            |(asset, _)| asset.guid,
        );
        let published = self
            .asset_db_writer()
            .publish_authored_source(PublishAuthoredSource {
                payload: WriteSourcePayload {
                    workspace_pk: workspace_id,
                    root_pk: root.root_id,
                    path: source_path.clone(),
                    document: payload.document.clone(),
                    schema: payload.schema.clone(),
                    encoding: payload.encoding,
                    expected_revision: Some(payload.revision),
                    revision: payload.revision,
                    saved: payload.saved,
                    digest: content_hash,
                    payload: payload.payload.clone(),
                    checkpoint: CheckpointWrite::Preserve,
                    session: Some(request.session_id.clone()),
                    project: workspace.project,
                    now: request.changed_unix_ms,
                },
                workspace_root_pk: policy.workspace_root_id,
                source: SweepEntry {
                    path: source_path.clone(),
                    guid,
                    schema: request.schema_type.clone(),
                    digest: content_hash,
                    diff: existing.as_ref().map_or(DbDiff::Added, |(_, entry)| {
                        if entry.digest == content_hash {
                            DbDiff::Clean
                        } else {
                            DbDiff::Modified
                        }
                    }),
                    diagnostics: request.diagnostics_count,
                    updated: request.changed_unix_ms,
                    src_bytes: payload.bytes,
                    src_mtime: request.changed_unix_ms,
                    meta_bytes: 0,
                    meta_mtime: 0,
                    observed: request.changed_unix_ms,
                    session: Some(request.session_id.clone()),
                },
            })
            .await?;
        let (asset, entry) = authored_publication_written(published, &source_path)?;
        let enqueued_jobs = match self.enqueue_jobs_for_source(&asset, &entry, false).await {
            Ok(count) => count,
            Err(error) => {
                warn!(
                    source_path,
                    %error,
                    "source publication committed; planning is deferred until the next reconcile"
                );
                0
            }
        };
        let entry = workspace_asset_entry_to_proto(&self.databases.query(), &asset, entry)?;

        info!(
            asset_guid = %asset.guid,
            entry_id = entry.entry_id,
            enqueued_jobs,
            "asset processor recorded saved source asset"
        );

        Ok(SourceAssetRecordResult {
            asset_guid: asset.guid,
            entry,
        })
    }

    /// Resolves the file-backed authoring workflow a create request targets.
    ///
    /// A published worker catalog wins over the compiled-in registration, so a
    /// project that replaced a schema authors against its own version. The
    /// registration is returned as well, because the payload templates come
    /// from it rather than from the catalog descriptor.
    fn source_file_create_workflow(
        &self,
        request: &SourceFileCreateRequest,
        source_path: &str,
    ) -> Result<Option<az_asset_builder::SourceSchemaRegistration>, AssetProcessorError> {
        let registered_source_schema = composed_source_schemas(self.registries)
            .into_iter()
            .find(|attributed| attributed.entry.schema_type().as_str() == request.schema_type)
            .map(|attributed| attributed.entry);
        let source_schema_authoring = self
            .published_catalog
            .borrow()
            .as_ref()
            .and_then(|catalog| {
                catalog
                    .source_schemas
                    .iter()
                    .find(|descriptor| descriptor.schema_type == request.schema_type)
            })
            .map(|descriptor| descriptor.authoring.clone())
            .or_else(|| {
                registered_source_schema
                    .map(|registration| source_schema_authoring_to_proto(registration.authoring()))
            })
            .ok_or_else(|| AssetProcessorError::SourceFileCreateUnknownSchema {
                schema_type: request.schema_type.clone(),
            })?;
        let SourceSchemaAuthoring::File { workflow } = source_schema_authoring else {
            return Err(AssetProcessorError::SourceFileCreateSchemaNotFileBacked {
                schema_type: request.schema_type.clone(),
            });
        };
        if matches!(&request.content, SourceFileCreateContent::DefaultTemplate)
            && !workflow.can_create
        {
            return Err(AssetProcessorError::SourceFileCreateSchemaNotCreatable {
                schema_type: request.schema_type.clone(),
            });
        }
        if request.source_root != workflow.source_root {
            return Err(AssetProcessorError::SourceFileCreateSourceRootMismatch {
                schema_type: request.schema_type.clone(),
                requested_source_root: request.source_root.clone(),
                workflow_source_root: workflow.source_root,
            });
        }
        validate_source_file_create_extension(source_path, &workflow.extensions)?;
        Ok(registered_source_schema)
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::InvalidSourceFileCreateRequest`] if the request is
    /// malformed, the `SourceFileCreate*` variants if the schema is unknown, not
    /// file-backed, not creatable, resolves to no or several project or source roots,
    /// or has no usable template, [`AssetProcessorError::SourceFileCodecRpc`] if the
    /// worker codec refuses the payload, the `SourceFile*` IO variants if the file
    /// cannot be staged or written, and [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[instrument(skip(self, request), fields(session_id = %request.session_id, source_root = %request.source_root, source_path = %request.source_path, schema_type = %request.schema_type))]
    pub async fn create_source_file(
        &self,
        request: &SourceFileCreateRequest,
    ) -> Result<SourceFileCreateResult, AssetProcessorError> {
        validate_source_file_create_capability(&request.capability, self.capability_grants())?;
        let source_path = validate_source_file_create_request(request)?;
        let workspace_id = self.attached_workspace_id()?;

        let registered_source_schema = self.source_file_create_workflow(request, &source_path)?;

        let source_bytes = create_source_file_payload(
            request,
            registered_source_schema.map(az_asset_builder::SourceSchemaRegistration::schema_type),
            &source_path,
            self.registries,
        )?;
        i64::try_from(source_bytes.len()).map_err(|_| {
            AssetProcessorError::SourceFileCreatePayloadTooLarge {
                source_path: source_path.clone(),
                byte_length: source_bytes.len() as u64,
            }
        })?;
        let content_hash = Digest::from(blake3::hash(&source_bytes));

        let (workspace, policy, root) = {
            let db = self.databases.query();
            source_file_create_source_root(
                &db,
                workspace_id,
                &request.session_id,
                &request.source_root,
            )?
        };

        let published = self
            .asset_db_writer()
            .publish_authored_source(PublishAuthoredSource {
                payload: WriteSourcePayload {
                    workspace_pk: workspace_id,
                    root_pk: root.root_id,
                    path: source_path.clone(),
                    document: source_path.clone(),
                    schema: request.schema_type.clone(),
                    encoding: Encoding::Bytes,
                    expected_revision: None,
                    revision: 1,
                    saved: Some(1),
                    digest: content_hash,
                    payload: source_bytes.clone(),
                    checkpoint: CheckpointWrite::Replace(source_bytes.clone()),
                    session: Some(request.session_id.clone()),
                    project: workspace.project,
                    now: request.changed_unix_ms,
                },
                workspace_root_pk: policy.workspace_root_id,
                source: SweepEntry {
                    path: source_path.clone(),
                    guid: resolve_source_asset_guid(&source_path, None),
                    schema: Some(request.schema_type.clone()),
                    digest: content_hash,
                    diff: DbDiff::Added,
                    diagnostics: 0,
                    updated: request.changed_unix_ms,
                    src_bytes: i64::try_from(source_bytes.len()).unwrap_or(i64::MAX),
                    src_mtime: request.changed_unix_ms,
                    meta_bytes: 0,
                    meta_mtime: 0,
                    observed: request.changed_unix_ms,
                    session: Some(request.session_id.clone()),
                },
            })
            .await?;
        let (asset, entry) = authored_publication_written(published, &source_path)?;
        let enqueued_jobs = match self.enqueue_jobs_for_source(&asset, &entry, false).await {
            Ok(count) => count,
            Err(error) => {
                warn!(
                    source_path,
                    %error,
                    "created source committed; planning is deferred until the next reconcile"
                );
                0
            }
        };
        let entry = workspace_asset_entry_to_proto(&self.databases.query(), &asset, entry)?;

        info!(
            asset_guid = %asset.guid,
            entry_id = entry.entry_id,
            schema_type = %request.schema_type,
            enqueued_jobs,
            "asset processor created DB-authored source file"
        );

        Ok(SourceFileCreateResult {
            record: SourceAssetRecordResult {
                asset_guid: asset.guid,
                entry,
            },
        })
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::InvalidSourceDependentsRequest`] if the request is
    /// malformed, [`AssetProcessorError::MissingAttachedWorkspace`] if no project-instance workspace is attached,
    /// [`AssetProcessorError::UnknownSourceRootSelector`] if the named source root is
    /// not registered, and [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
    #[instrument(skip(self, request), fields(session_id = %request.session_id, source_root = %request.source_root, source_path = %request.source_path))]
    pub fn source_dependents(
        &self,
        request: &SourceDependentsRequest,
    ) -> Result<SourceDependentsResult, AssetProcessorError> {
        validate_read_capability(&request.capability, self.capability_grants())?;
        let source_path = validate_source_dependents_request(request)?;
        let workspace_id = self.attached_workspace_id()?;
        let dependents = {
            let db = self.databases.query();
            let (_, _, root) = source_file_create_source_root(
                &db,
                workspace_id,
                &request.session_id,
                &request.source_root,
            )?;
            let (asset, _) = db
                .source_asset(workspace_id, root.root_id, &source_path)?
                .ok_or_else(|| AssetProcessorError::AuthoredSourcePublicationRejected {
                    source_path: source_path.clone(),
                    reason: "dependent lookup source does not exist",
                })?;
            let rows = db.source_dependents(&DbSourceDependentsInput {
                workspace_pk: workspace_id,
                asset_pk: asset.asset_id,
            })?;
            drop(db);
            source_dependents_to_proto(source_path, rows)?
        };
        info!(
            source_dependents = dependents.source_dependents.len(),
            job_dependents = dependents.job_dependents.len(),
            "asset processor queried source dependents"
        );
        Ok(dependents)
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[instrument(skip(self, request), fields(session_id = %request.session_id, source_root = %request.source_root, source_path = %request.source_path))]
    async fn delete_source_file(
        &self,
        request: &SourceFileDeleteRequest,
    ) -> Result<SourceFileDeleteResult, AssetProcessorError> {
        validate_source_file_create_capability(&request.capability, self.capability_grants())?;
        let source_path = validate_source_file_delete_request(request)?;
        let workspace_id = self.attached_workspace_id()?;
        let (policy, root, expected) = {
            let db = self.databases.query();
            let (_, policy, root) = source_file_create_source_root(
                &db,
                workspace_id,
                &request.session_id,
                &request.source_root,
            )?;
            let (_, entry) = db
                .source_asset(workspace_id, root.root_id, &source_path)?
                .ok_or_else(|| AssetProcessorError::AuthoredSourcePublicationRejected {
                    source_path: source_path.clone(),
                    reason: "source does not exist",
                })?;
            let payload = db.payload_for_source(workspace_id, root.root_id, &source_path)?;
            drop(db);
            let expected = SourceStateToken {
                revision: payload.as_ref().map(|payload| payload.revision),
                digest: entry.digest,
            };
            (policy, root, expected)
        };
        let staged = StagedSourceFileMutation::delete(&policy.path, &source_path)?;
        let result = self
            .asset_db_writer()
            .delete_source(DeleteSource {
                workspace_pk: workspace_id,
                root_pk: root.root_id,
                path: source_path.clone(),
                expected,
                now: request.changed_unix_ms,
            })
            .await;
        let result = match result {
            Ok(DeleteSourceResult::Deleted(deleted)) => {
                staged.commit();
                (deleted.asset, deleted.entry)
            }
            Ok(DeleteSourceResult::Conflict) => {
                return Err(staged.rollback(
                    "delete",
                    AssetProcessorError::AuthoredSourcePublicationRejected {
                        source_path,
                        reason: "source state changed before delete",
                    },
                ));
            }
            Ok(DeleteSourceResult::Unsaved) => {
                return Err(staged.rollback(
                    "delete",
                    AssetProcessorError::SourceFileHasUnsavedEdits {
                        operation: "delete",
                        source_path,
                    },
                ));
            }
            Ok(DeleteSourceResult::NotFound) => {
                return Err(staged.rollback(
                    "delete",
                    AssetProcessorError::AuthoredSourcePublicationRejected {
                        source_path,
                        reason: "source disappeared before delete",
                    },
                ));
            }
            Err(error) => return Err(staged.rollback("delete", error.into())),
        };
        let (asset, entry) = result;
        let entry = workspace_asset_entry_to_proto(&self.databases.query(), &asset, entry)?;
        info!(
            asset_guid = %asset.guid,
            entry_id = entry.entry_id,
            "asset processor deleted source file"
        );
        Ok(SourceFileDeleteResult {
            record: SourceAssetRecordResult {
                asset_guid: asset.guid,
                entry,
            },
        })
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[instrument(skip(self, request), fields(session_id = %request.session_id, source_root = %request.source_root, from_source_path = %request.from_source_path, to_source_path = %request.to_source_path))]
    async fn move_source_file(
        &self,
        request: &SourceFileMoveRequest,
    ) -> Result<SourceFileMoveResult, AssetProcessorError> {
        validate_source_file_create_capability(&request.capability, self.capability_grants())?;
        let (from_source_path, to_source_path) = validate_source_file_move_request(request)?;
        let workspace_id = self.attached_workspace_id()?;
        let (policy, root, expected) = {
            let db = self.databases.query();
            let (_, policy, root) = source_file_create_source_root(
                &db,
                workspace_id,
                &request.session_id,
                &request.source_root,
            )?;
            let (_, entry) = db
                .source_asset(workspace_id, root.root_id, &from_source_path)?
                .ok_or_else(|| AssetProcessorError::AuthoredSourcePublicationRejected {
                    source_path: from_source_path.clone(),
                    reason: "source does not exist",
                })?;
            let payload = db.payload_for_source(workspace_id, root.root_id, &from_source_path)?;
            drop(db);
            let expected = SourceStateToken {
                revision: payload.as_ref().map(|payload| payload.revision),
                digest: entry.digest,
            };
            (policy, root, expected)
        };
        let staged =
            StagedSourceFileMutation::move_file(&policy.path, &from_source_path, &to_source_path)?;
        let result = self
            .asset_db_writer()
            .move_source(MoveSource {
                workspace_pk: workspace_id,
                root_pk: root.root_id,
                from: from_source_path.clone(),
                to: to_source_path.clone(),
                expected,
                now: request.changed_unix_ms,
            })
            .await;
        let record = match result {
            Ok(MoveSourceResult::Moved(moved)) => {
                staged.commit();
                (moved.asset, moved.entry)
            }
            Ok(MoveSourceResult::Conflict) => {
                return Err(staged.rollback(
                    "move",
                    AssetProcessorError::AuthoredSourcePublicationRejected {
                        source_path: from_source_path,
                        reason: "source state changed before move",
                    },
                ));
            }
            Ok(MoveSourceResult::Unsaved) => {
                return Err(staged.rollback(
                    "move",
                    AssetProcessorError::SourceFileHasUnsavedEdits {
                        operation: "move",
                        source_path: from_source_path,
                    },
                ));
            }
            Ok(MoveSourceResult::NotFound) => {
                return Err(staged.rollback(
                    "move",
                    AssetProcessorError::AuthoredSourcePublicationRejected {
                        source_path: from_source_path,
                        reason: "source disappeared before move",
                    },
                ));
            }
            Err(error) => return Err(staged.rollback("move", error.into())),
        };
        let (asset, entry) = record;
        let entry = workspace_asset_entry_to_proto(&self.databases.query(), &asset, entry)?;
        info!(
            asset_guid = %asset.guid,
            entry_id = entry.entry_id,
            "asset processor moved source file"
        );
        Ok(SourceFileMoveResult {
            record: SourceAssetRecordResult {
                asset_guid: asset.guid,
                entry,
            },
            old_source_path: from_source_path,
        })
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation,
    /// [`AssetProcessorError::InvalidForceReprocessAssetRequest`] if the request is
    /// malformed, [`AssetProcessorError::ForceReprocessMissingSource`] if the source
    /// is not recorded, [`AssetProcessorError::ForceReprocessUnavailableStatus`] if
    /// the asset is in a status that cannot be reprocessed,
    /// [`AssetProcessorError::ForceReprocessNoJobs`] if no jobs were enqueued,
    /// [`AssetProcessorError::ForceReprocessJobCountOverflow`] if the enqueued count
    /// exceeds the protocol's `UInt32` range, and [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[instrument(skip(self, request), fields(session_id = %request.session_id, source_root = %request.source_root, source_path = %request.source_path))]
    pub async fn force_reprocess_asset(
        &self,
        request: &ForceReprocessAssetRequest,
    ) -> Result<ForceReprocessAssetResult, AssetProcessorError> {
        validate_source_file_create_capability(&request.capability, self.capability_grants())?;
        let source_path = validate_force_reprocess_asset_request(request)?;
        let workspace_id = self.attached_workspace_id()?;

        let (asset, entry) = {
            let db = self.databases.query();
            let (_, _, root) = source_file_create_source_root(
                &db,
                workspace_id,
                &request.session_id,
                &request.source_root,
            )?;
            let record = db.source_asset(workspace_id, root.root_id, &source_path)?;
            drop(db);
            let Some((asset, entry)) = record else {
                return Err(AssetProcessorError::ForceReprocessMissingSource {
                    session_id: request.session_id.clone(),
                    source_root: request.source_root.clone(),
                    source_path: source_path.clone(),
                });
            };
            if entry.diff == DbDiff::Conflicted {
                return Err(AssetProcessorError::ForceReprocessUnavailableStatus {
                    source_path: source_path.clone(),
                    status: entry.diff,
                });
            }
            (asset, entry)
        };

        let enqueued_jobs = self.enqueue_jobs_for_source(&asset, &entry, true).await?;
        if enqueued_jobs == 0 {
            return Err(AssetProcessorError::ForceReprocessNoJobs {
                source_path: source_path.clone(),
            });
        }
        let enqueued_jobs_u32 = u32::try_from(enqueued_jobs).map_err(|_| {
            AssetProcessorError::ForceReprocessJobCountOverflow {
                count: enqueued_jobs,
            }
        })?;
        let asset_guid = asset.guid;
        let entry = workspace_asset_entry_to_proto(&self.databases.query(), &asset, entry)?;

        info!(
            asset_guid = %asset_guid,
            entry_id = entry.entry_id,
            enqueued_jobs = enqueued_jobs_u32,
            "asset processor force-reprocessed source asset"
        );

        Ok(ForceReprocessAssetResult {
            record: SourceAssetRecordResult { asset_guid, entry },
            enqueued_jobs: enqueued_jobs_u32,
        })
    }

    /// Everything planning needs to know about a source before it can replan it.
    ///
    /// The saved payload is preferred over the file on disk, because a source
    /// the editor holds may not have been flushed yet; when there is one, its
    /// checkpoint has to hash to the digest already recorded for the entry.
    fn source_plan_facts(
        &self,
        asset: &SelectAssets,
        entry: &SelectEntries,
    ) -> Result<SourcePlanFacts, AssetProcessorError> {
        let db = self.databases.query();
        let policy = db
            .workspace_roots(entry.workspace_pk)?
            .into_iter()
            .find(|policy| policy.root_pk == entry.root_pk)
            .ok_or_else(|| AssetProcessorError::MissingWorkspaceEntrySourceRoot {
                workspace_id: entry.workspace_pk,
                scan_folder_id: entry.root_pk,
                source_path: entry.path.clone(),
            })?;
        let source_bytes = if let Some(payload) =
            db.payload_for_source(entry.workspace_pk, entry.root_pk, &entry.path)?
        {
            let checkpoint = payload.checkpoint.ok_or_else(|| {
                AssetProcessorError::AuthoredAssetMissingSavedPayload {
                    workspace_id: entry.workspace_pk,
                    source_path: entry.path.clone(),
                }
            })?;
            let actual = Digest::from(blake3::hash(&checkpoint));
            if actual != entry.digest {
                return Err(
                    AssetProcessorError::AuthoredAssetRecordPayloadHashMismatch {
                        workspace_id: entry.workspace_pk,
                        source_path: entry.path.clone(),
                        expected: entry.digest.to_string(),
                        actual: actual.to_string(),
                    },
                );
            }
            checkpoint
        } else {
            let path = source_file_absolute_path(&policy.path, &entry.path)?;
            std::fs::read(&path).map_err(|source| AssetProcessorError::ReadCreateJobsSource {
                source_path: entry.path.clone(),
                path,
                source,
            })?
        };
        let current_jobs = db.jobs_for_asset(entry.workspace_pk, asset.asset_id)?;
        let has_current_execution_job = if self.builders.iter().next().is_none() {
            current_jobs.iter().any(|job| {
                job.kind == DbWork::Plan
                    && !matches!(job.status, DbStatus::Failed | DbStatus::Abandoned)
            })
        } else {
            current_jobs.iter().any(|job| {
                job.kind == DbWork::Build
                    && !matches!(job.status, DbStatus::Failed | DbStatus::Abandoned)
            })
        };
        let retire_job_ids = current_jobs
            .into_iter()
            .map(|job| job.job_id)
            .collect::<Vec<_>>();
        let retire_source_edge_ids: Vec<i64> = db
            .source_edges_for_asset(entry.workspace_pk, asset.asset_id)?
            .into_iter()
            .map(|edge| edge.source_edge_id)
            .collect();
        drop(db);
        Ok(SourcePlanFacts {
            source_root: PathBuf::from(policy.path),
            source_bytes,
            retire_job_ids,
            retire_source_edge_ids,
            has_current_execution_job,
        })
    }

    /// Runs every builder that claims this source and records the jobs and
    /// dependency edges they ask for into `delta`.
    ///
    /// A builder that skips the source contributes nothing; one that fails
    /// aborts the whole plan, because a partially planned source would look
    /// complete to the queue.
    fn plan_builder_jobs(
        &self,
        asset: &SelectAssets,
        entry: &SelectEntries,
        source_root: &Path,
        source_bytes: &[u8],
        delta: &mut PlanDelta,
    ) -> Result<(), AssetProcessorError> {
        let builders = self
            .builders
            .matching_source(&entry.path, entry.schema.as_deref())
            .collect::<Vec<_>>();
        for builder in builders {
            let platforms = [DEFAULT_PLATFORM_ID];
            let jobs = (builder.create_jobs)(&CreateJobsRequest {
                builder_id: builder.id,
                source_path: SourcePath::new(&entry.path),
                source_root,
                source_uuid: asset.guid,
                source_schema_type: entry.schema.as_deref(),
                source_bytes,
                platforms: &platforms,
                context: &BuilderJobContext::new(self.registries),
            });
            match jobs.result {
                CreateJobsResult::Success => {}
                CreateJobsResult::Skipped => continue,
                CreateJobsResult::Failed => {
                    return Err(AssetProcessorError::BuilderCreateJobsFailed {
                        builder_name: builder.name,
                        source_path: entry.path.clone(),
                    });
                }
            }
            Self::validate_builder_job_descriptors(builder, &entry.path, &jobs)?;
            let source_dependencies =
                Self::validate_builder_source_dependencies(builder, &entry.path, &jobs)?;
            let job_dependencies =
                Self::validate_builder_job_dependencies(builder, &entry.path, &jobs)?;

            for (raw, validated) in jobs
                .source_dependencies
                .iter()
                .zip(source_dependencies.iter())
            {
                let target = Self::dependency_target(raw, builder.name, &entry.path)?;
                delta.source_edges.push(SourceEdgeInput {
                    builder: builder.id.0,
                    asset_pk: asset.asset_id,
                    depends_pk: None,
                    target,
                    relation: DbRelation::SourceToSource,
                });
                debug_assert!(!validated.is_empty());
            }
            for (job, dependencies) in jobs.jobs.iter().zip(job_dependencies.iter()) {
                let raw_dependencies = &job.job_dependencies;
                let mut edges = Vec::with_capacity(dependencies.len());
                for (raw, dependency) in raw_dependencies.iter().zip(dependencies.iter()) {
                    let target = Self::dependency_target(&raw.source, builder.name, &entry.path)?;
                    edges.push(JobEdgeInput {
                        asset_pk: None,
                        target: target.clone(),
                        key: dependency.dependency_job_key.clone(),
                        platform: if dependency.dependency_platform.is_empty() {
                            job.platform.as_str().to_owned()
                        } else {
                            dependency.dependency_platform.clone()
                        },
                        coupling: dependency.kind,
                    });
                    if dependency.dependency_source != entry.path {
                        delta.source_edges.push(SourceEdgeInput {
                            builder: builder.id.0,
                            asset_pk: asset.asset_id,
                            depends_pk: None,
                            target,
                            relation: DbRelation::JobToJob,
                        });
                    }
                }
                delta.replacements.push(PlannedJob::build(
                    asset.asset_id,
                    builder.id.0,
                    job.job_key.as_str(),
                    job.platform.as_str(),
                    edges,
                ));
            }
        }
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn enqueue_jobs_for_source(
        &self,
        asset: &SelectAssets,
        entry: &SelectEntries,
        force: bool,
    ) -> Result<usize, AssetProcessorError> {
        if !matches!(entry.diff, DbDiff::Added | DbDiff::Modified | DbDiff::Clean) {
            return Ok(0);
        }

        let SourcePlanFacts {
            source_root,
            source_bytes,
            retire_job_ids,
            retire_source_edge_ids,
            has_current_execution_job,
        } = self.source_plan_facts(asset, entry)?;

        if entry.diff == DbDiff::Clean && !force && has_current_execution_job {
            return Ok(0);
        }

        let mut delta = PlanDelta {
            retire_job_ids,
            retire_source_edge_ids,
            ..PlanDelta::default()
        };
        if self.builders.iter().next().is_none() {
            delta.replacements.push(PlannedJob::plan(
                asset.asset_id,
                ASSET_PLANNER_JOB_KEY,
                DEFAULT_PLATFORM_ID.as_str(),
                Vec::new(),
            ));
        } else {
            self.plan_builder_jobs(asset, entry, &source_root, &source_bytes, &mut delta)?;
        }

        let admitted = delta.replacements.len();
        self.asset_db_writer()
            .apply_plan_delta(ApplyPlanDelta {
                workspace_pk: entry.workspace_pk,
                delta,
            })
            .await?;
        if admitted != 0 {
            self.prioritized_asset_identities
                .borrow_mut()
                .insert(asset.asset_id);
        }
        Ok(admitted)
    }

    fn dependency_target(
        dependency: &SourceFileDependency,
        builder_name: &'static str,
        source_path: &str,
    ) -> Result<DbTarget, AssetProcessorError> {
        match dependency {
            SourceFileDependency::Uuid(guid) if !guid.is_nil() => Ok(DbTarget::Guid(*guid)),
            SourceFileDependency::Path(path) => DbTarget::path(path.clone()).map_err(|error| {
                AssetProcessorError::InvalidBuilderCreateJobs {
                    builder_name,
                    source_path: source_path.to_owned(),
                    reason: error.to_string(),
                }
            }),
            SourceFileDependency::Uuid(_) => Err(AssetProcessorError::InvalidBuilderCreateJobs {
                builder_name,
                source_path: source_path.to_owned(),
                reason: "source dependency UUID must not be nil".to_owned(),
            }),
        }
    }

    fn validate_builder_job_descriptors(
        builder: &BuildRule,
        source_path: &str,
        jobs: &CreateJobsResponse,
    ) -> Result<(), AssetProcessorError> {
        let mut seen = BTreeSet::new();
        for job in &jobs.jobs {
            if !seen.insert((job.platform.as_str(), job.job_key.as_str())) {
                return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                    builder_name: builder.name,
                    source_path: source_path.to_string(),
                    reason: format!(
                        "duplicate job descriptor `{}` for platform `{}`",
                        job.job_key, job.platform
                    ),
                });
            }
        }

        Ok(())
    }

    fn validate_builder_source_dependencies(
        builder: &BuildRule,
        source_path: &str,
        jobs: &CreateJobsResponse,
    ) -> Result<Vec<String>, AssetProcessorError> {
        let mut seen = BTreeSet::new();
        let mut dependencies = Vec::with_capacity(jobs.source_dependencies.len());

        for dependency in &jobs.source_dependencies {
            let target = match dependency {
                SourceFileDependency::Path(path) => {
                    let Some(path) = validate_asset_db_relative_path(path) else {
                        return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                            builder_name: builder.name,
                            source_path: source_path.to_string(),
                            reason: format!(
                                "source dependency path `{path}` is not an asset-db relative path"
                            ),
                        });
                    };
                    if path == source_path {
                        return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                            builder_name: builder.name,
                            source_path: source_path.to_string(),
                            reason: "source dependency must not point at the source itself"
                                .to_string(),
                        });
                    }
                    path
                }
                SourceFileDependency::Uuid(uuid) => {
                    if uuid.is_nil() {
                        return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                            builder_name: builder.name,
                            source_path: source_path.to_string(),
                            reason: "source dependency UUID must not be nil".to_string(),
                        });
                    }
                    format!("uuid:{uuid}")
                }
            };

            if !seen.insert(target.clone()) {
                return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                    builder_name: builder.name,
                    source_path: source_path.to_string(),
                    reason: format!("duplicate source dependency `{target}`"),
                });
            }
            dependencies.push(target);
        }

        Ok(dependencies)
    }

    fn validate_builder_job_dependencies(
        builder: &BuildRule,
        source_path: &str,
        jobs: &CreateJobsResponse,
    ) -> Result<Vec<Vec<ValidatedJobDependency>>, AssetProcessorError> {
        let mut validated = Vec::with_capacity(jobs.jobs.len());

        for job in &jobs.jobs {
            let mut seen = BTreeSet::new();
            let mut dependencies = Vec::with_capacity(job.job_dependencies.len());

            for dependency in &job.job_dependencies {
                if dependency.job_key.trim().is_empty()
                    || dependency.job_key.trim() != dependency.job_key.as_str()
                {
                    return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                        builder_name: builder.name,
                        source_path: source_path.to_string(),
                        reason: format!(
                            "job `{}` dependency job key must be non-empty and trimmed",
                            job.job_key
                        ),
                    });
                }
                if dependency.platform.trim() != dependency.platform.as_str() {
                    return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                        builder_name: builder.name,
                        source_path: source_path.to_string(),
                        reason: format!(
                            "job `{}` dependency platform must be trimmed",
                            job.job_key
                        ),
                    });
                }

                let dependency_source = match &dependency.source {
                    SourceFileDependency::Path(path) => {
                        let Some(path) = validate_asset_db_relative_path(path) else {
                            return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                                builder_name: builder.name,
                                source_path: source_path.to_string(),
                                reason: format!(
                                    "job `{}` dependency path `{path}` is not an asset-db relative path",
                                    job.job_key
                                ),
                            });
                        };
                        path
                    }
                    SourceFileDependency::Uuid(uuid) => {
                        if uuid.is_nil() {
                            return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                                builder_name: builder.name,
                                source_path: source_path.to_string(),
                                reason: format!(
                                    "job `{}` dependency UUID must not be nil",
                                    job.job_key
                                ),
                            });
                        }
                        format!("uuid:{uuid}")
                    }
                };
                let kind = match dependency.kind {
                    JobDependencyType::Order => DbCoupling::Order,
                    JobDependencyType::Fingerprint => DbCoupling::Fingerprint,
                    JobDependencyType::OrderOnly => DbCoupling::OrderOnly,
                };
                let effective_dependency_platform = if dependency.platform.is_empty() {
                    job.platform.as_str()
                } else {
                    dependency.platform.as_str()
                };
                if dependency_source == source_path
                    && dependency.job_key == job.job_key.as_str()
                    && effective_dependency_platform == job.platform.as_str()
                {
                    return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                        builder_name: builder.name,
                        source_path: source_path.to_string(),
                        reason: format!("job `{}` must not depend on itself", job.job_key),
                    });
                }

                if !seen.insert((
                    dependency_source.clone(),
                    dependency.job_key.clone(),
                    dependency.platform.clone(),
                    kind.as_i64(),
                )) {
                    return Err(AssetProcessorError::InvalidBuilderCreateJobs {
                        builder_name: builder.name,
                        source_path: source_path.to_string(),
                        reason: format!(
                            "job `{}` has duplicate dependency on `{}` job `{}` platform `{}`",
                            job.job_key, dependency_source, dependency.job_key, dependency.platform
                        ),
                    });
                }
                dependencies.push(ValidatedJobDependency {
                    dependency_source,
                    dependency_job_key: dependency.job_key.clone(),
                    dependency_platform: dependency.platform.clone(),
                    kind,
                });
            }

            validated.push(dependencies);
        }

        Ok(validated)
    }

    fn processing_status_subscription(&self) -> az_assetdb::AssetProcessingStatusSubscription {
        self.databases
            .dispatch()
            .subscribe_asset_processing_status()
    }

    fn current_unix_ms() -> Result<i64, AssetProcessorError> {
        current_unix_ms_i64().map_err(AssetProcessorError::Clock)
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn recover_expired_leases_at_startup(&self) -> Result<(), AssetProcessorError> {
        let now = Self::current_unix_ms()?;
        let db = self.dispatch_db();
        let mut after = 0;
        let mut recovered = 0usize;
        loop {
            let page = db.expired_attempts(now, after, 256)?;
            if page.is_empty() {
                break;
            }
            after = page.last().expect("non-empty page").attempt_id;
            let attempts = page
                .into_iter()
                .filter_map(|attempt| {
                    attempt.owner.map(|owner| AttemptFence {
                        attempt_id: attempt.attempt_id,
                        owner,
                    })
                })
                .collect::<Vec<_>>();
            if attempts.is_empty() {
                continue;
            }
            let result = self
                .asset_db_writer()
                .abandon_attempts(AbandonAttempts {
                    attempts,
                    finished: now,
                })
                .await?;
            recovered += result.requeued.len() + result.exhausted.len();
        }
        drop(db);
        if recovered != 0 {
            info!(
                recovered,
                "asset dispatcher recovered leases from a previous processor process"
            );
        }
        Ok(())
    }

    fn validate_lease_admission(
        &self,
        request: &LeaseAssetJobRequest,
    ) -> Result<LeaseEnvelope, AssetProcessorError> {
        validate_jobs_capability(&request.capability, self.capability_grants())?;
        validate_lease_owner(&request.lease_owner)?;
        validate_optional_staging_root(request.staging_root.as_deref())?;
        let staging_root = request.staging_root.as_ref().ok_or(
            AssetProcessorError::MissingSourcePayloadStagingRoot {
                asset_job_attempt_id: 0,
            },
        )?;
        Ok(LeaseEnvelope::new(
            LeaseRequest::validated(
                request.lease_owner.clone(),
                request.lease_duration_ms,
                PathBuf::from(staging_root),
            )?,
            PayloadAuthority::validated(request.capability.clone()),
        ))
    }

    fn validate_renewal_admission(
        &self,
        request: &RenewAssetJobLeaseRequest,
    ) -> Result<(), AssetProcessorError> {
        validate_jobs_capability(&request.capability, self.capability_grants())?;
        validate_renew_asset_job_lease_request(request)?;
        let db = self.databases.query();
        validate_asset_job_attempt_scope(
            &db,
            &request.capability,
            self.workspace_id,
            request.asset_job_attempt_id,
            "renew asset job lease",
        )
    }

    /// Claims one ready job directly, bypassing the dispatcher's parking and
    /// grant bookkeeping. Production always claims through the dispatcher, so
    /// the only callers of this single-shot form are the dispatcher's own
    /// tests; it stays compiled outside them because it is the crate's only
    /// use of the `AssetProcessorQueue` claim path.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "single-shot claim path used only by dispatcher tests"
        )
    )]
    #[instrument(level = "trace", skip(self, envelope), fields(lease_owner = %envelope.request().owner()))]
    async fn claim_lease_once(
        &self,
        envelope: &LeaseEnvelope,
    ) -> Result<Option<LeasedAssetJobPreparation>, AssetProcessorError> {
        let request = envelope.request();
        let db = self.dispatch_db();
        let queue = AssetProcessorQueue::new(Rc::clone(&db), self.asset_db_writer().clone());
        let workspace_pk = self.attached_workspace_id()?;
        let staging = request.staging_root().to_string_lossy().into_owned();
        for kind in [DbWork::Plan, DbWork::Build] {
            for job in queue.ready_page(workspace_pk, kind, 0, 64)? {
                let claimed = queue
                    .claim(ClaimReadyJob {
                        job_id: job.job_id,
                        expected_attempts: job.attempts,
                        owner: request.owner().to_string(),
                        lease_duration_ms: request.duration_ms(),
                        staging: staging.clone(),
                    })
                    .await?;
                let ClaimReadyJobResult::Claimed { context } = claimed else {
                    continue;
                };
                let attempt_id = context.attempt.attempt_id;
                return match prepare_leased_asset_job_attempt(*context, envelope) {
                    Ok(preparation) => Ok(Some(preparation)),
                    Err(error) => {
                        let result = queue
                            .abandon(AbandonAttempts {
                                attempts: vec![AttemptFence {
                                    attempt_id,
                                    owner: request.owner().to_string(),
                                }],
                                finished: Self::current_unix_ms()?,
                            })
                            .await?;
                        drop(queue);
                        warn!(
                            attempt_id,
                            requeued = result.requeued.len(),
                            exhausted = result.exhausted.len(),
                            error = %error,
                            "asset processor abandoned a claim whose payload could not be staged"
                        );
                        Err(error)
                    }
                };
            }
        }
        Ok(None)
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn expire_lease(
        &self,
        asset_job_attempt_id: i64,
        lease_owner: &str,
        finished_unix_ms: i64,
    ) -> Result<bool, AssetProcessorError> {
        let result = self
            .asset_db_writer()
            .abandon_attempts(AbandonAttempts {
                attempts: vec![AttemptFence {
                    attempt_id: asset_job_attempt_id,
                    owner: lease_owner.to_owned(),
                }],
                finished: finished_unix_ms,
            })
            .await?;
        Ok(!result.requeued.is_empty() || !result.exhausted.is_empty())
    }

    /// # Errors
    ///
    /// Returns any error [`Self::complete_attempt_async`] returns; this is the
    /// blocking wrapper around it.
    #[cfg(test)]
    pub fn complete_attempt(
        &self,
        request: &CompleteAssetJobAttemptRequest,
    ) -> Result<bool, AssetProcessorError> {
        block_on_test_runtime(self.complete_attempt_async(request))
    }

    /// # Errors
    ///
    /// Returns [`AssetProcessorError::InvalidCapability`] if the request's capability is not valid for this operation, the
    /// `MissingProductManifest*` variants if a successful completion omits its
    /// product-manifest side channel or its capability, the `StagedProduct*` variants
    /// if a staged product escapes the staging root or does not match the manifest's
    /// length or hash, the `ProductCache*` variants if promotion or its compensation
    /// fails, and [`AssetProcessorError::Repo`] if an `AssetDB` query fails.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[cfg(test)]
    pub async fn complete_attempt_async(
        &self,
        request: &CompleteAssetJobAttemptRequest,
    ) -> Result<bool, AssetProcessorError> {
        self.validate_completion_admission(request)?;
        let prepared = self.prepare_attempt_completion(request).await?;
        match self.commit_prepared_attempt_completion(prepared).await? {
            DurableAttemptCompletion::NoLongerOwned => Ok(false),
            DurableAttemptCompletion::Committed(post_commit) => {
                if let Some(post_commit) = post_commit
                    && let Err(error) = self.post_commit_attempt_completion(*post_commit).await
                {
                    tracing::error!(
                        %error,
                        "asset completion post-commit consequence failed after durable commit"
                    );
                }
                Ok(true)
            }
        }
    }

    fn validate_completion_admission(
        &self,
        request: &CompleteAssetJobAttemptRequest,
    ) -> Result<(), AssetProcessorError> {
        validate_jobs_capability(&request.capability, self.capability_grants())?;
        validate_worker_attempt_request(
            request.asset_job_attempt_id,
            &request.lease_owner,
            request.finished_unix_ms,
            "complete asset job attempt",
        )?;
        if request.grant_key.is_nil() {
            return Err(invalid_worker_job_request("grant key must not be nil"));
        }
        {
            let db = self.databases.query();
            validate_asset_job_attempt_scope(
                &db,
                &request.capability,
                self.workspace_id,
                request.asset_job_attempt_id,
                "complete asset job attempt",
            )?;
            drop(db);
        }
        Ok(())
    }

    /// Prepares the completion of an attempt the worker reported as succeeded.
    ///
    /// A planner job expands into a plan delta and commits directly. A build job
    /// has its product manifest loaded and its products validated first, because
    /// committing it promotes those bytes into the product cache and that is the
    /// last point at which a bad manifest can still be refused.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn prepare_succeeded_completion(
        &self,
        request: &CompleteAssetJobAttemptRequest,
        status: DbStatus,
        attempt: SelectAttempts,
        job: SelectJobs,
        workspace: SelectWorkspaces,
    ) -> Result<PreparedAttemptCompletion, AssetProcessorError> {
        let manifest_handle = request
            .product_manifest
            .as_ref()
            .ok_or(AssetProcessorError::MissingProductManifest)?;
        validate_product_manifest_side_channel_capability(manifest_handle, &request.capability)?;
        validate_product_manifest_side_channel_scope(manifest_handle, &attempt)?;
        let manifest_started = Instant::now();
        let manifest_handle = manifest_handle.clone();
        let attempt_id = attempt.attempt_id;
        let manifest = tokio::task::spawn_blocking(move || {
            load_product_manifest_side_channel(&manifest_handle)
        })
        .await
        .map_err(|error| AssetProcessorError::DispatcherCompletionTask { attempt_id, error })??;
        let manifest_elapsed = manifest_started.elapsed();

        // Planner jobs expand into real create_jobs plans; they never
        // promote product bytes into the cache.
        if job.kind == DbWork::Plan {
            let plan_delta = self
                .planner_job_manifest_delta(&job, &attempt, &manifest)
                .await?;
            return Ok(PreparedAttemptCompletion::Direct {
                command: Box::new(CompleteAttempt {
                    attempt_id: request.asset_job_attempt_id,
                    owner: request.lease_owner.clone(),
                    status,
                    finished: request.finished_unix_ms,
                    errors: request.error_count,
                    warnings: request.warning_count,
                    products: Vec::new(),
                    job_edges: None,
                    plan_delta: Some(plan_delta),
                }),
                status: request.status,
            });
        }

        let product_formats = self
            .published_catalog
            .borrow()
            .as_ref()
            .map(|catalog| catalog.product_formats.clone())
            .unwrap_or_default();
        let prepare_attempt = attempt.clone();
        let prepare_manifest = manifest.clone();
        let registries = self.registries;
        let attempt_id = attempt.attempt_id;
        let products = tokio::task::spawn_blocking(move || {
            prepare_product_inputs(
                &prepare_attempt,
                &prepare_manifest,
                &product_formats,
                registries,
            )
        })
        .await
        .map_err(|error| AssetProcessorError::DispatcherCompletionTask { attempt_id, error })??;
        let project_data_paths = self.project_data_paths()?;
        let product_cache_root =
            product_cache_root_for_job(project_data_paths, &workspace, &job, attempt.attempt_id)?;
        let generated_rust_projection_affected =
            generated_rust_graph_projection_affected(&products);
        let owned_products = products
            .iter()
            .map(|product| product.as_input(job.asset_pk, &job.platform))
            .collect::<Vec<_>>();
        Ok(PreparedAttemptCompletion::Build(Box::new(
            BuildAttemptCompletion {
                command: CompleteAttempt {
                    attempt_id: request.asset_job_attempt_id,
                    owner: request.lease_owner.clone(),
                    status,
                    finished: request.finished_unix_ms,
                    errors: request.error_count,
                    warnings: request.warning_count,
                    products: owned_products,
                    job_edges: None,
                    plan_delta: None,
                },
                status: request.status,
                attempt_id: attempt.attempt_id,
                workspace,
                job,
                project_data_paths: project_data_paths.clone(),
                product_cache_root,
                products,
                generated_rust_projection_affected,
                manifest_elapsed,
            },
        )))
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn prepare_attempt_completion(
        &self,
        request: &CompleteAssetJobAttemptRequest,
    ) -> Result<PreparedAttemptCompletion, AssetProcessorError> {
        let status = completion_status_to_db(request.status)?;
        let (attempt, job, workspace) = {
            let db = self.databases.query();
            let Some(attempt) = db.attempt_by_id(request.asset_job_attempt_id)? else {
                return Ok(PreparedAttemptCompletion::NoLongerOwned);
            };
            let job = db.job_by_id(attempt.job_pk)?.ok_or(
                AssetProcessorError::MissingAssetJobAttempt {
                    asset_job_attempt_id: attempt.attempt_id,
                },
            )?;
            let workspace = db.workspace_by_id(job.workspace_pk)?.ok_or(
                AssetProcessorError::MissingProductCacheWorkspace {
                    asset_job_attempt_id: attempt.attempt_id,
                    workspace_id: job.workspace_pk,
                },
            )?;
            drop(db);
            (attempt, job, workspace)
        };
        match status {
            DbStatus::Succeeded => {
                self.prepare_succeeded_completion(request, status, attempt, job, workspace)
                    .await
            }
            status @ (DbStatus::Failed | DbStatus::Abandoned) => {
                Ok(PreparedAttemptCompletion::Direct {
                    command: Box::new(CompleteAttempt {
                        attempt_id: request.asset_job_attempt_id,
                        owner: request.lease_owner.clone(),
                        status,
                        finished: request.finished_unix_ms,
                        errors: request.error_count,
                        warnings: request.warning_count,
                        products: Vec::new(),
                        job_edges: None,
                        plan_delta: None,
                    }),
                    status: request.status,
                })
            }
            _ => unreachable!("completion_status_to_db only returns terminal worker statuses"),
        }
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn commit_prepared_attempt_completion(
        &self,
        prepared: PreparedAttemptCompletion,
    ) -> Result<DurableAttemptCompletion, AssetProcessorError> {
        let (result, status, post_commit) = match prepared {
            PreparedAttemptCompletion::NoLongerOwned => {
                return Ok(DurableAttemptCompletion::NoLongerOwned);
            }
            PreparedAttemptCompletion::Direct { command, status } => (
                self.asset_db_writer().complete_attempt(*command).await?,
                status,
                None,
            ),
            PreparedAttemptCompletion::Build(build) => {
                let BuildAttemptCompletion {
                    command,
                    status,
                    attempt_id,
                    workspace,
                    job,
                    project_data_paths,
                    product_cache_root,
                    products,
                    generated_rust_projection_affected,
                    manifest_elapsed,
                } = *build;
                // This guard spans provisional filesystem promotion through
                // the durable writer answer. A later local completion cannot
                // overwrite a preimage that this receipt may need to restore.
                let _promotion_gate = self.product_promotion_gate.lock().await;
                let promote_started = Instant::now();
                let promote_paths = project_data_paths.clone();
                let promote_root = product_cache_root.clone();
                let promote_products = products;
                let promotion = tokio::task::spawn_blocking(move || {
                    promote_products_to_cache(&promote_paths, &promote_root, &promote_products)
                })
                .await
                .map_err(|error| {
                    AssetProcessorError::DispatcherCompletionTask { attempt_id, error }
                })??;
                let promote_elapsed = promote_started.elapsed();
                let submit_started = Instant::now();
                let result = match self.asset_db_writer().complete_attempt(command).await {
                    Ok(CompleteAttemptResult::NoLongerOwned) => {
                        if let Err(rollback) =
                            compensate_product_promotion(attempt_id, promotion).await
                        {
                            return Err(AssetProcessorError::ProductCacheNoLongerOwnedRollback {
                                attempt_id,
                                rollback: Box::new(rollback),
                            });
                        }
                        return Ok(DurableAttemptCompletion::NoLongerOwned);
                    }
                    Ok(result) => result,
                    Err(writer_error) => {
                        if let Err(rollback) =
                            compensate_product_promotion(attempt_id, promotion).await
                        {
                            return Err(completion_writer_rollback_error(writer_error, rollback));
                        }
                        return Err(writer_error.into());
                    }
                };
                let generated_rust_projection_affected = generated_rust_projection_affected
                    || matches!(
                        &result,
                        CompleteAttemptResult::Completed {
                            replaced_product_formats,
                            ..
                        } if replaced_product_formats
                            .contains(GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID.as_str())
                    );
                let post_commit = PostCommitAttemptCompletion {
                    attempt_id,
                    promotion,
                    workspace,
                    job,
                    project_data_paths,
                    product_cache_root,
                    generated_rust_projection_affected,
                    manifest_elapsed,
                    promote_elapsed,
                    submit_elapsed: submit_started.elapsed(),
                    #[cfg(test)]
                    fail_for_test: false,
                };
                (result, status, Some(Box::new(post_commit)))
            }
        };
        let completed = !matches!(result, CompleteAttemptResult::NoLongerOwned);
        trace!(
            completed,
            status = ?status,
            "asset processor complete-attempt completed"
        );
        Ok(if completed {
            DurableAttemptCompletion::Committed(post_commit)
        } else {
            DurableAttemptCompletion::NoLongerOwned
        })
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn post_commit_attempt_completion(
        &self,
        post_commit: PostCommitAttemptCompletion,
    ) -> Result<(), AssetProcessorError> {
        let PostCommitAttemptCompletion {
            attempt_id,
            promotion,
            workspace,
            job,
            project_data_paths,
            product_cache_root,
            generated_rust_projection_affected,
            manifest_elapsed,
            promote_elapsed,
            submit_elapsed,
            #[cfg(test)]
            fail_for_test,
        } = post_commit;
        if let Some(promotion) = promotion {
            tokio::task::spawn_blocking(move || promotion.finalize())
                .await
                .map_err(|error| AssetProcessorError::DispatcherCompletionTask {
                    attempt_id,
                    error,
                })??;
        }
        #[cfg(test)]
        if fail_for_test {
            return Err(AssetProcessorError::DispatcherInitialization {
                reason: "injected generated-Rust projection failure".to_owned(),
            });
        }
        if generated_rust_projection_affected {
            let entries =
                generated_rust_graph_source_paths(&self.databases.query(), &workspace, &job)?;
            tokio::task::spawn_blocking(move || {
                sync_generated_rust_graph_sources(
                    &entries,
                    &workspace,
                    &project_data_paths,
                    &product_cache_root,
                )
            })
            .await
            .map_err(|error| AssetProcessorError::DispatcherCompletionTask {
                attempt_id,
                error,
            })??;
        }
        if let Some((manifest_us, promote_us, submit_us, max_us)) =
            self.complete_rpc_stats
                .record(manifest_elapsed, promote_elapsed, submit_elapsed)
        {
            info!(
                sample = WORKER_RPC_STATS_SAMPLE,
                avg_manifest_us = manifest_us,
                avg_promote_us = promote_us,
                avg_submit_us = submit_us,
                max_total_us = max_us,
                "asset job complete rpc timing"
            );
        }
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn planner_job_manifest_delta(
        &self,
        job: &SelectJobs,
        attempt: &SelectAttempts,
        manifest: &ProductManifest,
    ) -> Result<PlanDelta, AssetProcessorError> {
        let blocking_attempt = attempt.clone();
        let blocking_manifest = manifest.clone();
        let registries = self.registries;
        let attempt_id = attempt.attempt_id;
        let plan = tokio::task::spawn_blocking(move || {
            decode_planner_job_manifest(&blocking_attempt, &blocking_manifest, registries)
        })
        .await
        .map_err(|error| AssetProcessorError::DispatcherCompletionTask { attempt_id, error })??;
        let db = self.databases.query();
        let mut delta = PlanDelta {
            retire_job_ids: db
                .jobs_for_asset(job.workspace_pk, job.asset_pk)?
                .into_iter()
                .filter(|candidate| candidate.kind == DbWork::Build)
                .map(|candidate| candidate.job_id)
                .collect(),
            retire_source_edge_ids: db
                .source_edges_for_asset(job.workspace_pk, job.asset_pk)?
                .into_iter()
                .map(|edge| edge.source_edge_id)
                .collect(),
            ..PlanDelta::default()
        };
        drop(db);
        for builder_plan in plan.builders {
            for dependency in builder_plan.source_dependencies {
                delta.source_edges.push(SourceEdgeInput {
                    builder: builder_plan.builder_guid,
                    asset_pk: job.asset_pk,
                    depends_pk: None,
                    target: DbTarget::path(dependency).map_err(|error| {
                        AssetProcessorError::InvalidProductManifest {
                            reason: error.to_string(),
                        }
                    })?,
                    relation: DbRelation::SourceToSource,
                });
            }
            for planned in builder_plan.jobs {
                delta.replacements.push(PlannedJob::build(
                    job.asset_pk,
                    builder_plan.builder_guid,
                    planned.job_key,
                    planned.platform,
                    Vec::new(),
                ));
            }
        }
        Ok(delta)
    }

    fn event_snapshot_for_attempt(
        &self,
        asset_job_attempt_id: i64,
    ) -> Result<Option<WorkspaceEntry>, AssetProcessorError> {
        let db = self.databases.query();
        let Some(attempt) = db.attempt_by_id(asset_job_attempt_id)? else {
            return Ok(None);
        };
        let job =
            db.job_by_id(attempt.job_pk)?
                .ok_or(AssetProcessorError::MissingAssetJobAttempt {
                    asset_job_attempt_id,
                })?;
        let asset = db.asset_by_id(job.asset_pk)?.ok_or(
            AssetProcessorError::MissingAssetJobWorkspaceEntry {
                asset_job_attempt_id,
                workspace_id: job.workspace_pk,
                asset_identity_id: job.asset_pk,
            },
        )?;
        let entry = db.entry_by_asset(job.workspace_pk, job.asset_pk)?.ok_or(
            AssetProcessorError::MissingAssetJobWorkspaceEntry {
                asset_job_attempt_id,
                workspace_id: job.workspace_pk,
                asset_identity_id: job.asset_pk,
            },
        )?;
        let entry = workspace_asset_entry_to_proto(&db, &asset, entry)?;
        drop(db);
        Ok(Some(entry))
    }
}

fn decode_planner_job_manifest(
    attempt: &SelectAttempts,
    manifest: &ProductManifest,
    registries: &Registries,
) -> Result<WorkerCreateJobsPlan, AssetProcessorError> {
    let [plan_product] = manifest.products.as_slice() else {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "planner job attempt {} must complete with exactly one `{ASSET_JOB_PLAN_PRODUCT_FORMAT}` product",
                attempt.attempt_id,
            ),
        });
    };
    if plan_product.product_path != ASSET_JOB_PLAN_PRODUCT_PATH
        || plan_product.asset_type != ASSET_JOB_PLAN_ASSET_TYPE
        || plan_product.sub_id != 0
        || plan_product.product_format != ASSET_JOB_PLAN_PRODUCT_FORMAT
        || plan_product.product_format_version != 1
        || !plan_product.dependencies.is_empty()
    {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "planner job attempt {} returned an invalid planner control product identity",
                attempt.attempt_id,
            ),
        });
    }
    let staging_root =
        attempt
            .staging
            .as_deref()
            .ok_or_else(|| AssetProcessorError::InvalidProductManifest {
                reason: "planner attempt has a product but no staging root".to_string(),
            })?;
    let staging_root = Path::new(staging_root);
    validate_staging_root(staging_root)?;
    let prepared_product = prepare_product_input(staging_root, plan_product, &[], registries)?;
    let plan_bytes = std::fs::read(&prepared_product.staged_file_path).map_err(|source| {
        AssetProcessorError::StagedProductRead {
            product_path: plan_product.product_path.clone(),
            path: prepared_product.staged_file_path.clone(),
            source,
        }
    })?;
    serde_json::from_slice(&plan_bytes).map_err(|source| {
        AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "planner job attempt {} plan JSON decode failed: {source}",
                attempt.attempt_id
            ),
        }
    })
}

fn write_source_payload_side_channel(
    capability: &Capability,
    asset_job_attempt_id: i64,
    staging_root: &Path,
    bytes: &[u8],
) -> Result<SideChannelHandle, AssetProcessorError> {
    let path = staging_root.join(format!("attempt-{asset_job_attempt_id}-source.ron"));
    let written = write_named_staging_file_atomic(&path, bytes).map_err(|error| {
        AssetProcessorError::WriteSourcePayload {
            asset_job_attempt_id,
            path: error.path,
            source: error.source,
        }
    })?;
    Ok(SideChannelHandle::staging_file(
        written.path.to_string_lossy(),
        written.byte_length,
        written.content_hash,
        std::env::consts::OS,
    )
    .with_capability(capability.clone()))
}

struct SourceFileStagingCleanup {
    path: PathBuf,
}

impl SourceFileStagingCleanup {
    const fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for SourceFileStagingCleanup {
    fn drop(&mut self) {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => warn!(
                path = %self.path.display(),
                %error,
                "failed to consume structured source-file staging file"
            ),
        }
    }
}

struct ResolvedSourceFile {
    source: WorkspaceSourceFileRef,
    path: PathBuf,
}

/// What a codec pass produced for one source file.
struct SourceFileEdit {
    /// The bytes on disk the edit was fenced against, kept so a failed
    /// publication can put the file back exactly as it was.
    authoritative_bytes: Vec<u8>,
    /// The bytes the codec wants written in their place.
    saved_bytes: Vec<u8>,
    document: az_proto_asset::SourceFileEditDocument,
}

#[derive(Debug)]
struct CompletedSourceFileTransaction {
    snapshot: SourceFileEditSnapshot,
}

type SourceFileTransactionOutcome = Result<CompletedSourceFileTransaction, AssetProcessorError>;

fn commit_source_file_publication(
    transaction_root: &Path,
    target: &Path,
    bytes: Vec<u8>,
) -> Result<(), AssetProcessorError> {
    FileTransaction::new(transaction_root.to_path_buf())
        .recover_pending()
        .map_err(
            |source| AssetProcessorError::SourceFileTransactionRecovery {
                root: transaction_root.to_path_buf(),
                source,
            },
        )?;
    FileTransaction::new(transaction_root.to_path_buf())
        .commit([FileWrite::new(target.to_path_buf(), bytes)])
        .map_err(|source| AssetProcessorError::SourceFileTransaction {
            root: transaction_root.to_path_buf(),
            source,
        })?;

    Ok(())
}

fn compensate_source_file_publication(
    transaction_root: &Path,
    target: &Path,
    bytes: Vec<u8>,
) -> Result<(), az_filesystem::FileTransactionError> {
    FileTransaction::new(transaction_root.to_path_buf())
        .commit([FileWrite::new(target.to_path_buf(), bytes)])?;
    Ok(())
}

pub struct AssetProcessorRpc {
    processor: Rc<AssetProcessor>,
    job_dispatcher: Rc<RefCell<Option<AssetJobDispatcherOwner>>>,
    source_file_codec: Rc<RefCell<Option<ActiveSourceFileCodec>>>,
    event_subscribers: Rc<RefCell<Vec<AssetProcessorEventSubscriber>>>,
    consequence_health: AssetProcessorConsequenceHealth,
    next_event_seq: Rc<Cell<u64>>,
    next_subscriber_id: Rc<Cell<u64>>,
    connection_id: Uuid,
    service_run: Uuid,
    started_at: Instant,
    db_path: Option<PathBuf>,
    source_coordination: AssetSourceServiceCoordination,
}

impl Drop for AssetProcessorRpc {
    fn drop(&mut self) {
        if let Some(dispatcher) = self.job_dispatcher.borrow().as_ref() {
            dispatcher
                .dispatcher()
                .disconnect_connection(self.connection_id);
        }
    }
}

#[derive(Clone)]
struct ActiveSourceFileCodec {
    client: asset_capnp::source_file_codec::Client,
    capability: Capability,
}

#[derive(Clone)]
pub(crate) struct AssetSourceServiceCoordination {
    builder_catalog: Arc<Mutex<Option<AssetBuilderCatalogResult>>>,
    sweep: Arc<Mutex<Option<SweepHandle>>>,
}

impl AssetSourceServiceCoordination {
    fn in_process() -> Self {
        Self {
            builder_catalog: Arc::new(Mutex::new(None)),
            sweep: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn production() -> Self {
        Self {
            builder_catalog: Arc::new(Mutex::new(None)),
            sweep: Arc::new(Mutex::new(None)),
        }
    }

    fn publish_builder_catalog(
        &self,
        catalog: AssetBuilderCatalogResult,
    ) -> Result<(), AssetProcessorError> {
        {
            let mut published = self
                .builder_catalog
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *published = Some(catalog);
        }
        if let Some(sweep) = self.sweep_handle() {
            sweep.catalog_changed()?;
        }
        Ok(())
    }

    pub(crate) fn builder_catalog_store(&self) -> Arc<Mutex<Option<AssetBuilderCatalogResult>>> {
        Arc::clone(&self.builder_catalog)
    }

    pub(crate) fn install_sweep(&self, sweep: SweepHandle) {
        *self
            .sweep
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sweep);
    }

    pub(crate) fn sweep_handle(&self) -> Option<SweepHandle> {
        self.sweep
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Default for AssetSourceServiceCoordination {
    fn default() -> Self {
        Self::in_process()
    }
}

struct AssetProcessorEventSubscriber {
    id: u64,
    workspace_id: i64,
    sink: asset_capnp::asset_processor_event_sink::Client,
    /// Latest event not yet delivered. Updates coalesce while the sink has
    /// one request in flight; delivery is therefore ordered by the events
    /// actually observed, never by a task per producer event.
    pending: Option<AssetProcessorEvent>,
    in_flight: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum AssetProcessorConsequenceFault {
    PostCommit {
        attempt_id: i64,
        reason: String,
    },
    JobCompletedProjection {
        attempt_id: i64,
        reason: String,
    },
    CatalogPublication {
        workspace_id: i64,
        platform: String,
        reason: String,
    },
}

impl fmt::Display for AssetProcessorConsequenceFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PostCommit { attempt_id, reason } => {
                write!(
                    f,
                    "attempt {attempt_id} post-commit consequence failed: {reason}"
                )
            }
            Self::JobCompletedProjection { attempt_id, reason } => {
                write!(
                    f,
                    "attempt {attempt_id} completed-event projection failed: {reason}"
                )
            }
            Self::CatalogPublication {
                workspace_id,
                platform,
                reason,
            } => {
                write!(
                    f,
                    "catalog publication for workspace {workspace_id} platform `{platform}` failed: {reason}"
                )
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AssetProcessorConsequenceHealth {
    state: Arc<Mutex<AssetProcessorConsequenceHealthState>>,
}

#[derive(Debug, Default)]
struct AssetProcessorConsequenceHealthState {
    fault_count: u64,
    latest: Option<AssetProcessorConsequenceFault>,
}

impl AssetProcessorConsequenceHealth {
    pub(crate) fn record(&self, fault: AssetProcessorConsequenceFault) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.fault_count = state.fault_count.saturating_add(1);
        state.latest = Some(fault);
    }

    fn snapshot(&self) -> Option<(u64, AssetProcessorConsequenceFault)> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some((state.fault_count, state.latest.clone()?))
    }
}

#[derive(Clone)]
pub(crate) struct AssetProcessorEventPublisher {
    subscribers: Rc<RefCell<Vec<AssetProcessorEventSubscriber>>>,
    next_event_seq: Rc<Cell<u64>>,
    consequence_health: AssetProcessorConsequenceHealth,
}

impl AssetProcessorEventPublisher {
    fn record_fault(&self, fault: AssetProcessorConsequenceFault) {
        self.consequence_health.record(fault);
    }

    fn publish(&self, kind: AssetProcessorEventKind, event_unix_ms: i64, entry: WorkspaceEntry) {
        let seq = self.next_event_seq.get();
        self.next_event_seq.set(seq.saturating_add(1).max(1));
        let event = AssetProcessorEvent {
            seq,
            kind,
            event_unix_ms,
            entry,
        };
        let newly_armed = {
            let mut subscribers = self.subscribers.borrow_mut();
            subscribers
                .iter_mut()
                .filter(|subscriber| subscriber.workspace_id == event.entry.workspace_id)
                .filter_map(|subscriber| subscriber.enqueue(event.clone()).then_some(subscriber.id))
                .collect::<Vec<_>>()
        };
        for id in newly_armed {
            let publisher = self.clone();
            tokio::task::spawn_local(async move {
                publisher.drain_subscriber(id).await;
            });
        }
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn drain_subscriber(&self, id: u64) {
        loop {
            let Some((sink, event)) = ({
                let mut subscribers = self.subscribers.borrow_mut();
                let Some(subscriber) = subscribers
                    .iter_mut()
                    .find(|subscriber| subscriber.id == id)
                else {
                    return;
                };
                let Some(event) = subscriber.pending.take() else {
                    subscriber.in_flight = false;
                    return;
                };
                Some((subscriber.sink.clone(), event))
            }) else {
                return;
            };
            let mut request = sink.update_request();
            if let Err(err) = event.to_capnp(request.get().init_event()) {
                warn!(subscriber = id, error = %err, "failed to encode asset processor event; dropping subscription");
                self.subscribers
                    .borrow_mut()
                    .retain(|subscriber| subscriber.id != id);
                return;
            }
            if let Err(err) = request.send().promise.await {
                warn!(subscriber = id, error = %err, "asset processor event subscriber rejected update; dropping subscription");
                self.subscribers
                    .borrow_mut()
                    .retain(|subscriber| subscriber.id != id);
                return;
            }
        }
    }
}

impl AssetProcessorEventSubscriber {
    /// Returns true exactly when the caller must start this subscriber's sole
    /// delivery drain. Later events overwrite the one pending slot.
    fn enqueue(&mut self, event: AssetProcessorEvent) -> bool {
        self.pending = Some(event);
        !std::mem::replace(&mut self.in_flight, true)
    }
}

impl fmt::Debug for AssetProcessorRpc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetProcessorRpc")
            .field("processor", &self.processor)
            .field(
                "event_subscriber_count",
                &self.event_subscribers.borrow().len(),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(any(test, feature = "test-support"))]
fn only_workspace_id(db: &AssetDb) -> i64 {
    let workspaces = db.workspaces_for_test().expect("workspaces");
    assert_eq!(
        workspaces.len(),
        1,
        "an in-process asset-processor test harness must bind one workspace"
    );
    workspaces[0].workspace_id
}

impl AssetProcessorRpc {
    #[must_use]
    pub(crate) fn new(processor: AssetProcessor) -> Self {
        let processor = Rc::new(processor);
        let consequence_health = processor.consequence_health.clone();
        Self {
            processor,
            job_dispatcher: Rc::new(RefCell::new(None)),
            source_file_codec: Rc::new(RefCell::new(None)),
            event_subscribers: Rc::new(RefCell::new(Vec::new())),
            consequence_health,
            next_event_seq: Rc::new(Cell::new(1)),
            next_subscriber_id: Rc::new(Cell::new(1)),
            connection_id: Uuid::now_v7(),
            // In-process/test RPC servers still expose the same health
            // contract as supervised servers, so they need a real run label.
            // Production launch replaces this with the supervisor-minted run.
            service_run: Uuid::now_v7(),
            started_at: Instant::now(),
            db_path: None,
            source_coordination: AssetSourceServiceCoordination::in_process(),
        }
    }

    pub(crate) fn for_connection(&self) -> Self {
        Self {
            processor: Rc::clone(&self.processor),
            job_dispatcher: Rc::clone(&self.job_dispatcher),
            source_file_codec: Rc::clone(&self.source_file_codec),
            event_subscribers: Rc::clone(&self.event_subscribers),
            consequence_health: self.consequence_health.clone(),
            next_event_seq: Rc::clone(&self.next_event_seq),
            next_subscriber_id: Rc::clone(&self.next_subscriber_id),
            connection_id: Uuid::now_v7(),
            service_run: self.service_run,
            started_at: self.started_at,
            db_path: self.db_path.clone(),
            source_coordination: self.source_coordination.clone(),
        }
    }

    #[must_use]
    pub(crate) fn with_source_service_coordination(
        mut self,
        coordination: AssetSourceServiceCoordination,
    ) -> Self {
        self.source_coordination = coordination;
        self
    }

    #[must_use]
    pub(crate) const fn with_service_run(mut self, run: Uuid) -> Self {
        self.service_run = run;
        self
    }

    #[must_use]
    pub(crate) fn with_db_path(mut self, db_path: impl Into<PathBuf>) -> Self {
        self.db_path = Some(db_path.into());
        self
    }

    /// # Panics
    ///
    /// Panics unless the database holds exactly one workspace and that
    /// workspace row is readable. A test harness binds one workspace up front,
    /// so either failure means the fixture was built wrong.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_db(db: AssetDb, capability_grants: CapabilityGrantSet) -> Self {
        let workspace_id = only_workspace_id(&db);
        let workspace = db
            .workspace_by_id(workspace_id)
            .expect("test workspace query")
            .expect("test workspace");
        let workspace_root = PathBuf::from(&workspace.root);
        let project_data_paths = AzothDataHome::new(workspace_root.join(".azoth-test-home"))
            .project(&workspace.project, &workspace_root);
        Self::new(
            AssetProcessor::new(db, workspace_id, project_data_paths, capability_grants)
                .expect("asset processor test database handles open"),
        )
    }

    #[cfg(test)]
    #[must_use]
    pub fn processor(&self) -> &AssetProcessor {
        self.processor.as_ref()
    }

    fn job_dispatcher(&self) -> AssetJobDispatcher {
        if let Some(owner) = self.job_dispatcher.borrow().as_ref() {
            return owner.dispatcher();
        }
        let owner = AssetJobDispatcherOwner::start(
            Rc::clone(&self.processor),
            self.source_coordination.clone(),
            self.event_publisher(),
        );
        let dispatcher = owner.dispatcher();
        *self.job_dispatcher.borrow_mut() = Some(owner);
        dispatcher
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn acquire_root_operation(
        &self,
        selector: &str,
    ) -> Result<Option<sweep::RootMutationPermit>, AssetProcessorError> {
        let Some(sweeps) = self.source_coordination.sweep_handle() else {
            return Ok(None);
        };
        let root = self.processor.sweep_root_for_selector(selector)?;
        sweeps.acquire_mutation(root).await.map(Some)
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn acquire_workspace_root_operation(
        &self,
        workspace_root_pk: i64,
    ) -> Result<Option<sweep::RootMutationPermit>, AssetProcessorError> {
        let Some(sweeps) = self.source_coordination.sweep_handle() else {
            return Ok(None);
        };
        sweeps
            .acquire_mutation(SweepRoot::workspace_root(workspace_root_pk))
            .await
            .map(Some)
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub(crate) async fn shutdown_job_dispatcher(&self) -> Result<(), AssetProcessorError> {
        let owner = self.job_dispatcher.borrow_mut().take();
        if let Some(owner) = owner {
            owner.shutdown().await?;
        }
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub(crate) async fn shutdown_catalog_publisher(&self) -> Result<(), AssetProcessorError> {
        self.processor.shutdown_catalog_publisher().await
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    pub(crate) async fn shutdown_owned_services(&self) -> Result<(), AssetProcessorError> {
        let dispatcher = self.shutdown_job_dispatcher().await;
        let catalog = self.shutdown_catalog_publisher().await;
        dispatcher.and(catalog)
    }

    fn active_source_file_codec(&self) -> Result<ActiveSourceFileCodec, AssetProcessorError> {
        self.source_file_codec
            .borrow()
            .clone()
            .ok_or(AssetProcessorError::SourceFileCodecUnavailable)
    }

    fn event_publisher(&self) -> AssetProcessorEventPublisher {
        AssetProcessorEventPublisher {
            subscribers: Rc::clone(&self.event_subscribers),
            next_event_seq: Rc::clone(&self.next_event_seq),
            consequence_health: self.consequence_health.clone(),
        }
    }

    /// Checks the shape of a source-file request before anything is resolved.
    ///
    /// Returns the canonical asset-db relative source path. Everything here is
    /// about the request itself, so a malformed one is rejected before it can
    /// reach the published catalog or the database.
    fn validate_source_file_ref(
        session_id: &str,
        source: &WorkspaceSourceFileRef,
    ) -> Result<String, AssetProcessorError> {
        parse_non_nil_session_uuid(session_id).map_err(|reason| {
            AssetProcessorError::InvalidSourceFileRequest {
                reason: format!("session id {reason}"),
            }
        })?;
        let source_path =
            validate_asset_db_relative_path(&source.source_path).ok_or_else(|| {
                AssetProcessorError::InvalidSourceFileRequest {
                    reason: format!(
                        "source path `{}` must be a canonical asset-db relative path",
                        source.source_path
                    ),
                }
            })?;
        if source_path.is_empty() || source_path != source.source_path {
            return Err(AssetProcessorError::InvalidSourceFileRequest {
                reason: format!(
                    "source path `{}` must be a canonical asset-db relative path",
                    source.source_path
                ),
            });
        }
        if source.source_root_key.trim().is_empty()
            || source.source_root_key.trim() != source.source_root_key
            || source
                .source_root_key
                .chars()
                .any(|character| matches!(character, '/' | '\\'))
        {
            return Err(AssetProcessorError::InvalidSourceFileRequest {
                reason: "source root key must be a non-empty canonical portable key".to_string(),
            });
        }
        if source.schema_type.trim().is_empty() || source.schema_type.trim() != source.schema_type {
            return Err(AssetProcessorError::InvalidSourceFileRequest {
                reason: "schema type must be non-empty and trimmed".to_string(),
            });
        }
        Ok(source_path)
    }

    fn resolve_source_file(
        &self,
        session_id: &str,
        source: &WorkspaceSourceFileRef,
        require_editable: bool,
    ) -> Result<ResolvedSourceFile, AssetProcessorError> {
        let source_path = Self::validate_source_file_ref(session_id, source)?;

        let catalog = self
            .processor
            .published_catalog
            .borrow()
            .clone()
            .ok_or(AssetProcessorError::BuilderCatalogUnavailable)?;
        let descriptor = catalog
            .source_schemas
            .iter()
            .find(|descriptor| descriptor.schema_type == source.schema_type)
            .ok_or_else(|| AssetProcessorError::SourceFileSchemaUnavailable {
                schema_type: source.schema_type.clone(),
            })?;
        let SourceSchemaAuthoring::File { workflow } = &descriptor.authoring else {
            return Err(AssetProcessorError::SourceFileSchemaNotFileBacked {
                schema_type: source.schema_type.clone(),
            });
        };
        if require_editable && !workflow.can_edit {
            return Err(AssetProcessorError::SourceFileSchemaNotEditable {
                schema_type: source.schema_type.clone(),
            });
        }
        if !source_path_matches_extensions(&source_path, &workflow.extensions) {
            return Err(AssetProcessorError::InvalidSourceFileRequest {
                reason: format!(
                    "source path `{source_path}` does not match schema extensions {:?}",
                    workflow.extensions
                ),
            });
        }

        let workspace_id = self.processor.attached_workspace_id()?;
        let (source_root, root) = {
            let db = self.processor.databases.query();
            let (_, source_root, root) = source_file_create_source_root(
                &db,
                workspace_id,
                session_id,
                &source.source_root_key,
            )?;
            (source_root, root)
        };
        let expected_key = if workflow.source_root == PROJECT_SOURCE_ROOT {
            PortableKey::project_assets(&source_root.owner)
                .as_str()
                .to_owned()
        } else {
            workflow.source_root.clone()
        };
        if root.key != expected_key {
            return Err(AssetProcessorError::InvalidSourceFileRequest {
                reason: format!(
                    "schema `{}` targets source root `{}`, not `{}`",
                    source.schema_type, workflow.source_root, source.source_root_key
                ),
            });
        }
        let path = source_file_absolute_path(&source_root.path, &source_path)?;
        Ok(ResolvedSourceFile {
            source: WorkspaceSourceFileRef {
                source_root_key: root.key,
                source_path,
                schema_type: source.schema_type.clone(),
            },
            path,
        })
    }

    fn stage_source_file(
        &self,
        capability: &Capability,
        bytes: &[u8],
    ) -> Result<(SideChannelHandle, SourceFileStagingCleanup), AssetProcessorError> {
        let path = self
            .processor
            .project_data_paths()?
            .derived_dir()
            .join("asset-processor")
            .join("source-file-staging")
            .join(format!("{}.source", uuid::Uuid::now_v7().as_simple()));
        let written = write_named_staging_file_atomic(&path, bytes).map_err(|error| {
            AssetProcessorError::SourceFileStage {
                path: error.path,
                source: error.source,
            }
        })?;
        let cleanup = SourceFileStagingCleanup::new(written.path.clone());
        let handle = SideChannelHandle::staging_file(
            written.path.to_string_lossy(),
            written.byte_length,
            written.content_hash,
            std::env::consts::OS,
        )
        .with_capability(capability.clone());
        Ok((handle, cleanup))
    }

    fn reserve_source_file_codec_output(
        &self,
        capability: &Capability,
    ) -> Result<(SourceFileCodecOutputDestination, SourceFileStagingCleanup), AssetProcessorError>
    {
        let path = self
            .processor
            .project_data_paths()?
            .derived_dir()
            .join("asset-processor")
            .join("source-file-staging")
            .join(format!(
                "{}-worker-output.source",
                uuid::Uuid::now_v7().as_simple()
            ));
        let cleanup = SourceFileStagingCleanup::new(path.clone());
        Ok((
            SourceFileCodecOutputDestination {
                locator: path.to_string_lossy().into_owned(),
                platform: std::env::consts::OS.to_string(),
                capability: capability.clone(),
            },
            cleanup,
        ))
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn dispatch_source_file_codec(
        &self,
        codec: &ActiveSourceFileCodec,
        request: &SourceFileCodecRequest,
    ) -> Result<SourceFileCodecResult, AssetProcessorError> {
        let mut rpc = codec.client.execute_request();
        request
            .to_capnp(rpc.get().init_request())
            .map_err(|error| AssetProcessorError::SourceFileCodecRpc {
                reason: error.to_string(),
            })?;
        let response =
            rpc.send()
                .promise
                .await
                .map_err(|error| AssetProcessorError::SourceFileCodecRpc {
                    reason: error.to_string(),
                })?;
        let response = response
            .get()
            .map_err(|error| AssetProcessorError::SourceFileCodecRpc {
                reason: error.to_string(),
            })?;
        let result =
            response
                .get_result()
                .map_err(|error| AssetProcessorError::SourceFileCodecRpc {
                    reason: error.to_string(),
                })?;
        SourceFileCodecResult::from_capnp(result).map_err(|error| {
            AssetProcessorError::SourceFileCodecRpc {
                reason: error.to_string(),
            }
        })
    }

    fn consume_saved_source(
        handle: &SideChannelHandle,
        destination: &SourceFileCodecOutputDestination,
    ) -> Result<Vec<u8>, AssetProcessorError> {
        let path = validated_staging_file_path(handle)
            .map_err(|source| AssetProcessorError::SourceFileCodecSideChannel { source })?;
        let expected_path = Path::new(&destination.locator);
        if path != expected_path
            || handle.platform != destination.platform
            || handle.capability.as_ref() != Some(&destination.capability)
        {
            return Err(AssetProcessorError::SourceFileCodecOutputDestination {
                expected: destination.locator.clone(),
                actual: handle.locator.clone(),
            });
        }
        validate_side_channel_capability_matches(
            handle,
            &destination.capability,
            "saved structured source",
        )?;
        let bytes = read_verified_staging_file(handle)
            .map_err(|source| AssetProcessorError::SourceFileCodecSideChannel { source })?
            .bytes;
        Ok(bytes)
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn open_source_file_transaction(
        &self,
        request: &SourceFileOpenRequest,
    ) -> Result<SourceFileOpenResult, AssetProcessorError> {
        validate_read_capability(&request.capability, self.processor.capability_grants())?;
        validate_source_file_capability_session(&request.capability, &request.session_id)?;
        let codec = self.active_source_file_codec()?;
        let resolved = self.resolve_source_file(&request.session_id, &request.source, false)?;
        let bytes =
            fs::read(&resolved.path).map_err(|source| AssetProcessorError::SourceFileRead {
                path: resolved.path.clone(),
                source,
            })?;
        let fingerprint = blake3::hash(&bytes).as_bytes().to_vec();
        let (authoritative_source, _authoritative_cleanup) =
            self.stage_source_file(&codec.capability, &bytes)?;
        let (output_destination, _output_cleanup) =
            self.reserve_source_file_codec_output(&codec.capability)?;
        let result = self
            .dispatch_source_file_codec(
                &codec,
                &SourceFileCodecRequest {
                    source: resolved.source.clone(),
                    authoritative_source,
                    operation: SourceFileCodecOperation::Load,
                    output_destination: output_destination.clone(),
                },
            )
            .await?;
        if let SourceFileCodecReplacement::SavedSource(handle) = &result.replacement {
            Self::consume_saved_source(handle, &output_destination)?;
            return Err(AssetProcessorError::SourceFileCodecReplacement {
                operation: "load",
                reason: "load returned saved source bytes".to_string(),
            });
        }
        Ok(SourceFileOpenResult {
            snapshot: SourceFileEditSnapshot {
                source: resolved.source,
                source_fingerprint: fingerprint,
                document: result.document,
            },
        })
    }

    fn begin_open_source_file_transaction(
        self: &capnp::capability::Rc<Self>,
        request: SourceFileOpenRequest,
    ) -> tokio::sync::oneshot::Receiver<Result<SourceFileOpenResult, AssetProcessorError>> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let this = capnp::capability::Rc::clone(self);
        tokio::task::spawn_local(async move {
            let result = this.open_source_file_transaction(&request).await;
            let _ = sender.send(result);
        });
        receiver
    }

    fn begin_owned_source_operation<T, F, Fut>(
        self: &capnp::capability::Rc<Self>,
        operation: F,
    ) -> tokio::sync::oneshot::Receiver<Result<T, AssetProcessorError>>
    where
        T: 'static,
        F: FnOnce(capnp::capability::Rc<Self>) -> Fut + 'static,
        Fut: std::future::Future<Output = Result<T, AssetProcessorError>> + 'static,
    {
        let (send, receive) = tokio::sync::oneshot::channel();
        let this = capnp::capability::Rc::clone(self);
        tokio::task::spawn_local(async move {
            let _ = send.send(operation(this).await);
        });
        receive
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[cfg(test)]
    async fn write_source_file_transaction(
        &self,
        capability: &Capability,
        session_id: &str,
        source: &WorkspaceSourceFileRef,
        expected_source_fingerprint: &[u8],
        operation: SourceFileCodecOperation,
        operation_name: &'static str,
    ) -> Result<CompletedSourceFileTransaction, AssetProcessorError> {
        self.write_source_file_transaction_guarded(
            capability,
            session_id,
            source,
            expected_source_fingerprint,
            operation,
            operation_name,
        )
        .await
    }

    /// Runs the registered codec over the source file and returns the bytes it
    /// wants written.
    ///
    /// The file is read twice on purpose: once to fence the caller against the
    /// fingerprint it thinks it is editing, and once after the codec answers, so
    /// an edit that raced a write on disk is refused instead of overwriting it.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn apply_source_file_codec(
        &self,
        resolved: &ResolvedSourceFile,
        expected_source_fingerprint: &[u8],
        operation: SourceFileCodecOperation,
        operation_name: &'static str,
    ) -> Result<SourceFileEdit, AssetProcessorError> {
        let codec = self.active_source_file_codec()?;
        let authoritative_bytes =
            fs::read(&resolved.path).map_err(|source| AssetProcessorError::SourceFileRead {
                path: resolved.path.clone(),
                source,
            })?;
        let authoritative_fingerprint = blake3::hash(&authoritative_bytes);
        if authoritative_fingerprint.as_bytes() != expected_source_fingerprint {
            return Err(AssetProcessorError::SourceFileFingerprintConflict {
                source_path: resolved.source.source_path.clone(),
                expected: hex_lower(expected_source_fingerprint),
                actual: authoritative_fingerprint.to_hex().to_string(),
            });
        }
        let (authoritative_source, _authoritative_cleanup) =
            self.stage_source_file(&codec.capability, &authoritative_bytes)?;
        let (output_destination, _output_cleanup) =
            self.reserve_source_file_codec_output(&codec.capability)?;
        let result = self
            .dispatch_source_file_codec(
                &codec,
                &SourceFileCodecRequest {
                    source: resolved.source.clone(),
                    authoritative_source,
                    operation,
                    output_destination: output_destination.clone(),
                },
            )
            .await?;
        let SourceFileCodecReplacement::SavedSource(saved_source) = &result.replacement else {
            return Err(AssetProcessorError::SourceFileCodecReplacement {
                operation: operation_name,
                reason: "mutating operation returned unchanged".to_string(),
            });
        };
        let saved_bytes = Self::consume_saved_source(saved_source, &output_destination)?;

        let current_bytes =
            fs::read(&resolved.path).map_err(|source| AssetProcessorError::SourceFileRead {
                path: resolved.path.clone(),
                source,
            })?;
        let current_fingerprint = blake3::hash(&current_bytes);
        if current_fingerprint != authoritative_fingerprint {
            return Err(AssetProcessorError::SourceFileFingerprintConflict {
                source_path: resolved.source.source_path.clone(),
                expected: authoritative_fingerprint.to_hex().to_string(),
                actual: current_fingerprint.to_hex().to_string(),
            });
        }
        Ok(SourceFileEdit {
            authoritative_bytes,
            saved_bytes,
            document: result.document,
        })
    }

    /// Writes the edited bytes to disk, then publishes the matching revision.
    ///
    /// The file lands first because the database revision has to describe a file
    /// that exists; if the writer then refuses, the file is put back from the
    /// bytes the edit was fenced against, so a rejected publication leaves no
    /// trace on disk.
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn commit_source_file_edit(
        &self,
        transaction_root: &Path,
        resolved: &ResolvedSourceFile,
        edit: SourceFileEdit,
        publication: PublishAuthoredSource,
        operation_name: &'static str,
    ) -> Result<(SelectAssets, SelectEntries), AssetProcessorError> {
        let SourceFileEdit {
            authoritative_bytes,
            saved_bytes,
            ..
        } = edit;
        commit_source_file_publication(transaction_root, &resolved.path, saved_bytes)?;
        let database = match self
            .processor
            .asset_db_writer()
            .publish_authored_source(publication)
            .await
        {
            Ok(result) => authored_publication_written(result, &resolved.source.source_path),
            Err(error) => Err(error.into()),
        };
        let (asset, entry) = match database {
            Ok(written) => written,
            Err(database) => {
                if let Err(rollback) = compensate_source_file_publication(
                    transaction_root,
                    &resolved.path,
                    authoritative_bytes,
                ) {
                    return Err(AssetProcessorError::SourceFilePublicationRollback {
                        operation: operation_name,
                        database: Box::new(database),
                        rollback,
                    });
                }
                return Err(database);
            }
        };
        Ok((asset, entry))
    }

    /// Builds the durable publication for one edited source file.
    ///
    /// The revision advances from whatever the database already has saved, and
    /// `expected_revision` carries the value it advanced from, so a write that
    /// raced this edit is rejected by the writer instead of being overwritten.
    fn source_file_edit_publication(
        &self,
        resolved: &ResolvedSourceFile,
        session_id: &str,
        saved_bytes: &[u8],
        digest: Digest,
    ) -> Result<PublishAuthoredSource, AssetProcessorError> {
        let workspace_pk = self.processor.attached_workspace_id()?;
        let (workspace, policy, root, existing_payload, asset, entry) = {
            let db = self.processor.databases.query();
            let (workspace, policy, root) = source_file_create_source_root(
                &db,
                workspace_pk,
                session_id,
                &resolved.source.source_root_key,
            )?;
            let existing_payload =
                db.payload_for_source(workspace_pk, root.root_id, &resolved.source.source_path)?;
            let (asset, entry) = db
                .source_asset(workspace_pk, root.root_id, &resolved.source.source_path)?
                .ok_or_else(|| AssetProcessorError::AuthoredSourcePublicationRejected {
                    source_path: resolved.source.source_path.clone(),
                    reason: "structured edit source is not registered",
                })?;
            drop(db);
            (workspace, policy, root, existing_payload, asset, entry)
        };
        let revision = existing_payload
            .as_ref()
            .map_or(1, |payload| payload.revision.saturating_add(1));
        let now = current_unix_ms_i64()?;
        let publication = PublishAuthoredSource {
            payload: WriteSourcePayload {
                workspace_pk,
                root_pk: root.root_id,
                path: resolved.source.source_path.clone(),
                document: existing_payload.as_ref().map_or_else(
                    || resolved.source.source_path.clone(),
                    |row| row.document.clone(),
                ),
                schema: resolved.source.schema_type.clone(),
                encoding: Encoding::Bytes,
                expected_revision: existing_payload.as_ref().map(|payload| payload.revision),
                revision,
                saved: Some(revision),
                digest,
                payload: saved_bytes.to_vec(),
                checkpoint: CheckpointWrite::Replace(saved_bytes.to_vec()),
                session: Some(session_id.to_owned()),
                project: workspace.project,
                now,
            },
            workspace_root_pk: policy.workspace_root_id,
            source: SweepEntry {
                path: resolved.source.source_path.clone(),
                guid: asset.guid,
                schema: Some(resolved.source.schema_type.clone()),
                digest,
                diff: DbDiff::Modified,
                diagnostics: entry.diagnostics,
                updated: now,
                src_bytes: i64::try_from(saved_bytes.len()).unwrap_or(i64::MAX),
                src_mtime: now,
                meta_bytes: entry.meta_bytes,
                meta_mtime: entry.meta_mtime,
                observed: now,
                session: Some(session_id.to_owned()),
            },
        };
        Ok(publication)
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    #[allow(clippy::too_many_arguments)]
    async fn write_source_file_transaction_guarded(
        &self,
        capability: &Capability,
        session_id: &str,
        source: &WorkspaceSourceFileRef,
        expected_source_fingerprint: &[u8],
        operation: SourceFileCodecOperation,
        operation_name: &'static str,
    ) -> SourceFileTransactionOutcome {
        validate_source_file_capability_session(capability, session_id)?;
        let resolved = self.resolve_source_file(session_id, source, true)?;
        let SourceFileEdit {
            authoritative_bytes,
            saved_bytes,
            document,
        } = self
            .apply_source_file_codec(
                &resolved,
                expected_source_fingerprint,
                operation,
                operation_name,
            )
            .await?;

        let project_data_paths = self.processor.project_data_paths()?;
        let transaction_root = project_data_paths
            .derived_dir()
            .join("asset-processor")
            .join("source-file-transactions");
        let fingerprint = blake3::hash(&saved_bytes);
        let digest = Digest::from(fingerprint);
        let publication =
            self.source_file_edit_publication(&resolved, session_id, &saved_bytes, digest)?;
        let (asset, entry) = self
            .commit_source_file_edit(
                &transaction_root,
                &resolved,
                SourceFileEdit {
                    authoritative_bytes,
                    saved_bytes,
                    document: document.clone(),
                },
                publication,
                operation_name,
            )
            .await?;
        if let Err(error) = self
            .processor
            .enqueue_jobs_for_source(&asset, &entry, false)
            .await
        {
            warn!(
                source_path = %resolved.source.source_path,
                %error,
                "edited source committed; planning is deferred until the next reconcile"
            );
        }
        info!(
            source_path = %resolved.source.source_path,
            schema_type = %resolved.source.schema_type,
            operation = operation_name,
            "asset processor committed structured source-file publication and AssetDB revision"
        );
        Ok(CompletedSourceFileTransaction {
            snapshot: SourceFileEditSnapshot {
                source: resolved.source,
                source_fingerprint: fingerprint.as_bytes().to_vec(),
                document,
            },
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_write_source_file_transaction(
        self: &capnp::capability::Rc<Self>,
        capability: Capability,
        session_id: String,
        source: WorkspaceSourceFileRef,
        expected_source_fingerprint: Vec<u8>,
        operation: SourceFileCodecOperation,
        operation_name: &'static str,
    ) -> tokio::sync::oneshot::Receiver<SourceFileTransactionOutcome> {
        self.begin_owned_source_operation(move |this| async move {
            this.write_source_file_transaction_guarded(
                &capability,
                &session_id,
                &source,
                &expected_source_fingerprint,
                operation,
                operation_name,
            )
            .await
        })
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn lease_job(
        &self,
        request: &LeaseAssetJobRequest,
    ) -> Result<LeaseAssetJobResult, AssetProcessorError> {
        let envelope = self.processor.validate_lease_admission(request)?;
        self.job_dispatcher()
            .lease(self.connection_id, envelope)
            .await
    }

    #[must_use]
    pub(crate) fn into_client(self) -> asset_capnp::asset_processor::Client {
        capnp_rpc::new_client(self)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn client_from_rc(this: &Rc<Self>) -> asset_capnp::asset_processor::Client {
        capnp_rpc::new_client_from_rc(Rc::clone(this))
    }

    #[must_use]
    fn health_snapshot(&self) -> ServiceHealth {
        let activity = source_reconcile_activity_snapshot();
        let base = ServiceHealth::ready(
            ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
            ServiceRole::AssetProcessor,
            self.service_run,
            az_proto_core::ProtocolVersion::CURRENT,
        )
        .with_uptime_ms(duration_millis_u64(self.started_at.elapsed()))
        .with_last_event_seq(self.last_event_seq());

        if let Some((fault_count, latest)) = self.consequence_health.snapshot() {
            return base
                .with_state(ServiceHealthState::Degraded)
                .with_active_operation("job-completion-consequence")
                .with_message(format!(
                    "{fault_count} completion consequence fault(s); latest: {latest}"
                ));
        }

        if activity.active {
            let message = format!(
                "{}; {} discovered, {} recorded, {} deleted, {} queued jobs",
                activity.message,
                activity.discovered_source_asset_count,
                activity.recorded_source_asset_count,
                activity.deleted_source_asset_count,
                activity.planned_job_count
            );
            return base
                .with_state(ServiceHealthState::Busy)
                .with_active_operation("source-reconcile")
                .with_message(message);
        }

        if activity.message.starts_with("source scan failed:") {
            return base
                .with_state(ServiceHealthState::Degraded)
                .with_active_operation("source-reconcile")
                .with_message(activity.message);
        }

        if !activity.message.is_empty() {
            return base
                .with_active_operation("source-reconcile")
                .with_message(activity.message);
        }

        base.with_message("ready")
    }

    #[must_use]
    fn last_event_seq(&self) -> u64 {
        self.next_event_seq.get().saturating_sub(1)
    }

    fn add_event_subscriber(
        &self,
        request: &AssetProcessorEventSubscriptionRequest,
        sink: asset_capnp::asset_processor_event_sink::Client,
    ) -> Result<AssetProcessorEventSubscriptionResult, AssetProcessorError> {
        self.processor.validate_event_subscription(request)?;
        let workspace_id = self.processor.attached_workspace_id()?;
        let id = self.next_subscriber_id.get();
        self.next_subscriber_id.set(id.saturating_add(1));
        self.event_subscribers
            .borrow_mut()
            .push(AssetProcessorEventSubscriber {
                id,
                workspace_id,
                sink,
                pending: None,
                in_flight: false,
            });
        Ok(AssetProcessorEventSubscriptionResult {
            subscribed: true,
            initial_health: self.health_snapshot(),
        })
    }

    /// Deliver one event to every subscriber on the entry's workspace.
    ///
    /// A slow sink retains only its latest not-yet-sent update and has one
    /// in-flight request. A rejected sink is removed without affecting peers.
    fn publish_event(
        self: &capnp::capability::Rc<Self>,
        kind: AssetProcessorEventKind,
        event_unix_ms: i64,
        entry: WorkspaceEntry,
    ) {
        self.event_publisher().publish(kind, event_unix_ms, entry);
    }
}

impl asset_capnp::asset_processor::Server for AssetProcessorRpc {
    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn health(
        self: capnp::capability::Rc<Self>,
        _params: asset_capnp::asset_processor::HealthParams,
        mut results: asset_capnp::asset_processor::HealthResults,
    ) -> Result<(), Error> {
        (self.health_snapshot()).to_capnp(results.get().init_health())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn inspect_job(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::InspectJobParams,
        mut results: asset_capnp::asset_processor::InspectJobResults,
    ) -> Result<(), Error> {
        let request = InspectJobRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .processor
            .inspect_job(&request)
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn builder_catalog(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::BuilderCatalogParams,
        mut results: asset_capnp::asset_processor::BuilderCatalogResults,
    ) -> Result<(), Error> {
        let request = AssetBuilderCatalogRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .processor
            .builder_catalog(&request)
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn publish_builder_catalog(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::PublishBuilderCatalogParams,
        mut results: asset_capnp::asset_processor::PublishBuilderCatalogResults,
    ) -> Result<(), Error> {
        let params = params.get()?;
        let request = PublishBuilderCatalogRequest::from_capnp(params.get_request()?)?;
        let codec = params.get_codec()?;
        let result = self
            .processor
            .publish_builder_catalog(&request)
            .await
            .map_err(asset_processor_error_to_capnp)?;
        *self.source_file_codec.borrow_mut() = Some(ActiveSourceFileCodec {
            client: codec,
            capability: request.capability.clone(),
        });
        self.source_coordination
            .publish_builder_catalog(request.catalog)
            .map_err(asset_processor_error_to_capnp)?;
        result.to_capnp(results.get().init_result());
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn workspace_entry_page(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::WorkspaceEntryPageParams,
        mut results: asset_capnp::asset_processor::WorkspaceEntryPageResults,
    ) -> Result<(), Error> {
        let request = WorkspaceEntryPageRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .processor
            .workspace_entry_page(&request)
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn record_source_asset(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::RecordSourceAssetParams,
        mut results: asset_capnp::asset_processor::RecordSourceAssetResults,
    ) -> Result<(), Error> {
        let request = SourceAssetRecordRequest::from_capnp(params.get()?.get_request()?)?;
        let event_unix_ms = request.changed_unix_ms;
        let result = self
            .begin_owned_source_operation(move |this| async move {
                let _root = this
                    .acquire_workspace_root_operation(request.workspace_root_id)
                    .await?;
                this.processor.record_source_asset(&request).await
            })
            .await
            .map_err(|_| Error::failed("record-source task ended".into()))?
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        self.publish_event(
            AssetProcessorEventKind::SourceRecorded,
            event_unix_ms,
            result.entry,
        );
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn create_source_file(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::CreateSourceFileParams,
        mut results: asset_capnp::asset_processor::CreateSourceFileResults,
    ) -> Result<(), Error> {
        let request = SourceFileCreateRequest::from_capnp(params.get()?.get_request()?)?;
        let event_unix_ms = request.changed_unix_ms;
        let result = self
            .begin_owned_source_operation(move |this| async move {
                let _root = this.acquire_root_operation(&request.source_root).await?;
                this.processor.create_source_file(&request).await
            })
            .await
            .map_err(|_| Error::failed("create-source task ended".into()))?
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        self.publish_event(
            AssetProcessorEventKind::SourceCreated,
            event_unix_ms,
            result.record.entry,
        );
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn open_source_file(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::OpenSourceFileParams,
        mut results: asset_capnp::asset_processor::OpenSourceFileResults,
    ) -> Result<(), Error> {
        let request = SourceFileOpenRequest::from_capnp(params.get()?.get_request()?)?;
        let _root = self
            .acquire_root_operation(&request.source.source_root_key)
            .await
            .map_err(asset_processor_error_to_capnp)?;
        let result = self
            .begin_open_source_file_transaction(request)
            .await
            .map_err(|_| Error::failed("source-file open transaction task ended".into()))?
            .map_err(asset_processor_error_to_capnp)?;
        result.to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn edit_source_file(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::EditSourceFileParams,
        mut results: asset_capnp::asset_processor::EditSourceFileResults,
    ) -> Result<(), Error> {
        let request = SourceFileEditRequest::from_capnp(params.get()?.get_request()?)?;
        validate_editor_asset_write_capability(
            &request.capability,
            self.processor.capability_grants(),
        )
        .map_err(asset_processor_error_to_capnp)?;
        let _root = self
            .acquire_root_operation(&request.source.source_root_key)
            .await
            .map_err(asset_processor_error_to_capnp)?;
        let completed = self
            .begin_write_source_file_transaction(
                request.capability,
                request.session_id,
                request.source,
                request.expected_source_fingerprint,
                SourceFileCodecOperation::Edit(request.operation),
                "edit",
            )
            .await
            .map_err(|_| Error::failed("source-file edit transaction task ended".into()))?
            .map_err(asset_processor_error_to_capnp)?;
        SourceFileEditResult {
            snapshot: completed.snapshot,
        }
        .to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn restore_source_file(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::RestoreSourceFileParams,
        mut results: asset_capnp::asset_processor::RestoreSourceFileResults,
    ) -> Result<(), Error> {
        let request = SourceFileRestoreRequest::from_capnp(params.get()?.get_request()?)?;
        validate_project_host_asset_write_capability(
            &request.capability,
            self.processor.capability_grants(),
        )
        .map_err(asset_processor_error_to_capnp)?;
        let _root = self
            .acquire_root_operation(&request.source.source_root_key)
            .await
            .map_err(asset_processor_error_to_capnp)?;
        let completed = self
            .begin_write_source_file_transaction(
                request.capability,
                request.session_id,
                request.source,
                request.expected_source_fingerprint,
                SourceFileCodecOperation::RestoreDocument(request.document),
                "restore",
            )
            .await
            .map_err(|_| Error::failed("source-file restore transaction task ended".into()))?
            .map_err(asset_processor_error_to_capnp)?;
        SourceFileRestoreResult {
            snapshot: completed.snapshot,
        }
        .to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn delete_source_file(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::DeleteSourceFileParams,
        mut results: asset_capnp::asset_processor::DeleteSourceFileResults,
    ) -> Result<(), Error> {
        let request = SourceFileDeleteRequest::from_capnp(params.get()?.get_request()?)?;
        let event_unix_ms = request.changed_unix_ms;
        let result = self
            .begin_owned_source_operation(move |this| async move {
                let _root = this.acquire_root_operation(&request.source_root).await?;
                this.processor.delete_source_file(&request).await
            })
            .await
            .map_err(|_| Error::failed("delete-source task ended".into()))?
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        self.publish_event(
            AssetProcessorEventKind::SourceDeleted,
            event_unix_ms,
            result.record.entry,
        );
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn move_source_file(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::MoveSourceFileParams,
        mut results: asset_capnp::asset_processor::MoveSourceFileResults,
    ) -> Result<(), Error> {
        let request = SourceFileMoveRequest::from_capnp(params.get()?.get_request()?)?;
        let event_unix_ms = request.changed_unix_ms;
        let result = self
            .begin_owned_source_operation(move |this| async move {
                let _root = this.acquire_root_operation(&request.source_root).await?;
                this.processor.move_source_file(&request).await
            })
            .await
            .map_err(|_| Error::failed("move-source task ended".into()))?
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        self.publish_event(
            AssetProcessorEventKind::SourceMoved,
            event_unix_ms,
            result.record.entry,
        );
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn source_dependents(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::SourceDependentsParams,
        mut results: asset_capnp::asset_processor::SourceDependentsResults,
    ) -> Result<(), Error> {
        let request = SourceDependentsRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .processor
            .source_dependents(&request)
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn force_reprocess_asset(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::ForceReprocessAssetParams,
        mut results: asset_capnp::asset_processor::ForceReprocessAssetResults,
    ) -> Result<(), Error> {
        let request = ForceReprocessAssetRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .processor
            .force_reprocess_asset(&request)
            .await
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        self.publish_event(
            AssetProcessorEventKind::SourceReprocessed,
            result.record.entry.updated_unix_ms,
            result.record.entry,
        );
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn reconcile_asset_sources(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::ReconcileAssetSourcesParams,
        mut results: asset_capnp::asset_processor::ReconcileAssetSourcesResults,
    ) -> Result<(), Error> {
        let request = ReconcileAssetSourcesRequest::from_capnp(params.get()?.get_request()?)?;
        let result = if let Some(sweeps) = self.source_coordination.sweep_handle() {
            validate_reconcile_asset_sources_request(&request, self.processor.capability_grants())
                .map_err(asset_processor_error_to_capnp)?;
            let roots = self
                .processor
                .source_roots
                .iter()
                .filter(|root| {
                    request.root_scope == AssetRootScope::All || is_browser_asset_source_root(root)
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut pending = futures::stream::FuturesUnordered::new();
            for root in &roots {
                pending.push(sweeps.run(SweepRequest {
                    root: SweepRoot::registered(root),
                    scope: SweepScope::Root,
                    provenance: SweepProvenance::Explicit {
                        session: request.session_id.clone(),
                    },
                }));
            }
            let mut recorded = 0_usize;
            let mut removed = 0_usize;
            while let Some(effect) = pending.next().await {
                let effect = effect.map_err(asset_processor_error_to_capnp)?;
                recorded += effect.summary.recorded;
                removed += effect.summary.deleted;
            }
            ReconcileAssetSourcesResult {
                source_root_count: u32::try_from(roots.len()).map_err(|_| {
                    asset_processor_error_to_capnp(
                        AssetProcessorError::AssetSourceReconcileCountOverflow {
                            field: "source_root_count",
                            count: roots.len(),
                        },
                    )
                })?,
                recorded_source_asset_count: u32::try_from(recorded).map_err(|_| {
                    asset_processor_error_to_capnp(
                        AssetProcessorError::AssetSourceReconcileCountOverflow {
                            field: "recorded_source_asset_count",
                            count: recorded,
                        },
                    )
                })?,
                deleted_source_asset_count: u32::try_from(removed).map_err(|_| {
                    asset_processor_error_to_capnp(
                        AssetProcessorError::AssetSourceReconcileCountOverflow {
                            field: "deleted_source_asset_count",
                            count: removed,
                        },
                    )
                })?,
            }
        } else {
            self.processor
                .reconcile_asset_sources(&request)
                .map_err(asset_processor_error_to_capnp)?
        };
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn workspace_snapshot(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::WorkspaceSnapshotParams,
        mut results: asset_capnp::asset_processor::WorkspaceSnapshotResults,
    ) -> Result<(), Error> {
        let request = WorkspaceSnapshotRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .processor
            .workspace_snapshot(&request)
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn catalog_products(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::CatalogProductsParams,
        mut results: asset_capnp::asset_processor::CatalogProductsResults,
    ) -> Result<(), Error> {
        let request = CatalogProductsRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .processor
            .catalog_products(&request)
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn release_content(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::ReleaseContentParams,
        mut results: asset_capnp::asset_processor::ReleaseContentResults,
    ) -> Result<(), Error> {
        let request = ReleaseContentReadRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .processor
            .release_content(&request)
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn processing_status(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::ProcessingStatusParams,
        mut results: asset_capnp::asset_processor::ProcessingStatusResults,
    ) -> Result<(), Error> {
        let request = AssetProcessingStatusRequest::from_capnp(params.get()?.get_request()?)?;
        let mut result = self
            .processor
            .processing_status(&request)
            .map_err(asset_processor_error_to_capnp)?;
        result.in_flight_sweeps = self.source_coordination.sweep_handle().map_or(0, |sweep| {
            u32::try_from(sweep.in_flight()).unwrap_or(u32::MAX)
        });
        result.to_capnp(results.get().init_result());
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn wait_for_idle(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::WaitForIdleParams,
        mut results: asset_capnp::asset_processor::WaitForIdleResults,
    ) -> Result<(), Error> {
        let request = AssetProcessingStatusRequest::from_capnp(params.get()?.get_request()?)?;
        // Subscribe before the first coherent snapshot. A transition in
        // the snapshot/park gap then leaves `changed()` immediately ready
        // instead of stranding this RPC until a later job mutation.
        let mut changes = self.processor.subscribe_processing_status_changes();
        let sweep = self.source_coordination.sweep_handle();
        let mut sweep_changes = sweep.as_ref().map(SweepHandle::subscribe);
        loop {
            let mut result = self
                .processor
                .processing_status(&request)
                .map_err(asset_processor_error_to_capnp)?;
            result.in_flight_sweeps = sweep.as_ref().map_or(0, |sweep| {
                u32::try_from(sweep.in_flight()).unwrap_or(u32::MAX)
            });
            if result.active() == 0 {
                result.to_capnp(results.get().init_result());
                return Ok(());
            }
            let observed_revision = changes.revision();
            trace!(
                observed_revision,
                active = result.active(),
                "asset processor wait-for-idle parked on processing-status revision"
            );
            tokio::select! {
                alive = changes.changed() => if !alive {
                    return Err(asset_processor_error_to_capnp(
                        AssetProcessorError::ProcessingStatusPublisherClosed,
                    ));
                },
                alive = async {
                    match &mut sweep_changes {
                        Some(sweeps) => sweeps.changed().await.is_ok(),
                        None => futures::future::pending::<bool>().await,
                    }
                } => if !alive {
                    return Err(asset_processor_error_to_capnp(
                        AssetProcessorError::SweepOwnerClosed,
                    ));
                },
            }
        }
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn publish_asset_catalog(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::PublishAssetCatalogParams,
        mut results: asset_capnp::asset_processor::PublishAssetCatalogResults,
    ) -> Result<(), Error> {
        let request = PublishAssetCatalogRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .processor
            .publish_asset_catalog(&request)
            .await
            .map_err(asset_processor_error_to_capnp)?;
        result.to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn lease(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::LeaseParams,
        mut results: asset_capnp::asset_processor::LeaseResults,
    ) -> Result<(), Error> {
        let request = LeaseAssetJobRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .lease_job(&request)
            .await
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn renew_lease(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::RenewLeaseParams,
        mut results: asset_capnp::asset_processor::RenewLeaseResults,
    ) -> Result<(), Error> {
        let request = RenewAssetJobLeaseRequest::from_capnp(params.get()?.get_request()?)?;
        self.processor
            .validate_renewal_admission(&request)
            .map_err(asset_processor_error_to_capnp)?;
        let renewed = self
            .job_dispatcher()
            .renew(GrantIdentity::new(
                request.asset_job_attempt_id,
                request.lease_owner,
                self.connection_id,
                request.grant_key,
            ))
            .await
            .map_err(asset_processor_error_to_capnp)?;
        results.get().set_renewed(renewed);
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn complete_attempt(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::CompleteAttemptParams,
        mut results: asset_capnp::asset_processor::CompleteAttemptResults,
    ) -> Result<(), Error> {
        let request = CompleteAssetJobAttemptRequest::from_capnp(params.get()?.get_request()?)?;
        self.processor
            .validate_completion_admission(&request)
            .map_err(asset_processor_error_to_capnp)?;
        let completed = self
            .job_dispatcher()
            .complete(
                GrantIdentity::new(
                    request.asset_job_attempt_id,
                    request.lease_owner.clone(),
                    self.connection_id,
                    request.grant_key,
                ),
                request.clone(),
            )
            .await
            .map_err(asset_processor_error_to_capnp)?;
        results.get().set_completed(completed);
        Ok(())
    }

    // The asset-processor dispatcher is single-threaded by design: this future holds
    // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
    // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
    #[allow(clippy::future_not_send)]
    async fn subscribe_events(
        self: capnp::capability::Rc<Self>,
        params: asset_capnp::asset_processor::SubscribeEventsParams,
        mut results: asset_capnp::asset_processor::SubscribeEventsResults,
    ) -> Result<(), Error> {
        let params = params.get()?;
        let request = AssetProcessorEventSubscriptionRequest::from_capnp(params.get_request()?)?;
        let sink = params.get_sink()?;
        let result = self
            .add_event_subscriber(&request, sink)
            .map_err(asset_processor_error_to_capnp)?;
        (result).to_capnp(results.get().init_result());
        Ok(())
    }
}

#[must_use]
#[cfg(any(test, feature = "test-support"))]
pub fn asset_processor_client(processor: AssetProcessor) -> asset_capnp::asset_processor::Client {
    AssetProcessorRpc::new(processor).into_client()
}

/// The two claims that collided on one virtual asset path.
///
/// Boxed at [`AssetProcessorError::AssetSourceCollision`]. Inline, these five
/// fields made that one variant 136 bytes and set the size of every
/// `Result<_, AssetProcessorError>` in the crate -- which is most of its API.
#[derive(Debug)]
pub struct AssetSourceCollisionDetail {
    pub virtual_path: String,
    pub first_root: String,
    pub first_path: PathBuf,
    pub second_root: String,
    pub second_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum AssetProcessorError {
    #[error("asset DB open failed: {0}")]
    Open(#[from] OpenError),

    #[error("asset processor DB operation failed: {0}")]
    Repo(#[from] RepoError),

    #[error("scoped asset reconcile omitted workspace root {workspace_root_id}")]
    MissingSweepScope { workspace_root_id: i64 },

    #[error("asset job attempt {asset_job_attempt_id} has no owning job")]
    MissingAssetJobAttempt { asset_job_attempt_id: i64 },

    #[error("asset processor clock failed: {0}")]
    Clock(#[from] std::time::SystemTimeError),

    #[error("asset job dispatcher initialization failed: {reason}")]
    DispatcherInitialization { reason: String },

    #[error("asset job dispatcher stopped")]
    DispatcherStopped,

    #[error("asset job dispatcher task failed: {error}")]
    DispatcherTask {
        #[source]
        error: tokio::task::JoinError,
    },

    #[error("asset job attempt {attempt_id} payload staging exceeded its lease deadline")]
    DispatcherStagingTimeout { attempt_id: i64 },

    #[error("asset job attempt {attempt_id} completion exceeded its lease deadline")]
    DispatcherCompletionTimeout { attempt_id: i64 },

    #[error("asset job attempt {attempt_id} completion task panicked")]
    DispatcherCompletionPanicked { attempt_id: i64 },

    #[error("asset job attempt {attempt_id} completion worker failed: {error}")]
    DispatcherCompletionTask {
        attempt_id: i64,
        #[source]
        error: tokio::task::JoinError,
    },

    #[error("project manifest operation failed: {0}")]
    ProjectManifest(#[from] ProjectManifestError),

    #[error("machine-local Azoth data-home operation failed: {0}")]
    DataHome(#[from] az_filesystem::DataHomeError),

    #[error("asset catalog operation failed: {0}")]
    AssetCatalog(#[from] az_asset::AssetCatalogError),

    #[error("asset catalog product metadata is invalid: {0}")]
    AssetCatalogProductMetadata(#[from] az_asset::PackageManifestError),

    #[error("asset catalog path `{path}` has no parent")]
    AssetCatalogInvalidPath { path: PathBuf },

    #[error("failed to atomically write asset catalog `{path}`: {source}")]
    AssetCatalogWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("asset catalog publication worker failed: {error}")]
    AssetCatalogWorker {
        #[source]
        error: tokio::task::JoinError,
    },

    #[error("asset catalog publisher thread could not start: {error}")]
    CatalogPublisherStart {
        #[source]
        error: std::io::Error,
    },

    #[error("asset catalog publisher stopped")]
    CatalogPublisherStopped,

    #[error("asset catalog publisher command queue is full")]
    CatalogPublisherOverloaded,

    #[error("asset catalog publisher thread panicked")]
    CatalogPublisherPanicked,

    #[error("asset catalog publication failed: {reason}")]
    CatalogPublicationFailed { reason: Arc<str> },

    #[error("catalog product {product_id} belongs to non-build job {job_id}")]
    CatalogProductMissingBuilder { product_id: i64, job_id: i64 },

    #[error(
        "catalog product {product_id} dependency {asset_guid}:{sub_id} has no resolved target type"
    )]
    CatalogDependencyMissingType {
        product_id: i64,
        asset_guid: Uuid,
        sub_id: i64,
    },

    #[error("invalid job inspection for job {job_id}: {reason}")]
    InvalidJobInspection { job_id: i64, reason: String },

    #[error("asset processing status {field} count {count} exceeds protocol UInt32 range")]
    AssetProcessingStatusCountOverflow { field: &'static str, count: u64 },

    #[error("asset processing-status publisher closed while waiting for idle")]
    ProcessingStatusPublisherClosed,

    #[error(
        "builder catalog is unavailable because no project asset-worker has published one yet; wait for worker connect/publishBuilderCatalog"
    )]
    BuilderCatalogUnavailable,

    #[error("builder catalog for workspace {workspace_id} changed before replacement committed")]
    BuilderCatalogSnapshotConflict { workspace_id: i64 },

    #[error("asset-processor protocol version mismatch: {0}")]
    ProtocolVersionMismatch(#[from] az_proto_core::ProtocolVersionMismatch),

    #[error("asset-processor project id is required for DB workspace registration")]
    ProjectIdRequired,

    #[error("asset-processor change provenance session id is required")]
    SessionIdRequired,

    #[error("asset-processor session id `{session_id}` is not a UUID: {source}")]
    InvalidSessionId {
        session_id: String,
        #[source]
        source: uuid::Error,
    },

    #[error("asset-processor session id `{session_id}` must not be the nil UUID")]
    NilSessionId { session_id: String },

    #[error("asset-processor workspace branch is required for DB workspace registration")]
    WorkspaceBranchRequired,

    #[error(
        "project manifest at {workspace_root} has project id `{actual}`, expected `{expected}`"
    )]
    ProjectManifestIdMismatch {
        workspace_root: PathBuf,
        expected: String,
        actual: String,
    },

    #[error("asset-processor workspace root {workspace_root} is not absolute")]
    WorkspaceRootNotAbsolute { workspace_root: PathBuf },

    #[error("asset-processor workspace root {workspace_root} could not be read: {source}")]
    WorkspaceRootRead {
        workspace_root: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("asset-processor workspace root {workspace_root} is not a directory")]
    WorkspaceRootNotDirectory { workspace_root: PathBuf },

    #[error("asset-processor workspace root {workspace_root} could not be canonicalized: {source}")]
    WorkspaceRootCanonicalize {
        workspace_root: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("asset source root `{display_name}` for `{owner_id}` at {root} is not absolute")]
    SourceRootNotAbsolute {
        owner_id: String,
        display_name: String,
        root: PathBuf,
    },

    #[error(
        "asset source root `{display_name}` for `{owner_id}` at {root} could not be read: {source}"
    )]
    SourceRootRead {
        owner_id: String,
        display_name: String,
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("asset source root `{display_name}` for `{owner_id}` at {root} is not a directory")]
    SourceRootNotDirectory {
        owner_id: String,
        display_name: String,
        root: PathBuf,
    },

    #[error(
        "asset source root `{display_name}` for `{owner_id}` at {root} could not be canonicalized: {source}"
    )]
    SourceRootCanonicalize {
        owner_id: String,
        display_name: String,
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("native source path `{path}` is invalid: {reason}")]
    InvalidNativeSourcePath { path: PathBuf, reason: String },

    #[error("asset sweep owner is closed")]
    SweepOwnerClosed,

    #[error("asset sweep owner has {capacity} admitted commands in flight")]
    SweepOwnerOverloaded { capacity: usize },

    #[error("asset sweep root {workspace_root_pk} is not registered")]
    UnknownSweepRoot { workspace_root_pk: i64 },

    #[error("asset sweep for root {workspace_root_pk} failed: {reason}")]
    SweepFailed {
        workspace_root_pk: i64,
        reason: String,
    },

    #[error("source root selector `{selector}` is not registered")]
    UnknownSourceRootSelector { selector: String },

    #[error(
        "native asset collision at `{}`: root `{}` claims {} and root `{}` claims {}; add an [[asset_overrides]] entry naming the winning and replaced roots",
        .0.virtual_path, .0.first_root, .0.first_path.display(), .0.second_root, .0.second_path.display()
    )]
    AssetSourceCollision(Box<AssetSourceCollisionDetail>),

    #[error(
        "asset override for `{virtual_path}` from `{replaced_root}` to `{winning_root}` does not match a current collision"
    )]
    AssetOverrideDoesNotMatchCollision {
        virtual_path: String,
        winning_root: String,
        replaced_root: String,
    },

    #[error("registered native asset namespace exclusions are invalid: {source}")]
    AssetNamespaceExclusionsParse {
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to reconcile asset source root directory `{path}`: {source}")]
    SourceRootReconcileDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to inspect asset source root entry `{path}`: {source}")]
    SourceRootReconcileEntry {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read registered source asset `{path}`: {source}")]
    SourceRootReconcileFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid source metadata sidecar `{path}`: {reason}")]
    SourceMetaSidecar { path: PathBuf, reason: String },

    #[error("asset source reconcile found no workspace for session `{session_id}`")]
    AssetSourceReconcileMissingWorkspace { session_id: String },

    #[error("asset source reconcile {field} count {count} exceeds protocol UInt32 range")]
    AssetSourceReconcileCountOverflow { field: &'static str, count: usize },

    #[error("invalid asset processor capability: {reason}")]
    InvalidCapability { reason: String },

    #[error("invalid DB product format version {version} for product `{product_path}`")]
    InvalidDbProductFormatVersion { product_path: String, version: i64 },

    #[error("invalid DB product dependency {dependency_id}: {reason}")]
    InvalidDbProductDependency { dependency_id: i64, reason: String },

    #[error("asset job attempt cannot be completed with status {status:?}")]
    InvalidCompletionStatus { status: AttemptStatus },

    #[error("abandoned expired attempt count {count} exceeds protocol UInt32 range")]
    AbandonedAttemptCountOverflow { count: usize },

    #[error("successful asset job completion requires a product manifest side channel")]
    MissingProductManifest,

    #[error("successful asset job completion requires product manifest side-channel capability")]
    MissingProductManifestCapability,

    #[error("product manifest side-channel capability does not match completion capability")]
    ProductManifestCapabilityMismatch,

    #[error("asset job staging root `{staging_root}` must be an absolute normalized path")]
    InvalidStagingRoot { staging_root: String },

    #[error(
        "product manifest side-channel file `{manifest_path}` is outside asset job staging root `{staging_root}`"
    )]
    ProductManifestOutsideStagingRoot {
        manifest_path: PathBuf,
        staging_root: PathBuf,
    },

    #[error("invalid product manifest side channel: {0}")]
    ProductManifest(#[from] ProductManifestSideChannelError),

    #[error("invalid product manifest: {reason}")]
    InvalidProductManifest { reason: String },

    #[error("invalid asset builder catalog: {reason}")]
    InvalidBuilderCatalog { reason: String },

    #[error(
        "staged product `{path}` for `{product_path}` escaped staging root `{staging_root}` after path resolution"
    )]
    StagedProductOutsideStagingRoot {
        product_path: String,
        path: PathBuf,
        staging_root: PathBuf,
    },

    #[error("failed to read staged product `{path}` for `{product_path}`: {source}")]
    StagedProductRead {
        product_path: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "staged product `{path}` for `{product_path}` has {actual} bytes, manifest declared {expected}"
    )]
    StagedProductLengthMismatch {
        product_path: String,
        path: PathBuf,
        expected: u64,
        actual: u64,
    },

    #[error("staged product `{path}` for `{product_path}` hash does not match product manifest")]
    StagedProductHashMismatch { product_path: String, path: PathBuf },

    #[error(
        "asset job attempt {asset_job_attempt_id} references missing workspace {workspace_id} for product cache promotion"
    )]
    MissingProductCacheWorkspace {
        asset_job_attempt_id: i64,
        workspace_id: i64,
    },

    #[error(
        "asset job attempt {asset_job_attempt_id} has invalid product cache root `{root}`: {reason}"
    )]
    InvalidProductCacheRoot {
        asset_job_attempt_id: i64,
        root: PathBuf,
        reason: String,
    },

    #[error(
        "asset job attempt {asset_job_attempt_id} has invalid product cache platform `{platform}`"
    )]
    InvalidProductCachePlatform {
        asset_job_attempt_id: i64,
        platform: String,
    },

    #[error("failed to recover product cache transactions under `{root}`: {source}")]
    ProductCacheTransactionRecovery {
        root: PathBuf,
        #[source]
        source: az_filesystem::FileTransactionError,
    },

    #[error("failed to commit product cache transaction under `{root}`: {source}")]
    ProductCacheTransaction {
        root: PathBuf,
        #[source]
        source: az_filesystem::FileTransactionError,
    },

    #[error("failed to commit product cache compensation transaction under `{root}`: {source}")]
    ProductCacheCompensationTransaction {
        root: PathBuf,
        #[source]
        source: az_filesystem::FileTransactionError,
    },

    #[error("failed to snapshot existing product cache path `{path}` before promotion: {source}")]
    ProductCachePromotionPreimage {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "failed to compensate product cache path `{path}` after an uncommitted promotion: {source}"
    )]
    ProductCachePromotionCompensation {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to remove product cache promotion receipt under `{root}`: {source}")]
    ProductCachePromotionReceiptCleanup {
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "product cache promotion failed ({promotion}) and required recovery/compensation failed: {rollback}"
    )]
    ProductCachePromotionRollback {
        promotion: Box<Self>,
        #[source]
        rollback: Box<Self>,
    },

    #[error(
        "durable asset completion writer failed ({writer}) and product cache compensation failed: {rollback}"
    )]
    ProductCacheCompletionRollback {
        writer: RepoError,
        #[source]
        rollback: Box<Self>,
    },

    #[error(
        "asset job attempt {attempt_id} was no longer owned and product cache compensation failed: {rollback}"
    )]
    ProductCacheNoLongerOwnedRollback {
        attempt_id: i64,
        #[source]
        rollback: Box<Self>,
    },

    #[error("duplicate generated Rust graph source product path `{product_path}`")]
    DuplicateGeneratedRustGraphSource { product_path: String },

    #[error("invalid generated Rust graph source product `{product_path}`: {reason}")]
    InvalidGeneratedRustGraphSourceProduct {
        product_path: String,
        reason: String,
    },

    #[error("invalid generated Rust graph source root `{root}`: {reason}")]
    InvalidGeneratedRustGraphSourceRoot { root: PathBuf, reason: String },

    #[error(
        "failed to create generated Rust graph source directory `{path}` for `{product_path}`: {source}"
    )]
    GeneratedRustGraphSourceCreateDir {
        product_path: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "failed to copy generated Rust graph source product `{from}` to `{to}` for `{product_path}`: {source}"
    )]
    GeneratedRustGraphSourceCopy {
        product_path: String,
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to remove existing generated Rust graph source root `{path}`: {source}")]
    GeneratedRustGraphSourceRemoveExisting {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to promote generated Rust graph source temp root `{from}` to `{to}`: {source}")]
    GeneratedRustGraphSourcePromote {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid source asset record: {reason}")]
    InvalidSourceAssetRecord { reason: String },

    #[error("authored source publication for `{source_path}` was rejected: {reason}")]
    AuthoredSourcePublicationRejected {
        source_path: String,
        reason: &'static str,
    },

    #[error("invalid source file create request: {reason}")]
    InvalidSourceFileCreateRequest { reason: String },

    #[error("invalid structured source-file request: {reason}")]
    InvalidSourceFileRequest { reason: String },

    #[error("no active project asset-worker source-file codec is registered")]
    SourceFileCodecUnavailable,

    #[error("source schema `{schema_type}` is not available from the active worker catalog")]
    SourceFileSchemaUnavailable { schema_type: String },

    #[error("source schema `{schema_type}` is not file-backed")]
    SourceFileSchemaNotFileBacked { schema_type: String },

    #[error("source schema `{schema_type}` is not editor-editable")]
    SourceFileSchemaNotEditable { schema_type: String },

    #[error("source root `{source_root_key}` is read-only")]
    SourceFileRootReadOnly { source_root_key: String },

    #[error("failed to read structured source file `{path}`: {source}")]
    SourceFileRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "source file `{source_path}` changed since it was opened: expected {expected}, found {actual}"
    )]
    SourceFileFingerprintConflict {
        source_path: String,
        expected: String,
        actual: String,
    },

    #[error("failed to stage authoritative source file `{path}`: {source}")]
    SourceFileStage {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("source-file codec RPC failed: {reason}")]
    SourceFileCodecRpc { reason: String },

    #[error("source-file codec violated the {operation} replacement contract: {reason}")]
    SourceFileCodecReplacement {
        operation: &'static str,
        reason: String,
    },

    #[error(
        "source-file codec returned output destination `{actual}`, expected AP-owned `{expected}`"
    )]
    SourceFileCodecOutputDestination { expected: String, actual: String },

    #[error("source-file codec side-channel capability is invalid: {0}")]
    SourceFileCodecCapability(#[from] SideChannelCapabilityError),

    #[error("source-file codec side channel is invalid: {source}")]
    SourceFileCodecSideChannel {
        #[source]
        source: StagingFileSideChannelError,
    },

    #[error("failed to recover source-file transactions under `{root}`: {source}")]
    SourceFileTransactionRecovery {
        root: PathBuf,
        #[source]
        source: az_filesystem::FileTransactionError,
    },

    #[error("failed to commit source-file transaction under `{root}`: {source}")]
    SourceFileTransaction {
        root: PathBuf,
        #[source]
        source: az_filesystem::FileTransactionError,
    },

    #[error(
        "source-file {operation} database publication failed ({database}) and filesystem compensation failed: {rollback}"
    )]
    SourceFilePublicationRollback {
        operation: &'static str,
        database: Box<Self>,
        #[source]
        rollback: az_filesystem::FileTransactionError,
    },

    #[error("invalid source file delete request: {reason}")]
    InvalidSourceFileDeleteRequest { reason: String },

    #[error("invalid source file move request: {reason}")]
    InvalidSourceFileMoveRequest { reason: String },

    #[error("invalid force reprocess asset request: {reason}")]
    InvalidForceReprocessAssetRequest { reason: String },

    #[error("invalid asset source reconcile request: {reason}")]
    InvalidAssetSourceReconcileRequest { reason: String },

    #[error("invalid source dependents request: {reason}")]
    InvalidSourceDependentsRequest { reason: String },

    #[error("source file create schema `{schema_type}` is not registered")]
    SourceFileCreateUnknownSchema { schema_type: String },

    #[error("source file create schema `{schema_type}` is not file-backed")]
    SourceFileCreateSchemaNotFileBacked { schema_type: String },

    #[error("source file create schema `{schema_type}` is not editor-creatable")]
    SourceFileCreateSchemaNotCreatable { schema_type: String },

    #[error(
        "source file create schema `{schema_type}` targets source root `{workflow_source_root}`, not requested source root `{requested_source_root}`"
    )]
    SourceFileCreateSourceRootMismatch {
        schema_type: String,
        requested_source_root: String,
        workflow_source_root: String,
    },

    #[error(
        "source file create path `{source_path}` does not match schema extensions {extensions:?}"
    )]
    SourceFileCreateExtensionMismatch {
        source_path: String,
        extensions: Vec<String>,
    },

    #[error("source file create found no project-owned root for session `{session_id}`")]
    SourceFileCreateMissingProjectRoot { session_id: String },

    #[error(
        "source file create found multiple project-owned roots for session `{session_id}`: {roots:?}"
    )]
    SourceFileCreateAmbiguousProjectRoot {
        session_id: String,
        roots: Vec<String>,
    },

    #[error("source file create found no source root `{source_root}` for session `{session_id}`")]
    SourceFileCreateMissingSourceRoot {
        session_id: String,
        source_root: String,
    },

    #[error(
        "source file create found multiple source roots `{source_root}` for session `{session_id}`: {roots:?}"
    )]
    SourceFileCreateAmbiguousSourceRoot {
        session_id: String,
        source_root: String,
        roots: Vec<String>,
    },

    #[error(
        "workspace entry `{source_path}` in view {workspace_id} has no source root for scan folder {scan_folder_id}"
    )]
    MissingWorkspaceEntrySourceRoot {
        workspace_id: i64,
        scan_folder_id: i64,
        source_path: String,
    },

    #[error(
        "workspace entry `{source_path}` in view {workspace_id} has multiple source roots for scan folder {scan_folder_id}: {roots:?}"
    )]
    AmbiguousWorkspaceEntrySourceRoot {
        workspace_id: i64,
        scan_folder_id: i64,
        source_path: String,
        roots: Vec<String>,
    },

    #[error(
        "source file create has no registered template for schema `{schema_type}` path `{source_path}`"
    )]
    SourceFileCreateTemplateUnavailable {
        schema_type: String,
        source_path: String,
    },

    #[error(
        "source file create template is ambiguous for schema `{schema_type}` path `{source_path}`: {owners:?}"
    )]
    SourceFileCreateTemplateAmbiguous {
        schema_type: String,
        source_path: String,
        owners: Vec<&'static str>,
    },

    #[error(
        "source file create template `{owner}` failed for schema `{schema_type}` path `{source_path}`: {reason}"
    )]
    SourceFileCreateTemplateFailed {
        owner: &'static str,
        schema_type: String,
        source_path: String,
        reason: String,
    },

    #[error("source file create payload side-channel is missing its brokered capability")]
    MissingSourceFileCreatePayloadCapability,

    #[error("source file create payload side-channel capability does not match request")]
    SourceFileCreatePayloadCapabilityMismatch,

    #[error("invalid source file create payload side channel: {0}")]
    SourceFileCreatePayload(#[from] StagingFileSideChannelError),

    #[error(
        "source file create payload for `{source_path}` has {byte_length} bytes, exceeding DB i64 range"
    )]
    SourceFileCreatePayloadTooLarge {
        source_path: String,
        byte_length: u64,
    },

    #[error(
        "cannot {operation} source file `{source_path}` because the DB-owned document or payload has unsaved editor revisions"
    )]
    SourceFileHasUnsavedEdits {
        operation: &'static str,
        source_path: String,
    },

    #[error(
        "source path `{source_path}` under source root `{source_root}` escapes the source root"
    )]
    SourceFilePathEscapesRoot {
        source_root: PathBuf,
        source_path: String,
    },

    #[error("cannot delete source file `{path}` because it is a directory")]
    SourceFileDeleteDirectory { path: PathBuf },

    #[error("cannot stage source-file deletion outside root `{path}`")]
    SourceFileDeleteStagingUnavailable { path: PathBuf },

    #[error("failed to delete source file `{path}`: {source}")]
    SourceFileDeleteIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot move source file `{path}` because it is a directory")]
    SourceFileMoveDirectory { path: PathBuf },

    #[error("cannot move source file to `{path}` because the target exists")]
    SourceFileMoveTargetExists { path: PathBuf },

    #[error("failed to create parent directory `{path}` for moved source file: {source}")]
    SourceFileMoveCreateParent {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to move source file `{from}` to `{to}`: {source}")]
    SourceFileMoveIo {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "source file {operation} database write failed ({database}) and filesystem rollback from `{from}` to `{to}` also failed: {rollback}"
    )]
    SourceFileMutationRollback {
        operation: &'static str,
        database: Box<Self>,
        from: PathBuf,
        to: PathBuf,
        #[source]
        rollback: std::io::Error,
    },

    #[error(
        "force reprocess found no source `{source_path}` under source root `{source_root}` for session `{session_id}`"
    )]
    ForceReprocessMissingSource {
        session_id: String,
        source_root: String,
        source_path: String,
    },

    #[error("cannot force reprocess source `{source_path}` while it has asset status {status:?}")]
    ForceReprocessUnavailableStatus { source_path: String, status: DbDiff },

    #[error("force reprocess source `{source_path}` produced no asset jobs")]
    ForceReprocessNoJobs { source_path: String },

    #[error("force reprocess enqueued job count {count} exceeds protocol UInt32 range")]
    ForceReprocessJobCountOverflow { count: usize },

    #[error("invalid workspace entry page request: {reason}")]
    InvalidWorkspaceEntryPageRequest { reason: String },

    #[error("asset processor has no project-instance workspace attached")]
    MissingAttachedWorkspace,

    #[error("asset processor has no project-instance data paths attached")]
    MissingProjectDataPaths,

    #[error(
        "asset processor project data belongs to workspace `{expected}`, but workspace {workspace_id} resolves to `{actual}`"
    )]
    ProjectDataWorkspaceMismatch {
        workspace_id: i64,
        expected: PathBuf,
        actual: PathBuf,
    },

    #[error("invalid catalog products request: {reason}")]
    InvalidCatalogProductsRequest { reason: String },

    #[error("invalid release content request: {reason}")]
    InvalidReleaseContentRequest { reason: String },

    #[error(
        "asset-processor workspace {workspace_id} has invalid release-content cache root `{root}`: {reason}"
    )]
    InvalidReleaseContentCacheRoot {
        workspace_id: i64,
        root: PathBuf,
        reason: String,
    },

    #[error(
        "asset-processor workspace {workspace_id} has invalid release-content platform `{platform}`"
    )]
    InvalidReleaseContentPlatform { workspace_id: i64, platform: String },

    #[error("invalid release content product `{product_path}`: {reason}")]
    InvalidReleaseContentProduct {
        product_path: String,
        reason: String,
    },

    #[error("duplicate release content product for asset {asset_guid}:{sub_id}")]
    DuplicateReleaseContentProduct { asset_guid: uuid::Uuid, sub_id: u32 },

    #[error("failed to read release content asset catalog `{path}`: {source}")]
    ReleaseContentCatalogRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("release content asset catalog `{path}` is not a file")]
    ReleaseContentCatalogNotFile { path: PathBuf },

    #[error("release content asset catalog `{path}` is empty")]
    ReleaseContentCatalogEmpty { path: PathBuf },

    #[error("failed to read release content product `{product_path}` at `{path}`: {source}")]
    ReleaseContentProductRead {
        product_path: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("release content product `{product_path}` at `{path}` is not a file")]
    ReleaseContentProductNotFile { product_path: String, path: PathBuf },

    #[error(
        "release content product `{product_path}` at `{path}` has {actual} bytes, expected {expected}"
    )]
    ReleaseContentProductLengthMismatch {
        product_path: String,
        path: PathBuf,
        expected: u64,
        actual: u64,
    },

    #[error("invalid asset worker job request: {reason}")]
    InvalidWorkerJobRequest { reason: String },

    #[error("asset builder `{builder_name}` failed create_jobs for source `{source_path}`")]
    BuilderCreateJobsFailed {
        builder_name: &'static str,
        source_path: String,
    },

    #[error("create_jobs source `{source_path}` references missing workspace {workspace_id}")]
    MissingCreateJobsWorkspace {
        source_path: String,
        workspace_id: i64,
    },

    #[error("create_jobs source path `{source_path}` is not safe relative")]
    UnsafeCreateJobsSourcePath { source_path: String },

    #[error("failed to read create_jobs source `{source_path}` at `{path}`: {source}")]
    ReadCreateJobsSource {
        source_path: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "asset builder `{builder_name}` returned invalid create_jobs result for source `{source_path}`: {reason}"
    )]
    InvalidBuilderCreateJobs {
        builder_name: &'static str,
        source_path: String,
        reason: String,
    },

    #[error(
        "asset job attempt {asset_job_attempt_id} references missing asset identity {asset_identity_id}"
    )]
    MissingAssetJobSourceContext {
        asset_job_attempt_id: i64,
        asset_identity_id: i64,
    },

    #[error(
        "workspace asset entry {workspace_asset_entry_id} references missing asset identity {asset_identity_id}"
    )]
    MissingWorkspaceAssetIdentity {
        workspace_asset_entry_id: i64,
        asset_identity_id: i64,
    },

    #[error(
        "asset job attempt {asset_job_attempt_id} references missing workspace asset entry for view {workspace_id} asset identity {asset_identity_id}"
    )]
    MissingAssetJobWorkspaceEntry {
        asset_job_attempt_id: i64,
        workspace_id: i64,
        asset_identity_id: i64,
    },

    #[error(
        "authored asset `{source_path}` in workspace {workspace_id} has no saved payload checkpoint"
    )]
    AuthoredAssetMissingSavedPayload {
        workspace_id: i64,
        source_path: String,
    },

    #[error(
        "authored asset `{source_path}` in workspace {workspace_id} saved payload hash `{actual}` does not match record hash `{expected}`"
    )]
    AuthoredAssetRecordPayloadHashMismatch {
        workspace_id: i64,
        source_path: String,
        expected: String,
        actual: String,
    },

    #[error(
        "authored source payload for attempt {asset_job_attempt_id} `{source_path}` hash `{actual}` does not match recorded asset hash `{expected}`"
    )]
    AuthoredAssetSavedPayloadHashMismatch {
        asset_job_attempt_id: i64,
        source_path: String,
        expected: String,
        actual: String,
    },

    #[error(
        "leased attempt {asset_job_attempt_id} needs a staging root for DB-authored source payload"
    )]
    MissingSourcePayloadStagingRoot { asset_job_attempt_id: i64 },

    #[error(
        "leased attempt {asset_job_attempt_id} source path `{source_path}` is not safe relative"
    )]
    UnsafeLeasedJobSourcePath {
        asset_job_attempt_id: i64,
        source_path: String,
    },

    #[error(
        "leased attempt {asset_job_attempt_id} source `{source_path}` preserves GUID {preserved}, but the recorded source identity is {recorded}"
    )]
    LeasedSourceMetaGuidMismatch {
        asset_job_attempt_id: i64,
        source_path: String,
        recorded: uuid::Uuid,
        preserved: uuid::Uuid,
    },

    #[error(
        "failed to read leased source `{source_path}` at `{path}` for attempt {asset_job_attempt_id}: {source}"
    )]
    ReadLeasedJobSource {
        asset_job_attempt_id: i64,
        source_path: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "source file for leased attempt {asset_job_attempt_id} `{source_path}` hash `{actual}` does not match recorded asset hash `{expected}`"
    )]
    LeasedSourceFileHashMismatch {
        asset_job_attempt_id: i64,
        source_path: String,
        expected: String,
        actual: String,
    },

    #[error(
        "failed to write source payload side channel `{path}` for attempt {asset_job_attempt_id}: {source}"
    )]
    WriteSourcePayload {
        asset_job_attempt_id: i64,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("source payload staging task failed: {error}")]
    SourcePayloadTask { error: tokio::task::JoinError },
}

fn validate_jobs_capability(
    capability: &Capability,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), AssetProcessorError> {
    validate_capability_basics(capability)?;
    if !matches!(capability.role, ServiceRole::Worker) {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!("expected role Worker but got {:?}", capability.role),
        });
    }
    validate_asset_worker_service(capability)?;
    validate_permission(capability, ASSET_JOBS_PERMISSION)?;
    validate_capability_grant(capability, capability_grants, ASSET_JOBS_PERMISSION)
}

fn validate_asset_worker_service(capability: &Capability) -> Result<(), AssetProcessorError> {
    if capability.service.namespace != ASSET_WORKER_SERVICE_NAMESPACE
        || capability.service.name != ASSET_WORKER_SERVICE_NAME
    {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!(
                "expected asset.jobs service `{ASSET_WORKER_SERVICE_NAMESPACE}/{ASSET_WORKER_SERVICE_NAME}` but got `{}/{}`",
                capability.service.namespace, capability.service.name
            ),
        });
    }
    Ok(())
}

fn validate_renew_asset_job_lease_request(
    request: &RenewAssetJobLeaseRequest,
) -> Result<(), AssetProcessorError> {
    if request.asset_job_attempt_id <= 0 {
        return Err(invalid_worker_job_request(
            "renew asset job lease requires a positive asset job attempt id",
        ));
    }
    validate_lease_owner(&request.lease_owner)?;
    if request.grant_key.is_nil() {
        return Err(invalid_worker_job_request("grant key must not be nil"));
    }
    Ok(())
}

fn validate_worker_attempt_request(
    asset_job_attempt_id: i64,
    lease_owner: &str,
    event_unix_ms: i64,
    operation: &'static str,
) -> Result<(), AssetProcessorError> {
    if asset_job_attempt_id <= 0 {
        return Err(invalid_worker_job_request(format!(
            "{operation} requires a positive asset job attempt id"
        )));
    }
    validate_lease_owner(lease_owner)?;
    validate_unix_ms("event_unix_ms", event_unix_ms)
}

fn validate_lease_owner(lease_owner: &str) -> Result<(), AssetProcessorError> {
    if lease_owner.trim().is_empty() {
        return Err(invalid_worker_job_request("lease owner must not be empty"));
    }
    if lease_owner.trim() != lease_owner {
        return Err(invalid_worker_job_request(
            "lease owner must not have leading or trailing whitespace",
        ));
    }
    Ok(())
}

fn validate_unix_ms(field: &'static str, value: i64) -> Result<(), AssetProcessorError> {
    if value < 0 {
        return Err(invalid_worker_job_request(format!(
            "{field} must be non-negative"
        )));
    }
    Ok(())
}

fn invalid_worker_job_request(reason: impl Into<String>) -> AssetProcessorError {
    AssetProcessorError::InvalidWorkerJobRequest {
        reason: reason.into(),
    }
}

fn validate_read_capability(
    capability: &Capability,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), AssetProcessorError> {
    validate_capability_basics(capability)?;
    if !matches!(
        capability.role,
        ServiceRole::Editor
            | ServiceRole::ProjectHost
            | ServiceRole::SessionSupervisor
            | ServiceRole::AssetProcessor
            | ServiceRole::Worker
    ) {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!(
                "expected role Editor, ProjectHost, SessionSupervisor, AssetProcessor, or Worker but got {:?}",
                capability.role
            ),
        });
    }
    validate_permission(capability, ASSET_READ_PERMISSION)?;
    validate_capability_grant(capability, capability_grants, ASSET_READ_PERMISSION)
}

fn validate_source_asset_record_capability(
    capability: &Capability,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), AssetProcessorError> {
    validate_capability_basics(capability)?;
    if !matches!(
        capability.role,
        ServiceRole::ProjectHost | ServiceRole::SessionSupervisor | ServiceRole::AssetProcessor
    ) {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!(
                "expected role ProjectHost, SessionSupervisor, or AssetProcessor but got {:?}",
                capability.role
            ),
        });
    }
    validate_permission(capability, ASSET_WRITE_PERMISSION)?;
    validate_capability_grant(capability, capability_grants, ASSET_WRITE_PERMISSION)
}

fn validate_source_file_create_capability(
    capability: &Capability,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), AssetProcessorError> {
    validate_editor_asset_write_capability(capability, capability_grants)
}

fn validate_editor_asset_write_capability(
    capability: &Capability,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), AssetProcessorError> {
    validate_capability_basics(capability)?;
    if !matches!(
        capability.role,
        ServiceRole::Editor
            | ServiceRole::ProjectHost
            | ServiceRole::SessionSupervisor
            | ServiceRole::AssetProcessor
    ) {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!(
                "expected role Editor, ProjectHost, SessionSupervisor, or AssetProcessor but got {:?}",
                capability.role
            ),
        });
    }
    validate_permission(capability, ASSET_WRITE_PERMISSION)?;
    validate_capability_grant(capability, capability_grants, ASSET_WRITE_PERMISSION)
}

fn validate_project_host_asset_write_capability(
    capability: &Capability,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), AssetProcessorError> {
    validate_capability_basics(capability)?;
    if capability.role != ServiceRole::ProjectHost {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!(
                "structured source restore requires role ProjectHost, got {:?}",
                capability.role
            ),
        });
    }
    validate_permission(capability, ASSET_WRITE_PERMISSION)?;
    validate_capability_grant(capability, capability_grants, ASSET_WRITE_PERMISSION)
}

fn validate_source_file_capability_session(
    capability: &Capability,
    session_id: &str,
) -> Result<(), AssetProcessorError> {
    let session = parse_non_nil_session_uuid(session_id).map_err(|reason| {
        AssetProcessorError::InvalidSourceFileRequest {
            reason: format!("session id {reason}"),
        }
    })?;
    if capability
        .session
        .is_some_and(|capability_session| capability_session != session)
    {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!(
                "capability session does not match structured source-file session `{session_id}`"
            ),
        });
    }
    Ok(())
}

fn validate_capability_grant(
    capability: &Capability,
    capability_grants: &CapabilityGrantSet,
    required_permission: &'static str,
) -> Result<(), AssetProcessorError> {
    capability_grants
        .validate(capability, required_permission)
        .map_err(|error| AssetProcessorError::InvalidCapability {
            reason: error.to_string(),
        })?;
    Ok(())
}

fn validate_source_asset_record_request(
    request: &SourceAssetRecordRequest,
) -> Result<String, AssetProcessorError> {
    if request.session_id.trim().is_empty() {
        return Err(AssetProcessorError::InvalidSourceAssetRecord {
            reason: "session id cannot be empty".to_string(),
        });
    }
    parse_non_nil_session_uuid(&request.session_id).map_err(|reason| {
        AssetProcessorError::InvalidSourceAssetRecord {
            reason: format!("session id {reason}"),
        }
    })?;
    if request.workspace_root_id <= 0 {
        return Err(AssetProcessorError::InvalidSourceAssetRecord {
            reason: format!(
                "workspace source root id must be positive, got {}",
                request.workspace_root_id
            ),
        });
    }
    if request.owner_id.trim().is_empty() {
        return Err(AssetProcessorError::InvalidSourceAssetRecord {
            reason: "owner id cannot be empty".to_string(),
        });
    }
    if request.source_path.trim().is_empty() {
        return Err(AssetProcessorError::InvalidSourceAssetRecord {
            reason: "source path cannot be empty".to_string(),
        });
    }
    if let Some(schema_type) = request.schema_type.as_deref()
        && schema_type.trim().is_empty()
    {
        return Err(AssetProcessorError::InvalidSourceAssetRecord {
            reason: "schema type cannot be empty when present".to_string(),
        });
    }
    let source_path = validate_asset_db_relative_path(&request.source_path).ok_or_else(|| {
        AssetProcessorError::InvalidSourceAssetRecord {
            reason: format!(
                "source path `{}` must be a canonical asset-db relative path",
                request.source_path
            ),
        }
    })?;
    if source_path.is_empty() {
        return Err(AssetProcessorError::InvalidSourceAssetRecord {
            reason: format!(
                "source path `{}` must be a canonical asset-db relative path",
                request.source_path
            ),
        });
    }
    if request.content_hash.len() != blake3::OUT_LEN {
        return Err(AssetProcessorError::InvalidSourceAssetRecord {
            reason: format!(
                "content hash must be {} bytes, got {}",
                blake3::OUT_LEN,
                request.content_hash.len()
            ),
        });
    }
    if request.diagnostics_count < 0 {
        return Err(AssetProcessorError::InvalidSourceAssetRecord {
            reason: "diagnostics count cannot be negative".to_string(),
        });
    }
    Ok(source_path)
}

fn validate_source_file_create_request(
    request: &SourceFileCreateRequest,
) -> Result<String, AssetProcessorError> {
    if request.session_id.trim().is_empty() {
        return Err(invalid_source_file_create_request(
            "session id cannot be empty",
        ));
    }
    parse_non_nil_session_uuid(&request.session_id)
        .map_err(|reason| invalid_source_file_create_request(format!("session id {reason}")))?;
    if request.source_root.trim().is_empty() {
        return Err(invalid_source_file_create_request(
            "source root cannot be empty",
        ));
    }
    if request.source_root.trim() != request.source_root {
        return Err(invalid_source_file_create_request(
            "source root cannot have leading or trailing whitespace",
        ));
    }
    if request.source_path.trim().is_empty() {
        return Err(invalid_source_file_create_request(
            "source path cannot be empty",
        ));
    }
    if request.schema_type.trim().is_empty() {
        return Err(invalid_source_file_create_request(
            "schema type cannot be empty",
        ));
    }
    let source_path = validate_asset_db_relative_path(&request.source_path).ok_or_else(|| {
        invalid_source_file_create_request(format!(
            "source path `{}` must be a canonical asset-db relative path",
            request.source_path
        ))
    })?;
    if source_path.is_empty() {
        return Err(invalid_source_file_create_request(format!(
            "source path `{}` must be a canonical asset-db relative path",
            request.source_path
        )));
    }
    if request.changed_unix_ms < 0 {
        return Err(invalid_source_file_create_request(
            "changed timestamp cannot be negative",
        ));
    }
    Ok(source_path)
}

fn invalid_source_file_create_request(reason: impl Into<String>) -> AssetProcessorError {
    AssetProcessorError::InvalidSourceFileCreateRequest {
        reason: reason.into(),
    }
}

fn validate_source_file_delete_request(
    request: &SourceFileDeleteRequest,
) -> Result<String, AssetProcessorError> {
    if request.session_id.trim().is_empty() {
        return Err(invalid_source_file_delete_request(
            "session id cannot be empty",
        ));
    }
    parse_non_nil_session_uuid(&request.session_id)
        .map_err(|reason| invalid_source_file_delete_request(format!("session id {reason}")))?;
    if request.source_root.trim().is_empty() {
        return Err(invalid_source_file_delete_request(
            "source root cannot be empty",
        ));
    }
    if request.source_root.trim() != request.source_root {
        return Err(invalid_source_file_delete_request(
            "source root cannot have leading or trailing whitespace",
        ));
    }
    if request.source_path.trim().is_empty() {
        return Err(invalid_source_file_delete_request(
            "source path cannot be empty",
        ));
    }
    let source_path = validate_asset_db_relative_path(&request.source_path).ok_or_else(|| {
        invalid_source_file_delete_request(format!(
            "source path `{}` must be a canonical asset-db relative path",
            request.source_path
        ))
    })?;
    if request.changed_unix_ms < 0 {
        return Err(invalid_source_file_delete_request(
            "changed timestamp cannot be negative",
        ));
    }
    Ok(source_path)
}

fn invalid_source_file_delete_request(reason: impl Into<String>) -> AssetProcessorError {
    AssetProcessorError::InvalidSourceFileDeleteRequest {
        reason: reason.into(),
    }
}

fn validate_source_file_move_request(
    request: &SourceFileMoveRequest,
) -> Result<(String, String), AssetProcessorError> {
    if request.session_id.trim().is_empty() {
        return Err(invalid_source_file_move_request(
            "session id cannot be empty",
        ));
    }
    parse_non_nil_session_uuid(&request.session_id)
        .map_err(|reason| invalid_source_file_move_request(format!("session id {reason}")))?;
    if request.source_root.trim().is_empty() {
        return Err(invalid_source_file_move_request(
            "source root cannot be empty",
        ));
    }
    if request.source_root.trim() != request.source_root {
        return Err(invalid_source_file_move_request(
            "source root cannot have leading or trailing whitespace",
        ));
    }
    let from_source_path =
        validate_asset_db_relative_path(&request.from_source_path).ok_or_else(|| {
            invalid_source_file_move_request(format!(
                "from source path `{}` must be a canonical asset-db relative path",
                request.from_source_path
            ))
        })?;
    let to_source_path =
        validate_asset_db_relative_path(&request.to_source_path).ok_or_else(|| {
            invalid_source_file_move_request(format!(
                "to source path `{}` must be a canonical asset-db relative path",
                request.to_source_path
            ))
        })?;
    if from_source_path == to_source_path {
        return Err(invalid_source_file_move_request(
            "from source path and to source path must differ",
        ));
    }
    if request.changed_unix_ms < 0 {
        return Err(invalid_source_file_move_request(
            "changed timestamp cannot be negative",
        ));
    }
    Ok((from_source_path, to_source_path))
}

fn invalid_source_file_move_request(reason: impl Into<String>) -> AssetProcessorError {
    AssetProcessorError::InvalidSourceFileMoveRequest {
        reason: reason.into(),
    }
}

fn validate_force_reprocess_asset_request(
    request: &ForceReprocessAssetRequest,
) -> Result<String, AssetProcessorError> {
    if request.session_id.trim().is_empty() {
        return Err(invalid_force_reprocess_asset_request(
            "session id cannot be empty",
        ));
    }
    parse_non_nil_session_uuid(&request.session_id)
        .map_err(|reason| invalid_force_reprocess_asset_request(format!("session id {reason}")))?;
    if request.source_root.trim().is_empty() {
        return Err(invalid_force_reprocess_asset_request(
            "source root cannot be empty",
        ));
    }
    if request.source_root.trim() != request.source_root {
        return Err(invalid_force_reprocess_asset_request(
            "source root cannot have leading or trailing whitespace",
        ));
    }
    if request.source_path.trim().is_empty() {
        return Err(invalid_force_reprocess_asset_request(
            "source path cannot be empty",
        ));
    }
    validate_asset_db_relative_path(&request.source_path).ok_or_else(|| {
        invalid_force_reprocess_asset_request(format!(
            "source path `{}` must be a canonical asset-db relative path",
            request.source_path
        ))
    })
}

fn invalid_force_reprocess_asset_request(reason: impl Into<String>) -> AssetProcessorError {
    AssetProcessorError::InvalidForceReprocessAssetRequest {
        reason: reason.into(),
    }
}

fn validate_reconcile_asset_sources_request(
    request: &ReconcileAssetSourcesRequest,
    capability_grants: &CapabilityGrantSet,
) -> Result<(), AssetProcessorError> {
    validate_editor_asset_write_capability(&request.capability, capability_grants)?;
    if request.session_id.trim().is_empty() {
        return Err(invalid_asset_source_reconcile_request(
            "session id cannot be empty",
        ));
    }
    parse_non_nil_session_uuid(&request.session_id)
        .map_err(|reason| invalid_asset_source_reconcile_request(format!("session id {reason}")))?;
    Ok(())
}

fn invalid_asset_source_reconcile_request(reason: impl Into<String>) -> AssetProcessorError {
    AssetProcessorError::InvalidAssetSourceReconcileRequest {
        reason: reason.into(),
    }
}

fn validate_source_dependents_request(
    request: &SourceDependentsRequest,
) -> Result<String, AssetProcessorError> {
    if request.session_id.trim().is_empty() {
        return Err(invalid_source_dependents_request(
            "session id cannot be empty",
        ));
    }
    parse_non_nil_session_uuid(&request.session_id)
        .map_err(|reason| invalid_source_dependents_request(format!("session id {reason}")))?;
    if request.source_root.trim().is_empty() {
        return Err(invalid_source_dependents_request(
            "source root cannot be empty",
        ));
    }
    if request.source_root.trim() != request.source_root {
        return Err(invalid_source_dependents_request(
            "source root cannot have leading or trailing whitespace",
        ));
    }
    if request.source_path.trim().is_empty() {
        return Err(invalid_source_dependents_request(
            "source path cannot be empty",
        ));
    }
    validate_asset_db_relative_path(&request.source_path).ok_or_else(|| {
        invalid_source_dependents_request(format!(
            "source path `{}` must be a canonical asset-db relative path",
            request.source_path
        ))
    })
}

fn invalid_source_dependents_request(reason: impl Into<String>) -> AssetProcessorError {
    AssetProcessorError::InvalidSourceDependentsRequest {
        reason: reason.into(),
    }
}

fn source_file_absolute_path(
    source_root: &str,
    source_path: &str,
) -> Result<PathBuf, AssetProcessorError> {
    let root = PathBuf::from(source_root);
    let path = root.join(source_path);
    if !path.starts_with(&root) {
        return Err(AssetProcessorError::SourceFilePathEscapesRoot {
            source_root: root,
            source_path: source_path.to_string(),
        });
    }
    Ok(path)
}

struct StagedSourceFileMutation {
    rollback_from: Option<PathBuf>,
    rollback_to: Option<PathBuf>,
    cleanup_after_commit: Option<PathBuf>,
}

impl StagedSourceFileMutation {
    fn delete(source_root: &str, source_path: &str) -> Result<Self, AssetProcessorError> {
        let original = source_file_absolute_path(source_root, source_path)?;
        let root = Path::new(source_root);
        let staging_parent = root.parent().ok_or_else(|| {
            AssetProcessorError::SourceFileDeleteStagingUnavailable {
                path: root.to_path_buf(),
            }
        })?;
        Self::delete_path(original, &staging_parent.join(".azoth-source-mutations"))
    }

    fn delete_path(original: PathBuf, staging_root: &Path) -> Result<Self, AssetProcessorError> {
        let metadata = match fs::symlink_metadata(&original) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::absent());
            }
            Err(source) => {
                return Err(AssetProcessorError::SourceFileDeleteIo {
                    path: original,
                    source,
                });
            }
        };
        if metadata.is_dir() {
            return Err(AssetProcessorError::SourceFileDeleteDirectory { path: original });
        }
        let file_name = original
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("source");
        fs::create_dir_all(staging_root).map_err(|source| {
            AssetProcessorError::SourceFileDeleteIo {
                path: staging_root.to_path_buf(),
                source,
            }
        })?;
        let tombstone = staging_root.join(format!("{file_name}.delete-{}", Uuid::now_v7()));
        fs::rename(&original, &tombstone).map_err(|source| {
            AssetProcessorError::SourceFileDeleteIo {
                path: original.clone(),
                source,
            }
        })?;
        Ok(Self {
            rollback_from: Some(tombstone.clone()),
            rollback_to: Some(original),
            cleanup_after_commit: Some(tombstone),
        })
    }

    fn move_file(
        source_root: &str,
        from_source_path: &str,
        to_source_path: &str,
    ) -> Result<Self, AssetProcessorError> {
        let from = source_file_absolute_path(source_root, from_source_path)?;
        let to = source_file_absolute_path(source_root, to_source_path)?;
        Self::move_paths(from, to)
    }

    fn move_paths(from: PathBuf, to: PathBuf) -> Result<Self, AssetProcessorError> {
        if fs::symlink_metadata(&to).is_ok() {
            return Err(AssetProcessorError::SourceFileMoveTargetExists { path: to });
        }
        let metadata = match fs::symlink_metadata(&from) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::absent());
            }
            Err(source) => {
                return Err(AssetProcessorError::SourceFileMoveIo { from, to, source });
            }
        };
        if metadata.is_dir() {
            return Err(AssetProcessorError::SourceFileMoveDirectory { path: from });
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                AssetProcessorError::SourceFileMoveCreateParent {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }
        fs::rename(&from, &to).map_err(|source| AssetProcessorError::SourceFileMoveIo {
            from: from.clone(),
            to: to.clone(),
            source,
        })?;
        Ok(Self {
            rollback_from: Some(to),
            rollback_to: Some(from),
            cleanup_after_commit: None,
        })
    }

    const fn absent() -> Self {
        Self {
            rollback_from: None,
            rollback_to: None,
            cleanup_after_commit: None,
        }
    }

    fn commit(mut self) {
        self.rollback_from = None;
        self.rollback_to = None;
        if let Some(path) = self.cleanup_after_commit.take()
            && let Err(source) = fs::remove_file(&path)
        {
            warn!(
                path = %path.display(),
                error = %source,
                "committed source deletion retained a tombstone"
            );
        }
    }

    fn rollback(
        mut self,
        operation: &'static str,
        database: AssetProcessorError,
    ) -> AssetProcessorError {
        let (Some(from), Some(to)) = (self.rollback_from.take(), self.rollback_to.take()) else {
            return database;
        };
        match fs::rename(&from, &to) {
            Ok(()) => database,
            Err(rollback) => AssetProcessorError::SourceFileMutationRollback {
                operation,
                database: Box::new(database),
                from,
                to,
                rollback,
            },
        }
    }
}

impl Drop for StagedSourceFileMutation {
    fn drop(&mut self) {
        let (Some(from), Some(to)) = (self.rollback_from.take(), self.rollback_to.take()) else {
            return;
        };
        if let Err(error) = fs::rename(&from, &to) {
            warn!(
                from = %from.display(),
                to = %to.display(),
                %error,
                "failed to roll back staged source-file mutation during unwind"
            );
        }
    }
}

fn validate_source_file_create_extension(
    source_path: &str,
    extensions: &[impl AsRef<str>],
) -> Result<(), AssetProcessorError> {
    if source_path_matches_extensions(source_path, extensions) {
        return Ok(());
    }

    Err(AssetProcessorError::SourceFileCreateExtensionMismatch {
        source_path: source_path.to_string(),
        extensions: extensions
            .iter()
            .map(|extension| extension.as_ref().to_string())
            .collect(),
    })
}

fn source_path_matches_extensions(source_path: &str, extensions: &[impl AsRef<str>]) -> bool {
    matching_source_extension_specificity(source_path, extensions).is_some()
}

fn matching_source_extension_specificity(
    source_path: &str,
    extensions: &[impl AsRef<str>],
) -> Option<usize> {
    let file_name = source_path
        .rsplit('/')
        .next()
        .unwrap_or(source_path)
        .to_ascii_lowercase();
    extensions
        .iter()
        .filter_map(|extension| {
            let extension = extension.as_ref().to_ascii_lowercase();
            if extension == "*" {
                Some(0)
            } else if file_name == extension || file_name.ends_with(&format!(".{extension}")) {
                Some(extension.len())
            } else {
                None
            }
        })
        .max()
}

fn source_file_create_source_root(
    db: &AssetDb,
    workspace_pk: i64,
    session_id: &str,
    source_root: &str,
) -> Result<
    (
        SelectWorkspaces,
        SelectWorkspaceRoots,
        az_assetdb::SelectRoots,
    ),
    AssetProcessorError,
> {
    let workspace = db.workspace_by_id(workspace_pk)?.ok_or_else(|| {
        AssetProcessorError::SourceFileCreateMissingSourceRoot {
            session_id: session_id.to_string(),
            source_root: source_root.to_string(),
        }
    })?;
    let expected_key = if source_root == PROJECT_SOURCE_ROOT {
        PortableKey::project_assets(&workspace.project)
            .as_str()
            .to_owned()
    } else {
        source_root.to_owned()
    };
    let mut candidates = Vec::new();
    for policy in db.workspace_roots(workspace_pk)? {
        let Some(root) = db.root_by_id(policy.root_pk)? else {
            continue;
        };
        // A root owner id is a project id, so the project source root is the
        // one this workspace owns rather than one a gem contributed.
        let owned_by_this_project = policy.owner == workspace.project;
        if root.key == expected_key && (source_root != PROJECT_SOURCE_ROOT || owned_by_this_project)
        {
            candidates.push((policy, root));
        }
    }
    match candidates.as_slice() {
        [] => Err(AssetProcessorError::SourceFileCreateMissingSourceRoot {
            session_id: session_id.to_string(),
            source_root: source_root.to_string(),
        }),
        [(policy, root)] => Ok((workspace, policy.clone(), root.clone())),
        roots => Err(AssetProcessorError::SourceFileCreateAmbiguousSourceRoot {
            session_id: session_id.to_string(),
            source_root: source_root.to_string(),
            roots: roots
                .iter()
                .map(|(policy, root)| format!("{}:{}", policy.workspace_root_id, root.key))
                .collect(),
        }),
    }
}

fn create_source_file_payload(
    request: &SourceFileCreateRequest,
    schema_type: Option<SourceSchemaType>,
    source_path: &str,
    registries: &Registries,
) -> Result<Vec<u8>, AssetProcessorError> {
    match &request.content {
        SourceFileCreateContent::DefaultTemplate => {
            let schema_type = schema_type.ok_or_else(|| {
                AssetProcessorError::SourceFileCreateTemplateUnavailable {
                    schema_type: request.schema_type.clone(),
                    source_path: source_path.to_string(),
                }
            })?;
            create_source_file_default_template(
                &request.schema_type,
                schema_type,
                source_path,
                registries,
            )
        }
        SourceFileCreateContent::Payload(handle) => {
            validate_side_channel_capability_matches(
                handle,
                &request.capability,
                "source file create payload",
            )
            .map_err(|error| match error {
                SideChannelCapabilityError::Missing { .. } => {
                    AssetProcessorError::MissingSourceFileCreatePayloadCapability
                }
                SideChannelCapabilityError::Mismatch { .. } => {
                    AssetProcessorError::SourceFileCreatePayloadCapabilityMismatch
                }
            })?;
            Ok(read_verified_staging_file(handle)?.bytes)
        }
    }
}

fn create_source_file_default_template(
    schema_type_name: &str,
    schema_type: SourceSchemaType,
    source_path: &str,
    registries: &Registries,
) -> Result<Vec<u8>, AssetProcessorError> {
    let template_request = SourceFileTemplateRequest {
        schema_type,
        source_path,
    };
    let unavailable = || AssetProcessorError::SourceFileCreateTemplateUnavailable {
        schema_type: schema_type_name.to_string(),
        source_path: source_path.to_string(),
    };

    // A schema has at most one composed template: `SourceFileTemplateRegistration`
    // keys on the schema type, so two contributions offering one schema a
    // starting point fail composition instead of racing here.
    let attributed =
        composed_source_file_template(registries, schema_type).ok_or_else(unavailable)?;
    match attributed.entry.create(&template_request) {
        Ok(bytes) => Ok(bytes),
        Err(SourceFileTemplateError::Unsupported { .. }) => Err(unavailable()),
        Err(SourceFileTemplateError::Failed { reason }) => {
            Err(AssetProcessorError::SourceFileCreateTemplateFailed {
                owner: attributed.instance.gem.as_str(),
                schema_type: schema_type_name.to_string(),
                source_path: source_path.to_string(),
                reason,
            })
        }
    }
}

fn validate_workspace_entry_page_request(
    request: &WorkspaceEntryPageRequest,
) -> Result<usize, AssetProcessorError> {
    if request.page_size == 0 {
        return Err(AssetProcessorError::InvalidWorkspaceEntryPageRequest {
            reason: "page size must be greater than zero".to_string(),
        });
    }
    if let Some(after_entry_id) = request.after_entry_id
        && after_entry_id <= 0
    {
        return Err(AssetProcessorError::InvalidWorkspaceEntryPageRequest {
            reason: format!("after entry id must be positive, got {after_entry_id}"),
        });
    }
    Ok(request.page_size as usize)
}

fn validate_catalog_products_request(
    request: &CatalogProductsRequest,
) -> Result<(), AssetProcessorError> {
    if !is_safe_platform_component(&request.platform) {
        return Err(AssetProcessorError::InvalidCatalogProductsRequest {
            reason: format!(
                "platform `{}` must be one safe path component",
                request.platform
            ),
        });
    }
    Ok(())
}

fn validate_release_content_request(
    request: &ReleaseContentReadRequest,
) -> Result<(), AssetProcessorError> {
    if !is_safe_platform_component(&request.platform) {
        return Err(AssetProcessorError::InvalidReleaseContentRequest {
            reason: format!(
                "platform `{}` must be one safe path component",
                request.platform
            ),
        });
    }
    if let ReleaseContentTarget::ProductAsset { asset_guid, .. } = request.target
        && asset_guid.is_nil()
    {
        return Err(AssetProcessorError::InvalidReleaseContentRequest {
            reason: "product asset guid must not be nil".to_string(),
        });
    }
    Ok(())
}

fn parse_non_nil_session_uuid(session_id: &str) -> Result<uuid::Uuid, String> {
    let session_id = session_id.trim();
    let uuid = uuid::Uuid::parse_str(session_id)
        .map_err(|source| format!("`{session_id}` must be a UUID: {source}"))?;
    if uuid.is_nil() {
        return Err(format!("`{session_id}` must not be the nil UUID"));
    }
    Ok(uuid)
}

fn validate_asset_db_relative_path(path: &str) -> Option<String> {
    if path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains(':')
        || path.trim() != path
    {
        return None;
    }

    let mut has_component = false;
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return None;
        }
        has_component = true;
    }

    has_component.then(|| path.to_string())
}

fn validate_capability_basics(capability: &Capability) -> Result<(), AssetProcessorError> {
    if capability.audience != ASSET_PROCESSOR_AUDIENCE {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!(
                "expected audience `{ASSET_PROCESSOR_AUDIENCE}` but got `{}`",
                capability.audience
            ),
        });
    }
    if let Err(error) = capability.validate_lifetime() {
        return Err(AssetProcessorError::InvalidCapability {
            reason: error.to_string(),
        });
    }
    if let Some(session) = capability.session {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!(
                "asset-processor capabilities are project-scoped, but this capability is scoped to session `{session}`"
            ),
        });
    }
    Ok(())
}

fn validate_permission(
    capability: &Capability,
    permission: &'static str,
) -> Result<(), AssetProcessorError> {
    if !capability
        .permissions
        .iter()
        .any(|candidate| candidate == permission)
    {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!("missing required permission `{permission}`"),
        });
    }
    Ok(())
}

fn browser_asset_scan_folder_ids(
    db: &AssetDb,
    workspace_id: i64,
) -> Result<Vec<i64>, AssetProcessorError> {
    Ok(db
        .workspace_roots(workspace_id)?
        .into_iter()
        .map(|root| root.root_pk)
        .collect())
}

fn validate_asset_job_attempt_scope(
    db: &AssetDb,
    capability: &Capability,
    attached_workspace_id: Option<i64>,
    asset_job_attempt_id: i64,
    operation: &'static str,
) -> Result<(), AssetProcessorError> {
    let Some(attempt) = db.attempt_by_id(asset_job_attempt_id)? else {
        return Ok(());
    };
    let job = db
        .job_by_id(attempt.job_pk)?
        .ok_or(AssetProcessorError::MissingAssetJobAttempt {
            asset_job_attempt_id,
        })?;
    validate_attached_workspace_scope(
        capability,
        attached_workspace_id,
        job.workspace_pk,
        operation,
    )
}

fn validate_attached_workspace_scope(
    _capability: &Capability,
    attached_workspace_id: Option<i64>,
    workspace_id: i64,
    operation: &'static str,
) -> Result<(), AssetProcessorError> {
    let attached_workspace_id =
        attached_workspace_id.ok_or(AssetProcessorError::MissingAttachedWorkspace)?;
    if workspace_id != attached_workspace_id {
        return Err(AssetProcessorError::InvalidCapability {
            reason: format!(
                "capability cannot {operation} for workspace {workspace_id}; this project instance owns workspace {attached_workspace_id}"
            ),
        });
    }
    Ok(())
}

fn asset_builder_to_proto(builder: &BuildRule) -> AssetBuilderDescriptor {
    AssetBuilderDescriptor {
        name: builder.name.to_string(),
        builder_guid: builder.id.0,
        version: builder.version,
        analysis_fingerprint: build_rule_analysis_fingerprint(builder),
        patterns: builder
            .primary_source
            .patterns()
            .iter()
            .map(asset_builder_pattern_to_proto)
            .collect(),
        source_schema_types: builder
            .primary_source
            .schema_types()
            .iter()
            .map(|schema_type| (*schema_type).to_string())
            .collect(),
    }
}

fn build_rule_analysis_fingerprint(builder: &BuildRule) -> String {
    let mut formats = builder.product_formats.declared().map(|formats| {
        formats
            .iter()
            .map(|format| format.as_str())
            .collect::<Vec<_>>()
    });
    if let Some(formats) = formats.as_mut() {
        formats.sort_unstable();
    }
    format!(
        "{}\nproduct-formats={}",
        builder.analysis_fingerprint,
        formats.map_or_else(|| "dynamic".to_owned(), |formats| formats.join(","))
    )
}

fn source_schema_to_proto(
    attributed: &Attributed<SourceSchemaRegistration>,
    source_file_templates: &[Attributed<SourceFileTemplateRegistration>],
) -> Result<SourceSchemaDescriptor, AssetProcessorError> {
    let registration = attributed.entry;
    let schema_type = registration.schema_type();
    let authoring = registration.authoring();
    let file_templates = match authoring {
        BuilderSourceSchemaAuthoring::File { workflow } if workflow.can_create() => {
            source_file_templates_to_proto(schema_type, workflow, source_file_templates)?
        }
        BuilderSourceSchemaAuthoring::File { .. }
        | BuilderSourceSchemaAuthoring::ProjectDocument { .. } => Vec::new(),
    };
    Ok(SourceSchemaDescriptor {
        schema_type: schema_type.as_str().to_string(),
        owner: attributed.instance.gem.as_str().to_string(),
        label: registration.label().to_string(),
        category: registration.category().to_string(),
        authoring: source_schema_authoring_to_proto(authoring),
        file_templates,
    })
}

fn composed_product_formats_to_proto(registries: &Registries) -> Vec<ProductFormatDescriptor> {
    composed_product_formats(registries)
        .into_iter()
        .map(|attributed| ProductFormatDescriptor {
            id: attributed.entry.id().as_str().to_string(),
            current_version: attributed.entry.current_version(),
            owner: attributed.instance.gem.as_str().to_string(),
        })
        .collect()
}

fn graph_source_schemas_to_proto(
    registries: &Registries,
) -> Result<Vec<SourceSchemaDescriptor>, AssetProcessorError> {
    let descriptors = graph_source_schemas(registries)
        .map_err(|source| AssetProcessorError::InvalidBuilderCatalog {
            reason: format!("composed graph source schemas are invalid: {source}"),
        })?
        .into_iter()
        .map(graph_source_schema_to_proto)
        .collect::<Vec<_>>();
    Ok(descriptors)
}

fn graph_source_schema_to_proto(schema: RegisteredGraphSourceSchema) -> SourceSchemaDescriptor {
    let authoring = match schema.authoring {
        RegisteredGraphSourceAuthoring::ProjectDocument { schema_type } => {
            SourceSchemaAuthoring::ProjectDocument { schema_type }
        }
        RegisteredGraphSourceAuthoring::File {
            source_root,
            default_path_prefix,
            extensions,
            can_create,
            can_edit,
        } => SourceSchemaAuthoring::File {
            workflow: SourceFileWorkflowDescriptor {
                source_root,
                default_path_prefix,
                extensions,
                can_create,
                can_edit,
            },
        },
    };
    SourceSchemaDescriptor {
        schema_type: schema.schema_type,
        owner: schema.owner,
        label: schema.label,
        category: schema.category,
        authoring,
        file_templates: Vec::new(),
    }
}

fn source_file_templates_to_proto(
    schema_type: SourceSchemaType,
    workflow: az_asset_builder::SourceFileWorkflow,
    source_file_templates: &[Attributed<SourceFileTemplateRegistration>],
) -> Result<Vec<SourceFileTemplateDescriptor>, AssetProcessorError> {
    let mut seen_paths = BTreeMap::<String, String>::new();
    let mut templates = Vec::new();

    for attributed in source_file_templates
        .iter()
        .filter(|attributed| attributed.entry.schema_type() == schema_type)
    {
        let registration = attributed.entry;
        let owner = attributed.instance.gem.as_str();
        for candidate in registration.candidates() {
            let source_path = validate_asset_db_relative_path(&candidate.source_path).ok_or_else(
                || AssetProcessorError::InvalidBuilderCatalog {
                    reason: format!(
                        "source schema `{}` template from `{}` has non-canonical source path `{}`",
                        schema_type, owner, candidate.source_path
                    ),
                },
            )?;
            if !source_path_matches_extensions(&source_path, workflow.extensions()) {
                return Err(AssetProcessorError::InvalidBuilderCatalog {
                    reason: format!(
                        "source schema `{}` template from `{}` uses source path `{}` outside registered extensions {:?}",
                        schema_type,
                        owner,
                        source_path,
                        workflow.extensions()
                    ),
                });
            }
            if let Some(first_owner) = seen_paths.insert(source_path.clone(), owner.to_string()) {
                return Err(AssetProcessorError::InvalidBuilderCatalog {
                    reason: format!(
                        "source schema `{schema_type}` has duplicate source file template path `{source_path}` from `{first_owner}` and `{owner}`"
                    ),
                });
            }
            if candidate.label.trim() != candidate.label
                || candidate.description.trim() != candidate.description
            {
                return Err(AssetProcessorError::InvalidBuilderCatalog {
                    reason: format!(
                        "source schema `{schema_type}` template `{source_path}` from `{owner}` has untrimmed UI text"
                    ),
                });
            }
            templates.push(SourceFileTemplateDescriptor {
                owner: owner.to_string(),
                source_path,
                label: candidate.label,
                description: candidate.description,
            });
        }
    }

    templates.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.owner.cmp(&right.owner))
    });
    Ok(templates)
}

fn source_schema_authoring_to_proto(
    authoring: BuilderSourceSchemaAuthoring,
) -> SourceSchemaAuthoring {
    match authoring {
        BuilderSourceSchemaAuthoring::ProjectDocument { schema_type } => {
            SourceSchemaAuthoring::ProjectDocument {
                schema_type: schema_type.to_string(),
            }
        }
        BuilderSourceSchemaAuthoring::File { workflow } => SourceSchemaAuthoring::File {
            workflow: SourceFileWorkflowDescriptor {
                source_root: workflow.source_root().to_string(),
                default_path_prefix: workflow.default_path_prefix().to_string(),
                extensions: workflow
                    .extensions()
                    .iter()
                    .map(|extension| (*extension).to_string())
                    .collect(),
                can_create: workflow.can_create(),
                can_edit: workflow.can_edit(),
            },
        },
    }
}

fn ensure_builder_catalog_source_schema_coverage(
    builders: &[AssetBuilderDescriptor],
    source_schemas: &[SourceSchemaDescriptor],
) -> Result<(), AssetProcessorError> {
    let source_schema_types = source_schemas
        .iter()
        .map(|source_schema| source_schema.schema_type.as_str())
        .collect::<BTreeSet<_>>();

    for builder in builders {
        for source_schema_type in &builder.source_schema_types {
            if !source_schema_types.contains(source_schema_type.as_str()) {
                return Err(AssetProcessorError::InvalidBuilderCatalog {
                    reason: format!(
                        "builder `{}` references source schema type `{}` without a source schema registration",
                        builder.name, source_schema_type
                    ),
                });
            }
        }
    }

    Ok(())
}

fn ensure_unique_source_schema_descriptors(
    source_schemas: &[SourceSchemaDescriptor],
) -> Result<(), AssetProcessorError> {
    let mut seen = BTreeMap::<&str, &str>::new();
    for source_schema in source_schemas {
        if let Some(first_owner) = seen.insert(
            source_schema.schema_type.as_str(),
            source_schema.owner.as_str(),
        ) {
            return Err(AssetProcessorError::InvalidBuilderCatalog {
                reason: format!(
                    "source schema `{}` is registered by both `{}` and `{}`",
                    source_schema.schema_type, first_owner, source_schema.owner
                ),
            });
        }
    }
    Ok(())
}

fn ensure_source_file_template_registrations_match_schemas(
    source_schemas: &[Attributed<SourceSchemaRegistration>],
    source_file_templates: &[Attributed<SourceFileTemplateRegistration>],
) -> Result<(), AssetProcessorError> {
    let source_schema_authoring = source_schemas
        .iter()
        .map(|attributed| (attributed.entry.schema_type(), attributed.entry.authoring()))
        .collect::<BTreeMap<_, _>>();

    for attributed in source_file_templates {
        let schema_type = attributed.entry.schema_type();
        let owner = attributed.instance.gem.as_str();
        match source_schema_authoring.get(&schema_type).copied() {
            Some(BuilderSourceSchemaAuthoring::File { workflow }) if workflow.can_create() => {}
            Some(BuilderSourceSchemaAuthoring::File { .. }) => {
                return Err(AssetProcessorError::InvalidBuilderCatalog {
                    reason: format!(
                        "source file template from `{owner}` targets source schema `{schema_type}` but that schema is not default-creatable"
                    ),
                });
            }
            Some(BuilderSourceSchemaAuthoring::ProjectDocument { .. }) => {
                return Err(AssetProcessorError::InvalidBuilderCatalog {
                    reason: format!(
                        "source file template from `{owner}` targets project-document source schema `{schema_type}`"
                    ),
                });
            }
            None => {
                return Err(AssetProcessorError::InvalidBuilderCatalog {
                    reason: format!(
                        "source file template from `{owner}` targets unregistered source schema `{schema_type}`"
                    ),
                });
            }
        }
    }

    Ok(())
}

fn asset_builder_pattern_to_proto(pattern: &AssetBuilderPattern) -> AssetBuilderPatternDescriptor {
    match pattern {
        AssetBuilderPattern::StaticWildcard(pattern) => AssetBuilderPatternDescriptor {
            kind: AssetBuilderPatternKind::Wildcard,
            pattern: (*pattern).to_string(),
        },
        AssetBuilderPattern::Wildcard(pattern) => AssetBuilderPatternDescriptor {
            kind: AssetBuilderPatternKind::Wildcard,
            pattern: pattern.clone(),
        },
        AssetBuilderPattern::Regex(regex) => AssetBuilderPatternDescriptor {
            kind: AssetBuilderPatternKind::Regex,
            pattern: regex.as_str().to_string(),
        },
    }
}

fn workspace_snapshot_to_proto(
    row: SelectWorkspaces,
    source_roots: Vec<RegisteredSourceRoot>,
) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        workspace_id: row.workspace_id,
        project_id: row.project,
        workspace_root: row.root,
        branch: row.branch,
        created_unix_ms: row.created,
        updated_unix_ms: row.updated,
        roots: source_roots
            .into_iter()
            .map(workspace_root_to_proto)
            .collect(),
    }
}

fn workspace_root_to_proto(row: RegisteredSourceRoot) -> WorkspaceRoot {
    WorkspaceRoot {
        workspace_root_id: row.workspace_root_pk,
        workspace_id: row.workspace_pk,
        root_id: row.root_pk,
        declared_root_id: row.id,
        owner_id: row.owner,
        source_root: row.path,
        display_name: row.display_name,
        portable_key: row.portable_key,
        mount: row.mount,
        recursive: row.recursive,
        watch: row.watch,
        writable: row.writable,
        output_prefix: row.output_prefix,
        is_root: row.role.is_required(),
    }
}

fn workspace_asset_entry_to_proto(
    db: &AssetDb,
    asset: &SelectAssets,
    row: SelectEntries,
) -> Result<WorkspaceEntry, AssetProcessorError> {
    if row.asset_pk != asset.asset_id {
        return Err(AssetProcessorError::MissingWorkspaceAssetIdentity {
            workspace_asset_entry_id: row.entry_id,
            asset_identity_id: row.asset_pk,
        });
    }
    let source_root = db
        .workspace_root_for_root(row.workspace_pk, row.root_pk)?
        .ok_or(AssetProcessorError::MissingWorkspaceAssetIdentity {
            workspace_asset_entry_id: row.entry_id,
            asset_identity_id: row.asset_pk,
        })?;
    let jobs = db
        .job_activities_for_assets(row.workspace_pk, &[asset.asset_id])?
        .remove(&asset.asset_id)
        .unwrap_or_default()
        .into_iter()
        .map(|activity| {
            job_activity_to_proto(
                activity,
                asset.guid,
                &row.path,
                &source_root.path,
                row.schema.as_deref(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WorkspaceEntry {
        entry_id: row.entry_id,
        workspace_id: row.workspace_pk,
        asset_guid: asset.guid,
        root_id: row.root_pk,
        source_path: row.path,
        schema_type: row.schema.clone(),
        content_hash: row.digest.to_string(),
        diff: db_workspace_entry_diff_to_proto(row.diff),
        diagnostics_count: row.diagnostics,
        updated_unix_ms: row.updated,
        jobs,
    })
}

fn workspace_entry_snapshot_to_proto(
    row: WorkspaceEntrySnapshot,
) -> Result<WorkspaceEntry, AssetProcessorError> {
    let jobs = row
        .jobs
        .into_iter()
        .map(|activity| {
            job_activity_to_proto(
                activity,
                row.asset_guid,
                &row.source_path,
                &row.source_root,
                row.schema.as_deref(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WorkspaceEntry {
        entry_id: row.entry_id,
        workspace_id: row.workspace_pk,
        asset_guid: row.asset_guid,
        root_id: row.root_pk,
        source_path: row.source_path,
        schema_type: row.schema,
        content_hash: row.digest.to_string(),
        diff: db_workspace_entry_diff_to_proto(row.diff),
        diagnostics_count: row.diagnostics,
        updated_unix_ms: row.updated,
        jobs,
    })
}

fn authored_publication_written(
    result: PublishAuthoredSourceResult,
    source_path: &str,
) -> Result<(SelectAssets, SelectEntries), AssetProcessorError> {
    match result {
        PublishAuthoredSourceResult::Written(written) => Ok((written.asset, written.entry)),
        PublishAuthoredSourceResult::Conflict(_) => {
            Err(AssetProcessorError::AuthoredSourcePublicationRejected {
                source_path: source_path.to_owned(),
                reason: "payload revision conflict",
            })
        }
        PublishAuthoredSourceResult::Missing => {
            Err(AssetProcessorError::AuthoredSourcePublicationRejected {
                source_path: source_path.to_owned(),
                reason: "expected payload is missing",
            })
        }
        PublishAuthoredSourceResult::ScopeMismatch => {
            Err(AssetProcessorError::AuthoredSourcePublicationRejected {
                source_path: source_path.to_owned(),
                reason: "workspace/root/project scope mismatch",
            })
        }
        PublishAuthoredSourceResult::InvalidCheckpoint => {
            Err(AssetProcessorError::AuthoredSourcePublicationRejected {
                source_path: source_path.to_owned(),
                reason: "invalid saved checkpoint transition",
            })
        }
        PublishAuthoredSourceResult::InvalidSourceProjection => {
            Err(AssetProcessorError::AuthoredSourcePublicationRejected {
                source_path: source_path.to_owned(),
                reason: "payload and source projection disagree",
            })
        }
        PublishAuthoredSourceResult::LocatorConflict { .. } => {
            Err(AssetProcessorError::AuthoredSourcePublicationRejected {
                source_path: source_path.to_owned(),
                reason: "source locator belongs to a different asset identity",
            })
        }
    }
}

fn source_dependents_to_proto(
    source_path: String,
    row: az_assetdb::SourceDependents,
) -> Result<SourceDependentsResult, AssetProcessorError> {
    let source_dependents = row
        .sources
        .into_iter()
        .map(|dependent| SourceDependentSource {
            source_edge_id: dependent.edge_id,
            source_path: dependent.source_path,
            builder_guid: dependent.builder,
            relation: match dependent.relation {
                DbRelation::SourceToSource => SourceRelation::SourceToSource,
                DbRelation::JobToJob => SourceRelation::JobToJob,
                DbRelation::SourceLikeMatch => SourceRelation::SourceLikeMatch,
            },
        })
        .collect();
    let job_dependents = row
        .jobs
        .into_iter()
        .map(|dependent| {
            Ok(SourceDependentJob {
                job_edge_id: dependent.edge_id,
                job_id: dependent.job_id,
                latest_attempt_id: dependent.latest_attempt_id,
                source_path: dependent.source_path,
                owner: job_owner(dependent.job_id, dependent.kind, dependent.builder)?,
                job_key: dependent.job_key,
                platform: dependent.platform,
                dependency_job_key: dependent.dependency_job_key,
                dependency_platform: dependent.dependency_platform,
                kind: db_job_dependency_kind_to_proto(dependent.coupling),
                product_paths: dependent.product_paths,
            })
        })
        .collect::<Result<Vec<_>, AssetProcessorError>>()?;
    Ok(SourceDependentsResult {
        source_path,
        source_dependents,
        job_dependents,
    })
}

fn job_owner(
    job_id: i64,
    kind: DbWork,
    builder: Option<Uuid>,
) -> Result<JobOwner, AssetProcessorError> {
    match (kind, builder) {
        (DbWork::Plan, None) => Ok(JobOwner::Plan),
        (DbWork::Build, Some(builder)) => Ok(JobOwner::Build(builder)),
        _ => Err(AssetProcessorError::InvalidJobInspection {
            job_id,
            reason: "job kind and builder identity disagree".to_string(),
        }),
    }
}

fn job_record_to_proto(
    job: &SelectJobs,
    source_guid: Uuid,
    source_path: &str,
    source_root: &str,
    source_schema_type: Option<&str>,
) -> Result<JobRecord, AssetProcessorError> {
    let owner = job_owner(job.job_id, job.kind, job.builder)?;
    Ok(JobRecord {
        job_id: job.job_id,
        workspace_id: job.workspace_pk,
        source_guid,
        source_path: source_path.to_string(),
        source_root: source_root.to_string(),
        source_schema_type: source_schema_type.map(ToOwned::to_owned),
        owner,
        key: job.key.clone(),
        platform: job.platform.clone(),
        status: db_job_status_to_proto(job.job_id, job.status)?,
        ready: job.ready,
        attempts: job.attempts,
    })
}

fn attempt_record_to_proto(
    attempt: SelectAttempts,
) -> Result<JobAttemptRecord, AssetProcessorError> {
    Ok(JobAttemptRecord {
        attempt_id: attempt.attempt_id,
        job_id: attempt.job_pk,
        ordinal: attempt.ordinal,
        status: db_attempt_status_to_proto(attempt.attempt_id, attempt.status)?,
        owner: attempt.owner,
        staging: attempt.staging,
        finished_unix_ms: attempt.finished,
        error_count: attempt.errors,
        warning_count: attempt.warnings,
    })
}

fn job_activity_to_proto(
    activity: JobActivitySnapshot,
    source_guid: Uuid,
    source_path: &str,
    source_root: &str,
    source_schema_type: Option<&str>,
) -> Result<az_proto_asset::JobActivity, AssetProcessorError> {
    Ok(az_proto_asset::JobActivity {
        job: job_record_to_proto(
            &activity.job,
            source_guid,
            source_path,
            source_root,
            source_schema_type,
        )?,
        attempt: activity.attempt.map(attempt_record_to_proto).transpose()?,
    })
}

fn db_job_inspection_to_proto(
    inspection: DbJobInspection,
) -> Result<ProtoJobInspection, AssetProcessorError> {
    let job_id = inspection.job.job_id;
    let job = job_record_to_proto(
        &inspection.job,
        inspection.asset.guid,
        &inspection.entry.path,
        &inspection.workspace_root.path,
        inspection.entry.schema.as_deref(),
    )?;
    let attempt = inspection
        .attempt
        .map(attempt_record_to_proto)
        .transpose()?;
    let products = inspection
        .products
        .into_iter()
        .map(|inspected| {
            let product = inspected.product;
            let product_format_version =
                db_product_format_version_to_proto(&product.path, product.version)?;
            Ok(JobProductRecord {
                product_id: product.product_id,
                job_id: product.job_pk,
                path: product.path,
                asset_type: product.kind,
                sub_id: product.sub_id,
                product_format: product.format,
                product_format_version,
                catalog_path_registration: db_catalog_path_registration_to_proto(
                    product.registration,
                ),
                content_hash: product.digest.to_string(),
                byte_length: product.bytes,
                aliases: product.aliases.into_vec(),
                edges: inspected
                    .edges
                    .into_iter()
                    .map(|edge| JobProductEdgeRecord {
                        product_edge_id: edge.product_edge_id,
                        product_id: edge.product_pk,
                        asset_guid: edge.guid,
                        sub_id: edge.sub_id,
                        flags: edge.flags,
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, AssetProcessorError>>()?;
    let dependencies = inspection
        .edges
        .into_iter()
        .map(|edge| {
            let target = match edge.target {
                DbTarget::Guid(guid) => JobDependencyTarget::Guid(guid),
                DbTarget::Path(path) => JobDependencyTarget::Path(path.as_str().to_string()),
            };
            Ok(JobDependencyRecord {
                job_edge_id: edge.job_edge_id,
                job_id: edge.job_pk,
                target,
                key: edge.key,
                platform: edge.platform,
                kind: db_job_dependency_kind_to_proto(edge.coupling),
            })
        })
        .collect::<Result<Vec<_>, AssetProcessorError>>()?;
    if job.job_id != job_id {
        return Err(AssetProcessorError::InvalidJobInspection {
            job_id,
            reason: "job projection changed identity".to_string(),
        });
    }
    Ok(ProtoJobInspection {
        job,
        attempt,
        products,
        dependencies,
    })
}

fn leased_asset_job_to_proto(
    context: &ClaimedJobContext,
) -> Result<LeasedAssetJob, AssetProcessorError> {
    let owner = job_owner(context.job.job_id, context.job.kind, context.job.builder)?;
    Ok(LeasedAssetJob {
        attempt_id: context.attempt.attempt_id,
        workspace_id: context.job.workspace_pk,
        owner,
        source_guid: context.asset.guid,
        preserved_source_sub_id: None,
        source_path: context.entry.path.clone(),
        source_root: context.workspace_root.path.clone(),
        source_schema_type: context.entry.schema.clone(),
        job_key: context.job.key.clone(),
        platform: context.job.platform.clone(),
        ordinal: context.attempt.ordinal,
        staging_root: context.attempt.staging.clone().unwrap_or_default(),
        source_payload: None,
    })
}

struct LeasedAssetJobPreparation {
    attempt: LeasedAssetJob,
    source_file_path: PathBuf,
    expected_content_hash: Digest,
    saved_payload: Option<Vec<u8>>,
    staging_root: PathBuf,
    capability: Capability,
}

impl LeasedAssetJobPreparation {
    const fn asset_job_attempt_id(&self) -> i64 {
        self.attempt.attempt_id
    }

    async fn stage(self) -> Result<LeasedAssetJob, AssetProcessorError> {
        tokio::task::spawn_blocking(move || self.stage_blocking())
            .await
            .map_err(|error| AssetProcessorError::SourcePayloadTask { error })?
    }

    fn stage_blocking(mut self) -> Result<LeasedAssetJob, AssetProcessorError> {
        let source_meta =
            source_meta::read_source_asset_meta(&self.source_file_path).map_err(|error| {
                let reason = source_meta_error_reason(error);
                AssetProcessorError::SourceMetaSidecar {
                    path: source_meta::source_meta_sidecar_path(&self.source_file_path),
                    reason,
                }
            })?;
        if let Some(preserved_guid) = source_meta
            .as_ref()
            .and_then(source_meta::SourceAssetMeta::preserved_guid)
            && preserved_guid != self.attempt.source_guid
        {
            return Err(AssetProcessorError::LeasedSourceMetaGuidMismatch {
                asset_job_attempt_id: self.attempt.attempt_id,
                source_path: self.attempt.source_path.clone(),
                recorded: self.attempt.source_guid,
                preserved: preserved_guid,
            });
        }
        if let Some(preserved) = source_meta
            .as_ref()
            .and_then(|meta| meta.preserved_asset_id)
        {
            self.attempt.preserved_source_sub_id = Some(preserved.sub_id);
        }

        let payload_bytes = match self.saved_payload {
            Some(payload) => payload,
            None => fs::read(&self.source_file_path).map_err(|source| {
                AssetProcessorError::ReadLeasedJobSource {
                    asset_job_attempt_id: self.attempt.attempt_id,
                    source_path: self.attempt.source_path.clone(),
                    path: self.source_file_path.clone(),
                    source,
                }
            })?,
        };
        let payload_hash = Digest::from(blake3::hash(&payload_bytes));
        if payload_hash != self.expected_content_hash {
            return Err(AssetProcessorError::LeasedSourceFileHashMismatch {
                asset_job_attempt_id: self.attempt.attempt_id,
                source_path: self.attempt.source_path.clone(),
                expected: self.expected_content_hash.to_string(),
                actual: payload_hash.to_string(),
            });
        }
        self.attempt.source_payload = Some(write_source_payload_side_channel(
            &self.capability,
            self.attempt.attempt_id,
            &self.staging_root,
            &payload_bytes,
        )?);
        Ok(self.attempt)
    }
}

fn prepare_leased_asset_job_attempt(
    context: ClaimedJobContext,
    envelope: &LeaseEnvelope,
) -> Result<LeasedAssetJobPreparation, AssetProcessorError> {
    let request = envelope.request();
    let attempt = leased_asset_job_to_proto(&context)?;
    let source_file_path = PathBuf::from(&context.workspace_root.path).join(&context.entry.path);
    if !is_normal_absolute_path(&source_file_path) {
        return Err(AssetProcessorError::UnsafeLeasedJobSourcePath {
            asset_job_attempt_id: attempt.attempt_id,
            source_path: attempt.source_path,
        });
    }
    let saved_payload = context.payload.and_then(|payload| {
        payload
            .checkpoint
            .or_else(|| (payload.saved == Some(payload.revision)).then_some(payload.payload))
    });
    Ok(LeasedAssetJobPreparation {
        attempt,
        source_file_path,
        expected_content_hash: context.entry.digest,
        saved_payload,
        staging_root: request.staging_root().to_path_buf(),
        capability: envelope.payload_authority().capability().clone(),
    })
}

fn asset_processing_status_to_proto(
    status: ProcessingStatus,
) -> Result<AssetProcessingStatusResult, AssetProcessorError> {
    Ok(AssetProcessingStatusResult {
        queued: processing_status_count_to_proto("queued", status.queued)?,
        leased: processing_status_count_to_proto("leased", status.leased)?,
        failed: processing_status_count_to_proto("failed", status.failed)?,
        in_flight_sweeps: 0,
    })
}

fn processing_status_count_to_proto(
    field: &'static str,
    count: u64,
) -> Result<u32, AssetProcessorError> {
    u32::try_from(count)
        .map_err(|_| AssetProcessorError::AssetProcessingStatusCountOverflow { field, count })
}

pub(crate) fn release_content_cache_root(
    project_data_paths: &ProjectDataPaths,
    workspace: &SelectWorkspaces,
    platform: &str,
) -> Result<PathBuf, AssetProcessorError> {
    validate_release_content_scope(project_data_paths, workspace)?;
    validate_release_content_platform(workspace.workspace_id, platform)?;
    Ok(project_data_paths.product_cache_dir(platform)?)
}

pub(crate) fn validate_release_content_scope(
    project_data_paths: &ProjectDataPaths,
    workspace: &SelectWorkspaces,
) -> Result<(), AssetProcessorError> {
    let workspace_root = PathBuf::from(&workspace.root);
    if !workspace_root.is_absolute() || !is_normal_absolute_path(&workspace_root) {
        return Err(AssetProcessorError::InvalidReleaseContentCacheRoot {
            workspace_id: workspace.workspace_id,
            root: workspace_root,
            reason: "workspace root is not an absolute normalized path".to_string(),
        });
    }
    validate_workspace_project_data_paths(project_data_paths, workspace)
}

pub(crate) fn validate_release_content_platform(
    workspace_id: i64,
    platform: &str,
) -> Result<(), AssetProcessorError> {
    if !is_safe_platform_component(platform) {
        return Err(AssetProcessorError::InvalidReleaseContentPlatform {
            workspace_id,
            platform: platform.to_string(),
        });
    }
    Ok(())
}

fn validate_workspace_project_data_paths(
    project_data_paths: &ProjectDataPaths,
    workspace: &SelectWorkspaces,
) -> Result<(), AssetProcessorError> {
    let workspace_root = normalize(Path::new(&workspace.root));
    let owned_root = project_data_paths.project_root();
    if workspace_root != owned_root {
        return Err(AssetProcessorError::ProjectDataWorkspaceMismatch {
            workspace_id: workspace.workspace_id,
            expected: owned_root.to_path_buf(),
            actual: workspace_root,
        });
    }
    Ok(())
}

fn release_asset_catalog_side_channel(
    capability: &Capability,
    platform: &str,
    cache_root: &Path,
) -> Result<Option<SideChannelHandle>, AssetProcessorError> {
    let catalog_path = cache_root.join(RELEASE_ASSET_CATALOG_FILE_NAME);
    let metadata = match std::fs::metadata(&catalog_path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(AssetProcessorError::ReleaseContentCatalogRead {
                path: catalog_path,
                source,
            });
        }
    };
    if !metadata.is_file() {
        return Err(AssetProcessorError::ReleaseContentCatalogNotFile { path: catalog_path });
    }
    let byte_length = metadata.len();
    if byte_length == 0 {
        return Err(AssetProcessorError::ReleaseContentCatalogEmpty { path: catalog_path });
    }
    Ok(Some(
        SideChannelHandle::mmap_file(side_channel_locator(&catalog_path), byte_length, platform)
            .with_capability(capability.clone()),
    ))
}

// Four unrelated things this one lookup needs at once: the read handle, the
// caller's capability, the workspace/platform/cache-root triple the read
// resolves against, and the product key. Its single caller assembles them from
// a request, a workspace row and a resolved cache root, and no other signature
// shares the combination, so a parameter struct would be a bag named after this
// one call.
#[expect(
    clippy::too_many_arguments,
    reason = "single-caller lookup whose arguments are four unrelated groups no other signature shares"
)]
fn release_content_product_side_channel(
    db: &AssetDb,
    capability: &Capability,
    workspace_pk: i64,
    platform: &str,
    cache_root: &Path,
    published_catalog: Option<&AssetBuilderCatalogResult>,
    asset_guid: uuid::Uuid,
    sub_id: u32,
) -> Result<Option<ReleaseContentProduct>, AssetProcessorError> {
    let mut cursor = None;
    let mut matched = None;
    loop {
        let page = db.catalog_page(
            workspace_pk,
            platform,
            cursor.as_ref(),
            catalog::CATALOG_PAGE_SIZE,
        )?;
        for row in page.rows {
            if row.guid == asset_guid
                && row.sub_id == i64::from(sub_id)
                && catalog::active_catalog_row(&row, published_catalog)?
                && matched.replace(row).is_some()
            {
                return Err(AssetProcessorError::DuplicateReleaseContentProduct {
                    asset_guid,
                    sub_id,
                });
            }
        }
        let Some(next) = page.next else {
            break;
        };
        cursor = Some(next);
    }
    let Some(row) = matched else {
        return Ok(None);
    };
    release_content_product_from_row(capability, platform, cache_root, &row).map(Some)
}

fn release_content_product_from_row(
    capability: &Capability,
    platform: &str,
    cache_root: &Path,
    row: &SelectCatalog,
) -> Result<ReleaseContentProduct, AssetProcessorError> {
    let sub_id = db_product_sub_id_to_proto(&row.path, row.sub_id)?;
    let product_format_version = db_product_format_version_to_proto(&row.path, row.version)?;
    let byte_length = db_product_byte_length_to_proto(&row.path, row.bytes)?;
    let content_hash = db_product_content_hash_to_proto(&row.digest);
    let product_path = release_content_product_cache_path(cache_root, &row.path)?;
    validate_release_content_product_cache_file(&row.path, &product_path, byte_length)?;

    Ok(ReleaseContentProduct {
        asset_guid: row.guid,
        sub_id,
        product_path: row.path.clone(),
        product_format: row.format.clone(),
        product_format_version,
        byte_length,
        content_hash,
        payload: SideChannelHandle::mmap_file(
            side_channel_locator(&product_path),
            byte_length,
            platform,
        )
        .with_capability(capability.clone()),
    })
}

fn release_content_product_cache_path(
    cache_root: &Path,
    product_path: &str,
) -> Result<PathBuf, AssetProcessorError> {
    let product_path = validate_asset_db_relative_path(product_path).ok_or_else(|| {
        AssetProcessorError::InvalidReleaseContentProduct {
            product_path: product_path.to_string(),
            reason: "product path must be an asset-DB relative path".to_string(),
        }
    })?;
    let path = cache_root.join(Path::new(&product_path));
    if !path.starts_with(cache_root) {
        return Err(AssetProcessorError::InvalidReleaseContentProduct {
            product_path,
            reason: format!(
                "product path escaped release content cache root `{}`",
                cache_root.display()
            ),
        });
    }
    Ok(path)
}

fn validate_release_content_product_cache_file(
    product_path: &str,
    path: &Path,
    expected_byte_length: u64,
) -> Result<(), AssetProcessorError> {
    let metadata = std::fs::metadata(path).map_err(|source| {
        AssetProcessorError::ReleaseContentProductRead {
            product_path: product_path.to_string(),
            path: path.to_path_buf(),
            source,
        }
    })?;
    if !metadata.is_file() {
        return Err(AssetProcessorError::ReleaseContentProductNotFile {
            product_path: product_path.to_string(),
            path: path.to_path_buf(),
        });
    }
    let actual = metadata.len();
    if actual != expected_byte_length {
        return Err(AssetProcessorError::ReleaseContentProductLengthMismatch {
            product_path: product_path.to_string(),
            path: path.to_path_buf(),
            expected: expected_byte_length,
            actual,
        });
    }
    Ok(())
}

pub(crate) fn db_product_format_version_to_proto(
    product_path: &str,
    version: i64,
) -> Result<u32, AssetProcessorError> {
    let version =
        u32::try_from(version).map_err(|_| AssetProcessorError::InvalidDbProductFormatVersion {
            product_path: product_path.to_string(),
            version,
        })?;
    if version == 0 {
        return Err(AssetProcessorError::InvalidDbProductFormatVersion {
            product_path: product_path.to_string(),
            version: i64::from(version),
        });
    }
    Ok(version)
}

pub(crate) const fn db_catalog_path_registration_to_proto(
    value: Registration,
) -> CatalogPathRegistration {
    match value {
        Registration::Registered => CatalogPathRegistration::Registered,
        Registration::AssetIdOnly => CatalogPathRegistration::AssetIdOnly,
    }
}

pub(crate) fn db_product_sub_id_to_proto(
    product_path: &str,
    sub_id: i64,
) -> Result<u32, AssetProcessorError> {
    u32::try_from(sub_id).map_err(|_| AssetProcessorError::InvalidReleaseContentProduct {
        product_path: product_path.to_string(),
        reason: format!("sub id {sub_id} cannot fit runtime asset id u32"),
    })
}

pub(crate) fn db_product_byte_length_to_proto(
    product_path: &str,
    byte_length: i64,
) -> Result<u64, AssetProcessorError> {
    let byte_length = u64::try_from(byte_length).map_err(|_| {
        AssetProcessorError::InvalidReleaseContentProduct {
            product_path: product_path.to_string(),
            reason: format!("byte length {byte_length} cannot fit u64"),
        }
    })?;
    if byte_length == 0 {
        return Err(AssetProcessorError::InvalidReleaseContentProduct {
            product_path: product_path.to_string(),
            reason: "byte length must be positive for release side-channel content".to_string(),
        });
    }
    Ok(byte_length)
}

fn db_product_content_hash_to_proto(content_hash: &Digest) -> Vec<u8> {
    content_hash.as_bytes().to_vec()
}

fn side_channel_locator(path: &Path) -> String {
    normalize(path).to_string_lossy().into_owned()
}

const fn db_job_dependency_kind_to_proto(kind: DbCoupling) -> JobDependencyKind {
    match kind {
        DbCoupling::Order => JobDependencyKind::Order,
        DbCoupling::Fingerprint => JobDependencyKind::Fingerprint,
        DbCoupling::OrderOnly => JobDependencyKind::OrderOnly,
    }
}

fn db_job_status_to_proto(job_id: i64, status: DbStatus) -> Result<JobStatus, AssetProcessorError> {
    match status {
        DbStatus::Queued => Ok(JobStatus::Queued),
        DbStatus::Leased => Ok(JobStatus::Leased),
        DbStatus::Succeeded => Ok(JobStatus::Succeeded),
        DbStatus::Failed => Ok(JobStatus::Failed),
        DbStatus::Abandoned => Err(AssetProcessorError::InvalidJobInspection {
            job_id,
            reason: "Job cannot be Abandoned".to_string(),
        }),
    }
}

fn db_attempt_status_to_proto(
    attempt_id: i64,
    status: DbStatus,
) -> Result<AttemptStatus, AssetProcessorError> {
    match status {
        DbStatus::Leased => Ok(AttemptStatus::Leased),
        DbStatus::Succeeded => Ok(AttemptStatus::Succeeded),
        DbStatus::Failed => Ok(AttemptStatus::Failed),
        DbStatus::Abandoned => Ok(AttemptStatus::Abandoned),
        DbStatus::Queued => Err(AssetProcessorError::InvalidJobInspection {
            job_id: attempt_id,
            reason: "Attempt cannot be Queued".to_string(),
        }),
    }
}

const fn db_workspace_entry_diff_to_proto(status: DbDiff) -> WorkspaceEntryDiff {
    match status {
        DbDiff::Clean => WorkspaceEntryDiff::Clean,
        DbDiff::Added => WorkspaceEntryDiff::Added,
        DbDiff::Modified => WorkspaceEntryDiff::Modified,
        DbDiff::Deleted => WorkspaceEntryDiff::Deleted,
        DbDiff::Conflicted => WorkspaceEntryDiff::Conflicted,
    }
}

const fn completion_status_to_db(status: AttemptStatus) -> Result<DbStatus, AssetProcessorError> {
    match status {
        AttemptStatus::Succeeded => Ok(DbStatus::Succeeded),
        AttemptStatus::Failed => Ok(DbStatus::Failed),
        AttemptStatus::Abandoned => Ok(DbStatus::Abandoned),
        AttemptStatus::Leased => Err(AssetProcessorError::InvalidCompletionStatus { status }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedProduct {
    product_path: String,
    asset_type: uuid::Uuid,
    sub_id: i64,
    product_format: String,
    product_format_version: i64,
    catalog_aliases: Aliases,
    catalog_path_registration: Registration,
    staged_path: String,
    staged_file_path: PathBuf,
    content_hash: Digest,
    byte_length: i64,
    dependencies: Vec<ProductEdgeInput>,
}

/// A build attempt's prepared completion: the writer command plus everything
/// promotion and the post-commit consequence need once the command lands.
///
/// Boxed at [`PreparedAttemptCompletion::Build`]. Inline, these ten fields made
/// that one variant several hundred bytes and set the size of every
/// `Result<PreparedAttemptCompletion, _>` the dispatcher moves between tasks.
struct BuildAttemptCompletion {
    command: CompleteAttempt,
    status: AttemptStatus,
    attempt_id: i64,
    workspace: SelectWorkspaces,
    job: SelectJobs,
    project_data_paths: ProjectDataPaths,
    product_cache_root: PathBuf,
    products: Vec<PreparedProduct>,
    generated_rust_projection_affected: bool,
    manifest_elapsed: Duration,
}

enum PreparedAttemptCompletion {
    NoLongerOwned,
    Direct {
        // `CompleteAttempt` carries the products and plan delta inline, so it
        // dominates this variant on its own.
        command: Box<CompleteAttempt>,
        status: AttemptStatus,
    },
    Build(Box<BuildAttemptCompletion>),
}

enum DurableAttemptCompletion {
    NoLongerOwned,
    Committed(Option<Box<PostCommitAttemptCompletion>>),
}

struct PostCommitAttemptCompletion {
    attempt_id: i64,
    promotion: Option<ProductPromotionReceipt>,
    workspace: SelectWorkspaces,
    job: SelectJobs,
    project_data_paths: ProjectDataPaths,
    product_cache_root: PathBuf,
    generated_rust_projection_affected: bool,
    manifest_elapsed: Duration,
    promote_elapsed: Duration,
    submit_elapsed: Duration,
    #[cfg(test)]
    fail_for_test: bool,
}

impl PreparedProduct {
    fn as_input(&self, asset_pk: i64, platform: &str) -> ProductInput {
        ProductInput {
            asset_pk,
            platform: platform.to_owned(),
            path: self.product_path.clone(),
            kind: self.asset_type,
            sub_id: self.sub_id,
            format: self.product_format.clone(),
            version: self.product_format_version,
            aliases: self.catalog_aliases.clone(),
            registration: self.catalog_path_registration,
            digest: self.content_hash,
            bytes: self.byte_length,
            edges: self.dependencies.clone(),
        }
    }
}

fn prepare_product_inputs(
    attempt: &SelectAttempts,
    manifest: &ProductManifest,
    product_formats: &[ProductFormatDescriptor],
    registries: &Registries,
) -> Result<Vec<PreparedProduct>, AssetProcessorError> {
    if manifest.products.is_empty() {
        return Ok(Vec::new());
    }
    let staging_root =
        attempt
            .staging
            .as_deref()
            .ok_or_else(|| AssetProcessorError::InvalidProductManifest {
                reason: "attempt has products but no staging root".to_string(),
            })?;
    let staging_root = Path::new(staging_root);
    validate_staging_root(staging_root)?;

    if manifest
        .products
        .iter()
        .any(|product| product.product_format == ASSET_JOB_PLAN_PRODUCT_FORMAT)
    {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "`{ASSET_JOB_PLAN_PRODUCT_FORMAT}` is reserved for `{ASSET_PLANNER_JOB_KEY}` attempts"
            ),
        });
    }

    let products = manifest
        .products
        .iter()
        .map(|product| prepare_product_input(staging_root, product, product_formats, registries))
        .collect::<Result<Vec<_>, _>>()?;
    validate_prepared_shared_products(&products)?;
    Ok(products)
}

fn prepare_product_input(
    staging_root: &Path,
    product: &ProductManifestProduct,
    product_formats: &[ProductFormatDescriptor],
    registries: &Registries,
) -> Result<PreparedProduct, AssetProcessorError> {
    if product.product_path.trim().is_empty() {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: "product path is empty".to_string(),
        });
    }
    let product_path = validate_asset_db_relative_path(&product.product_path).ok_or_else(|| {
        AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "product path `{}` must be a canonical asset-db relative path",
                product.product_path
            ),
        }
    })?;
    validate_manifest_product_format(product, product_formats, registries)?;
    if product.staged_path.trim().is_empty() {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: format!("staged path for `{}` is empty", product.product_path),
        });
    }
    if product.content_hash.len() != 32 {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "product `{}` hash must be 32 bytes, got {}",
                product.product_path,
                product.content_hash.len()
            ),
        });
    }
    let byte_length = i64::try_from(product.byte_length).map_err(|_| {
        AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "product `{}` byte length exceeds DB range",
                product.product_path
            ),
        }
    })?;
    let staged_path = Path::new(&product.staged_path);
    if staged_path.is_absolute() {
        if !is_normal_absolute_path(staged_path) {
            return Err(AssetProcessorError::InvalidProductManifest {
                reason: format!(
                    "absolute staged path `{}` must be normalized",
                    product.staged_path
                ),
            });
        }
        if !staged_path.starts_with(staging_root) {
            return Err(AssetProcessorError::InvalidProductManifest {
                reason: format!(
                    "absolute staged path `{}` is outside staging root `{}`",
                    product.staged_path,
                    staging_root.display()
                ),
            });
        }
    } else if !is_safe_relative_path(staged_path) {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "relative staged path `{}` escapes staging root",
                product.staged_path
            ),
        });
    }
    let staged_file_path = verify_staged_product_file(staging_root, staged_path, product)?;
    let catalog_aliases = Aliases::from(product.catalog_aliases.clone());
    let dependencies = prepare_product_dependencies(product)?;
    let content_hash = product.content_hash.as_slice().try_into().map_err(|_| {
        AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "product `{}` hash must be {} bytes",
                product.product_path,
                blake3::OUT_LEN
            ),
        }
    })?;

    Ok(PreparedProduct {
        product_path,
        asset_type: product.asset_type,
        sub_id: i64::from(product.sub_id),
        product_format: product.product_format.clone(),
        product_format_version: i64::from(product.product_format_version),
        catalog_aliases,
        catalog_path_registration: match product.catalog_path_registration {
            CatalogPathRegistration::Registered => Registration::Registered,
            CatalogPathRegistration::AssetIdOnly => Registration::AssetIdOnly,
        },
        staged_path: product.staged_path.clone(),
        staged_file_path,
        content_hash: Digest::from_bytes(content_hash),
        byte_length,
        dependencies,
    })
}

fn validate_prepared_shared_products(
    products: &[PreparedProduct],
) -> Result<(), AssetProcessorError> {
    let mut catalog_paths = BTreeSet::new();
    let mut physical_products = BTreeMap::<&str, &PreparedProduct>::new();
    for product in products {
        let registration = db_catalog_path_registration_to_proto(product.catalog_path_registration);
        let aliases = product.catalog_aliases.as_slice();
        if registration == CatalogPathRegistration::AssetIdOnly && !aliases.is_empty() {
            return Err(AssetProcessorError::InvalidProductManifest {
                reason: format!(
                    "asset-id-only product `{}` cannot declare catalog aliases",
                    product.product_path
                ),
            });
        }
        if registration == CatalogPathRegistration::Registered
            && (!catalog_paths.insert(product.product_path.clone())
                || aliases
                    .iter()
                    .any(|alias| !catalog_paths.insert(alias.clone())))
        {
            return Err(AssetProcessorError::InvalidProductManifest {
                reason: format!(
                    "more than one product claims catalog lookup path `{}`",
                    product.product_path
                ),
            });
        }
        if let Some(first) = physical_products.get(product.product_path.as_str()) {
            if first.asset_type != product.asset_type
                || first.product_format != product.product_format
                || first.product_format_version != product.product_format_version
                || first.staged_file_path != product.staged_file_path
                || first.content_hash != product.content_hash
                || first.byte_length != product.byte_length
                || first.dependencies != product.dependencies
            {
                return Err(AssetProcessorError::InvalidProductManifest {
                    reason: format!(
                        "logical products share physical path `{}` but disagree on its backing payload",
                        product.product_path
                    ),
                });
            }
        } else {
            physical_products.insert(product.product_path.as_str(), product);
        }
    }
    Ok(())
}

fn prepare_product_dependencies(
    product: &ProductManifestProduct,
) -> Result<Vec<ProductEdgeInput>, AssetProcessorError> {
    let mut seen = BTreeSet::new();
    product
        .dependencies
        .iter()
        .map(|dependency| {
            if dependency.asset_guid.is_nil() {
                return Err(AssetProcessorError::InvalidProductManifest {
                    reason: format!(
                        "product `{}` dependency asset guid cannot be nil",
                        product.product_path
                    ),
                });
            }
            if !seen.insert((dependency.asset_guid, dependency.sub_id)) {
                return Err(AssetProcessorError::InvalidProductManifest {
                    reason: format!(
                        "product `{}` repeats dependency {}:{}",
                        product.product_path, dependency.asset_guid, dependency.sub_id
                    ),
                });
            }
            Ok(ProductEdgeInput {
                guid: dependency.asset_guid,
                sub_id: i64::from(dependency.sub_id),
                flags: i64::from(dependency.flags),
            })
        })
        .collect()
}

fn validate_manifest_product_format(
    product: &ProductManifestProduct,
    product_formats: &[ProductFormatDescriptor],
    registries: &Registries,
) -> Result<(), AssetProcessorError> {
    if product.product_format.trim().is_empty() {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: format!("product `{}` format is empty", product.product_path),
        });
    }
    // Planner side-channel products are host-protocol metadata, not registered
    // product formats. They are expanded into real jobs and never promoted.
    if product.product_format == ASSET_JOB_PLAN_PRODUCT_FORMAT {
        if product.product_format_version != 1 {
            return Err(AssetProcessorError::InvalidProductManifest {
                reason: format!(
                    "product `{}` uses unsupported planner format version {}",
                    product.product_path, product.product_format_version
                ),
            });
        }
        return Ok(());
    }
    let current_version = composed_product_format_by_id(registries, &product.product_format)
        .map(|attributed| attributed.entry.current_version())
        .or_else(|| {
            product_formats
                .iter()
                .find(|descriptor| descriptor.id == product.product_format)
                .map(|descriptor| descriptor.current_version)
        });
    let Some(current_version) = current_version else {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "product `{}` uses unregistered product format `{}`",
                product.product_path, product.product_format
            ),
        });
    };
    if product.product_format_version == 0 || product.product_format_version > current_version {
        return Err(AssetProcessorError::InvalidProductManifest {
            reason: format!(
                "product `{}` uses unsupported version {} for product format `{}` (registered current version {})",
                product.product_path,
                product.product_format_version,
                product.product_format,
                current_version
            ),
        });
    }
    Ok(())
}

fn verify_staged_product_file(
    staging_root: &Path,
    staged_path: &Path,
    product: &ProductManifestProduct,
) -> Result<PathBuf, AssetProcessorError> {
    let path = if staged_path.is_absolute() {
        staged_path.to_path_buf()
    } else {
        staging_root.join(staged_path)
    };
    let canonical_root =
        staging_root
            .canonicalize()
            .map_err(|source| AssetProcessorError::StagedProductRead {
                product_path: product.product_path.clone(),
                path: staging_root.to_path_buf(),
                source,
            })?;
    let canonical_path =
        path.canonicalize()
            .map_err(|source| AssetProcessorError::StagedProductRead {
                product_path: product.product_path.clone(),
                path: path.clone(),
                source,
            })?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(AssetProcessorError::StagedProductOutsideStagingRoot {
            product_path: product.product_path.clone(),
            path: canonical_path,
            staging_root: canonical_root,
        });
    }

    let mut file = std::fs::File::open(&canonical_path).map_err(|source| {
        AssetProcessorError::StagedProductRead {
            product_path: product.product_path.clone(),
            path: canonical_path.clone(),
            source,
        }
    })?;
    let actual_len = file
        .metadata()
        .map_err(|source| AssetProcessorError::StagedProductRead {
            product_path: product.product_path.clone(),
            path: canonical_path.clone(),
            source,
        })?
        .len();
    if actual_len != product.byte_length {
        return Err(AssetProcessorError::StagedProductLengthMismatch {
            product_path: product.product_path.clone(),
            path: canonical_path,
            expected: product.byte_length,
            actual: actual_len,
        });
    }

    let mut hasher = blake3::Hasher::new();
    // Heap, not stack: this runs per staged product inside the completion path,
    // where a 64 KiB frame is not worth the stack it would cost.
    let mut buffer = vec![0_u8; STAGED_PRODUCT_HASH_BUFFER_BYTES];
    loop {
        let read =
            file.read(&mut buffer)
                .map_err(|source| AssetProcessorError::StagedProductRead {
                    product_path: product.product_path.clone(),
                    path: canonical_path.clone(),
                    source,
                })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if hasher.finalize().as_bytes().as_slice() != product.content_hash.as_slice() {
        return Err(AssetProcessorError::StagedProductHashMismatch {
            product_path: product.product_path.clone(),
            path: canonical_path,
        });
    }

    Ok(canonical_path)
}

fn product_cache_root_for_job(
    project_data_paths: &ProjectDataPaths,
    workspace: &SelectWorkspaces,
    job: &SelectJobs,
    attempt_id: i64,
) -> Result<PathBuf, AssetProcessorError> {
    let workspace_root = PathBuf::from(&workspace.root);
    if !workspace_root.is_absolute() || !is_normal_absolute_path(&workspace_root) {
        return Err(AssetProcessorError::InvalidProductCacheRoot {
            asset_job_attempt_id: attempt_id,
            root: workspace_root,
            reason: "workspace root is not an absolute normalized path".to_string(),
        });
    }
    if !is_safe_platform_component(&job.platform) {
        return Err(AssetProcessorError::InvalidProductCachePlatform {
            asset_job_attempt_id: attempt_id,
            platform: job.platform.clone(),
        });
    }
    validate_workspace_project_data_paths(project_data_paths, workspace)?;
    Ok(project_data_paths.product_cache_dir(&job.platform)?)
}

pub(crate) fn product_cache_transaction_root(project_data_paths: &ProjectDataPaths) -> PathBuf {
    project_data_paths
        .derived_dir()
        .join("asset-processor")
        .join("product-cache-transactions")
}

fn product_cache_compensation_root(project_data_paths: &ProjectDataPaths) -> PathBuf {
    project_data_paths
        .derived_dir()
        .join("asset-processor")
        .join("product-cache-compensations")
}

pub(crate) fn recover_product_cache_transactions(
    project_data_paths: &ProjectDataPaths,
) -> Result<usize, az_filesystem::FileTransactionError> {
    let transaction_root = product_cache_transaction_root(project_data_paths);
    Ok(FileTransaction::new(transaction_root)
        .recover_pending()?
        .into_iter()
        .map(|report| report.recovered_write_count)
        .sum())
}

#[derive(Debug)]
struct ProductPromotionReceipt {
    transaction_root: PathBuf,
    backup_root: PathBuf,
    originals: Vec<ProductPromotionOriginal>,
}

#[derive(Debug)]
enum ProductPromotionOriginal {
    Existing { target: PathBuf, backup: PathBuf },
    Missing { target: PathBuf },
}

impl ProductPromotionReceipt {
    fn finalize(self) -> Result<(), AssetProcessorError> {
        remove_product_promotion_backup(&self.backup_root)
    }

    fn compensate(self) -> Result<(), AssetProcessorError> {
        let restores = self
            .originals
            .iter()
            .filter_map(|original| match original {
                ProductPromotionOriginal::Existing { target, backup } => {
                    Some(FileWrite::from_path(target.clone(), backup.clone()))
                }
                ProductPromotionOriginal::Missing { .. } => None,
            })
            .collect::<Vec<_>>();
        if !restores.is_empty() {
            FileTransaction::new(self.transaction_root.clone())
                .commit(restores)
                .map_err(
                    |source| AssetProcessorError::ProductCacheCompensationTransaction {
                        root: self.transaction_root.clone(),
                        source,
                    },
                )?;
        }
        for original in &self.originals {
            let ProductPromotionOriginal::Missing { target } = original else {
                continue;
            };
            match fs::remove_file(target) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(AssetProcessorError::ProductCachePromotionCompensation {
                        path: target.clone(),
                        source,
                    });
                }
            }
        }
        remove_product_promotion_backup(&self.backup_root)
    }
}

fn remove_product_promotion_backup(backup_root: &Path) -> Result<(), AssetProcessorError> {
    match fs::remove_dir_all(backup_root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AssetProcessorError::ProductCachePromotionReceiptCleanup {
            root: backup_root.to_path_buf(),
            source,
        }),
    }
}

async fn compensate_product_promotion(
    attempt_id: i64,
    promotion: Option<ProductPromotionReceipt>,
) -> Result<(), AssetProcessorError> {
    let Some(promotion) = promotion else {
        return Ok(());
    };
    tokio::task::spawn_blocking(move || promotion.compensate())
        .await
        .map_err(|error| AssetProcessorError::DispatcherCompletionTask { attempt_id, error })?
}

fn completion_writer_rollback_error(
    writer: RepoError,
    rollback: AssetProcessorError,
) -> AssetProcessorError {
    AssetProcessorError::ProductCacheCompletionRollback {
        writer,
        rollback: Box::new(rollback),
    }
}

fn promote_products_to_cache(
    project_data_paths: &ProjectDataPaths,
    cache_root: &Path,
    products: &[PreparedProduct],
) -> Result<Option<ProductPromotionReceipt>, AssetProcessorError> {
    promote_products_to_cache_inner(project_data_paths, cache_root, products, || {}, || {})
}

#[cfg(test)]
/// Deterministically introduces then clears an apply-time filesystem obstacle
/// after preimages have been captured. Production uses the no-op boundaries
/// above; this seam exercises the same recovery-and-compensation call stack.
fn promote_products_to_cache_with_transient_apply_failure(
    project_data_paths: &ProjectDataPaths,
    cache_root: &Path,
    products: &[PreparedProduct],
    before_apply: impl FnOnce(),
    after_apply_failure: impl FnOnce(),
) -> Result<Option<ProductPromotionReceipt>, AssetProcessorError> {
    promote_products_to_cache_inner(
        project_data_paths,
        cache_root,
        products,
        before_apply,
        after_apply_failure,
    )
}

fn promote_products_to_cache_inner(
    project_data_paths: &ProjectDataPaths,
    cache_root: &Path,
    products: &[PreparedProduct],
    before_apply: impl FnOnce(),
    after_apply_failure: impl FnOnce(),
) -> Result<Option<ProductPromotionReceipt>, AssetProcessorError> {
    if products.is_empty() {
        return Ok(None);
    }

    let mut promoted_paths = BTreeSet::new();
    let mut writes = Vec::with_capacity(products.len());
    for product in products {
        if !promoted_paths.insert(product.product_path.as_str()) {
            continue;
        }
        let destination = cache_root.join(Path::new(&product.product_path));
        if !destination.starts_with(cache_root) {
            return Err(AssetProcessorError::InvalidProductManifest {
                reason: format!(
                    "product path `{}` escapes product cache root `{}`",
                    product.product_path,
                    cache_root.display()
                ),
            });
        }
        writes.push((destination, product.staged_file_path.clone()));
    }

    let transaction_root = product_cache_transaction_root(project_data_paths);
    let recovered_write_count =
        recover_product_cache_transactions(project_data_paths).map_err(|source| {
            AssetProcessorError::ProductCacheTransactionRecovery {
                root: transaction_root.clone(),
                source,
            }
        })?;
    if recovered_write_count != 0 {
        info!(
            recovered_write_count,
            root = %transaction_root.display(),
            "recovered pending product cache transaction"
        );
    }

    let backup_root = product_cache_compensation_root(project_data_paths)
        .join(format!("promotion-{}", Uuid::now_v7()));
    let (originals, backup_writes) = capture_product_promotion_preimages(&writes, &backup_root)?;
    if !backup_writes.is_empty() {
        FileTransaction::new(transaction_root.clone())
            .commit(backup_writes)
            .map_err(
                |source| AssetProcessorError::ProductCacheCompensationTransaction {
                    root: transaction_root.clone(),
                    source,
                },
            )?;
    }

    let receipt = ProductPromotionReceipt {
        transaction_root: transaction_root.clone(),
        backup_root,
        originals,
    };
    before_apply();
    if let Err(source) = FileTransaction::new(transaction_root.clone()).commit(
        writes
            .into_iter()
            .map(|(destination, staged)| FileWrite::from_path(destination, staged)),
    ) {
        // A file transaction may already have applied a prefix and left an
        // Applying marker. Resolve that transaction forward first, then use
        // the receipt to restore the complete pre-promotion filesystem view.
        // Preimages remain owned by the receipt until that compensation ends.
        after_apply_failure();
        let promotion = AssetProcessorError::ProductCacheTransaction {
            root: transaction_root,
            source,
        };
        return Err(recover_and_compensate_failed_product_promotion(
            project_data_paths,
            receipt,
            promotion,
        ));
    }

    Ok(Some(receipt))
}

/// Records what every promotion target looked like before the write.
///
/// A target that already holds a file is copied aside first; the returned
/// backup writes are committed through the same transaction machinery as the
/// promotion, so a crash between the two leaves a recoverable marker rather
/// than a half-copied backup. Targets that do not exist yet are recorded as
/// missing so compensation deletes them instead of restoring them.
fn capture_product_promotion_preimages(
    writes: &[(PathBuf, PathBuf)],
    backup_root: &Path,
) -> Result<(Vec<ProductPromotionOriginal>, Vec<FileWrite>), AssetProcessorError> {
    let mut originals = Vec::with_capacity(writes.len());
    let mut backup_writes = Vec::new();
    for (index, (target, _)) in writes.iter().enumerate() {
        match fs::metadata(target) {
            Ok(metadata) if metadata.is_file() => {
                let backup = backup_root.join(format!("{index:04}.backup"));
                backup_writes.push(FileWrite::from_path(backup.clone(), target.clone()));
                originals.push(ProductPromotionOriginal::Existing {
                    target: target.clone(),
                    backup,
                });
            }
            Ok(_) => {
                return Err(AssetProcessorError::ProductCachePromotionPreimage {
                    path: target.clone(),
                    source: std::io::Error::other("product cache target is not a file"),
                });
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                originals.push(ProductPromotionOriginal::Missing {
                    target: target.clone(),
                });
            }
            Err(source) => {
                return Err(AssetProcessorError::ProductCachePromotionPreimage {
                    path: target.clone(),
                    source,
                });
            }
        }
    }
    Ok((originals, backup_writes))
}

fn recover_and_compensate_failed_product_promotion(
    project_data_paths: &ProjectDataPaths,
    receipt: ProductPromotionReceipt,
    promotion: AssetProcessorError,
) -> AssetProcessorError {
    let transaction_root = receipt.transaction_root.clone();
    let rollback = match recover_product_cache_transactions(project_data_paths) {
        Ok(_) => receipt.compensate(),
        Err(source) => Err(AssetProcessorError::ProductCacheTransactionRecovery {
            root: transaction_root,
            source,
        }),
    };
    match rollback {
        Ok(()) => promotion,
        Err(rollback) => AssetProcessorError::ProductCachePromotionRollback {
            promotion: Box::new(promotion),
            rollback: Box::new(rollback),
        },
    }
}

fn generated_rust_graph_source_paths(
    db: &AssetDb,
    workspace: &SelectWorkspaces,
    job: &SelectJobs,
) -> Result<Vec<String>, AssetProcessorError> {
    let generated_format = GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID.as_str();
    let mut cursor = None;
    let mut entries = Vec::new();
    loop {
        let page = db.catalog_page(
            workspace.workspace_id,
            &job.platform,
            cursor.as_ref(),
            catalog::CATALOG_PAGE_SIZE,
        )?;
        entries.extend(
            page.rows
                .iter()
                .filter(|entry| entry.format == generated_format)
                .map(|entry| entry.path.clone()),
        );
        let Some(next) = page.next else {
            break;
        };
        cursor = Some(next);
    }

    let mut seen = BTreeSet::new();
    for entry in &entries {
        if !seen.insert(entry.clone()) {
            return Err(AssetProcessorError::DuplicateGeneratedRustGraphSource {
                product_path: entry.clone(),
            });
        }
    }

    Ok(entries)
}

fn sync_generated_rust_graph_sources(
    entries: &[String],
    workspace: &SelectWorkspaces,
    project_data_paths: &ProjectDataPaths,
    cache_root: &Path,
) -> Result<(), AssetProcessorError> {
    let root = generated_rust_graph_source_root_for_workspace(project_data_paths, workspace)?;
    if entries.is_empty() {
        if root.exists() {
            remove_generated_rust_graph_source_root(&root)?;
        }
        return Ok(());
    }

    let parent =
        root.parent().ok_or_else(
            || AssetProcessorError::InvalidGeneratedRustGraphSourceRoot {
                root: root.clone(),
                reason: "generated source root has no parent".to_string(),
            },
        )?;
    std::fs::create_dir_all(parent).map_err(|source| {
        AssetProcessorError::GeneratedRustGraphSourceCreateDir {
            product_path: "<generated-source-root>".to_string(),
            path: parent.to_path_buf(),
            source,
        }
    })?;

    let temp_root = parent.join(format!(".graphs-{}.tmp", uuid::Uuid::now_v7().as_simple()));
    std::fs::create_dir_all(&temp_root).map_err(|source| {
        AssetProcessorError::GeneratedRustGraphSourceCreateDir {
            product_path: "<generated-source-root>".to_string(),
            path: temp_root.clone(),
            source,
        }
    })?;

    for entry in entries {
        copy_generated_rust_graph_source(cache_root, &temp_root, entry)?;
    }

    if root.exists() {
        remove_generated_rust_graph_source_root(&root)?;
    }
    std::fs::rename(&temp_root, &root).map_err(|source| {
        AssetProcessorError::GeneratedRustGraphSourcePromote {
            from: temp_root,
            to: root,
            source,
        }
    })?;

    Ok(())
}

fn generated_rust_graph_projection_affected(products: &[PreparedProduct]) -> bool {
    let generated_format = GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID.as_str();
    products
        .iter()
        .any(|product| product.product_format == generated_format)
}

fn generated_rust_graph_source_root_for_workspace(
    project_data_paths: &ProjectDataPaths,
    workspace: &SelectWorkspaces,
) -> Result<PathBuf, AssetProcessorError> {
    let workspace_root = PathBuf::from(&workspace.root);
    if !workspace_root.is_absolute() {
        return Err(AssetProcessorError::InvalidGeneratedRustGraphSourceRoot {
            root: workspace_root,
            reason: "workspace root is not absolute".to_string(),
        });
    }
    validate_workspace_project_data_paths(project_data_paths, workspace)?;
    Ok(project_data_paths.graphs_dir())
}

fn copy_generated_rust_graph_source(
    cache_root: &Path,
    generated_root: &Path,
    product_path: &str,
) -> Result<(), AssetProcessorError> {
    let product_path = validate_asset_db_relative_path(product_path).ok_or_else(|| {
        AssetProcessorError::InvalidGeneratedRustGraphSourceProduct {
            product_path: product_path.to_owned(),
            reason: "product path must be an asset-DB relative path".to_string(),
        }
    })?;
    let source = cache_root.join(Path::new(&product_path));
    if !source.starts_with(cache_root) {
        return Err(
            AssetProcessorError::InvalidGeneratedRustGraphSourceProduct {
                product_path,
                reason: format!(
                    "product path escaped product cache root `{}`",
                    cache_root.display()
                ),
            },
        );
    }

    let destination = generated_root.join(Path::new(&product_path));
    if !destination.starts_with(generated_root) {
        return Err(
            AssetProcessorError::InvalidGeneratedRustGraphSourceProduct {
                product_path,
                reason: format!(
                    "product path escaped generated source root `{}`",
                    generated_root.display()
                ),
            },
        );
    }

    let parent = destination.parent().unwrap_or(generated_root);
    std::fs::create_dir_all(parent).map_err(|source| {
        AssetProcessorError::GeneratedRustGraphSourceCreateDir {
            product_path: product_path.clone(),
            path: parent.to_path_buf(),
            source,
        }
    })?;
    std::fs::copy(&source, &destination).map_err(|source_error| {
        AssetProcessorError::GeneratedRustGraphSourceCopy {
            product_path: product_path.clone(),
            from: source,
            to: destination,
            source: source_error,
        }
    })?;

    Ok(())
}

fn remove_generated_rust_graph_source_root(root: &Path) -> Result<(), AssetProcessorError> {
    let metadata = std::fs::symlink_metadata(root).map_err(|source| {
        AssetProcessorError::GeneratedRustGraphSourceRemoveExisting {
            path: root.to_path_buf(),
            source,
        }
    })?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(root)
    } else {
        std::fs::remove_file(root)
    }
    .map_err(
        |source| AssetProcessorError::GeneratedRustGraphSourceRemoveExisting {
            path: root.to_path_buf(),
            source,
        },
    )
}

fn validate_product_manifest_side_channel_scope(
    handle: &SideChannelHandle,
    attempt: &SelectAttempts,
) -> Result<(), AssetProcessorError> {
    let staging_root =
        attempt
            .staging
            .as_deref()
            .ok_or_else(|| AssetProcessorError::InvalidProductManifest {
                reason: "successful attempt has no staging root".to_string(),
            })?;
    let staging_root = PathBuf::from(staging_root);
    validate_staging_root(&staging_root)?;
    let manifest_path =
        validated_staging_file_path(handle).map_err(ProductManifestSideChannelError::from)?;
    if !manifest_path.starts_with(&staging_root) {
        return Err(AssetProcessorError::ProductManifestOutsideStagingRoot {
            manifest_path,
            staging_root,
        });
    }
    Ok(())
}

fn validate_product_manifest_side_channel_capability(
    handle: &SideChannelHandle,
    completion_capability: &Capability,
) -> Result<(), AssetProcessorError> {
    validate_side_channel_capability_matches(
        handle,
        completion_capability,
        "asset product manifest",
    )
    .map_err(|error| match error {
        SideChannelCapabilityError::Missing { .. } => {
            AssetProcessorError::MissingProductManifestCapability
        }
        SideChannelCapabilityError::Mismatch { .. } => {
            AssetProcessorError::ProductManifestCapabilityMismatch
        }
    })
}

fn validate_optional_staging_root(staging_root: Option<&str>) -> Result<(), AssetProcessorError> {
    if let Some(staging_root) = staging_root {
        validate_staging_root(Path::new(staging_root))?;
    }
    Ok(())
}

fn validate_staging_root(staging_root: &Path) -> Result<(), AssetProcessorError> {
    if is_normal_absolute_path(staging_root) {
        Ok(())
    } else {
        Err(AssetProcessorError::InvalidStagingRoot {
            staging_root: staging_root.display().to_string(),
        })
    }
}

fn is_normal_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
}

fn is_safe_relative_path(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn is_safe_platform_component(platform: &str) -> bool {
    if platform.trim().is_empty() || platform.trim() != platform {
        return false;
    }
    let mut components = Path::new(platform).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

// Passed to `map_err` by name at every call site, and `map_err` hands its
// closure the error by value; taking a reference here would force a closure at
// each one.
#[allow(clippy::needless_pass_by_value)]
fn asset_processor_error_to_capnp(error: AssetProcessorError) -> Error {
    Error::failed(format!("asset-processor failed: {error}"))
}

#[cfg(test)]
mod architecture_tests;

#[cfg(test)]
mod tests {
    /// The Rust source a generated-graph build job writes as its product.
    const GENERATED_SOURCE_BYTES: &[u8] = b"pub fn generated_graph_entry() {}
";
    /// A file already sitting in the generated-source root before a build runs.
    const SENTINEL_BYTES: &[u8] = b"existing generated graph source";

    use std::{
        rc::Rc,
        sync::{
            OnceLock,
            atomic::{AtomicBool, Ordering},
        },
    };

    use az_architecture_guard::{
        public_functions_are_cfg_test_or_test_support, public_functions_have_cfg,
    };
    use az_asset_builder::{
        AssetBuilderPattern, BuildProduct, BuildRule, BuilderId, CreateJobsRequest,
        CreateJobsResponse, JobDescriptor, ProcessJobRequest, ProcessJobResponse, ProcessJobResult,
        SourceFormat, SourceSchemaType,
    };
    use az_asset_worker::{
        AssetWorkerError, AssetWorkerRunOnce, AssetWorkerRunOutcome,
        run_asset_worker_blocking_task_with_lease_for_test, run_asset_worker_once,
    };
    use az_core::{AssetData, AssetType, AssetTypeRegistration, AzRtti, AzTypeInfo};
    use az_graph_builder::{
        AOT_GRAPH_MANIFEST_FORMAT_ID, AOT_GRAPH_MANIFEST_PRODUCT_FORMAT_VERSION,
        GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID, GENERATED_RUST_GRAPH_SOURCE_PRODUCT_FORMAT_VERSION,
        GENERATED_RUST_GRAPH_SOURCE_PRODUCT_SUB_ID, GRAPH_COMPILER_BUILDER_ID,
        GRAPH_COMPILER_JOB_KEY, PACKED_GRAPH_IR_ASSET_TYPE_NAME, RUNTIME_GRAPH_PRODUCT_SUB_ID,
    };
    use az_graph_runtime::decode_aot_graph_manifest;
    use az_node_graph::{
        GraphCompilerBackendDescriptor, GraphNode, GraphNodeCatalogRequirement, GraphNodeId,
        GraphSourceWorkflow, GraphTypeDescriptor, GraphTypeRegistration, NodePortDescriptor,
        NodePortDirection, NodePortId, NodePortValue, NodeRuntimeBinding, NodeTypeDescriptor,
        NodeTypeRegistration, RuntimeGraphExecutionStrategy, RuntimeGraphProductDescriptor,
        VisualGraphDocument, encode_visual_graph_document_ron,
    };
    use az_proto_asset::{
        AssetBuilderCatalogRequest, AssetBuilderCatalogResult, AssetBuilderPatternKind,
        AssetProcessorEvent, AssetProcessorEventKind, AssetProcessorEventSubscriptionRequest,
        AssetProcessorEventSubscriptionResult, CompleteAssetJobAttemptRequest,
        LeaseAssetJobRequest, LeaseAssetJobResult, ProductManifest, ProductManifestProduct,
        ProductManifestProductDependency, ReconcileAssetSourcesRequest, SourceAssetRecordRequest,
        SourceAssetRecordResult, encode_product_manifest,
    };
    use az_proto_core::{
        Capability, CapabilityGrantSet, ServiceId, SideChannelHandle, SideChannelKind,
    };
    use futures::executor;
    use uuid::{Uuid, uuid};

    use super::*;

    #[test]
    fn staged_source_file_mutations_restore_files_after_database_rejection() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("source.ron");
        let moved = temp.path().join("nested").join("moved.ron");
        fs::write(&original, b"source contents").unwrap();

        let staged = StagedSourceFileMutation::move_paths(original.clone(), moved.clone()).unwrap();
        assert!(!original.exists());
        assert_eq!(fs::read(&moved).unwrap(), b"source contents");
        let error = staged.rollback("move", AssetProcessorError::BuilderCatalogUnavailable);
        assert!(matches!(
            error,
            AssetProcessorError::BuilderCatalogUnavailable
        ));
        assert_eq!(fs::read(&original).unwrap(), b"source contents");
        assert!(!moved.exists());

        let delete_staging = temp.path().join("delete-staging");
        let staged =
            StagedSourceFileMutation::delete_path(original.clone(), &delete_staging).unwrap();
        assert!(!original.exists());
        assert_eq!(
            fs::read_dir(&delete_staging).unwrap().count(),
            1,
            "delete tombstone must be staged outside the watched source directory"
        );
        let error = staged.rollback("delete", AssetProcessorError::BuilderCatalogUnavailable);
        assert!(matches!(
            error,
            AssetProcessorError::BuilderCatalogUnavailable
        ));
        assert_eq!(fs::read(&original).unwrap(), b"source contents");
        assert_eq!(
            fs::read_dir(&delete_staging)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".delete-"))
                .count(),
            0
        );
    }

    #[test]
    fn committed_source_mutation_does_not_report_tombstone_cleanup_failure() {
        let temp = tempfile::tempdir().unwrap();
        let retained_tombstone = temp.path().join("retained-tombstone");
        fs::create_dir(&retained_tombstone).unwrap();
        let staged = StagedSourceFileMutation {
            rollback_from: Some(retained_tombstone.clone()),
            rollback_to: Some(temp.path().join("source.ron")),
            cleanup_after_commit: Some(retained_tombstone.clone()),
        };

        staged.commit();

        assert!(retained_tombstone.is_dir());
    }

    #[test]
    fn source_extension_matching_accepts_bare_and_prefixed_compound_names() {
        let extensions = ["mapsettings.json"];

        assert!(source_path_matches_extensions(
            "levels/tutorial/mapsettings.json",
            &extensions
        ));
        assert!(source_path_matches_extensions(
            "levels/tutorial/region.mapsettings.json",
            &extensions
        ));
        assert!(!source_path_matches_extensions(
            "levels/tutorial/settings.json",
            &extensions
        ));
    }

    fn namespace_root(id: &str, root: PathBuf) -> AssetSourceRootSpec {
        AssetSourceRootSpec {
            id: id.to_string(),
            owner_id: id.to_string(),
            root,
            display_name: id.to_string(),
            portable_key: PortableKey::gem_assets(id),
            mount: AssetMount::assets(),
            recursive: true,
            watch: true,
            writable: true,
            excluded_paths: Exclusions::default(),
            output_prefix: String::new(),
            role: SourceRootRole::ProjectAssets,
        }
    }

    fn fixture_registered_source_root(fixture: &Fixture) -> RegisteredSourceRoot {
        let root = fixture
            .db
            .root_by_id(fixture.workspace_source_root.root_pk)
            .unwrap()
            .expect("fixture root");
        RegisteredSourceRoot {
            workspace_pk: fixture.workspace.workspace_id,
            workspace_root_pk: fixture.workspace_source_root.workspace_root_id,
            root_pk: root.root_id,
            id: root.key.clone(),
            owner: fixture.workspace_source_root.owner.clone(),
            path: fixture.workspace_source_root.path.clone(),
            display_name: "Project Assets".to_string(),
            portable_key: root.key,
            mount: "@assets@".to_string(),
            recursive: true,
            watch: true,
            writable: true,
            exclusions: fixture.workspace_source_root.exclusions.clone(),
            output_prefix: String::new(),
            role: SourceRootRole::ProjectAssets,
        }
    }

    #[test]
    fn file_source_classification_uses_the_most_specific_workflow_prefix() {
        let fixture = fixture();
        let source_root = fixture_registered_source_root(&fixture);
        drop(fixture);
        let classifiers = SourceAssetClassifiers {
            project_documents: Vec::new(),
            builder_claims: Vec::new(),
            file_sources: vec![
                FileSourceClassifier {
                    source_schema_type: "azoth.GenericSettings".to_string(),
                    source_root: PROJECT_SOURCE_ROOT.to_string(),
                    default_path_prefix: String::new(),
                    source_patterns: vec![AssetBuilderPattern::wildcard("*.settings.ron")],
                    extensions: vec!["settings.ron".to_string()],
                },
                FileSourceClassifier {
                    source_schema_type: "azoth.WeatherSettings".to_string(),
                    source_root: PROJECT_SOURCE_ROOT.to_string(),
                    default_path_prefix: "gems/weather".to_string(),
                    source_patterns: vec![AssetBuilderPattern::wildcard("*.settings.ron")],
                    extensions: vec!["settings.ron".to_string()],
                },
                FileSourceClassifier {
                    source_schema_type: "sample.DeploymentProfile".to_string(),
                    source_root: PROJECT_SOURCE_ROOT.to_string(),
                    default_path_prefix: "deployments".to_string(),
                    source_patterns: vec![AssetBuilderPattern::wildcard("*.settings.ron")],
                    extensions: vec!["settings.ron".to_string()],
                },
            ],
        };

        assert_eq!(
            classify_file_source_asset(
                &source_root,
                "deployments/remote.settings.ron",
                &classifiers,
            )
            .as_deref(),
            Some("sample.DeploymentProfile")
        );
        assert_eq!(
            classify_file_source_asset(
                &source_root,
                "gems/weather/default.settings.ron",
                &classifiers,
            )
            .as_deref(),
            Some("azoth.WeatherSettings")
        );
        assert_eq!(
            classify_file_source_asset(&source_root, "other.settings.ron", &classifiers).as_deref(),
            Some("azoth.GenericSettings")
        );
    }

    #[test]
    fn file_source_classification_prefers_compound_extension_over_catch_all() {
        let fixture = fixture();
        let source_root = fixture_registered_source_root(&fixture);
        drop(fixture);
        let classifiers = SourceAssetClassifiers {
            project_documents: Vec::new(),
            builder_claims: Vec::new(),
            file_sources: vec![
                FileSourceClassifier {
                    source_schema_type: "azoth.compat.sample.Source".to_string(),
                    source_root: PROJECT_SOURCE_ROOT.to_string(),
                    default_path_prefix: String::new(),
                    source_patterns: vec![AssetBuilderPattern::wildcard("*")],
                    extensions: vec!["*".to_string()],
                },
                FileSourceClassifier {
                    source_schema_type: "azoth.compat.sample.TerrainDatabaseSource".to_string(),
                    source_root: PROJECT_SOURCE_ROOT.to_string(),
                    default_path_prefix: "terrain".to_string(),
                    source_patterns: vec![AssetBuilderPattern::wildcard("*.terrain.ron")],
                    extensions: vec!["terrain.ron".to_string()],
                },
                FileSourceClassifier {
                    source_schema_type: "azoth.compat.sample.ActionGridSource".to_string(),
                    source_root: PROJECT_SOURCE_ROOT.to_string(),
                    default_path_prefix: "actions".to_string(),
                    source_patterns: vec![AssetBuilderPattern::wildcard("*.grid.ron")],
                    extensions: vec!["grid.ron".to_string()],
                },
                FileSourceClassifier {
                    source_schema_type: "azoth.compat.sample.StationDatabaseSource".to_string(),
                    source_root: PROJECT_SOURCE_ROOT.to_string(),
                    default_path_prefix: "stations".to_string(),
                    source_patterns: vec![AssetBuilderPattern::wildcard("*.stations.ron")],
                    extensions: vec!["stations.ron".to_string()],
                },
            ],
        };

        assert_eq!(
            classify_file_source_asset(
                &source_root,
                "sharedassets/terrain/default.terrain.ron",
                &classifiers,
            )
            .as_deref(),
            Some("azoth.compat.sample.TerrainDatabaseSource")
        );
        assert_eq!(
            classify_file_source_asset(
                &source_root,
                "sharedassets/actions/player.grid.ron",
                &classifiers,
            )
            .as_deref(),
            Some("azoth.compat.sample.ActionGridSource")
        );
        assert_eq!(
            classify_file_source_asset(
                &source_root,
                "sharedassets/stations/default.stations.ron",
                &classifiers,
            )
            .as_deref(),
            Some("azoth.compat.sample.StationDatabaseSource")
        );
        assert_eq!(
            classify_file_source_asset(&source_root, "misc/unknown.bin", &classifiers).as_deref(),
            Some("azoth.compat.sample.Source")
        );
    }

    #[test]
    fn file_source_classification_respects_builder_source_patterns() {
        let fixture = fixture();
        let source_root = fixture_registered_source_root(&fixture);
        drop(fixture);
        let classifiers = SourceAssetClassifiers {
            project_documents: Vec::new(),
            builder_claims: Vec::new(),
            file_sources: vec![FileSourceClassifier {
                source_schema_type: "azoth.gamedata.TableSource".to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                default_path_prefix: "gamedata".to_string(),
                source_patterns: vec![AssetBuilderPattern::wildcard("gamedata/*.ron")],
                extensions: vec!["ron".to_string()],
            }],
        };

        assert_eq!(
            classify_file_source_asset(
                &source_root,
                "gamedata/achievement_data/achievement_data_table.ron",
                &classifiers,
            )
            .as_deref(),
            Some("azoth.gamedata.TableSource")
        );
        assert_eq!(
            classify_file_source_asset(&source_root, "coatgen/chunk.material.ron", &classifiers)
                .as_deref(),
            None
        );
    }

    #[test]
    fn published_builder_claims_gate_distributed_planner_work() {
        let catalog = AssetBuilderCatalogResult {
            builders: vec![AssetBuilderDescriptor {
                name: "sample.gamedata.table-source".to_string(),
                builder_guid: uuid!("c9ea40b3-250a-44f3-ae3c-0fa6bb998074"),
                version: 1,
                analysis_fingerprint: String::new(),
                patterns: vec![AssetBuilderPatternDescriptor {
                    kind: AssetBuilderPatternKind::Wildcard,
                    pattern: "gamedata/*.ron".to_string(),
                }],
                source_schema_types: vec!["azoth.gamedata.TableSource".to_string()],
            }],
            source_schemas: vec![
                SourceSchemaDescriptor {
                    schema_type: "azoth.gamedata.TableSource".to_string(),
                    owner: "gamedata".to_string(),
                    label: "GameData Table".to_string(),
                    category: "GameData".to_string(),
                    authoring: SourceSchemaAuthoring::File {
                        workflow: SourceFileWorkflowDescriptor {
                            source_root: PROJECT_SOURCE_ROOT.to_string(),
                            default_path_prefix: "gamedata".to_string(),
                            extensions: vec!["ron".to_string()],
                            can_create: true,
                            can_edit: true,
                        },
                    },
                    file_templates: Vec::new(),
                },
                SourceSchemaDescriptor {
                    schema_type: "azoth.compat.sample.Source".to_string(),
                    owner: "sample".to_string(),
                    label: "Sample Compatibility Source".to_string(),
                    category: "Compatibility".to_string(),
                    authoring: SourceSchemaAuthoring::File {
                        workflow: SourceFileWorkflowDescriptor {
                            source_root: PROJECT_SOURCE_ROOT.to_string(),
                            default_path_prefix: String::new(),
                            extensions: vec!["*".to_string()],
                            can_create: false,
                            can_edit: false,
                        },
                    },
                    file_templates: Vec::new(),
                },
            ],
            product_formats: Vec::new(),
        };
        let classifiers = source_asset_classifiers_from_catalog(&catalog);

        assert!(classifiers.source_has_builder_claim(
            "gamedata/achievement_data/achievement_data_table.ron",
            "azoth.gamedata.TableSource",
        ));
        assert!(
            !classifiers.source_has_builder_claim(
                "coatgen/chunk.material.ron",
                "azoth.compat.sample.Source",
            )
        );
    }

    #[test]
    fn native_namespace_collision_names_both_physical_sources() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project-assets");
        let gem_root = temp.path().join("gem-assets");
        let project_file = project_root.join("Rendering/Textures/Hero.PNG");
        let gem_file = gem_root.join("rendering/textures/hero.png");
        std::fs::create_dir_all(project_file.parent().unwrap()).unwrap();
        std::fs::create_dir_all(gem_file.parent().unwrap()).unwrap();
        std::fs::write(&project_file, b"project").unwrap();
        std::fs::write(&gem_file, b"gem").unwrap();
        let mut roots = vec![
            namespace_root("project", project_root),
            namespace_root("render-gem", gem_root),
        ];

        let error = apply_asset_namespace_policy(&mut roots, &[]).unwrap_err();
        let message = error.to_string();

        let AssetProcessorError::AssetSourceCollision(collision) = &error else {
            panic!("expected collision, found {error}");
        };
        let AssetSourceCollisionDetail {
            virtual_path,
            first_path,
            second_path,
            ..
        } = collision.as_ref();
        assert_eq!(virtual_path, "@assets@/rendering/textures/hero.png");
        let actual_paths = BTreeSet::from([
            first_path.canonicalize().unwrap(),
            second_path.canonicalize().unwrap(),
        ]);
        let expected_paths = BTreeSet::from([
            project_file.canonicalize().unwrap(),
            gem_file.canonicalize().unwrap(),
        ]);
        assert_eq!(actual_paths, expected_paths);
        assert!(message.contains("Hero.PNG"));
        assert!(message.contains("hero.png"));
        assert!(message.contains("[[asset_overrides]]"));
    }

    #[test]
    fn explicit_native_override_excludes_only_the_replaced_root() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project-assets");
        let gem_root = temp.path().join("gem-assets");
        for root in [&project_root, &gem_root] {
            let source = root.join("ui/images/logo.png");
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(source, b"source").unwrap();
        }
        let mut roots = vec![
            namespace_root("project", project_root),
            namespace_root("ui-gem", gem_root),
        ];
        let overrides = [ProjectAssetOverride {
            path: r"UI\Images\Logo.PNG".to_string(),
            winning_root: "project".to_string(),
            replaced_root: "ui-gem".to_string(),
        }];

        apply_asset_namespace_policy(&mut roots, &overrides).unwrap();

        let project = roots.iter().find(|root| root.id == "project").unwrap();
        let gem = roots.iter().find(|root| root.id == "ui-gem").unwrap();
        assert!(project.excluded_paths.as_set().is_empty());
        assert_eq!(
            gem.excluded_paths.as_set(),
            &BTreeSet::from(["ui/images/logo.png".to_string()])
        );
    }

    #[test]
    fn scan_path_normalization_is_case_and_separator_stable() {
        let source_root = Path::new("assets");
        let source = source_root.join("Rendering/Textures/Hero.PNG");

        let normalized = source_root_relative_asset_path(source_root, &source)
            .unwrap()
            .unwrap();

        assert_eq!(normalized, "rendering/textures/hero.png");
    }

    struct Fixture {
        temp_dir: tempfile::TempDir,
        db: AssetDb,
        writer: AssetDbWriter,
        workspace_root: PathBuf,
        project_data_paths: ProjectDataPaths,
        source_root: PathBuf,
        workspace: SelectWorkspaces,
        workspace_source_root: SelectWorkspaceRoots,
        asset: SelectAssets,
        builder_guid: Uuid,
    }

    struct TestAssetProcessorEventSink {
        tx: tokio::sync::mpsc::UnboundedSender<AssetProcessorEvent>,
    }

    impl asset_capnp::asset_processor_event_sink::Server for TestAssetProcessorEventSink {
        // The asset-processor dispatcher is single-threaded by design: this future holds
        // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
        // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
        #[allow(clippy::future_not_send)]
        async fn update(
            self: capnp::capability::Rc<Self>,
            params: asset_capnp::asset_processor_event_sink::UpdateParams,
            _results: asset_capnp::asset_processor_event_sink::UpdateResults,
        ) -> Result<(), capnp::Error> {
            let event = AssetProcessorEvent::from_capnp(params.get()?.get_event()?)?;
            let _ = self.tx.send(event);
            Ok(())
        }
    }

    struct RejectingAssetProcessorEventSink {
        attempts: Rc<std::cell::Cell<usize>>,
    }

    impl asset_capnp::asset_processor_event_sink::Server for RejectingAssetProcessorEventSink {
        fn update(
            self: capnp::capability::Rc<Self>,
            _params: asset_capnp::asset_processor_event_sink::UpdateParams,
            _results: asset_capnp::asset_processor_event_sink::UpdateResults,
        ) -> impl std::future::Future<Output = Result<(), capnp::Error>> + 'static {
            self.attempts.set(self.attempts.get() + 1);
            async {
                Err(capnp::Error::failed(
                    "test sink rejected update".to_string(),
                ))
            }
        }
    }

    #[test]
    fn blocked_event_sink_keeps_one_drain_and_coalesces_the_latest_update() {
        let fixture = fixture();
        let row = fixture
            .db
            .entry_by_asset(fixture.workspace.workspace_id, fixture.asset.asset_id)
            .unwrap()
            .expect("fixture workspace entry");
        let entry = workspace_asset_entry_to_proto(&fixture.db, &fixture.asset, row).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut subscriber = AssetProcessorEventSubscriber {
            id: 1,
            workspace_id: fixture.workspace.workspace_id,
            sink: capnp_rpc::new_client(TestAssetProcessorEventSink { tx }),
            pending: None,
            in_flight: false,
        };
        drop(fixture);

        assert!(subscriber.enqueue(AssetProcessorEvent {
            seq: 10,
            kind: AssetProcessorEventKind::SourceRecorded,
            event_unix_ms: 10,
            entry: entry.clone(),
        }));
        assert!(subscriber.in_flight);
        assert!(!subscriber.enqueue(AssetProcessorEvent {
            seq: 11,
            kind: AssetProcessorEventKind::JobCompleted,
            event_unix_ms: 11,
            entry,
        }));
        assert_eq!(subscriber.pending.as_ref().map(|event| event.seq), Some(11));
        assert!(subscriber.in_flight, "the blocked sink has no second drain");
    }

    #[derive(Clone, Copy)]
    enum TestSourceFileCodecBehavior {
        ContractCompliant,
        LoadReturnsSavedSource,
        MutationReturnsUnchanged,
        RpcFailsAfterWritingOutput,
        ReturnsWrongOutputDestination,
        WritesOutputAfterDelay,
    }

    struct TestSourceFileCodec {
        behavior: TestSourceFileCodecBehavior,
        calls: Rc<std::cell::Cell<usize>>,
        seen_capability: Rc<RefCell<Option<Capability>>>,
    }

    fn test_source_file_document() -> az_proto_asset::SourceFileEditDocument {
        az_proto_asset::SourceFileEditDocument {
            root_object_id: Some("row:one".to_string()),
            root_schema: "az.test.Row".to_string(),
            value: az_proto_asset::ReflectedValueEnvelope::typed_ron(
                "az.test.Row",
                "(id: \"one\")",
            ),
            objects: vec![az_proto_asset::SourceFileEditObject {
                object_id: "row:one".to_string(),
                schema: "az.test.Row".to_string(),
                value: az_proto_asset::ReflectedValueEnvelope::typed_ron(
                    "az.test.Row",
                    "(id: \"one\")",
                ),
            }],
            codec_state: b"stable-row-identity".to_vec(),
        }
    }

    impl asset_capnp::source_file_codec::Server for TestSourceFileCodec {
        // The asset-processor dispatcher is single-threaded by design: this future holds
        // `Rc`-based dispatcher state (`Rc<DispatcherHandle>`, `Rc<Notify>`), so it can only
        // be `Send` if the whole dispatcher moves from `Rc` to `Arc`.
        #[allow(clippy::future_not_send)]
        async fn execute(
            self: capnp::capability::Rc<Self>,
            params: asset_capnp::source_file_codec::ExecuteParams,
            mut results: asset_capnp::source_file_codec::ExecuteResults,
        ) -> Result<(), capnp::Error> {
            self.calls.set(self.calls.get() + 1);
            let request = SourceFileCodecRequest::from_capnp(params.get()?.get_request()?)?;
            *self.seen_capability.borrow_mut() = request.authoritative_source.capability.clone();
            if matches!(
                self.behavior,
                TestSourceFileCodecBehavior::WritesOutputAfterDelay
            ) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            if matches!(
                self.behavior,
                TestSourceFileCodecBehavior::RpcFailsAfterWritingOutput
            ) {
                write_named_staging_file_atomic(
                    &request.output_destination.locator,
                    b"unclaimed worker output\n",
                )
                .map_err(|error| capnp::Error::failed(error.to_string()))?;
                return Err(capnp::Error::failed(
                    "test callback failed after writing AP-owned output".to_string(),
                ));
            }
            let replacement = match (&request.operation, self.behavior) {
                (
                    SourceFileCodecOperation::Load,
                    TestSourceFileCodecBehavior::ContractCompliant,
                )
                | (
                    SourceFileCodecOperation::Edit(_)
                    | SourceFileCodecOperation::RestoreDocument(_),
                    TestSourceFileCodecBehavior::MutationReturnsUnchanged,
                ) => SourceFileCodecReplacement::Unchanged,
                _ => {
                    let bytes = b"canonical edited source\n";
                    let path = PathBuf::from(&request.output_destination.locator);
                    let written = write_named_staging_file_atomic(&path, bytes)
                        .map_err(|error| capnp::Error::failed(error.to_string()))?;
                    let locator = if matches!(
                        self.behavior,
                        TestSourceFileCodecBehavior::ReturnsWrongOutputDestination
                    ) {
                        path.with_extension("wrong")
                    } else {
                        written.path
                    };
                    SourceFileCodecReplacement::SavedSource(Box::new(
                        SideChannelHandle::staging_file(
                            locator.to_string_lossy(),
                            written.byte_length,
                            written.content_hash,
                            std::env::consts::OS,
                        )
                        .with_capability(request.output_destination.capability.clone()),
                    ))
                }
            };
            SourceFileCodecResult {
                document: test_source_file_document(),
                replacement,
            }
            .to_capnp(results.get().init_result())
        }
    }

    const TEST_SESSION_ID: &str = "66666666-6666-6666-6666-666666666666";

    #[derive(SourceFormat)]
    #[source(schema = "az.test.Prefab", ext = "prefab.ron")]
    struct TestPrefabSourceFormat;

    #[derive(SourceFormat)]
    #[source(schema = "az.test.FileSource")]
    struct TestFileSourceFormat;

    #[derive(SourceFormat)]
    #[source(schema = "az.test.GemFileSource")]
    struct TestGemFileSourceFormat;

    #[derive(SourceFormat)]
    #[source(schema = "az.test.ImportFileSource")]
    struct TestImportFileSourceFormat;

    const TEST_FILE_SOURCE_SCHEMA: SourceSchemaType =
        match <TestFileSourceFormat as SourceFormat>::SCHEMA {
            Some(schema) => schema,
            None => panic!("TestFileSourceFormat declares a schema"),
        };
    const TEST_GEM_FILE_SOURCE_SCHEMA: SourceSchemaType =
        match <TestGemFileSourceFormat as SourceFormat>::SCHEMA {
            Some(schema) => schema,
            None => panic!("TestGemFileSourceFormat declares a schema"),
        };
    const TEST_GEM_SOURCE_ROOT: &str = "gem:az.test.asset-processor:assets";

    struct GeneratedGraphAssetData;

    impl AzTypeInfo for GeneratedGraphAssetData {
        const NAME: &'static str = "AzAssetProcessorTests::GeneratedGraphAssetData";
        const TYPE_ID: Uuid = uuid!("018f0c5a-0000-7000-8000-00000000a5e1");
    }

    impl AzRtti for GeneratedGraphAssetData {}

    impl AssetData for GeneratedGraphAssetData {
        const STABLE_NAME: &'static str = "az.asset_processor.tests.generated-rust-product";
    }

    const GENERATED_GRAPH_ASSET_TYPE_NAME: &str =
        <GeneratedGraphAssetData as AssetData>::STABLE_NAME;
    const GENERATED_GRAPH_ASSET_TYPE: AssetType =
        <GeneratedGraphAssetData as AssetData>::ASSET_TYPE;

    fn test_graph_node_type() -> NodeTypeDescriptor {
        NodeTypeDescriptor::new("az.asset_processor.tests.Print", 1, "Print")
            .with_port(NodePortDescriptor::new(
                NodePortId::new(1),
                "value",
                NodePortDirection::Input,
                NodePortValue::Data {
                    schema_type: "core.string".to_string(),
                },
            ))
            .with_runtime_binding(NodeRuntimeBinding::rust_symbol(
                "az-asset-processor-tests",
                "az_asset_processor_tests::print",
            ))
    }

    fn test_graph_type() -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            "az.asset_processor.tests.logic-graph",
            1,
            "Asset Processor Test Logic Graph",
            GraphSourceWorkflow::file("az.asset_processor.tests.logic-graph.source", "azgraph.ron")
                .with_default_path_prefix("graphs"),
            GraphCompilerBackendDescriptor::packed_ir(
                "az.asset_processor.tests.logic-graph.compiler",
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
            "az.asset_processor.tests.nodes",
        ))
    }

    fn test_generated_rust_graph_type() -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            "az.asset_processor.tests.generated-graph",
            1,
            "Asset Processor Test Generated Graph",
            GraphSourceWorkflow::file(
                "az.asset_processor.tests.generated-graph.source",
                "azgraph.ron",
            )
            .with_default_path_prefix("graphs/generated"),
            GraphCompilerBackendDescriptor::generated_rust_context_schedule(
                "az.asset_processor.tests.generated-graph.compiler",
                "az-asset-processor-generated-tests",
                "az_asset_processor_generated_tests::compile_graph",
            )
            .with_capability_marker("generated-rust"),
            RuntimeGraphProductDescriptor::new(
                GENERATED_GRAPH_ASSET_TYPE_NAME,
                "azoth.graph.generated-rust",
                RuntimeGraphExecutionStrategy::aot_compiled_rust(
                    "az-asset-processor-generated-tests",
                    "az_asset_processor_generated_tests::execute_graph",
                    "az_asset_processor_generated_tests::RuntimeContext",
                ),
            ),
        )
        .with_node_catalog(GraphNodeCatalogRequirement::new(
            "az.asset_processor.tests.nodes",
        ))
    }

    fn test_source_schemas() -> [SourceSchemaRegistration; 4] {
        [
            SourceSchemaRegistration::for_source::<TestPrefabSourceFormat>()
                .with_label("Prefab")
                .with_category("Tests")
                .with_creatable_document_schema("az.test.Prefab"),
            SourceSchemaRegistration::for_source::<TestFileSourceFormat>()
                .with_label("File Source")
                .with_category("Tests")
                .with_creatable_file("sources", &["ron"]),
            SourceSchemaRegistration::for_source::<TestGemFileSourceFormat>()
                .with_label("Gem File Source")
                .with_category("Tests")
                .with_creatable_file_in_source_root(TEST_GEM_SOURCE_ROOT, "sources", &["ron"]),
            SourceSchemaRegistration::for_source::<TestImportFileSourceFormat>()
                .with_label("Imported File Source")
                .with_category("Tests")
                .with_import_file("imports", &["mtl"]),
        ]
    }

    fn test_file_source_template(
        request: &az_asset_builder::SourceFileTemplateRequest<'_>,
    ) -> az_asset_builder::SourceFileTemplateResult {
        if request.source_path == "sources/created.ron" {
            return Ok(b"created source\n".to_vec());
        }
        Err(az_asset_builder::SourceFileTemplateError::unsupported(
            "test template only supports sources/created.ron",
        ))
    }

    fn test_file_source_template_candidates() -> Vec<az_asset_builder::SourceFileTemplateCandidate>
    {
        vec![
            az_asset_builder::SourceFileTemplateCandidate::new("sources/created.ron")
                .with_label("Created Source")
                .with_description("Test source template"),
        ]
    }

    fn wrong_extension_source_template_candidates()
    -> Vec<az_asset_builder::SourceFileTemplateCandidate> {
        vec![az_asset_builder::SourceFileTemplateCandidate::new(
            "sources/created.txt",
        )]
    }

    fn duplicate_source_template_candidates() -> Vec<az_asset_builder::SourceFileTemplateCandidate>
    {
        vec![
            az_asset_builder::SourceFileTemplateCandidate::new("sources/created.ron"),
            az_asset_builder::SourceFileTemplateCandidate::new("sources/created.ron"),
        ]
    }

    fn unsafe_source_template_candidates() -> Vec<az_asset_builder::SourceFileTemplateCandidate> {
        vec![az_asset_builder::SourceFileTemplateCandidate::new(
            "../created.ron",
        )]
    }

    fn test_source_file_templates() -> [az_asset_builder::SourceFileTemplateRegistration; 2] {
        [
            az_asset_builder::SourceFileTemplateRegistration::for_source::<TestFileSourceFormat>(
                test_file_source_template,
            )
            .with_candidates(test_file_source_template_candidates),
            az_asset_builder::SourceFileTemplateRegistration::for_source::<TestGemFileSourceFormat>(
                test_file_source_template,
            )
            .with_candidates(test_file_source_template_candidates),
        ]
    }

    az_gem_contract::declare_caps!(TestCaps:);

    const HARNESS: az_gem_contract::ContributionDescriptor =
        az_gem_contract::ContributionDescriptor {
            gem: az_gem_contract::GemId::new("azoth.asset-processor-tests"),
            contribution: az_gem_contract::ContributionId::new("harness"),
            roles: &[],
        };

    /// Everything the in-process harness contributes: the source schemas and
    /// templates the editor-facing tests exercise, the graph and node types
    /// the projection tests compile, and the SDK fixture product format.
    struct Harness;

    impl az_gem_contract::Contribution for Harness {
        type Caps = TestCaps;

        fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
            HARNESS
        }

        fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, TestCaps>) {
            ctx.registrar::<SourceSchemaRegistration>()
                .register_many(test_source_schemas());
            ctx.registrar::<az_asset_builder::SourceFileTemplateRegistration>()
                .register_many(test_source_file_templates());
            ctx.registrar::<AssetTypeRegistration>().register(
                AssetTypeRegistration::for_asset::<GeneratedGraphAssetData>()
                    .with_owner("az-asset-processor-tests"),
            );
            ctx.registrar::<NodeTypeRegistration>()
                .register(NodeTypeRegistration::new(test_graph_node_type()));
            ctx.registrar::<GraphTypeRegistration>().register_many([
                GraphTypeRegistration::new(test_graph_type()),
                GraphTypeRegistration::new(test_generated_rust_graph_type()),
            ]);
            az_asset_builder::test_support::register(ctx);
            // The graph compiler rule, its product formats and its asset types:
            // these tests drive saved graphs all the way to compiled products.
            az_graph_builder::register(ctx);
        }
    }

    /// Attribute one registration to the harness contribution.
    ///
    /// The catalog validators take composed, attributed entries; these tests
    /// drive them directly with hand-built registrations, so they supply the
    /// same attribution the composer would have.
    fn attributed<T>(entry: T) -> az_gem_contract::Attributed<T> {
        az_gem_contract::Attributed {
            instance: az_gem_contract::InstanceId::new(HARNESS.gem, HARNESS.contribution, 0),
            entry,
        }
    }

    /// The build rules the harness composition contributes.
    fn composed_build_rules() -> BuildRuleRegistry {
        BuildRuleRegistry::compose(&BuilderJobContext::new(test_registries()))
    }

    /// The harness host composes once for the life of the test process, the
    /// same shape a real host has.
    pub fn test_registries() -> &'static Registries {
        static REGISTRIES: OnceLock<&'static Registries> = OnceLock::new();
        REGISTRIES.get_or_init(|| {
            let mut composer =
                az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::AssetWorker);
            composer
                .add(Harness, az_gem_contract::ProductActivation::default())
                .expect("an empty floor composes");
            composer
                .finalize()
                .expect("the harness composition is valid");
            Box::leak(Box::new(composer)).registries()
        })
    }

    fn other_session_uuid() -> Uuid {
        Uuid::from_bytes([0x67; 16])
    }

    fn write_project_manifest_with_lock(root: &Path, manifest: &az_project::ProjectManifest) {
        az_project::write_project_manifest(root, manifest).unwrap();
        az_project::refresh_project_lock(root).unwrap();
    }

    fn write_test_prefab_source(root: &Path, source_path: &str) -> Vec<u8> {
        let path = root.join(source_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let registry = bevy_reflect::TypeRegistry::default();
        let codec = az_prefab::PrefabCodec::new(&registry).unwrap();
        let source = codec.encode(&az_prefab::PrefabDocument::default()).unwrap();
        std::fs::write(&path, source).unwrap();
        std::fs::read(path).unwrap()
    }

    #[test]
    fn production_asset_processor_surface_keeps_raw_internals_test_only() {
        let source = include_str!("lib.rs");
        for signature in ["pub fn db(", "pub fn builders(", "pub fn processor("] {
            assert!(
                public_functions_have_cfg(source, signature, &["#[cfg(test)]"]),
                "`{signature}` must stay test-only; production callers use AssetProcessor RPC instead of raw DB or in-process registry access"
            );
        }
    }

    #[test]
    fn in_process_asset_processor_rpc_constructors_are_not_default_public_api() {
        let source = include_str!("lib.rs");

        for signature in [
            "pub(crate) fn new(processor: AssetProcessor)",
            "pub(crate) fn into_client(self)",
            "pub(crate) fn new(db: AssetDb",
            "pub(crate) fn with_builder_registry(",
        ] {
            assert!(
                source.contains(signature),
                "`{signature}` must stay crate-private; production callers should start an asset-processor RPC server instead of embedding it"
            );
        }

        for signature in [
            "pub fn with_db(",
            "pub fn client_from_rc(",
            "pub fn asset_processor_client(",
        ] {
            assert!(
                public_functions_are_cfg_test_or_test_support(source, signature),
                "`{signature}` must require the explicit test-support feature; in-process asset-processor clients are for tests/harnesses only"
            );
        }
    }

    #[test]
    fn direct_grant_asset_processor_rpc_server_starter_is_test_support_only() {
        let source = include_str!("transport.rs");

        assert!(
            public_functions_are_cfg_test_or_test_support(
                source,
                "pub fn start_asset_processor_rpc_server_with_capability_grants("
            ),
            "`start_asset_processor_rpc_server_with_capability_grants` must require explicit test-support; production asset-processor startup loads brokered grants from the session sidecar"
        );
        assert!(
            public_functions_are_cfg_test_or_test_support(
                source,
                "pub fn start_asset_processor_rpc_server_with_builder_registry_and_capability_grants("
            ),
            "`start_asset_processor_rpc_server_with_builder_registry_and_capability_grants` must require explicit test-support; production asset-processor startup loads registered builders"
        );
        assert!(
            source
                .contains("pub fn start_asset_processor_rpc_server_with_registered_workspace_db("),
            "asset-processor production startup must consume the registered database owner"
        );
        assert!(
            source.contains("let bytes = std::fs::read(capability_grants_file.as_ref())?;"),
            "asset-processor production startup must load its brokered capability grants"
        );
    }

    #[test]
    fn production_asset_processor_host_retains_registered_database_owner() {
        let host = include_str!("../../asset-processor-host/src/main.rs");

        assert!(
            host.contains("open_registered_workspace_asset_db("),
            "production startup must retain the database opened during source-root registration"
        );
        assert!(
            host.contains("start_asset_processor_rpc_server_with_registered_workspace_db("),
            "production startup must consume the registered database instead of reopening it"
        );
        assert!(
            !host.contains("register_workspace_asset_source_roots("),
            "the metadata-only registration API drops its database owner and is not a production startup primitive"
        );
        assert!(
            !host.contains("start_asset_processor_rpc_server_with_capability_grant_file("),
            "the path-based compatibility starter must not replace the registered-database production flow"
        );
    }

    /// The engine floor composes first, and it composes at all.
    ///
    /// Order is the assertion, not decoration: a gem entry that landed before
    /// an engine entry would mean the floor waited on a lock closure, which is
    /// precisely what D6 forbids. The ids come from the generated engine ids
    /// crate, so the manifest, the emitter and this host all read one source.
    #[test]
    fn the_engine_composes_ahead_of_every_gem() {
        az_gem_contract::declare_caps!(OrderCaps:);

        /// A stand-in gem, so the ordering assertion has a second side. It
        /// registers a clock rather than anything asset-shaped, because what
        /// is under test is where its entries land, not what they are.
        struct Gem;

        impl az_gem_contract::Contribution for Gem {
            type Caps = OrderCaps;

            fn descriptor(&self) -> az_gem_contract::ContributionDescriptor {
                az_gem_contract::ContributionDescriptor {
                    gem: az_gem_contract::GemId::new("azoth.order-tests"),
                    contribution: az_gem_contract::ContributionId::new("runtime"),
                    roles: &[],
                }
            }

            fn register(&self, ctx: &mut az_gem_contract::GemContext<'_, OrderCaps>) {
                ctx.registrar::<az_gem_contract::ClockDefinition>()
                    .register(az_gem_contract::ClockDefinition {
                        name: az_gem_contract::ClockName::new("order"),
                        rate_hz: None,
                    });
            }
        }

        let mut composer = Composer::new(GemTargetRole::AssetProcessor);
        compose_engine(&mut composer);
        composer
            .add(Gem, ProductActivation::default())
            .expect("an empty floor composes");
        let report = composer.finalize().expect("the composition is valid");

        let engine: Vec<_> = report
            .composed
            .iter()
            .take_while(|instance| instance.gem == az_engine_ids::ENGINE)
            .map(|instance| instance.contribution)
            .collect();
        assert_eq!(
            engine,
            [
                az_engine_ids::contributions::TYPES,
                az_engine_ids::contributions::ASSETS,
                az_engine_ids::contributions::BUILDERS,
            ],
            "{report}"
        );
        assert!(
            report.composed[engine.len()..]
                .iter()
                .all(|instance| instance.gem != az_engine_ids::ENGINE),
            "no engine contribution composes behind a gem: {report}"
        );
        assert_eq!(
            report.composed[engine.len()].gem.as_str(),
            "azoth.order-tests",
            "{report}"
        );
    }

    /// The builder floor, at the seam that used to be empty.
    ///
    /// `register_workspace_asset_source_roots_blocking` resolves classifiers
    /// from this composition when no worker has published a catalog; an empty
    /// set is `BuilderCatalogUnavailable`, which is what az-session's boundary
    /// test and az-editor's attach test hit before the engine was composed
    /// here (ticket 014, D7).
    #[test]
    fn the_engine_host_classifies_its_own_source_families() {
        let classifiers = source_asset_classifiers(None, engine_host_registries());
        assert!(
            !classifiers.file_sources.is_empty(),
            "the engine host classifies engine-owned source families at startup"
        );
    }

    fn unscoped_worker_capability() -> Capability {
        Capability::new(
            ServiceId::new(ASSET_WORKER_SERVICE_NAMESPACE, ASSET_WORKER_SERVICE_NAME),
            ServiceRole::Worker,
        )
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_JOBS_PERMISSION])
        .with_token_hash([0x6a, 0x6f, 0x62, 0x73])
    }

    fn capability() -> Capability {
        unscoped_worker_capability()
    }

    fn run_asset_worker_once_for_test(
        client: &asset_capnp::asset_processor::Client,
        builders: &BuildRuleRegistry,
        request: AssetWorkerRunOnce,
    ) -> Result<AssetWorkerRunOutcome, AssetWorkerError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("worker test runtime");
        let local = tokio::task::LocalSet::new();
        local.block_on(
            &runtime,
            run_asset_worker_once(client, builders, test_registries(), request),
        )
    }

    fn worker_run_once_request(temp: &Path) -> AssetWorkerRunOnce {
        AssetWorkerRunOnce {
            capability: capability(),
            lease_owner: "worker-a".to_string(),
            lease_duration: Duration::from_secs(30),
            staging_root: temp.join("staging"),
            cache_root: temp.join("cache"),
            cancellation: CancellationToken::new(),
        }
    }

    fn inspect_attempt(rpc: &AssetProcessorRpc, attempt_id: i64) -> az_proto_asset::JobInspection {
        rpc.processor()
            .inspect_job(&InspectJobRequest {
                capability: editor_read_capability(),
                selector: InspectJobSelector::Attempt(attempt_id),
            })
            .unwrap()
            .inspection
            .expect("completed worker attempt remains inspectable")
    }

    fn build_job(entry: &WorkspaceEntry) -> &JobActivity {
        entry
            .jobs
            .iter()
            .find(|activity| matches!(activity.job.owner, JobOwner::Build(_)))
            .expect("workspace entry build job")
    }

    trait WaitForTest: Future + Sized {
        fn wait(self) -> Self::Output {
            block_on_test_runtime(self)
        }
    }

    impl<F: Future> WaitForTest for F {}

    fn replan_source_for_test(
        processor: &AssetProcessor,
        workspace_id: i64,
        root_id: i64,
        source_path: &str,
    ) -> Result<usize, AssetProcessorError> {
        let (asset, entry) = processor
            .db()
            .source_asset(workspace_id, root_id, source_path)?
            .expect("published test source");
        processor
            .enqueue_jobs_for_source(&asset, &entry, false)
            .wait()
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn asset_worker_renews_lease_while_blocking_builder_runs() {
        let fixture = fixture();
        let job = install_fixture_build_job(&fixture, fixture.builder_guid, "default");
        let rpc = Rc::new(AssetProcessorRpc::new(
            grant_backed_processor_with_builder_registry(
                fixture.db,
                registry_with_fixture_builder(),
            ),
        ));
        let client = AssetProcessorRpc::client_from_rc(&rpc);
        let temp = tempfile::tempdir().unwrap();
        let lease_duration = Duration::from_millis(300);
        let worker_request = AssetWorkerRunOnce {
            capability: capability(),
            lease_owner: "worker-a".to_string(),
            lease_duration,
            staging_root: temp.path().join("staging"),
            cache_root: temp.path().join("cache"),
            cancellation: CancellationToken::new(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let attempt_id = local.block_on(&runtime, async {
            let mut lease_request = client.lease_request();
            LeaseAssetJobRequest {
                capability: capability(),
                lease_owner: "worker-a".to_string(),
                lease_duration_ms: u64::try_from(lease_duration.as_millis()).unwrap(),
                staging_root: Some(temp.path().join("staging").to_string_lossy().into_owned()),
            }
            .to_capnp(lease_request.get().init_request())
            .unwrap();
            let lease_response = lease_request.send().promise.await.unwrap();
            let lease = LeaseAssetJobResult::from_capnp(
                lease_response.get().unwrap().get_result().unwrap(),
            )
            .unwrap();
            assert_eq!(lease.leased.job_key, job.key);
            run_asset_worker_blocking_task_with_lease_for_test(
                &client,
                &worker_request,
                lease.leased.attempt_id,
                lease.grant_key,
                || {
                    std::thread::sleep(Duration::from_millis(1_300));
                    Ok(())
                },
            )
            .await
            .unwrap();
            lease.leased.attempt_id
        });

        let attempt = rpc
            .processor()
            .db()
            .attempt_by_id(attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(attempt.owner.as_deref(), Some("worker-a"));
        drop(rpc);
        assert_eq!(attempt.status, DbStatus::Leased);
    }

    /// The compile produced exactly the two products a generated graph owes:
    /// the AOT manifest and the projected Rust source, each under its own
    /// format id and sub-id.
    fn assert_generated_graph_product_records(products: &[JobProductRecord]) {
        let manifest_product = products
            .iter()
            .find(|product| product.path == "graphs/generated/saved.azgraph.azgaot")
            .unwrap();
        assert_eq!(
            manifest_product.asset_type,
            GENERATED_GRAPH_ASSET_TYPE.into_inner()
        );
        assert_eq!(
            manifest_product.product_format,
            AOT_GRAPH_MANIFEST_FORMAT_ID.as_str()
        );
        assert_eq!(
            manifest_product.product_format_version,
            AOT_GRAPH_MANIFEST_PRODUCT_FORMAT_VERSION
        );
        assert_eq!(
            manifest_product.sub_id,
            i64::from(RUNTIME_GRAPH_PRODUCT_SUB_ID)
        );

        let generated_product = products
            .iter()
            .find(|product| product.path == "graphs/generated/saved.azgraph.generated.rs")
            .unwrap();
        assert_eq!(
            generated_product.product_format,
            GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID.as_str()
        );
        assert_eq!(
            generated_product.product_format_version,
            GENERATED_RUST_GRAPH_SOURCE_PRODUCT_FORMAT_VERSION
        );
        assert_eq!(
            generated_product.sub_id,
            i64::from(GENERATED_RUST_GRAPH_SOURCE_PRODUCT_SUB_ID)
        );
    }

    /// The product bytes reached the platform cache, the manifest describes the
    /// graph the source declared, and the generated Rust was projected back into
    /// the project so a build can pick it up.
    fn assert_generated_graph_cache_and_projection(
        project_data_paths: &ProjectDataPaths,
        source_uuid: Uuid,
        source_suffix: &str,
    ) {
        let cache_root = project_data_paths
            .product_cache_dir(DEFAULT_PLATFORM)
            .unwrap();
        let manifest_bytes = std::fs::read(
            cache_root
                .join("graphs")
                .join("generated")
                .join("saved.azgraph.azgaot"),
        )
        .unwrap();
        let manifest = decode_aot_graph_manifest(&manifest_bytes).unwrap();
        assert_eq!(manifest.header.source_uuid, source_uuid);
        assert_eq!(
            manifest.graph_type,
            "az.asset_processor.tests.generated-graph"
        );
        assert_eq!(manifest.product_kind, "azoth.graph.generated-rust");
        assert_eq!(manifest.language, "rust");
        assert_eq!(manifest.package, "az-asset-processor-generated-tests");
        assert_eq!(
            manifest.entry_symbol,
            format!("az_asset_processor_generated_tests_{source_suffix}::execute_graph")
        );
        assert_eq!(
            manifest.context_type,
            "az_asset_processor_generated_tests::RuntimeContext"
        );

        let generated_cache_path = cache_root
            .join("graphs")
            .join("generated")
            .join("saved.azgraph.generated.rs");
        let generated_source = std::fs::read_to_string(&generated_cache_path).unwrap();
        assert!(
            generated_source.contains(&format!(
                "pub mod az_asset_processor_generated_tests_{source_suffix}"
            )),
            "{generated_source}"
        );
        assert!(
            generated_source.contains("pub fn execute_graph("),
            "{generated_source}"
        );
        assert!(
            generated_source.contains("AotGraphRuntimeRegistration::new"),
            "{generated_source}"
        );

        let projected_source_path = project_data_paths
            .graphs_dir()
            .join("graphs")
            .join("generated")
            .join("saved.azgraph.generated.rs");
        assert_eq!(
            std::fs::read_to_string(projected_source_path).unwrap(),
            generated_source
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn asset_worker_compiles_saved_generated_graph_to_aot_manifest_and_projected_rust_source() {
        let source_path = "graphs/generated/saved.azgraph.ron";
        let mut document = VisualGraphDocument::new("az.asset_processor.tests.generated-graph");
        document.nodes.push(GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-00000000a5f2")),
            "az.asset_processor.tests.Print",
            1,
        ));
        let payload = encode_visual_graph_document_ron(&document).unwrap();
        let fixture = fixture_with_source(
            "local.asset_processor_rpc",
            source_path,
            Some("az.asset_processor.tests.generated-graph"),
            payload.as_bytes(),
            Uuid::from_bytes([0x92; 16]),
            None,
        );
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            source_path,
            "az.asset_processor.tests.generated-graph",
            &payload,
        );
        let content_hash = blake3::hash(payload.as_bytes()).as_bytes().to_vec();
        let processor =
            grant_backed_processor_with_builder_registry(fixture.db, composed_build_rules());
        let record = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: fixture.workspace_source_root.workspace_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: source_path.to_string(),
                schema_type: Some("az.asset_processor.tests.generated-graph".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();
        assert!(record.entry.jobs.iter().any(|activity| {
            activity.job.owner == JobOwner::Build(GRAPH_COMPILER_BUILDER_ID.0)
                && activity.job.key == GRAPH_COMPILER_JOB_KEY
        }));

        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let client = AssetProcessorRpc::client_from_rc(&rpc);
        let builders = composed_build_rules();
        let worker_temp = tempfile::tempdir().unwrap();
        let outcome = run_asset_worker_once_for_test(
            &client,
            &builders,
            AssetWorkerRunOnce {
                capability: capability(),
                lease_owner: "worker-a".to_string(),
                lease_duration: Duration::from_secs(30),
                staging_root: worker_temp.path().join("staging"),
                cache_root: worker_temp.path().join("worker-cache"),
                cancellation: CancellationToken::new(),
            },
        )
        .unwrap();

        let AssetWorkerRunOutcome::Completed {
            asset_job_attempt_id,
            product_count,
        } = outcome
        else {
            panic!("expected generated graph worker completion");
        };
        assert_eq!(product_count, 2);

        let inspection = inspect_attempt(&rpc, asset_job_attempt_id);
        drop(rpc);
        assert_eq!(
            inspection.attempt.as_ref().unwrap().status,
            AttemptStatus::Succeeded
        );
        let source_uuid = record.asset_guid;
        let source_suffix = source_uuid.simple().to_string();

        let mut products = inspection.products;
        products.sort_by(|left, right| left.path.cmp(&right.path));
        assert_eq!(products.len(), 2);
        assert_generated_graph_product_records(&products);
        assert_generated_graph_cache_and_projection(
            &fixture.project_data_paths,
            source_uuid,
            &source_suffix,
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn asset_worker_runs_registered_process_job_through_rpc_boundary() {
        let source_path = "prefabs/saved.prefab.ron";
        let fixture = fixture_with_source(
            "local.asset_worker_rpc",
            source_path,
            Some("az.test.Prefab"),
            b"stale source-file bytes",
            Uuid::from_bytes([0x93; 16]),
            None,
        );
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_worker_rpc",
            source_path,
            "az.test.Prefab",
            "prefab bytes",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );
        let source_hash = blake3::hash(b"prefab bytes").as_bytes().to_vec();
        let record = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: fixture.workspace_source_root.workspace_root_id,
                owner_id: "local.asset_worker_rpc".to_string(),
                source_path: source_path.to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash: source_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();
        assert!(
            record
                .entry
                .jobs
                .iter()
                .all(|activity| activity.attempt.is_none()
                    || activity.attempt.as_ref().unwrap().staging.is_none()),
            "source payload handles and worker staging must not leak through source projection"
        );

        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let client = AssetProcessorRpc::client_from_rc(&rpc);
        let outcome = run_asset_worker_once_for_test(
            &client,
            &registry_with_prefab_builder(),
            worker_run_once_request(fixture.temp_dir.path()),
        )
        .unwrap();

        let AssetWorkerRunOutcome::Completed {
            asset_job_attempt_id,
            product_count,
        } = outcome
        else {
            panic!("expected completed worker outcome");
        };
        assert_eq!(product_count, 1);

        let inspection = inspect_attempt(&rpc, asset_job_attempt_id);
        drop(rpc);
        let attempt = inspection.attempt.unwrap();
        assert_eq!(attempt.status, AttemptStatus::Succeeded);
        let attempt_staging_root = PathBuf::from(attempt.staging.as_deref().unwrap());
        assert!(attempt_staging_root.starts_with(fixture.temp_dir.path().join("staging")));
        assert_ne!(
            attempt_staging_root,
            fixture.temp_dir.path().join("staging")
        );
        let products = inspection.products;
        assert_eq!(products.len(), 1);
        assert_eq!(products[0].path, "prefabs/saved.prefab.ron.compiled");
        assert_eq!(
            products[0].asset_type,
            az_core::asset::ids::CONFIG.into_inner()
        );
        assert_eq!(
            products[0].content_hash,
            Digest::from(blake3::hash(b"prefab bytes")).to_string()
        );
        assert!(
            !attempt_staging_root.exists(),
            "terminal worker completion must remove its attempt staging tree"
        );
        assert_eq!(
            std::fs::read(
                fixture
                    .project_data_paths
                    .product_cache_dir(DEFAULT_PLATFORM)
                    .unwrap()
                    .join("prefabs/saved.prefab.ron.compiled")
            )
            .unwrap(),
            b"prefab bytes"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn asset_worker_round_trips_planner_create_jobs_plan_through_host_rpc_boundary() {
        let source_path = "prefabs/planner-round-trip.prefab.ron";
        let fixture = fixture_with_source(
            "local.asset_worker_planner_rpc",
            source_path,
            Some("az.test.Prefab"),
            b"stale source-file bytes must not be used by the worker",
            Uuid::from_bytes([0x94; 16]),
            None,
        );
        let saved_payload = "prefab bytes from the planner source side channel";
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_worker_planner_rpc",
            source_path,
            "az.test.Prefab",
            saved_payload,
        );

        // The host has no in-process builder inventory. This is the production
        // shape: recording the source creates one planner attempt, and the
        // worker inventory below supplies create_jobs across the RPC boundary.
        let worker_builders = registry_with_prefab_builder();
        let worker_catalog = test_builder_catalog(&worker_builders);
        let processor = grant_backed_processor_with_builder_registry_and_catalog(
            fixture.db,
            BuildRuleRegistry::new(),
            Some(worker_catalog),
        );
        let record = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: fixture.workspace_source_root.workspace_root_id,
                owner_id: "local.asset_worker_planner_rpc".to_string(),
                source_path: source_path.to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash: blake3::hash(saved_payload.as_bytes()).as_bytes().to_vec(),
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();
        assert!(
            record
                .entry
                .jobs
                .iter()
                .all(|activity| { activity.job.owner != JobOwner::Build(Uuid::nil()) }),
            "planner control attempts stay out of the user-facing latest product attempt"
        );

        let planner = processor
            .db()
            .jobs_for_asset(fixture.workspace.workspace_id, fixture.asset.asset_id)
            .unwrap()
            .into_iter()
            .find(|job| job.builder.is_none())
            .expect("host must enqueue a planner job");
        assert_eq!(planner.key, ASSET_PLANNER_JOB_KEY);

        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let client = AssetProcessorRpc::client_from_rc(&rpc);
        let outcome = run_asset_worker_once_for_test(
            &client,
            &worker_builders,
            worker_run_once_request(fixture.temp_dir.path()),
        )
        .unwrap();

        let AssetWorkerRunOutcome::Completed {
            asset_job_attempt_id,
            product_count,
        } = outcome
        else {
            panic!("expected completed planner worker outcome");
        };
        assert_eq!(product_count, 1, "planner emits one control product");

        let completed_planner = inspect_attempt(&rpc, asset_job_attempt_id);
        assert_eq!(completed_planner.job.job_id, planner.job_id);
        assert_eq!(
            completed_planner.attempt.unwrap().status,
            AttemptStatus::Succeeded
        );

        // The host decoded the worker's private plan projection and expanded
        // it into the builder-specific attempt. Checking the durable attempt
        // proves the JSON shape survived worker serialization, the manifest
        // side channel, the Cap'n Proto completion RPC, and host parsing.
        let expanded = rpc
            .processor()
            .db()
            .jobs_for_asset(fixture.workspace.workspace_id, fixture.asset.asset_id)
            .unwrap()
            .into_iter()
            .find(|job| job.builder == Some(uuid!("00000000-0000-0000-0000-00000000b001")))
            .expect("host must expand the worker create_jobs plan");
        assert_eq!(expanded.key, "compile-prefab");
        drop(rpc);
        assert_eq!(expanded.platform, DEFAULT_PLATFORM);
        assert_eq!(expanded.status, DbStatus::Queued);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn asset_worker_uses_file_backed_source_payload_side_channel() {
        let source_path = "prefabs/file-backed.prefab.ron";
        let source_bytes = b"file backed prefab bytes";
        let preserved_asset_id = AssetId::new(Uuid::from_bytes([0x8b; 16]), 2);
        let fixture = fixture_with_source(
            "local.asset_worker_file_backed",
            source_path,
            Some("az.test.Prefab"),
            source_bytes,
            preserved_asset_id.guid,
            Some(preserved_asset_id.sub_id),
        );
        install_fixture_build_job(
            &fixture,
            uuid!("00000000-0000-0000-0000-00000000b001"),
            "compile-prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );
        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let client = AssetProcessorRpc::client_from_rc(&rpc);

        let outcome = run_asset_worker_once_for_test(
            &client,
            &registry_with_prefab_builder(),
            worker_run_once_request(fixture.temp_dir.path()),
        )
        .unwrap();

        let AssetWorkerRunOutcome::Completed {
            asset_job_attempt_id,
            product_count,
        } = outcome
        else {
            panic!("expected completed worker outcome");
        };
        assert_eq!(product_count, 1);
        let inspection = inspect_attempt(&rpc, asset_job_attempt_id);
        drop(rpc);
        let attempt = inspection.attempt.unwrap();
        assert_eq!(attempt.status, AttemptStatus::Succeeded);
        let products = inspection.products;
        assert_eq!(products.len(), 1);
        assert_eq!(products[0].sub_id, i64::from(preserved_asset_id.sub_id));
        let attempt_staging_root = PathBuf::from(attempt.staging.as_deref().unwrap());
        assert!(
            !attempt_staging_root.exists(),
            "terminal worker completion must remove its attempt staging tree"
        );
        assert_eq!(
            std::fs::read(
                fixture
                    .project_data_paths
                    .product_cache_dir(DEFAULT_PLATFORM)
                    .unwrap()
                    .join("prefabs/file-backed.prefab.ron.compiled")
            )
            .unwrap(),
            source_bytes
        );
    }

    fn scoped_worker_capability(session: Uuid) -> Capability {
        unscoped_worker_capability().with_session(session)
    }

    fn unscoped_editor_read_capability() -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience(ASSET_PROCESSOR_AUDIENCE)
            .with_permissions([ASSET_READ_PERMISSION])
            .with_token_hash([0x72, 0x65, 0x61, 0x64])
    }

    fn editor_read_capability() -> Capability {
        unscoped_editor_read_capability()
    }

    fn scoped_editor_read_capability(session: Uuid) -> Capability {
        unscoped_editor_read_capability().with_session(session)
    }

    fn unscoped_editor_write_capability() -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience(ASSET_PROCESSOR_AUDIENCE)
            .with_permissions([ASSET_WRITE_PERMISSION])
            .with_token_hash([0x63, 0x72, 0x65, 0x61, 0x74, 0x65])
    }

    fn editor_write_capability() -> Capability {
        unscoped_editor_write_capability()
    }

    fn unscoped_project_host_write_capability() -> Capability {
        Capability::new(
            ServiceId::new("azoth", "project-host"),
            ServiceRole::ProjectHost,
        )
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_WRITE_PERMISSION])
        .with_token_hash([0x77, 0x72, 0x69, 0x74, 0x65])
    }

    fn project_host_write_capability() -> Capability {
        unscoped_project_host_write_capability()
    }

    fn scoped_project_host_write_capability(session: Uuid) -> Capability {
        unscoped_project_host_write_capability().with_session(session)
    }

    fn default_capability_grants() -> CapabilityGrantSet {
        CapabilityGrantSet::from_grants(vec![
            capability(),
            editor_read_capability(),
            editor_write_capability(),
            project_host_write_capability(),
        ])
    }

    /// A processor scoped to a workspace, composed against the test
    /// harness rather than the empty engine-host composition: these tests
    /// stand in for a host that did compose asset-pipeline contributions.
    fn grant_backed_processor(db: AssetDb) -> AssetProcessor {
        let workspace_id = only_workspace_id(&db);
        let project_data_paths = test_project_data_paths(&db);
        let source_roots = test_processor_source_roots(&db, workspace_id);
        AssetProcessor::with_builder_registry_and_catalog(
            db,
            BuildRuleRegistry::new(),
            default_capability_grants(),
            test_registries(),
            Some(workspace_id),
            Some(project_data_paths),
            None,
        )
        .with_source_roots(source_roots)
    }

    fn test_processor_source_roots(db: &AssetDb, workspace_id: i64) -> Vec<RegisteredSourceRoot> {
        let workspace = db
            .workspace_by_id(workspace_id)
            .unwrap()
            .expect("test workspace");
        db.workspace_roots(workspace_id)
            .unwrap()
            .into_iter()
            .map(|policy| {
                let root = db
                    .root_by_id(policy.root_pk)
                    .unwrap()
                    .expect("test source root");
                let role =
                    SourceRootRole::from_is_project_assets(policy.owner == workspace.project);
                RegisteredSourceRoot {
                    workspace_pk: workspace_id,
                    workspace_root_pk: policy.workspace_root_id,
                    root_pk: root.root_id,
                    id: root.key.clone(),
                    owner: policy.owner.clone(),
                    path: policy.path,
                    display_name: if role.is_required() {
                        "Project Assets".to_string()
                    } else {
                        format!("{} Assets", policy.owner)
                    },
                    portable_key: root.key,
                    mount: "@assets@".to_string(),
                    recursive: true,
                    watch: true,
                    writable: true,
                    exclusions: policy.exclusions,
                    output_prefix: String::new(),
                    role,
                }
            })
            .collect()
    }

    fn grant_backed_processor_with_builder_registry(
        db: AssetDb,
        builders: BuildRuleRegistry,
    ) -> AssetProcessor {
        let durable_catalog = (!builders.is_empty()).then(|| test_builder_catalog(&builders));
        grant_backed_processor_with_builder_registry_and_catalog(db, builders, durable_catalog)
    }

    fn grant_backed_processor_with_builder_registry_and_catalog(
        db: AssetDb,
        builders: BuildRuleRegistry,
        durable_catalog: Option<AssetBuilderCatalogResult>,
    ) -> AssetProcessor {
        let workspace_id = only_workspace_id(&db);
        let project_data_paths = test_project_data_paths(&db);
        let source_roots = test_processor_source_roots(&db, workspace_id);
        let workspace = db
            .workspace_by_id(workspace_id)
            .unwrap()
            .expect("test workspace");
        if workspace.builders.is_none()
            && let Some(catalog) = durable_catalog.as_ref()
        {
            let outcome = db
                .writer()
                .unwrap()
                .replace_builder_catalog(ReplaceBuilderCatalog {
                    workspace_pk: workspace_id,
                    expected: None,
                    replacement: worker_builder_catalog_digest(catalog),
                    builders: worker_builder_catalog_descriptors(catalog),
                    plan_delta: PlanDelta::default(),
                    updated: 40,
                })
                .wait_blocking()
                .unwrap();
            assert_eq!(outcome, BuilderCatalogReplaceOutcome::Replaced);
        }
        AssetProcessor::with_builder_registry_and_catalog(
            db,
            builders,
            default_capability_grants(),
            test_registries(),
            Some(workspace_id),
            Some(project_data_paths),
            durable_catalog,
        )
        .with_source_roots(source_roots)
    }

    fn test_builder_catalog(builders: &BuildRuleRegistry) -> AssetBuilderCatalogResult {
        let source_schema_registrations = composed_source_schemas(test_registries());
        let source_file_templates = composed_source_file_templates(test_registries());
        let mut source_schemas = source_schema_registrations
            .into_iter()
            .map(|attributed| source_schema_to_proto(&attributed, &source_file_templates).unwrap())
            .collect::<Vec<_>>();
        source_schemas.extend(graph_source_schemas_to_proto(test_registries()).unwrap());
        source_schemas.sort_by(|left, right| {
            left.schema_type
                .cmp(&right.schema_type)
                .then_with(|| left.owner.cmp(&right.owner))
        });
        AssetBuilderCatalogResult {
            builders: builders.iter().map(asset_builder_to_proto).collect(),
            source_schemas,
            product_formats: composed_product_formats_to_proto(test_registries()),
        }
    }

    fn test_project_data_paths(db: &AssetDb) -> ProjectDataPaths {
        let workspace_id = only_workspace_id(db);
        let workspace = db
            .workspace_by_id(workspace_id)
            .unwrap()
            .expect("test workspace");
        let workspace_root = PathBuf::from(&workspace.root);
        explicit_test_project_data_paths(&workspace.project, &workspace_root)
    }

    fn explicit_test_project_data_paths(
        project_id: &str,
        workspace_root: &Path,
    ) -> ProjectDataPaths {
        AzothDataHome::new(workspace_root.join(".azoth-test-home"))
            .project(project_id, workspace_root)
    }

    fn path_string(path: &std::path::Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn canonical_path_string(path: &std::path::Path) -> String {
        canonical(path).unwrap().to_string_lossy().into_owned()
    }

    fn prefab_patterns() -> &'static [AssetBuilderPattern] {
        static PATTERNS: OnceLock<Box<[AssetBuilderPattern]>> = OnceLock::new();
        PATTERNS
            .get_or_init(|| vec![AssetBuilderPattern::wildcard("*.prefab.ron")].into_boxed_slice())
            .as_ref()
    }

    fn fixture_patterns() -> &'static [AssetBuilderPattern] {
        static PATTERNS: OnceLock<Box<[AssetBuilderPattern]>> = OnceLock::new();
        PATTERNS
            .get_or_init(|| vec![AssetBuilderPattern::wildcard("*.png")].into_boxed_slice())
            .as_ref()
    }

    fn file_source_patterns() -> &'static [AssetBuilderPattern] {
        static PATTERNS: OnceLock<Box<[AssetBuilderPattern]>> = OnceLock::new();
        PATTERNS
            .get_or_init(|| vec![AssetBuilderPattern::wildcard("*.ron")].into_boxed_slice())
            .as_ref()
    }

    #[derive(SourceFormat)]
    #[source(schema = "az.test.Material")]
    struct TestMaterialSourceFormat;

    const PREFAB_SOURCE_SCHEMA_TYPES: &[az_asset_builder::SourceSchemaType] =
        <TestPrefabSourceFormat as SourceFormat>::SCHEMA_TYPES;
    const MATERIAL_SOURCE_SCHEMA_TYPES: &[az_asset_builder::SourceSchemaType] =
        <TestMaterialSourceFormat as SourceFormat>::SCHEMA_TYPES;
    const FILE_SOURCE_SCHEMA_TYPES: &[az_asset_builder::SourceSchemaType] =
        &[TEST_FILE_SOURCE_SCHEMA, TEST_GEM_FILE_SOURCE_SCHEMA];

    fn prefab_create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
        CreateJobsResponse {
            jobs: request
                .platforms
                .iter()
                .map(|platform| JobDescriptor {
                    job_key: az_asset_builder::JobKey::new("compile-prefab").unwrap(),
                    platform: *platform,
                    job_dependencies: Vec::new(),
                    critical: false,
                })
                .collect(),
            source_dependencies: Vec::new(),
            result: CreateJobsResult::Success,
        }
    }

    fn schema_aware_prefab_create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
        if request.source_schema_type != Some("az.test.Prefab")
            || request.source_bytes != b"schema-aware prefab"
        {
            return CreateJobsResponse {
                jobs: Vec::new(),
                source_dependencies: Vec::new(),
                result: CreateJobsResult::Failed,
            };
        }

        prefab_create_jobs(request)
    }

    fn material_create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
        CreateJobsResponse {
            jobs: request
                .platforms
                .iter()
                .map(|platform| JobDescriptor {
                    job_key: az_asset_builder::JobKey::new("compile-material").unwrap(),
                    platform: *platform,
                    job_dependencies: Vec::new(),
                    critical: false,
                })
                .collect(),
            source_dependencies: Vec::new(),
            result: CreateJobsResult::Success,
        }
    }

    static FAIL_NEXT_PREFAB_CREATE_JOBS: AtomicBool = AtomicBool::new(false);

    fn flaky_prefab_create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
        if FAIL_NEXT_PREFAB_CREATE_JOBS.swap(false, Ordering::SeqCst) {
            return CreateJobsResponse {
                jobs: Vec::new(),
                source_dependencies: Vec::new(),
                result: CreateJobsResult::Failed,
            };
        }

        prefab_create_jobs(request)
    }

    fn duplicate_prefab_create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
        let platform = request.platforms[0];
        CreateJobsResponse {
            jobs: vec![
                JobDescriptor {
                    job_key: az_asset_builder::JobKey::new("compile-prefab").unwrap(),
                    platform,
                    job_dependencies: Vec::new(),
                    critical: false,
                },
                JobDescriptor {
                    job_key: az_asset_builder::JobKey::new("compile-prefab").unwrap(),
                    platform,
                    job_dependencies: Vec::new(),
                    critical: false,
                },
            ],
            source_dependencies: Vec::new(),
            result: CreateJobsResult::Success,
        }
    }

    fn dependency_prefab_create_jobs(request: &CreateJobsRequest<'_>) -> CreateJobsResponse {
        let mut response = prefab_create_jobs(request);
        response.source_dependencies = vec![
            SourceFileDependency::Path("materials/base.material.ron".to_string()),
            SourceFileDependency::Uuid(uuid!("77777777-7777-7777-7777-777777777777")),
        ];
        for job in &mut response.jobs {
            job.job_dependencies.push(az_asset_builder::JobDependency {
                source: SourceFileDependency::Path("materials/base.material.ron".to_string()),
                job_key: "compile-material".to_string(),
                platform: job.platform.to_string(),
                kind: JobDependencyType::Fingerprint,
            });
        }
        response
    }

    fn duplicate_dependency_prefab_create_jobs(
        request: &CreateJobsRequest<'_>,
    ) -> CreateJobsResponse {
        let mut response = prefab_create_jobs(request);
        response.source_dependencies = vec![
            SourceFileDependency::Path("materials/base.material.ron".to_string()),
            SourceFileDependency::Path("materials/base.material.ron".to_string()),
        ];
        response
    }

    fn invalid_dependency_prefab_create_jobs(
        request: &CreateJobsRequest<'_>,
    ) -> CreateJobsResponse {
        let mut response = prefab_create_jobs(request);
        response.source_dependencies = vec![SourceFileDependency::Path(
            "materials\\bad.material.ron".to_string(),
        )];
        response
    }

    fn duplicate_job_dependency_prefab_create_jobs(
        request: &CreateJobsRequest<'_>,
    ) -> CreateJobsResponse {
        let mut response = prefab_create_jobs(request);
        for job in &mut response.jobs {
            let dependency = az_asset_builder::JobDependency {
                source: SourceFileDependency::Path("materials/base.material.ron".to_string()),
                job_key: "compile-material".to_string(),
                platform: job.platform.to_string(),
                kind: JobDependencyType::Order,
            };
            job.job_dependencies.push(dependency.clone());
            job.job_dependencies.push(dependency);
        }
        response
    }

    fn prefab_process_job(request: &ProcessJobRequest<'_>) -> ProcessJobResponse {
        ProcessJobResponse {
            products: vec![
                BuildProduct::from_primary_source_dynamic_parts(
                    request,
                    format!("{}.compiled", request.source_path),
                    az_core::asset::ids::CONFIG,
                    az_asset_builder::test_support::TEST_RAW_PRODUCT_FORMAT_ID,
                    1,
                    1,
                    request.source_bytes.to_vec(),
                )
                .expect("test product path is valid"),
            ],
            product_dependencies: Vec::new(),
            result: ProcessJobResult::Success,
        }
    }

    fn prefab_builder_desc() -> BuildRule {
        BuildRule {
            name: "az.test.prefab",
            id: BuilderId::new(uuid!("00000000-0000-0000-0000-00000000b001")),
            primary_source: SourceMatcher::Schemas {
                patterns: prefab_patterns(),
                schemas: PREFAB_SOURCE_SCHEMA_TYPES.into(),
            },
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: prefab_create_jobs,
            process_job: prefab_process_job,
        }
    }

    fn fixture_builder_desc() -> BuildRule {
        BuildRule {
            name: "az.test.fixture",
            id: BuilderId::new(uuid!("00000000-0000-0000-0000-00000000b00f")),
            primary_source: SourceMatcher::PathOnly(fixture_patterns()),
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: prefab_create_jobs,
            process_job: prefab_process_job,
        }
    }

    fn flaky_prefab_builder_desc() -> BuildRule {
        BuildRule {
            create_jobs: flaky_prefab_create_jobs,
            ..prefab_builder_desc()
        }
    }

    fn schema_aware_prefab_builder_desc() -> BuildRule {
        BuildRule {
            create_jobs: schema_aware_prefab_create_jobs,
            ..prefab_builder_desc()
        }
    }

    fn material_builder_desc() -> BuildRule {
        BuildRule {
            name: "az.test.material",
            id: BuilderId::new(uuid!("00000000-0000-0000-0000-00000000b002")),
            primary_source: SourceMatcher::Schemas {
                patterns: prefab_patterns(),
                schemas: MATERIAL_SOURCE_SCHEMA_TYPES.into(),
            },
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: material_create_jobs,
            process_job: prefab_process_job,
        }
    }

    fn file_source_builder_desc() -> BuildRule {
        BuildRule {
            name: "az.test.file-source",
            id: BuilderId::new(uuid!("00000000-0000-0000-0000-00000000b009")),
            primary_source: SourceMatcher::Schemas {
                patterns: file_source_patterns(),
                schemas: FILE_SOURCE_SCHEMA_TYPES.into(),
            },
            source_dependencies: &[],
            version: 1,
            analysis_fingerprint: String::new(),
            product_formats: ProductFormatPolicy::Dynamic,
            create_jobs: prefab_create_jobs,
            process_job: prefab_process_job,
        }
    }

    fn duplicate_prefab_builder_desc() -> BuildRule {
        BuildRule {
            create_jobs: duplicate_prefab_create_jobs,
            ..prefab_builder_desc()
        }
    }

    fn dependency_prefab_builder_desc() -> BuildRule {
        BuildRule {
            create_jobs: dependency_prefab_create_jobs,
            ..prefab_builder_desc()
        }
    }

    fn duplicate_dependency_prefab_builder_desc() -> BuildRule {
        BuildRule {
            create_jobs: duplicate_dependency_prefab_create_jobs,
            ..prefab_builder_desc()
        }
    }

    fn invalid_dependency_prefab_builder_desc() -> BuildRule {
        BuildRule {
            create_jobs: invalid_dependency_prefab_create_jobs,
            ..prefab_builder_desc()
        }
    }

    fn duplicate_job_dependency_prefab_builder_desc() -> BuildRule {
        BuildRule {
            create_jobs: duplicate_job_dependency_prefab_create_jobs,
            ..prefab_builder_desc()
        }
    }

    fn registry_with_prefab_builder() -> BuildRuleRegistry {
        let mut registry = BuildRuleRegistry::new();
        registry.register(prefab_builder_desc());
        registry
    }

    fn registry_with_fixture_builder() -> BuildRuleRegistry {
        let mut registry = BuildRuleRegistry::new();
        registry.register(fixture_builder_desc());
        registry
    }

    fn registry_with_file_source_builder() -> BuildRuleRegistry {
        let mut registry = BuildRuleRegistry::new();
        registry.register(file_source_builder_desc());
        registry
    }

    fn registry_with_flaky_prefab_builder() -> BuildRuleRegistry {
        let mut registry = BuildRuleRegistry::new();
        registry.register(flaky_prefab_builder_desc());
        registry
    }

    fn registry_with_duplicate_prefab_builder() -> BuildRuleRegistry {
        let mut registry = BuildRuleRegistry::new();
        registry.register(duplicate_prefab_builder_desc());
        registry
    }

    fn registry_with_dependency_prefab_builder() -> BuildRuleRegistry {
        let mut registry = BuildRuleRegistry::new();
        registry.register(dependency_prefab_builder_desc());
        registry
    }

    fn registry_with_duplicate_dependency_prefab_builder() -> BuildRuleRegistry {
        let mut registry = BuildRuleRegistry::new();
        registry.register(duplicate_dependency_prefab_builder_desc());
        registry
    }

    fn registry_with_invalid_dependency_prefab_builder() -> BuildRuleRegistry {
        let mut registry = BuildRuleRegistry::new();
        registry.register(invalid_dependency_prefab_builder_desc());
        registry
    }

    fn registry_with_duplicate_job_dependency_prefab_builder() -> BuildRuleRegistry {
        let mut registry = BuildRuleRegistry::new();
        registry.register(duplicate_job_dependency_prefab_builder_desc());
        registry
    }

    fn fixture() -> Fixture {
        fixture_with_source(
            "local.asset_processor_rpc",
            "textures/rpc.png",
            None,
            b"rpc source fixture bytes",
            Uuid::from_bytes([0x91; 16]),
            None,
        )
    }

    fn fixture_with_source(
        project: &str,
        source_path: &str,
        schema: Option<&str>,
        source_bytes: &[u8],
        guid: Uuid,
        preserved_sub_id: Option<u32>,
    ) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let workspace_root = temp.path().join("asset-rpc-workspace");
        let source_root = workspace_root.join("assets");
        std::fs::create_dir_all(&source_root).unwrap();
        let source_abs = source_root.join(source_path);
        std::fs::create_dir_all(source_abs.parent().unwrap()).unwrap();
        std::fs::write(&source_abs, source_bytes).unwrap();
        std::fs::write(
            source_meta_sidecar_path(&source_abs),
            serde_json::to_vec(&SourceAssetMeta::preserving(AssetId::new(
                guid,
                preserved_sub_id.unwrap_or(0),
            )))
            .unwrap(),
        )
        .unwrap();
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let content_hash = Digest::from(blake3::hash(source_bytes));
        let workspace = writer
            .register_workspace(RegisterWorkspace {
                key: WorkspaceKey {
                    project: project.to_string(),
                    root: path_string(&workspace_root),
                    branch: "az/session/rpc".to_string(),
                },
                now: 20,
            })
            .wait_blocking()
            .unwrap();
        let (_, workspace_source_root) = writer
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: workspace.workspace_id,
                key: format!("project:{project}:assets"),
                owner: project.to_string(),
                path: path_string(&source_root),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .unwrap();
        writer
            .apply_sweep_delta(ApplySweepDelta {
                workspace_pk: workspace.workspace_id,
                workspace_root_pk: workspace_source_root.workspace_root_id,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: source_path.to_string(),
                        guid,
                        schema: schema.map(str::to_string),
                        digest: content_hash,
                        diff: DbDiff::Clean,
                        diagnostics: 0,
                        updated: 30,
                        src_bytes: i64::try_from(source_bytes.len()).unwrap(),
                        src_mtime: 10,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 30,
                        session: None,
                    },
                    planner: SweepPlannerJob {
                        key: ASSET_PLANNER_JOB_KEY.to_string(),
                        platform: DEFAULT_PLATFORM.to_string(),
                    },
                }],
                removals: Vec::new(),
            })
            .wait_blocking()
            .unwrap();
        let (asset, _) = db
            .source_asset(
                workspace.workspace_id,
                workspace_source_root.root_pk,
                source_path,
            )
            .unwrap()
            .expect("fixture source observation");
        let project_data_paths = explicit_test_project_data_paths(project, &workspace_root);

        Fixture {
            temp_dir: temp,
            db,
            writer,
            workspace_root,
            project_data_paths,
            source_root,
            workspace,
            workspace_source_root,
            asset,
            builder_guid: uuid!("00000000-0000-0000-0000-00000000b00f"),
        }
    }

    fn install_fixture_build_job(fixture: &Fixture, builder: Uuid, job_key: &str) -> SelectJobs {
        let catalog = if builder == uuid!("00000000-0000-0000-0000-00000000b001") {
            test_builder_catalog(&registry_with_prefab_builder())
        } else if builder == uuid!("00000000-0000-0000-0000-00000000b00f") {
            test_builder_catalog(&registry_with_fixture_builder())
        } else {
            AssetBuilderCatalogResult {
                builders: vec![AssetBuilderDescriptor {
                    name: "fixture builder".to_string(),
                    builder_guid: builder,
                    version: 1,
                    analysis_fingerprint: "fixture-v1".to_string(),
                    patterns: Vec::new(),
                    source_schema_types: Vec::new(),
                }],
                source_schemas: Vec::new(),
                product_formats: Vec::new(),
            }
        };
        let catalog_digest = worker_builder_catalog_digest(&catalog);
        let retire_job_ids = fixture
            .db
            .jobs_for_asset(fixture.workspace.workspace_id, fixture.asset.asset_id)
            .unwrap()
            .into_iter()
            .map(|job| job.job_id)
            .collect();
        let outcome = fixture
            .writer
            .replace_builder_catalog(ReplaceBuilderCatalog {
                workspace_pk: fixture.workspace.workspace_id,
                expected: fixture.workspace.builders,
                replacement: catalog_digest,
                builders: worker_builder_catalog_descriptors(&catalog),
                plan_delta: PlanDelta {
                    retire_job_ids,
                    replacements: vec![PlannedJob::build(
                        fixture.asset.asset_id,
                        builder,
                        job_key,
                        DEFAULT_PLATFORM,
                        Vec::new(),
                    )],
                    ..PlanDelta::default()
                },
                updated: 40,
            })
            .wait_blocking()
            .unwrap();
        assert_eq!(outcome, BuilderCatalogReplaceOutcome::Replaced);
        fixture
            .db
            .jobs_for_asset(fixture.workspace.workspace_id, fixture.asset.asset_id)
            .unwrap()
            .into_iter()
            .find(|job| job.builder == Some(builder) && job.key == job_key)
            .expect("fixture build job")
    }

    fn lease_fixture_build_job(
        fixture: Fixture,
        staging_root: &Path,
    ) -> (
        Rc<AssetProcessorRpc>,
        LeaseAssetJobResult,
        tempfile::TempDir,
    ) {
        let builder = fixture.builder_guid;
        let expected = install_fixture_build_job(&fixture, builder, "default");
        let rpc = Rc::new(AssetProcessorRpc::new(
            grant_backed_processor_with_builder_registry(
                fixture.db,
                registry_with_fixture_builder(),
            ),
        ));
        let request = LeaseAssetJobRequest {
            capability: capability(),
            lease_owner: "worker-a".to_string(),
            lease_duration_ms: 30_000,
            staging_root: Some(path_string(staging_root)),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let leased = local.block_on(&runtime, rpc.lease_job(&request)).unwrap();
        assert_eq!(leased.leased.job_key, expected.key);
        (rpc, leased, fixture.temp_dir)
    }

    fn structured_source_catalog() -> AssetBuilderCatalogResult {
        AssetBuilderCatalogResult {
            builders: Vec::new(),
            source_schemas: vec![SourceSchemaDescriptor {
                schema_type: "az.test.StructuredSource".to_string(),
                owner: "az-asset-processor-tests".to_string(),
                label: "Structured Source".to_string(),
                category: "Tests".to_string(),
                authoring: SourceSchemaAuthoring::File {
                    workflow: SourceFileWorkflowDescriptor {
                        source_root: PROJECT_SOURCE_ROOT.to_string(),
                        default_path_prefix: "sources".to_string(),
                        extensions: vec!["ron".to_string()],
                        can_create: false,
                        can_edit: true,
                    },
                },
                file_templates: Vec::new(),
            }],
            product_formats: Vec::new(),
        }
    }

    fn structured_source_ref() -> WorkspaceSourceFileRef {
        WorkspaceSourceFileRef {
            source_root_key: "project:local.asset_processor_rpc:assets".to_string(),
            source_path: "sources/structured.ron".to_string(),
            schema_type: "az.test.StructuredSource".to_string(),
        }
    }

    fn structured_source_rpc(fixture: &Fixture) -> Rc<AssetProcessorRpc> {
        let processor = AssetProcessor::with_builder_registry_and_catalog(
            fixture.db.new_runtime_handle().unwrap(),
            BuildRuleRegistry::new(),
            default_capability_grants(),
            test_registries(),
            Some(fixture.workspace.workspace_id),
            Some(fixture.project_data_paths.clone()),
            Some(structured_source_catalog()),
        )
        .with_source_roots(vec![fixture_registered_source_root(fixture)]);
        Rc::new(AssetProcessorRpc::new(processor))
    }

    fn install_test_source_file_codec(
        rpc: &AssetProcessorRpc,
        behavior: TestSourceFileCodecBehavior,
        calls: Rc<std::cell::Cell<usize>>,
    ) -> Rc<RefCell<Option<Capability>>> {
        let seen_capability = Rc::new(RefCell::new(None));
        *rpc.source_file_codec.borrow_mut() = Some(ActiveSourceFileCodec {
            client: capnp_rpc::new_client(TestSourceFileCodec {
                behavior,
                calls,
                seen_capability: Rc::clone(&seen_capability),
            }),
            capability: capability(),
        });
        seen_capability
    }

    fn write_structured_source(fixture: &Fixture, bytes: &[u8]) -> PathBuf {
        let source_path = "sources/structured.ron";
        let path = fixture.source_root.join("sources/structured.ron");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        fixture
            .writer
            .apply_sweep_delta(ApplySweepDelta {
                workspace_pk: fixture.workspace.workspace_id,
                workspace_root_pk: fixture.workspace_source_root.workspace_root_id,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: source_path.to_owned(),
                        guid: resolve_source_asset_guid(source_path, None),
                        schema: Some("az.test.StructuredSource".to_owned()),
                        digest: Digest::from(blake3::hash(bytes)),
                        diff: DbDiff::Added,
                        diagnostics: 0,
                        updated: 1_000,
                        src_bytes: i64::try_from(bytes.len()).unwrap(),
                        src_mtime: 1_000,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 1_000,
                        session: Some(TEST_SESSION_ID.to_owned()),
                    },
                    planner: SweepPlannerJob {
                        key: ASSET_PLANNER_JOB_KEY.to_owned(),
                        platform: DEFAULT_PLATFORM.to_owned(),
                    },
                }],
                removals: Vec::new(),
            })
            .wait_blocking()
            .unwrap();
        path
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn structured_source_open_loads_canonical_document_without_writing() {
        let fixture = fixture();
        let original = b"original structured source\n";
        let path = write_structured_source(&fixture, original);
        let calls = Rc::new(std::cell::Cell::new(0));
        let rpc = structured_source_rpc(&fixture);
        install_test_source_file_codec(
            &rpc,
            TestSourceFileCodecBehavior::ContractCompliant,
            Rc::clone(&calls),
        );

        let result = executor::block_on(rpc.open_source_file_transaction(&SourceFileOpenRequest {
            capability: editor_read_capability(),
            session_id: TEST_SESSION_ID.to_string(),
            source: structured_source_ref(),
        }))
        .unwrap();

        assert_eq!(result.snapshot.document, test_source_file_document());
        assert_eq!(
            result.snapshot.source_fingerprint,
            blake3::hash(original).as_bytes()
        );
        assert_eq!(std::fs::read(path).unwrap(), original);
        assert_eq!(calls.get(), 1);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn structured_source_edit_atomically_publishes_worker_saved_source() {
        let fixture = fixture();
        let original = b"original structured source\n";
        let path = write_structured_source(&fixture, original);
        let calls = Rc::new(std::cell::Cell::new(0));
        let rpc = structured_source_rpc(&fixture);
        let seen_capability = install_test_source_file_codec(
            &rpc,
            TestSourceFileCodecBehavior::ContractCompliant,
            Rc::clone(&calls),
        );

        let result = executor::block_on(rpc.write_source_file_transaction(
            &editor_write_capability(),
            TEST_SESSION_ID,
            &structured_source_ref(),
            blake3::hash(original).as_bytes(),
            SourceFileCodecOperation::Edit(az_proto_asset::SourceFileEditOperation::AppendDefault),
            "edit",
        ))
        .unwrap();
        drop(rpc);

        let saved = b"canonical edited source\n";
        assert_eq!(std::fs::read(path).unwrap(), saved);
        assert_eq!(
            result.snapshot.source_fingerprint,
            blake3::hash(saved).as_bytes()
        );
        assert_eq!(result.snapshot.document, test_source_file_document());
        assert_eq!(calls.get(), 1);
        let internal_capability = seen_capability
            .borrow()
            .clone()
            .expect("worker callback must receive an internal capability");
        assert_eq!(internal_capability.role, ServiceRole::Worker);
        assert_ne!(internal_capability, editor_write_capability());
        let authoritative_staging = fixture
            .project_data_paths
            .derived_dir()
            .join("asset-processor/source-file-staging");
        assert!(
            !authoritative_staging.exists()
                || std::fs::read_dir(authoritative_staging)
                    .unwrap()
                    .next()
                    .is_none(),
            "authoritative staging must be consumed after commit"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn structured_source_edit_rejects_fingerprint_conflict_before_worker_dispatch() {
        let fixture = fixture();
        let original = b"original structured source\n";
        let path = write_structured_source(&fixture, original);
        let calls = Rc::new(std::cell::Cell::new(0));
        let rpc = structured_source_rpc(&fixture);
        install_test_source_file_codec(
            &rpc,
            TestSourceFileCodecBehavior::ContractCompliant,
            Rc::clone(&calls),
        );

        let error = executor::block_on(rpc.write_source_file_transaction(
            &editor_write_capability(),
            TEST_SESSION_ID,
            &structured_source_ref(),
            &[0x55; blake3::OUT_LEN],
            SourceFileCodecOperation::Edit(az_proto_asset::SourceFileEditOperation::AppendDefault),
            "edit",
        ))
        .unwrap_err();
        drop(rpc);

        assert!(matches!(
            error,
            AssetProcessorError::SourceFileFingerprintConflict { .. }
        ));
        assert_eq!(calls.get(), 0);
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn structured_source_open_requires_active_worker_callback() {
        let fixture = fixture();
        let original = b"original structured source\n";
        write_structured_source(&fixture, original);
        let rpc = structured_source_rpc(&fixture);

        let error = executor::block_on(rpc.open_source_file_transaction(&SourceFileOpenRequest {
            capability: editor_read_capability(),
            session_id: TEST_SESSION_ID.to_string(),
            source: structured_source_ref(),
        }))
        .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::SourceFileCodecUnavailable
        ));
        let staging_root = fixture
            .project_data_paths
            .derived_dir()
            .join("asset-processor/source-file-staging");
        assert!(
            !staging_root.exists() || std::fs::read_dir(staging_root).unwrap().next().is_none(),
            "authoritative staging must be consumed when callback dispatch fails"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn structured_source_rejects_worker_replacement_shape_violations() {
        let fixture = fixture();
        let original = b"original structured source\n";
        let path = write_structured_source(&fixture, original);
        let rpc = structured_source_rpc(&fixture);
        install_test_source_file_codec(
            &rpc,
            TestSourceFileCodecBehavior::LoadReturnsSavedSource,
            Rc::new(std::cell::Cell::new(0)),
        );
        let load_error =
            executor::block_on(rpc.open_source_file_transaction(&SourceFileOpenRequest {
                capability: editor_read_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source: structured_source_ref(),
            }))
            .unwrap_err();
        assert!(matches!(
            load_error,
            AssetProcessorError::SourceFileCodecReplacement {
                operation: "load",
                ..
            }
        ));

        install_test_source_file_codec(
            &rpc,
            TestSourceFileCodecBehavior::MutationReturnsUnchanged,
            Rc::new(std::cell::Cell::new(0)),
        );
        let edit_error = executor::block_on(rpc.write_source_file_transaction(
            &editor_write_capability(),
            TEST_SESSION_ID,
            &structured_source_ref(),
            blake3::hash(original).as_bytes(),
            SourceFileCodecOperation::Edit(az_proto_asset::SourceFileEditOperation::AppendDefault),
            "edit",
        ))
        .unwrap_err();
        drop(rpc);
        assert!(matches!(
            edit_error,
            AssetProcessorError::SourceFileCodecReplacement {
                operation: "edit",
                ..
            }
        ));
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn structured_source_cleans_ap_owned_output_after_worker_rpc_failure() {
        let fixture = fixture();
        let original = b"original structured source\n";
        let path = write_structured_source(&fixture, original);
        let rpc = structured_source_rpc(&fixture);
        install_test_source_file_codec(
            &rpc,
            TestSourceFileCodecBehavior::RpcFailsAfterWritingOutput,
            Rc::new(std::cell::Cell::new(0)),
        );

        let error = executor::block_on(rpc.write_source_file_transaction(
            &editor_write_capability(),
            TEST_SESSION_ID,
            &structured_source_ref(),
            blake3::hash(original).as_bytes(),
            SourceFileCodecOperation::Edit(az_proto_asset::SourceFileEditOperation::AppendDefault),
            "edit",
        ))
        .unwrap_err();
        drop(rpc);

        assert!(matches!(
            error,
            AssetProcessorError::SourceFileCodecRpc { .. }
        ));
        assert_eq!(std::fs::read(path).unwrap(), original);
        let staging_root = fixture
            .project_data_paths
            .derived_dir()
            .join("asset-processor/source-file-staging");
        assert!(
            !staging_root.exists() || std::fs::read_dir(staging_root).unwrap().next().is_none(),
            "AP-owned staging must be consumed after worker transport failure"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn structured_source_rejects_non_ap_owned_worker_output_handle() {
        let fixture = fixture();
        let original = b"original structured source\n";
        let path = write_structured_source(&fixture, original);
        let rpc = structured_source_rpc(&fixture);
        install_test_source_file_codec(
            &rpc,
            TestSourceFileCodecBehavior::ReturnsWrongOutputDestination,
            Rc::new(std::cell::Cell::new(0)),
        );

        let error = executor::block_on(rpc.write_source_file_transaction(
            &editor_write_capability(),
            TEST_SESSION_ID,
            &structured_source_ref(),
            blake3::hash(original).as_bytes(),
            SourceFileCodecOperation::Edit(az_proto_asset::SourceFileEditOperation::AppendDefault),
            "edit",
        ))
        .unwrap_err();
        drop(rpc);

        assert!(matches!(
            error,
            AssetProcessorError::SourceFileCodecOutputDestination { .. }
        ));
        assert_eq!(std::fs::read(path).unwrap(), original);
        let staging_root = fixture
            .project_data_paths
            .derived_dir()
            .join("asset-processor/source-file-staging");
        assert!(
            !staging_root.exists() || std::fs::read_dir(staging_root).unwrap().next().is_none(),
            "AP-owned staging must be consumed after malformed worker response"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn structured_source_observer_drop_does_not_cancel_publication_or_planning() {
        let fixture = fixture();
        let original = b"original structured source\n";
        let path = write_structured_source(&fixture, original);
        let rpc = structured_source_rpc(&fixture);
        install_test_source_file_codec(
            &rpc,
            TestSourceFileCodecBehavior::WritesOutputAfterDelay,
            Rc::new(std::cell::Cell::new(0)),
        );
        let (_, _, root) = source_file_create_source_root(
            &fixture.db,
            fixture.workspace.workspace_id,
            TEST_SESSION_ID,
            &structured_source_ref().source_root_key,
        )
        .unwrap();
        let expected_digest = Digest::from(blake3::hash(b"canonical edited source\n"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("source-file cancellation test runtime");
        let local = tokio::task::LocalSet::new();
        let (asset, entry) = local.block_on(&runtime, async {
            let receiver = rpc.begin_write_source_file_transaction(
                editor_write_capability(),
                TEST_SESSION_ID.to_string(),
                structured_source_ref(),
                blake3::hash(original).as_bytes().to_vec(),
                SourceFileCodecOperation::Edit(
                    az_proto_asset::SourceFileEditOperation::AppendDefault,
                ),
                "edit",
            );
            tokio::task::yield_now().await;
            drop(receiver);
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let completed = {
                        let db = rpc.processor.db();
                        db.source_asset(
                            fixture.workspace.workspace_id,
                            root.root_id,
                            &structured_source_ref().source_path,
                        )
                        .unwrap()
                        .filter(|(asset, entry)| {
                            entry.digest == expected_digest
                                && !db
                                    .jobs_for_asset(fixture.workspace.workspace_id, asset.asset_id)
                                    .unwrap()
                                    .is_empty()
                        })
                    };
                    if let Some(completed) = completed {
                        return completed;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("detached source publication and planning must complete")
        });

        assert_eq!(std::fs::read(path).unwrap(), b"canonical edited source\n");
        let db = rpc.processor.db();
        assert_eq!(entry.digest, expected_digest);
        assert!(
            !db.jobs_for_asset(fixture.workspace.workspace_id, asset.asset_id)
                .unwrap()
                .is_empty(),
            "the detached application operation must admit planning before it finishes"
        );
        drop(db);
        let staging_root = fixture
            .project_data_paths
            .derived_dir()
            .join("asset-processor/source-file-staging");
        assert!(
            !staging_root.exists() || std::fs::read_dir(staging_root).unwrap().next().is_none(),
            "the background transaction must consume late worker output after cancellation"
        );
    }

    #[test]
    fn source_file_publication_commit_point_is_the_atomic_replace() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("source.ron");
        std::fs::write(&target, b"before").unwrap();
        let transaction_root = temp.path().join("transactions");

        commit_source_file_publication(&transaction_root, &target, b"after".to_vec())
            .expect("file publication is the caller-visible commit point");

        assert_eq!(std::fs::read(target).unwrap(), b"after");
        assert!(
            !transaction_root.exists()
                || std::fs::read_dir(transaction_root)
                    .unwrap()
                    .next()
                    .is_none(),
            "successful publication must leave no pending transaction"
        );
    }

    #[test]
    fn structured_source_restore_authorization_is_project_host_only() {
        let error = validate_project_host_asset_write_capability(
            &editor_write_capability(),
            &default_capability_grants(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AssetProcessorError::InvalidCapability { .. }
        ));
        validate_project_host_asset_write_capability(
            &project_host_write_capability(),
            &default_capability_grants(),
        )
        .unwrap();
    }

    fn upsert_saved_authored_source(
        db: &AssetDb,
        workspace_id: i64,
        project_id: &str,
        source_path: &str,
        schema_type: &str,
        saved_payload: &str,
    ) {
        let root = db
            .workspace_roots(workspace_id)
            .unwrap()
            .into_iter()
            .next()
            .expect("test workspace root");
        let result = db
            .writer()
            .unwrap()
            .write_source_payload(WriteSourcePayload {
                workspace_pk: workspace_id,
                root_pk: root.root_pk,
                path: source_path.to_string(),
                document: source_path.to_string(),
                schema: schema_type.to_string(),
                encoding: Encoding::Ron,
                expected_revision: None,
                revision: 1,
                saved: Some(1),
                digest: Digest::from(blake3::hash(saved_payload.as_bytes())),
                payload: saved_payload.as_bytes().to_vec(),
                checkpoint: CheckpointWrite::Replace(saved_payload.as_bytes().to_vec()),
                session: Some(TEST_SESSION_ID.to_string()),
                project: project_id.to_string(),
                now: 1_000,
            })
            .wait_blocking()
            .unwrap();
        assert!(matches!(result, WriteSourcePayloadResult::Written(_)));
    }

    fn product_manifest_handle(staging_root: &Path, staged_path: &str) -> SideChannelHandle {
        product_manifest_handle_with_product_path(
            staging_root,
            "cache/textures/rpc.dds",
            staged_path,
        )
    }

    fn product_manifest_handle_with_product_path(
        staging_root: &Path,
        product_path: &str,
        staged_path: &str,
    ) -> SideChannelHandle {
        const PRODUCT_BYTES: &[u8] = b"rpc product bytes";
        product_manifest_handle_with_declared_product(
            staging_root,
            product_path,
            staged_path,
            PRODUCT_BYTES,
            blake3::hash(PRODUCT_BYTES).as_bytes().to_vec(),
            PRODUCT_BYTES.len() as u64,
        )
    }

    fn product_manifest_handle_without_dependencies(
        staging_root: &Path,
        product_path: &str,
        staged_path: &str,
    ) -> SideChannelHandle {
        const PRODUCT_BYTES: &[u8] = b"rpc product bytes";
        try_product_manifest_handle_with_dependencies(
            staging_root,
            product_path,
            staged_path,
            PRODUCT_BYTES,
            blake3::hash(PRODUCT_BYTES).as_bytes().to_vec(),
            PRODUCT_BYTES.len() as u64,
            "az.test.raw",
            1,
            Vec::new(),
        )
        .unwrap()
    }

    fn try_product_manifest_handle_with_product_path(
        staging_root: &Path,
        product_path: &str,
        staged_path: &str,
    ) -> Result<SideChannelHandle, Error> {
        const PRODUCT_BYTES: &[u8] = b"rpc product bytes";
        try_product_manifest_handle_with_declared_product(
            staging_root,
            product_path,
            staged_path,
            PRODUCT_BYTES,
            blake3::hash(PRODUCT_BYTES).as_bytes().to_vec(),
            PRODUCT_BYTES.len() as u64,
        )
    }

    fn product_manifest_handle_with_declared_product(
        staging_root: &Path,
        product_path: &str,
        staged_path: &str,
        product_bytes: &[u8],
        content_hash: Vec<u8>,
        byte_length: u64,
    ) -> SideChannelHandle {
        try_product_manifest_handle_with_declared_product_format(
            staging_root,
            product_path,
            staged_path,
            product_bytes,
            content_hash,
            byte_length,
            "az.test.raw",
            1,
        )
        .unwrap()
    }

    fn try_product_manifest_handle_with_declared_product(
        staging_root: &Path,
        product_path: &str,
        staged_path: &str,
        product_bytes: &[u8],
        content_hash: Vec<u8>,
        byte_length: u64,
    ) -> Result<SideChannelHandle, Error> {
        try_product_manifest_handle_with_declared_product_format(
            staging_root,
            product_path,
            staged_path,
            product_bytes,
            content_hash,
            byte_length,
            "az.test.raw",
            1,
        )
    }

    fn product_manifest_handle_with_product_format(
        staging_root: &Path,
        product_path: &str,
        staged_path: &str,
        product_format: &str,
        product_format_version: u32,
    ) -> SideChannelHandle {
        const PRODUCT_BYTES: &[u8] = b"rpc product bytes";
        try_product_manifest_handle_with_declared_product_format(
            staging_root,
            product_path,
            staged_path,
            PRODUCT_BYTES,
            blake3::hash(PRODUCT_BYTES).as_bytes().to_vec(),
            PRODUCT_BYTES.len() as u64,
            product_format,
            product_format_version,
        )
        .unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn try_product_manifest_handle_with_declared_product_format(
        staging_root: &Path,
        product_path: &str,
        staged_path: &str,
        product_bytes: &[u8],
        content_hash: Vec<u8>,
        byte_length: u64,
        product_format: &str,
        product_format_version: u32,
    ) -> Result<SideChannelHandle, Error> {
        try_product_manifest_handle_with_dependencies(
            staging_root,
            product_path,
            staged_path,
            product_bytes,
            content_hash,
            byte_length,
            product_format,
            product_format_version,
            vec![ProductManifestProductDependency {
                asset_guid: Uuid::from_bytes([0xd7; 16]),
                sub_id: 6,
                flags: 1,
            }],
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn try_product_manifest_handle_with_dependencies(
        staging_root: &Path,
        product_path: &str,
        staged_path: &str,
        product_bytes: &[u8],
        content_hash: Vec<u8>,
        byte_length: u64,
        product_format: &str,
        product_format_version: u32,
        dependencies: Vec<ProductManifestProductDependency>,
    ) -> Result<SideChannelHandle, Error> {
        let staged_product_path = Path::new(staged_path);
        let staged_product_path = if staged_product_path.is_absolute() {
            staged_product_path.to_path_buf()
        } else {
            staging_root.join(staged_product_path)
        };
        if let Some(parent) = staged_product_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&staged_product_path, product_bytes).unwrap();

        let manifest = ProductManifest::new(
            1_000,
            vec![ProductManifestProduct {
                product_path: product_path.to_string(),
                catalog_aliases: Vec::new(),
                catalog_path_registration: CatalogPathRegistration::Registered,
                asset_type: Uuid::from_bytes([0xb3; 16]),
                sub_id: 4,
                product_format: product_format.to_string(),
                product_format_version,
                staged_path: staged_path.to_string(),
                content_hash,
                byte_length,
                dependencies,
            }],
        );
        let bytes = encode_product_manifest(&manifest)?;
        let hash = blake3::hash(&bytes).as_bytes().to_vec();
        std::fs::create_dir_all(staging_root).unwrap();
        let path = staging_root.join("product-manifest.capnp.packed");
        std::fs::write(&path, bytes).unwrap();
        let byte_length = std::fs::metadata(&path).unwrap().len();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            byte_length,
            hash,
            std::env::consts::OS,
        )
        .with_capability(capability());
        Ok(handle)
    }

    fn write_test_asset_catalog(project_data_paths: &ProjectDataPaths) {
        let cache_root = project_data_paths
            .product_cache_dir(DEFAULT_PLATFORM)
            .unwrap();
        std::fs::create_dir_all(&cache_root).unwrap();
        let catalog = az_asset::AssetCatalog::new(vec![az_asset::AssetCatalogEntry::new(
            az_asset::AssetId::new(Uuid::from_bytes([0x91; 16]), 4),
            Uuid::from_bytes([0xb3; 16]),
            "az.test.raw",
            1,
            "cache/textures/rpc.dds",
            None,
            b"rpc product bytes".len() as u64,
            *blake3::hash(b"rpc product bytes").as_bytes(),
        )])
        .unwrap();
        let mut file =
            std::fs::File::create(cache_root.join(RELEASE_ASSET_CATALOG_FILE_NAME)).unwrap();
        az_asset::write_asset_catalog(&catalog, &mut file).unwrap();
    }

    fn prepared_product_for_cache(
        project_path: &str,
        staged_file_path: &Path,
        bytes: &[u8],
    ) -> PreparedProduct {
        PreparedProduct {
            product_path: project_path.to_string(),
            asset_type: Uuid::from_bytes([0x11; 16]),
            sub_id: 0,
            product_format: "az.test.raw".to_string(),
            product_format_version: 1,
            catalog_aliases: Aliases::default(),
            catalog_path_registration: Registration::Registered,
            staged_path: staged_file_path.to_string_lossy().into_owned(),
            staged_file_path: staged_file_path.to_path_buf(),
            content_hash: Digest::from(blake3::hash(bytes)),
            byte_length: i64::try_from(bytes.len()).unwrap(),
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn product_cache_promotion_commits_a_path_backed_batch() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let project_data_paths = explicit_test_project_data_paths("batch", &project_root);
        let cache_root = project_data_paths
            .product_cache_dir(DEFAULT_PLATFORM)
            .unwrap();
        let staging_root = temp.path().join("staged");
        std::fs::create_dir_all(&staging_root).unwrap();

        let first_bytes = b"first product bytes";
        let second_bytes = b"second product bytes";
        let first_staged = staging_root.join("first.bin");
        let second_staged = staging_root.join("second.bin");
        std::fs::write(&first_staged, first_bytes).unwrap();
        std::fs::write(&second_staged, second_bytes).unwrap();

        let first_destination = cache_root.join("textures/first.bin");
        let second_destination = cache_root.join("textures/second.bin");
        std::fs::create_dir_all(first_destination.parent().unwrap()).unwrap();
        std::fs::write(&first_destination, b"old first").unwrap();
        std::fs::write(&second_destination, b"old second").unwrap();

        let products = vec![
            prepared_product_for_cache("textures/first.bin", &first_staged, first_bytes),
            prepared_product_for_cache("textures/second.bin", &second_staged, second_bytes),
        ];
        let receipt = promote_products_to_cache(&project_data_paths, &cache_root, &products)
            .unwrap()
            .expect("nonempty product batch retains a promotion receipt");

        assert_eq!(std::fs::read(&first_destination).unwrap(), first_bytes);
        assert_eq!(std::fs::read(&second_destination).unwrap(), second_bytes);
        let transaction_root = product_cache_transaction_root(&project_data_paths);
        assert!(transaction_root.is_dir());
        assert_eq!(
            std::fs::read_dir(&transaction_root).unwrap().count(),
            0,
            "one successful batch leaves no product transaction directory"
        );
        let receipt_root = receipt.backup_root.clone();
        receipt.finalize().unwrap();
        assert!(!receipt_root.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn product_cache_promotion_compensates_after_writer_error() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let project_data_paths = explicit_test_project_data_paths("writer-error", &project_root);
        let cache_root = project_data_paths
            .product_cache_dir(DEFAULT_PLATFORM)
            .unwrap();
        let staging_root = temp.path().join("staged");
        std::fs::create_dir_all(&staging_root).unwrap();

        let replaced_target = cache_root.join("textures/replaced.bin");
        let created_target = cache_root.join("textures/created.bin");
        std::fs::create_dir_all(replaced_target.parent().unwrap()).unwrap();
        std::fs::write(&replaced_target, b"previous product").unwrap();
        let replaced_staged = staging_root.join("replaced.bin");
        let created_staged = staging_root.join("created.bin");
        std::fs::write(&replaced_staged, b"new replacement").unwrap();
        std::fs::write(&created_staged, b"new created").unwrap();
        let products = vec![
            prepared_product_for_cache(
                "textures/replaced.bin",
                &replaced_staged,
                b"new replacement",
            ),
            prepared_product_for_cache("textures/created.bin", &created_staged, b"new created"),
        ];

        let receipt = promote_products_to_cache(&project_data_paths, &cache_root, &products)
            .unwrap()
            .expect("nonempty product batch retains a promotion receipt");
        let receipt_root = receipt.backup_root.clone();
        assert_eq!(std::fs::read(&replaced_target).unwrap(), b"new replacement");
        assert_eq!(std::fs::read(&created_target).unwrap(), b"new created");

        // The durable writer may fail after provisional promotion. Its caller
        // consumes this receipt before returning that terminal failure.
        compensate_product_promotion(1, Some(receipt))
            .await
            .unwrap();

        assert_eq!(
            std::fs::read(&replaced_target).unwrap(),
            b"previous product"
        );
        assert!(!created_target.exists());
        assert!(!receipt_root.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn product_cache_promotion_compensates_after_no_longer_owned() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let project_data_paths = explicit_test_project_data_paths("no-longer-owned", &project_root);
        let cache_root = project_data_paths
            .product_cache_dir(DEFAULT_PLATFORM)
            .unwrap();
        let staging_root = temp.path().join("staged");
        std::fs::create_dir_all(&staging_root).unwrap();

        let replaced_target = cache_root.join("textures/replaced.bin");
        let created_target = cache_root.join("textures/created.bin");
        std::fs::create_dir_all(replaced_target.parent().unwrap()).unwrap();
        std::fs::write(&replaced_target, b"previous product").unwrap();
        let replaced_staged = staging_root.join("replaced.bin");
        let created_staged = staging_root.join("created.bin");
        std::fs::write(&replaced_staged, b"new replacement").unwrap();
        std::fs::write(&created_staged, b"new created").unwrap();
        let products = vec![
            prepared_product_for_cache(
                "textures/replaced.bin",
                &replaced_staged,
                b"new replacement",
            ),
            prepared_product_for_cache("textures/created.bin", &created_staged, b"new created"),
        ];

        let receipt = promote_products_to_cache(&project_data_paths, &cache_root, &products)
            .unwrap()
            .expect("nonempty product batch retains a promotion receipt");
        let receipt_root = receipt.backup_root.clone();

        // A lease fence can lose ownership after staging but before the
        // complete-attempt transaction. That is equally non-durable.
        compensate_product_promotion(1, Some(receipt))
            .await
            .unwrap();

        assert_eq!(
            std::fs::read(&replaced_target).unwrap(),
            b"previous product"
        );
        assert!(!created_target.exists());
        assert!(!receipt_root.exists());
    }

    #[test]
    fn writer_failure_and_compensation_failure_share_one_typed_terminal() {
        let rollback = AssetProcessorError::ProductCachePromotionCompensation {
            path: PathBuf::from("cache/textures/product.bin"),
            source: std::io::Error::other("injected compensation failure"),
        };
        let error = completion_writer_rollback_error(RepoError::WriterStopped, rollback);

        let AssetProcessorError::ProductCacheCompletionRollback { writer, rollback } = error else {
            panic!("writer and rollback failures must remain one typed terminal");
        };
        assert!(matches!(writer, RepoError::WriterStopped));
        assert!(matches!(
            *rollback,
            AssetProcessorError::ProductCachePromotionCompensation { .. }
        ));
    }

    #[test]
    fn promotion_failure_and_recovery_failure_share_one_typed_terminal() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let project_data_paths = explicit_test_project_data_paths("rollback-error", &project_root);
        let transaction_root = product_cache_transaction_root(&project_data_paths);
        std::fs::create_dir_all(transaction_root.parent().unwrap()).unwrap();
        std::fs::write(&transaction_root, b"not a transaction directory").unwrap();
        let backup_root = product_cache_compensation_root(&project_data_paths).join("retained");
        std::fs::create_dir_all(&backup_root).unwrap();

        let error = recover_and_compensate_failed_product_promotion(
            &project_data_paths,
            ProductPromotionReceipt {
                transaction_root: transaction_root.clone(),
                backup_root: backup_root.clone(),
                originals: Vec::new(),
            },
            AssetProcessorError::ProductCacheTransaction {
                root: transaction_root,
                source: az_filesystem::FileTransactionError::EmptyTransaction,
            },
        );

        let AssetProcessorError::ProductCachePromotionRollback {
            promotion,
            rollback,
        } = error
        else {
            panic!("promotion and recovery failures must remain one typed terminal");
        };
        assert!(matches!(
            *promotion,
            AssetProcessorError::ProductCacheTransaction { .. }
        ));
        assert!(matches!(
            *rollback,
            AssetProcessorError::ProductCacheTransactionRecovery { .. }
        ));
        assert!(
            backup_root.exists(),
            "failed recovery must retain the receipt rather than deleting its preimages"
        );
    }

    #[test]
    fn product_cache_promotion_restores_preimages_after_an_interrupted_batch() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let project_data_paths = explicit_test_project_data_paths("recovery", &project_root);
        let cache_root = project_data_paths
            .product_cache_dir(DEFAULT_PLATFORM)
            .unwrap();
        let staging_root = temp.path().join("staged");
        std::fs::create_dir_all(&staging_root).unwrap();

        let first_target = cache_root.join("textures/first.bin");
        let created_target = cache_root.join("textures/created.bin");
        let interrupted_target = cache_root.join("textures/interrupted.bin");
        std::fs::create_dir_all(first_target.parent().unwrap()).unwrap();
        std::fs::write(&first_target, b"old first").unwrap();

        let first_bytes = b"new first";
        let created_bytes = b"new created";
        let interrupted_bytes = b"new interrupted";
        let first_staged = staging_root.join("first.bin");
        let created_staged = staging_root.join("created.bin");
        let interrupted_staged = staging_root.join("interrupted.bin");
        std::fs::write(&first_staged, first_bytes).unwrap();
        std::fs::write(&created_staged, created_bytes).unwrap();
        std::fs::write(&interrupted_staged, interrupted_bytes).unwrap();
        let products = vec![
            prepared_product_for_cache("textures/first.bin", &first_staged, first_bytes),
            prepared_product_for_cache("textures/created.bin", &created_staged, created_bytes),
            prepared_product_for_cache(
                "textures/interrupted.bin",
                &interrupted_staged,
                interrupted_bytes,
            ),
        ];

        let error = promote_products_to_cache_with_transient_apply_failure(
            &project_data_paths,
            &cache_root,
            &products,
            || std::fs::create_dir_all(&interrupted_target).unwrap(),
            || std::fs::remove_dir_all(&interrupted_target).unwrap(),
        )
        .expect_err("the transient directory target must interrupt the final product");
        assert!(matches!(
            error,
            AssetProcessorError::ProductCacheTransaction { .. }
        ));
        assert_eq!(std::fs::read(&first_target).unwrap(), b"old first");
        assert!(
            !created_target.exists(),
            "a target absent before promotion is removed by compensation"
        );
        assert!(
            !interrupted_target.exists(),
            "recovery's forward write is also removed when it had no preimage"
        );
        let transaction_root = product_cache_transaction_root(&project_data_paths);
        assert_eq!(std::fs::read_dir(transaction_root).unwrap().count(), 0);
        let compensation_root = product_cache_compensation_root(&project_data_paths);
        assert_eq!(std::fs::read_dir(compensation_root).unwrap().count(), 0);
    }

    fn catalog_builder(guid: Uuid, pattern: &str) -> AssetBuilderCatalogResult {
        AssetBuilderCatalogResult {
            builders: vec![AssetBuilderDescriptor {
                name: "catalog target builder".to_string(),
                builder_guid: guid,
                version: 1,
                analysis_fingerprint: "catalog-target-v1".to_string(),
                patterns: vec![AssetBuilderPatternDescriptor {
                    kind: AssetBuilderPatternKind::Wildcard,
                    pattern: pattern.to_string(),
                }],
                source_schema_types: Vec::new(),
            }],
            source_schemas: Vec::new(),
            product_formats: Vec::new(),
        }
    }

    #[test]
    fn catalog_dependency_type_resolution_is_reusable_for_edges_sharing_a_target() {
        let builder = Uuid::new_v4();
        let kind = Uuid::new_v4();
        let target = CatalogTarget {
            product_pk: 41,
            job_pk: 31,
            builder: Some(builder),
            guid: Uuid::new_v4(),
            sub_id: 7,
            source: "targets/shared.ron".to_string(),
            schema: None,
            kind,
        };
        let catalog = catalog_builder(builder, "targets/*.ron");

        assert_eq!(
            catalog::active_catalog_target_kind(Some(&target), Some(&catalog)).unwrap(),
            Some(kind)
        );
        assert_eq!(
            catalog::active_catalog_target_kind(Some(&target), Some(&catalog)).unwrap(),
            Some(kind),
            "resolving one dependency edge must not consume its shared target projection"
        );
    }

    #[test]
    fn catalog_dependency_omits_the_type_of_an_inactive_target() {
        let builder = Uuid::new_v4();
        let source_guid = Uuid::new_v4();
        let target_guid = Uuid::new_v4();
        let target = CatalogTarget {
            product_pk: 41,
            job_pk: 31,
            builder: Some(builder),
            guid: target_guid,
            sub_id: 7,
            source: "targets/inactive.ron".to_string(),
            schema: None,
            kind: Uuid::new_v4(),
        };
        let catalog = catalog_builder(builder, "active/*.ron");
        let projected = catalog::catalog_product_entry_to_proto(
            SelectCatalog {
                product_pk: 40,
                workspace_pk: 1,
                platform: "pc".to_string(),
                guid: source_guid,
                sub_id: 1,
                path: "products/source.bin".to_string(),
                kind: Uuid::new_v4(),
                format: "bin".to_string(),
                version: 1,
                aliases: Aliases::default(),
                registration: Registration::Registered,
                digest: Digest::from(blake3::hash(b"source product")),
                bytes: 14,
                source: "active/source.ron".to_string(),
                schema: None,
                job_pk: 30,
                builder: Some(builder),
                job_key: "build".to_string(),
            },
            builder,
            vec![CatalogProductEdge {
                edge: az_assetdb::SelectProductEdges {
                    product_edge_id: 50,
                    product_pk: 40,
                    guid: target_guid,
                    sub_id: 7,
                    flags: 1,
                },
                target: Some(target),
            }],
            Some(&catalog),
        )
        .unwrap();

        assert_eq!(projected.dependencies.len(), 1);
        assert_eq!(projected.dependencies[0].asset_guid, target_guid);
        assert_eq!(projected.dependencies[0].asset_type, None);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn move_restores_the_source_when_a_queued_sweep_makes_its_token_stale() {
        let fixture = fixture();
        let processor = grant_backed_processor(fixture.db.new_runtime_handle().unwrap());
        let original = fixture.source_root.join("textures/rpc.png");
        let moved = fixture.source_root.join("textures/moved.png");
        let original_bytes = std::fs::read(&original).unwrap();

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let barrier = fixture.writer.test_barrier(entered_tx, release_rx);
        entered_rx.recv().unwrap();
        let changed_digest = Digest::from(blake3::hash(b"concurrent source projection"));
        let concurrent_update = fixture.writer.apply_sweep_delta(ApplySweepDelta {
            workspace_pk: fixture.workspace.workspace_id,
            workspace_root_pk: fixture.workspace_source_root.workspace_root_id,
            records: vec![SweepRecord {
                source: SweepEntry {
                    path: "textures/rpc.png".to_string(),
                    guid: fixture.asset.guid,
                    schema: None,
                    digest: changed_digest,
                    diff: DbDiff::Modified,
                    diagnostics: 0,
                    updated: 50,
                    src_bytes: i64::try_from(original_bytes.len()).unwrap(),
                    src_mtime: 50,
                    meta_bytes: 0,
                    meta_mtime: 0,
                    observed: 50,
                    session: None,
                },
                planner: SweepPlannerJob {
                    key: ASSET_PLANNER_JOB_KEY.to_string(),
                    platform: DEFAULT_PLATFORM.to_string(),
                },
            }],
            removals: Vec::new(),
        });

        let request = SourceFileMoveRequest {
            capability: editor_write_capability(),
            session_id: TEST_SESSION_ID.to_string(),
            source_root: PROJECT_SOURCE_ROOT.to_string(),
            from_source_path: "textures/rpc.png".to_string(),
            to_source_path: "textures/moved.png".to_string(),
            changed_unix_ms: 60,
        };
        let mut move_future = Box::pin(processor.move_source_file(&request));
        let waker = futures::task::noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        assert!(
            std::future::Future::poll(move_future.as_mut(), &mut context).is_pending(),
            "the move must stage the file and wait behind the blocked writer"
        );
        assert!(!original.exists());
        assert_eq!(std::fs::read(&moved).unwrap(), original_bytes);

        release_tx.send(()).unwrap();
        barrier.wait_blocking().unwrap();
        concurrent_update.wait_blocking().unwrap();
        let error = executor::block_on(move_future).unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::AuthoredSourcePublicationRejected {
                reason: "source state changed before move",
                ..
            }
        ));
        assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
        assert!(!moved.exists());
        assert_eq!(
            fixture
                .db
                .entry_by_asset(fixture.workspace.workspace_id, fixture.asset.asset_id)
                .unwrap()
                .unwrap()
                .digest,
            changed_digest
        );
    }

    /// Registers a second source belonging to a schema the worker catalog marks
    /// creatable, so a queue-order test has one of each kind to compare.
    fn register_creatable_schema_source(fixture: &Fixture) -> SelectAssets {
        let prioritized_source_path = "deployments/remote.settings.ron";
        let prioritized_source_bytes = b"(profile: \"remote\")";
        let prioritized_source_file = fixture.source_root.join(prioritized_source_path);
        std::fs::create_dir_all(prioritized_source_file.parent().unwrap()).unwrap();
        std::fs::write(&prioritized_source_file, prioritized_source_bytes).unwrap();
        let prioritized_source_hash = Digest::from(blake3::hash(prioritized_source_bytes));
        fixture
            .writer
            .apply_sweep_delta(ApplySweepDelta {
                workspace_pk: fixture.workspace.workspace_id,
                workspace_root_pk: fixture.workspace_source_root.workspace_root_id,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: prioritized_source_path.to_owned(),
                        guid: Uuid::from_bytes([0xb3; 16]),
                        schema: Some("az.test.WorkerDeploymentProfile".to_owned()),
                        digest: prioritized_source_hash,
                        diff: DbDiff::Clean,
                        diagnostics: 0,
                        updated: 50,
                        src_bytes: i64::try_from(prioritized_source_bytes.len()).unwrap(),
                        src_mtime: 40,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 50,
                        session: None,
                    },
                    planner: SweepPlannerJob {
                        key: ASSET_PLANNER_JOB_KEY.to_owned(),
                        platform: DEFAULT_PLATFORM.to_owned(),
                    },
                }],
                removals: Vec::new(),
            })
            .wait_blocking()
            .unwrap();
        let (prioritized_asset, _) = fixture
            .db
            .source_asset(
                fixture.workspace.workspace_id,
                fixture.workspace_source_root.root_pk,
                prioritized_source_path,
            )
            .unwrap()
            .unwrap();
        prioritized_asset
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn worker_catalog_creatable_schemas_do_not_reorder_bulk_queue() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let prioritized_asset = register_creatable_schema_source(&fixture);
        let later_builder = Uuid::from_bytes([0xb4; 16]);
        let catalog_digest = Digest::from(blake3::hash(b"queue order builders"));
        fixture
            .writer
            .replace_builder_catalog(ReplaceBuilderCatalog {
                workspace_pk: fixture.workspace.workspace_id,
                expected: fixture.workspace.builders,
                replacement: catalog_digest,
                builders: vec![
                    BuilderDescriptor {
                        guid: fixture.builder_guid,
                        name: "ordinary builder".to_owned(),
                        version: 1,
                        digest: catalog_digest,
                    },
                    BuilderDescriptor {
                        guid: later_builder,
                        name: "creatable schema builder".to_owned(),
                        version: 1,
                        digest: catalog_digest,
                    },
                ],
                plan_delta: PlanDelta {
                    replacements: vec![
                        PlannedJob::build(
                            fixture.asset.asset_id,
                            fixture.builder_guid,
                            "default",
                            DEFAULT_PLATFORM,
                            Vec::new(),
                        ),
                        PlannedJob::build(
                            prioritized_asset.asset_id,
                            later_builder,
                            "default",
                            DEFAULT_PLATFORM,
                            Vec::new(),
                        ),
                    ],
                    ..PlanDelta::default()
                },
                updated: 60,
            })
            .wait_blocking()
            .unwrap();

        let processor = AssetProcessor::with_builder_registry_and_catalog(
            fixture.db,
            BuildRuleRegistry::new(),
            default_capability_grants(),
            test_registries(),
            Some(fixture.workspace.workspace_id),
            None,
            Some(AssetBuilderCatalogResult {
                builders: Vec::new(),
                source_schemas: vec![SourceSchemaDescriptor {
                    schema_type: "az.test.WorkerDeploymentProfile".to_string(),
                    owner: "az-test-worker".to_string(),
                    label: "Deployment Profile".to_string(),
                    category: "Deployment".to_string(),
                    authoring: SourceSchemaAuthoring::File {
                        workflow: SourceFileWorkflowDescriptor {
                            source_root: PROJECT_SOURCE_ROOT.to_string(),
                            default_path_prefix: "deployments".to_string(),
                            extensions: vec!["settings.ron".to_string()],
                            can_create: true,
                            can_edit: true,
                        },
                    },
                    file_templates: Vec::new(),
                }],
                product_formats: Vec::new(),
            }),
        );
        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let request = LeaseAssetJobRequest {
            capability: capability(),
            lease_owner: "worker-a".to_string(),
            lease_duration_ms: 400,
            staging_root: Some(temp.path().join("staging").to_string_lossy().into_owned()),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let leased = local
            .block_on(&runtime, rpc.lease_job(&request))
            .unwrap()
            .leased;

        assert_eq!(
            leased.source_guid, fixture.asset.guid,
            "authoring capabilities are catalog metadata, not an implicit queue priority"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_lease_validates_capability() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let job = install_fixture_build_job(&fixture, fixture.builder_guid, "default");
        let rpc = Rc::new(AssetProcessorRpc::new(
            grant_backed_processor_with_builder_registry(
                fixture.db,
                registry_with_fixture_builder(),
            ),
        ));
        let request = LeaseAssetJobRequest {
            capability: capability(),
            lease_owner: "worker-a".to_string(),
            lease_duration_ms: 400,
            staging_root: Some(staging_root.to_string_lossy().into_owned()),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let result = local.block_on(&runtime, rpc.lease_job(&request)).unwrap();

        let leased = result.leased;
        assert_eq!(leased.job_key, job.key);
        assert_eq!(leased.staging_root, staging_root.to_string_lossy());
        assert_eq!(leased.source_path, "textures/rpc.png");
        assert_eq!(leased.source_root, path_string(&fixture.source_root));

        let project_scoped = LeaseAssetJobRequest {
            capability: unscoped_worker_capability(),
            lease_owner: "worker-project".to_string(),
            lease_duration_ms: 400,
            staging_root: Some(staging_root.to_string_lossy().into_owned()),
        };
        assert!(
            rpc.processor
                .validate_lease_admission(&project_scoped)
                .is_ok()
        );

        let bad = rpc
            .processor
            .validate_lease_admission(&LeaseAssetJobRequest {
                capability: Capability::new(
                    ServiceId::new("azoth", "project-host"),
                    ServiceRole::ProjectHost,
                ),
                lease_owner: "worker-b".to_string(),
                lease_duration_ms: 400,
                staging_root: Some(staging_root.to_string_lossy().into_owned()),
            })
            .unwrap_err();
        assert!(matches!(bad, AssetProcessorError::InvalidCapability { .. }));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_priority_leasing_advances_past_an_inactive_window() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let expected = install_fixture_build_job(&fixture, fixture.builder_guid, "default");
        let asset_identity_pk = fixture.asset.asset_id;
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_fixture_builder(),
        );
        processor.prioritized_asset_identities.borrow_mut().extend(
            (-i64::try_from(PRIORITIZED_ASSET_LEASE_WINDOW).unwrap()..0)
                .chain(std::iter::once(asset_identity_pk)),
        );
        let request = LeaseAssetJobRequest {
            capability: capability(),
            lease_owner: "worker-a".to_string(),
            lease_duration_ms: 400,
            staging_root: Some(temp.path().join("staging").to_string_lossy().into_owned()),
        };
        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let leased = local.block_on(&runtime, rpc.lease_job(&request)).unwrap();
        drop(rpc);

        assert_eq!(leased.leased.job_key, expected.key);
        assert_eq!(leased.leased.source_guid, fixture.asset.guid);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn persistent_dispatch_handle_observes_committed_reconcile_metadata() {
        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let workspace_root_pk = fixture.workspace_source_root.workspace_root_id;
        let asset_pk = fixture.asset.asset_id;
        let asset_guid = fixture.asset.guid;
        let builder_guid = fixture.builder_guid;
        let writer = fixture.writer.clone();
        let entry = fixture
            .db
            .entry_by_asset(workspace_pk, asset_pk)
            .unwrap()
            .unwrap();
        install_fixture_build_job(&fixture, builder_guid, "default");

        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );
        writer
            .apply_sweep_delta(ApplySweepDelta {
                workspace_pk,
                workspace_root_pk,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: entry.path,
                        guid: asset_guid,
                        schema: Some("az.test.ReconciledSource".to_owned()),
                        digest: entry.digest,
                        diff: DbDiff::Clean,
                        diagnostics: 0,
                        updated: 100,
                        src_bytes: i64::try_from(b"rpc source fixture bytes".len()).unwrap(),
                        src_mtime: 10,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 100,
                        session: Some(TEST_SESSION_ID.to_owned()),
                    },
                    planner: SweepPlannerJob {
                        key: ASSET_PLANNER_JOB_KEY.to_owned(),
                        platform: DEFAULT_PLATFORM.to_owned(),
                    },
                }],
                removals: Vec::new(),
            })
            .wait_blocking()
            .unwrap();
        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let staging = tempfile::tempdir().unwrap();
        let request = LeaseAssetJobRequest {
            capability: capability(),
            lease_owner: "worker-authoritative-source".to_string(),
            lease_duration_ms: 800,
            staging_root: Some(path_string(staging.path())),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let leased = local
            .block_on(&runtime, rpc.lease_job(&request))
            .unwrap()
            .leased;

        assert_eq!(
            leased.source_schema_type.as_deref(),
            Some("az.test.ReconciledSource"),
            "leased payloads must use the source metadata reconciled by the primary handle"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_invalid_worker_lease_request_shape() {
        let fixture = fixture();
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );
        let staging_root = fixture.workspace_root.join("staging");

        let empty_owner = processor
            .validate_lease_admission(&LeaseAssetJobRequest {
                capability: capability(),
                lease_owner: " ".to_string(),
                lease_duration_ms: 400,
                staging_root: Some(path_string(&staging_root)),
            })
            .unwrap_err();
        assert!(matches!(
            empty_owner,
            AssetProcessorError::InvalidWorkerJobRequest { .. }
        ));

        let zero_duration = processor
            .validate_lease_admission(&LeaseAssetJobRequest {
                capability: capability(),
                lease_owner: "worker-a".to_string(),
                lease_duration_ms: 0,
                staging_root: Some(path_string(&staging_root)),
            })
            .unwrap_err();
        assert!(matches!(
            zero_duration,
            AssetProcessorError::InvalidWorkerJobRequest { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_requires_an_attached_workspace_for_project_workers() {
        let processor = AssetProcessor::with_builder_registry(
            AssetDb::open_in_memory().unwrap(),
            registry_with_prefab_builder(),
            default_capability_grants(),
            test_registries(),
        );
        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let staging = tempfile::tempdir().unwrap();
        let request = LeaseAssetJobRequest {
            capability: capability(),
            lease_owner: "worker-a".to_string(),
            lease_duration_ms: 400,
            staging_root: Some(path_string(staging.path())),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let error = local
            .block_on(&runtime, rpc.lease_job(&request))
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::MissingAttachedWorkspace
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_invalid_worker_attempt_request_shape() {
        let fixture = fixture();
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );

        let invalid_renew = processor
            .validate_renewal_admission(&RenewAssetJobLeaseRequest {
                capability: capability(),
                asset_job_attempt_id: 0,
                lease_owner: "worker-a".to_string(),
                grant_key: Uuid::from_bytes([0x71; 16]),
            })
            .unwrap_err();
        assert!(matches!(
            invalid_renew,
            AssetProcessorError::InvalidWorkerJobRequest { .. }
        ));

        let invalid_complete = processor
            .validate_completion_admission(&CompleteAssetJobAttemptRequest {
                capability: capability(),
                asset_job_attempt_id: 1,
                lease_owner: " worker-a".to_string(),
                grant_key: Uuid::from_bytes([0x72; 16]),
                status: AttemptStatus::Failed,
                finished_unix_ms: -1,
                error_count: 1,
                warning_count: 0,
                product_manifest: None,
            })
            .unwrap_err();
        assert!(matches!(
            invalid_complete,
            AssetProcessorError::InvalidWorkerJobRequest { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_asset_jobs_from_noncanonical_worker_identity() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let processor = grant_backed_processor(fixture.db);

        let mut noncanonical_worker = capability();
        noncanonical_worker.service = ServiceId::new("project", "custom-asset-worker");
        let service_error = processor
            .validate_lease_admission(&LeaseAssetJobRequest {
                capability: noncanonical_worker,
                lease_owner: "worker-a".to_string(),
                lease_duration_ms: 400,
                staging_root: Some(staging_root.to_string_lossy().into_owned()),
            })
            .unwrap_err();
        match service_error {
            AssetProcessorError::InvalidCapability { reason } => {
                assert!(reason.contains("azoth/asset-worker"));
            }
            error => panic!("expected invalid capability, got {error:?}"),
        }

        let processor_role = Capability::new(
            ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
            ServiceRole::AssetProcessor,
        )
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_JOBS_PERMISSION]);
        let role_error = processor
            .validate_lease_admission(&LeaseAssetJobRequest {
                capability: processor_role,
                lease_owner: "worker-a".to_string(),
                lease_duration_ms: 400,
                staging_root: Some(staging_root.to_string_lossy().into_owned()),
            })
            .unwrap_err();
        match role_error {
            AssetProcessorError::InvalidCapability { reason } => {
                assert!(reason.contains("expected role Worker"));
            }
            error => panic!("expected invalid capability, got {error:?}"),
        }
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_relative_staging_root() {
        let fixture = fixture();
        let processor = grant_backed_processor(fixture.db);

        let error = processor
            .validate_lease_admission(&LeaseAssetJobRequest {
                capability: capability(),
                lease_owner: "worker-a".to_string(),
                lease_duration_ms: 400,
                staging_root: Some("staging/worker-a".to_string()),
            })
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidStagingRoot { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_product_manifest_outside_staging_root() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let outside_root = temp.path().join("outside").join("worker-a");
        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);

        let error = rpc
            .processor()
            .complete_attempt(&CompleteAssetJobAttemptRequest {
                capability: capability(),
                asset_job_attempt_id: lease.leased.attempt_id,
                lease_owner: "worker-a".to_string(),
                grant_key: lease.grant_key,
                status: AttemptStatus::Succeeded,
                finished_unix_ms: 400,
                error_count: 0,
                warning_count: 0,
                product_manifest: Some(product_manifest_handle(&outside_root, "textures/rpc.dds")),
            })
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::ProductManifestOutsideStagingRoot { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_product_manifest_without_matching_handle_capability() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);

        let mut missing = product_manifest_handle(&staging_root, "textures/rpc.dds");
        missing.capability = None;
        let missing_error = rpc
            .processor()
            .complete_attempt(&CompleteAssetJobAttemptRequest {
                capability: capability(),
                asset_job_attempt_id: lease.leased.attempt_id,
                lease_owner: "worker-a".to_string(),
                grant_key: lease.grant_key,
                status: AttemptStatus::Succeeded,
                finished_unix_ms: 400,
                error_count: 0,
                warning_count: 0,
                product_manifest: Some(missing),
            })
            .unwrap_err();
        assert!(matches!(
            missing_error,
            AssetProcessorError::MissingProductManifestCapability
        ));

        let mismatched = product_manifest_handle(&staging_root, "textures/rpc.dds")
            .with_capability(capability().with_token_hash([0x99]));
        let mismatch_error = rpc
            .processor()
            .complete_attempt(&CompleteAssetJobAttemptRequest {
                capability: capability(),
                asset_job_attempt_id: lease.leased.attempt_id,
                lease_owner: "worker-a".to_string(),
                grant_key: lease.grant_key,
                status: AttemptStatus::Succeeded,
                finished_unix_ms: 400,
                error_count: 0,
                warning_count: 0,
                product_manifest: Some(mismatched),
            })
            .unwrap_err();
        assert!(matches!(
            mismatch_error,
            AssetProcessorError::ProductManifestCapabilityMismatch
        ));
    }

    #[test]
    fn product_manifest_transport_rejects_unsafe_product_paths_before_worker_completion() {
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");

        let error = try_product_manifest_handle_with_product_path(
            &staging_root,
            "../cache/escape.dds",
            "textures/rpc.dds",
        )
        .unwrap_err();

        assert!(error.to_string().contains("invalid product manifest"));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_unregistered_product_format_from_worker_manifest() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);

        let error = rpc
            .processor()
            .complete_attempt(&CompleteAssetJobAttemptRequest {
                capability: capability(),
                asset_job_attempt_id: lease.leased.attempt_id,
                lease_owner: "worker-a".to_string(),
                grant_key: lease.grant_key,
                status: AttemptStatus::Succeeded,
                finished_unix_ms: 400,
                error_count: 0,
                warning_count: 0,
                product_manifest: Some(product_manifest_handle_with_product_format(
                    &staging_root,
                    "cache/textures/rpc.dds",
                    "textures/rpc.dds",
                    "az.test.unregistered",
                    1,
                )),
            })
            .unwrap_err();

        assert!(
            matches!(error, AssetProcessorError::InvalidProductManifest { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("unregistered product format"));
    }

    /// Publishes a worker catalog that declares its own product format, then
    /// retires the fixture jobs and replans the asset onto that builder.
    ///
    /// Returns the build job the replan produced, which is the one a lease has
    /// to hand out for the product format to be exercised.
    fn publish_worker_product_format_and_replan(
        processor: &AssetProcessor,
        workspace_pk: i64,
        asset_pk: i64,
        builder_guid: Uuid,
    ) -> SelectJobs {
        processor
            .publish_builder_catalog(&PublishBuilderCatalogRequest {
                capability: capability(),
                protocol: ProtocolVersion::CURRENT,
                catalog: AssetBuilderCatalogResult {
                    builders: vec![AssetBuilderDescriptor {
                        name: "az.test.worker-published".to_string(),
                        builder_guid,
                        version: 1,
                        analysis_fingerprint: "worker-published-v1".to_string(),
                        patterns: vec![AssetBuilderPatternDescriptor {
                            kind: AssetBuilderPatternKind::Wildcard,
                            pattern: "*.png".to_string(),
                        }],
                        source_schema_types: Vec::new(),
                    }],
                    source_schemas: Vec::new(),
                    product_formats: vec![ProductFormatDescriptor {
                        id: "az.test.worker-published".to_string(),
                        current_version: 2,
                        owner: "az-test-worker".to_string(),
                    }],
                },
            })
            .wait()
            .unwrap();
        let retire_job_ids = processor
            .db()
            .jobs_for_asset(workspace_pk, asset_pk)
            .unwrap()
            .into_iter()
            .map(|job| job.job_id)
            .collect();
        processor
            .asset_db_writer()
            .apply_plan_delta(ApplyPlanDelta {
                workspace_pk,
                delta: PlanDelta {
                    retire_job_ids,
                    replacements: vec![PlannedJob::build(
                        asset_pk,
                        builder_guid,
                        "default",
                        DEFAULT_PLATFORM,
                        Vec::new(),
                    )],
                    ..PlanDelta::default()
                },
            })
            .wait()
            .unwrap();

        processor
            .db()
            .jobs_for_asset(workspace_pk, asset_pk)
            .unwrap()
            .into_iter()
            .find(|job| job.builder == Some(builder_guid) && job.key == "default")
            .expect("worker-published catalog build job")
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_accepts_product_format_from_published_worker_catalog() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let workspace_pk = fixture.workspace.workspace_id;
        let asset_pk = fixture.asset.asset_id;
        let builder_guid = fixture.builder_guid;
        let processor = grant_backed_processor(fixture.db);
        let job = publish_worker_product_format_and_replan(
            &processor,
            workspace_pk,
            asset_pk,
            builder_guid,
        );
        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let request = LeaseAssetJobRequest {
            capability: capability(),
            lease_owner: "worker-a".to_string(),
            lease_duration_ms: 400,
            staging_root: Some(staging_root.to_string_lossy().into_owned()),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let lease = local.block_on(&runtime, rpc.lease_job(&request)).unwrap();
        assert_eq!(lease.leased.job_key, job.key);

        assert!(
            local
                .block_on(
                    &runtime,
                    rpc.processor()
                        .complete_attempt_async(&CompleteAssetJobAttemptRequest {
                            capability: capability(),
                            asset_job_attempt_id: lease.leased.attempt_id,
                            lease_owner: "worker-a".to_string(),
                            grant_key: lease.grant_key,
                            status: AttemptStatus::Succeeded,
                            finished_unix_ms: 400,
                            error_count: 0,
                            warning_count: 0,
                            product_manifest: Some(product_manifest_handle_with_product_format(
                                &staging_root,
                                "cache/textures/rpc.dds",
                                "textures/rpc.dds",
                                "az.test.worker-published",
                                2,
                            )),
                        }),
                )
                .unwrap()
        );

        let inspection = rpc
            .processor()
            .inspect_job(&InspectJobRequest {
                capability: editor_read_capability(),
                selector: InspectJobSelector::Attempt(lease.leased.attempt_id),
            })
            .unwrap();
        let products = inspection.inspection.unwrap().products;
        assert_eq!(products.len(), 1);
        assert_eq!(products[0].product_format, "az.test.worker-published");
        assert_eq!(products[0].product_format_version, 2);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn builder_catalog_first_publication_replans_once_and_identical_republication_is_idempotent() {
        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let asset_pk = fixture.asset.asset_id;
        let builder_guid = uuid!("21fdcb7b-7628-4014-9976-96c5cf95ec31");
        let processor = grant_backed_processor(fixture.db);
        let catalog = AssetBuilderCatalogResult {
            builders: vec![AssetBuilderDescriptor {
                name: "az.test.png".to_string(),
                builder_guid,
                version: 1,
                analysis_fingerprint: "analysis-a".to_string(),
                patterns: vec![AssetBuilderPatternDescriptor {
                    kind: AssetBuilderPatternKind::Wildcard,
                    pattern: "*.png".to_string(),
                }],
                source_schema_types: Vec::new(),
            }],
            source_schemas: Vec::new(),
            product_formats: Vec::new(),
        };
        processor
            .publish_builder_catalog(&PublishBuilderCatalogRequest {
                capability: capability(),
                protocol: ProtocolVersion::CURRENT,
                catalog: catalog.clone(),
            })
            .wait()
            .unwrap();
        let first_job = processor
            .db()
            .jobs_for_asset(workspace_pk, asset_pk)
            .unwrap()
            .into_iter()
            .find(|job| job.kind == DbWork::Plan)
            .expect("first publication replaces the source planner Job");
        assert_eq!(
            processor
                .db()
                .workspace_by_id(workspace_pk)
                .unwrap()
                .unwrap()
                .builders,
            Some(worker_builder_catalog_digest(&catalog))
        );

        processor
            .publish_builder_catalog(&PublishBuilderCatalogRequest {
                capability: capability(),
                protocol: ProtocolVersion::CURRENT,
                catalog,
            })
            .wait()
            .unwrap();
        let unchanged_job = processor
            .db()
            .jobs_for_asset(workspace_pk, asset_pk)
            .unwrap()
            .into_iter()
            .find(|job| job.kind == DbWork::Plan)
            .expect("identical publication preserves the planner Job");
        assert_eq!(unchanged_job.job_id, first_job.job_id);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    /// Registers a second, differently-extensioned source in the fixture, so a
    /// test can tell a builder-scoped invalidation from a blanket one.
    fn register_second_source(fixture: &Fixture, dds_guid: Uuid) -> i64 {
        let workspace_pk = fixture.workspace.workspace_id;
        fixture
            .writer
            .apply_sweep_delta(ApplySweepDelta {
                workspace_pk,
                workspace_root_pk: fixture.workspace_source_root.workspace_root_id,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: "textures/other.dds".to_string(),
                        guid: dds_guid,
                        schema: None,
                        digest: Digest::from(blake3::hash(b"dds source")),
                        diff: DbDiff::Clean,
                        diagnostics: 0,
                        updated: 31,
                        src_bytes: 10,
                        src_mtime: 11,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 31,
                        session: None,
                    },
                    planner: SweepPlannerJob {
                        key: ASSET_PLANNER_JOB_KEY.to_string(),
                        platform: DEFAULT_PLATFORM.to_string(),
                    },
                }],
                removals: Vec::new(),
            })
            .wait_blocking()
            .unwrap();

        fixture
            .db
            .source_asset(
                workspace_pk,
                fixture.workspace_source_root.root_pk,
                "textures/other.dds",
            )
            .unwrap()
            .unwrap()
            .0
            .asset_id
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn published_builder_analysis_fingerprint_invalidates_only_matching_current_sources() {
        fn descriptor(
            name: &str,
            builder_guid: Uuid,
            pattern: &str,
            analysis_fingerprint: &str,
        ) -> AssetBuilderDescriptor {
            AssetBuilderDescriptor {
                name: name.to_string(),
                builder_guid,
                version: 1,
                analysis_fingerprint: analysis_fingerprint.to_string(),
                patterns: vec![AssetBuilderPatternDescriptor {
                    kind: AssetBuilderPatternKind::Wildcard,
                    pattern: pattern.to_string(),
                }],
                source_schema_types: Vec::new(),
            }
        }

        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let png_asset_pk = fixture.asset.asset_id;
        let dds_guid = Uuid::from_bytes([0x92; 16]);
        let dds_asset_pk = register_second_source(&fixture, dds_guid);
        let png_builder = uuid!("21fdcb7b-7628-4014-9976-96c5cf95ec31");
        let dds_builder = uuid!("87b6e895-f21b-4a03-ae49-c1c2fc0411bc");
        let catalog = |png_fingerprint: &str| AssetBuilderCatalogResult {
            builders: vec![
                descriptor("az.test.png", png_builder, "*.png", png_fingerprint),
                descriptor("az.test.dds", dds_builder, "*.dds", "analysis-dds"),
            ],
            source_schemas: Vec::new(),
            product_formats: Vec::new(),
        };
        let processor = grant_backed_processor(fixture.db);
        let publish = |catalog| {
            processor
                .publish_builder_catalog(&PublishBuilderCatalogRequest {
                    capability: capability(),
                    protocol: ProtocolVersion::CURRENT,
                    catalog,
                })
                .wait()
                .unwrap();
        };
        let plan_job_id = |asset_pk| {
            processor
                .db()
                .jobs_for_asset(workspace_pk, asset_pk)
                .unwrap()
                .into_iter()
                .find(|job| job.kind == DbWork::Plan)
                .unwrap()
                .job_id
        };

        publish(catalog("analysis-a"));
        let first_png_job = plan_job_id(png_asset_pk);
        let first_dds_job = plan_job_id(dds_asset_pk);

        publish(catalog("analysis-b"));
        let changed_png_job = plan_job_id(png_asset_pk);
        assert_ne!(changed_png_job, first_png_job);
        assert_eq!(plan_job_id(dds_asset_pk), first_dds_job);

        publish(catalog("analysis-b"));
        assert_eq!(plan_job_id(png_asset_pk), changed_png_job);
        assert_eq!(plan_job_id(dds_asset_pk), first_dds_job);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn removing_builder_retires_its_current_jobs_with_catalog_replacement() {
        let retained_guid = uuid!("21fdcb7b-7628-4014-9976-96c5cf95ec31");
        let removed_guid = uuid!("87b6e895-f21b-4a03-ae49-c1c2fc0411bc");
        let descriptor = |name: &str, builder_guid: Uuid| AssetBuilderDescriptor {
            name: name.to_string(),
            builder_guid,
            version: 1,
            analysis_fingerprint: "analysis-a".to_string(),
            patterns: vec![AssetBuilderPatternDescriptor {
                kind: AssetBuilderPatternKind::Wildcard,
                pattern: "*.png".to_string(),
            }],
            source_schema_types: Vec::new(),
        };
        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let asset_pk = fixture.asset.asset_id;
        let processor = grant_backed_processor(fixture.db);
        let publish = |builders| {
            processor
                .publish_builder_catalog(&PublishBuilderCatalogRequest {
                    capability: capability(),
                    protocol: ProtocolVersion::CURRENT,
                    catalog: AssetBuilderCatalogResult {
                        builders,
                        source_schemas: Vec::new(),
                        product_formats: Vec::new(),
                    },
                })
                .wait()
                .unwrap();
        };

        publish(vec![
            descriptor("az.test.retained", retained_guid),
            descriptor("az.test.removed", removed_guid),
        ]);
        processor
            .asset_db_writer
            .apply_plan_delta(ApplyPlanDelta {
                workspace_pk,
                delta: PlanDelta {
                    replacements: vec![
                        PlannedJob::build(
                            asset_pk,
                            retained_guid,
                            "compile-retained",
                            DEFAULT_PLATFORM,
                            Vec::new(),
                        ),
                        PlannedJob::build(
                            asset_pk,
                            removed_guid,
                            "compile-removed",
                            DEFAULT_PLATFORM,
                            Vec::new(),
                        ),
                    ],
                    ..PlanDelta::default()
                },
            })
            .wait()
            .unwrap();
        let retained_job_id = processor
            .db()
            .jobs_for_asset(workspace_pk, asset_pk)
            .unwrap()
            .into_iter()
            .find(|job| job.builder == Some(retained_guid))
            .unwrap()
            .job_id;

        publish(vec![descriptor("az.test.retained", retained_guid)]);
        let jobs = processor
            .db()
            .jobs_for_asset(workspace_pk, asset_pk)
            .unwrap();
        assert!(jobs.iter().all(|job| job.builder != Some(removed_guid)));
        drop(processor);
        assert_eq!(
            jobs.iter()
                .find(|job| job.builder == Some(retained_guid))
                .unwrap()
                .job_id,
            retained_job_id
        );
        assert!(jobs.iter().any(|job| job.kind == DbWork::Plan));
    }

    /// An in-memory workspace with one registered file-backed source root,
    /// ready for a startup reconcile.
    struct ObservationFixture {
        db: AssetDb,
        group: AssetDbWriter,
        workspace_pk: i64,
        root_pk: i64,
        roots: Vec<RegisteredSourceRoot>,
        classifiers: SourceAssetClassifiers,
    }

    /// Registers `source_root` under `workspace_root` and builds the classifiers
    /// that claim `sources/*.ron`, so a reconcile has exactly one source to see.
    fn observation_fixture(workspace_root: &Path, source_root: &Path) -> ObservationFixture {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let workspace = writer
            .register_workspace(RegisterWorkspace {
                key: WorkspaceKey {
                    project: "local.asset_observation".to_string(),
                    root: path_string(workspace_root),
                    branch: "az/session/asset-observation".to_string(),
                },
                now: 10,
            })
            .wait_blocking()
            .unwrap();
        let (root, workspace_source_root) = writer
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: workspace.workspace_id,
                key: "project:local.asset_observation:assets".to_string(),
                owner: "local.asset_observation".to_string(),
                path: path_string(source_root),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .unwrap();
        let roots = vec![RegisteredSourceRoot {
            workspace_pk: workspace.workspace_id,
            workspace_root_pk: workspace_source_root.workspace_root_id,
            root_pk: root.root_id,
            id: root.key.clone(),
            owner: workspace_source_root.owner.clone(),
            path: workspace_source_root.path.clone(),
            display_name: "Assets".to_string(),
            portable_key: root.key,
            mount: "@assets@".to_string(),
            recursive: true,
            watch: true,
            writable: true,
            exclusions: workspace_source_root.exclusions.clone(),
            output_prefix: String::new(),
            role: SourceRootRole::ProjectAssets,
        }];
        let classifiers = SourceAssetClassifiers {
            project_documents: Vec::new(),
            file_sources: vec![FileSourceClassifier {
                source_schema_type: TEST_FILE_SOURCE_SCHEMA.as_str().to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                default_path_prefix: "sources".to_string(),
                source_patterns: file_source_patterns().to_vec(),
                extensions: vec!["ron".to_string()],
            }],
            builder_claims: vec![SourceBuilderClassifier {
                source_schema_types: vec![TEST_FILE_SOURCE_SCHEMA.as_str().to_string()],
                source_patterns: file_source_patterns().to_vec(),
            }],
        };
        let group = writer;
        ObservationFixture {
            workspace_pk: workspace.workspace_id,
            root_pk: workspace_source_root.root_pk,
            db,
            group,
            roots,
            classifiers,
        }
    }

    #[test]
    fn startup_reconcile_uses_observation_fast_path_for_unchanged_file_sources() {
        let temp = tempfile::tempdir().unwrap();
        let workspace_root = temp.path().join("asset-observation-workspace");
        let source_root = workspace_root.join("assets");
        let source_path = "sources/fast.ron";
        let source_file = source_root.join(source_path);
        std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
        std::fs::write(&source_file, b"fast path source\n").unwrap();

        let ObservationFixture {
            db,
            group,
            workspace_pk,
            root_pk,
            roots,
            classifiers,
        } = observation_fixture(&workspace_root, &source_root);

        let first = reconcile_registered_source_assets(
            ReconcilePass {
                db: &db,
                writer: &group,
                changed_by_session: Some(TEST_SESSION_ID),
                classifiers: &classifiers,
                now_unix_ms: 1_000,
            },
            &roots,
        )
        .unwrap();
        assert_eq!(first.recorded, 1);
        assert_eq!(first.observed, 0);
        assert_eq!(first.planned_jobs, 1);
        let first_entry = db
            .source_asset(workspace_pk, root_pk, source_path)
            .unwrap()
            .unwrap()
            .1;
        assert_eq!(
            first_entry.schema.as_deref(),
            Some(TEST_FILE_SOURCE_SCHEMA.as_str())
        );
        assert_eq!(first_entry.observed, 1_000);

        let second = reconcile_registered_source_assets(
            ReconcilePass {
                db: &db,
                writer: &group,
                changed_by_session: Some(TEST_SESSION_ID),
                classifiers: &classifiers,
                now_unix_ms: 2_000,
            },
            &roots,
        )
        .unwrap();
        assert_eq!(second.recorded, 0);
        assert_eq!(second.observed, 1);
        assert_eq!(second.deleted, 0);
        assert_eq!(second.planned_jobs, 0);
        let second_entry = db
            .source_asset(workspace_pk, root_pk, source_path)
            .unwrap()
            .unwrap()
            .1;
        assert_eq!(
            second_entry.digest,
            Digest::from(blake3::hash(b"fast path source\n"))
        );
        drop(db);
        assert_eq!(second_entry.observed, 1_000);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_commits_valid_product_paths_from_worker_manifest() {
        let fixture = fixture();
        let project_data_paths = fixture.project_data_paths.clone();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);

        let completed = rpc
            .processor()
            .complete_attempt(&CompleteAssetJobAttemptRequest {
                capability: capability(),
                asset_job_attempt_id: lease.leased.attempt_id,
                lease_owner: "worker-a".to_string(),
                grant_key: lease.grant_key,
                status: AttemptStatus::Succeeded,
                finished_unix_ms: 400,
                error_count: 0,
                warning_count: 0,
                product_manifest: Some(product_manifest_handle_without_dependencies(
                    &staging_root,
                    "cache/textures/rpc.dds",
                    "textures/rpc.dds",
                )),
            })
            .unwrap();

        assert!(completed);
        let inspection = rpc
            .processor()
            .inspect_job(&InspectJobRequest {
                capability: editor_read_capability(),
                selector: InspectJobSelector::Attempt(lease.leased.attempt_id),
            })
            .unwrap();
        let products = inspection.inspection.unwrap().products;
        assert_eq!(products.len(), 1);
        assert_eq!(products[0].path, "cache/textures/rpc.dds");
        assert_eq!(products[0].product_format, "az.test.raw");
        assert_eq!(products[0].product_format_version, 1);
        assert!(products[0].edges.is_empty());
        assert_eq!(
            std::fs::read(
                project_data_paths
                    .product_cache_dir(DEFAULT_PLATFORM)
                    .unwrap()
                    .join("cache/textures/rpc.dds")
            )
            .unwrap(),
            b"rpc product bytes"
        );
        let published = rpc
            .processor()
            .publish_asset_catalog(&PublishAssetCatalogRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                platform: DEFAULT_PLATFORM.to_string(),
            })
            .wait()
            .unwrap();
        assert_eq!(published.entry_count, 1);
        let catalog_path = PathBuf::from(&published.catalog_path);
        assert_eq!(
            catalog_path,
            project_data_paths
                .product_cache_dir(DEFAULT_PLATFORM)
                .unwrap()
                .join(ASSET_CATALOG_FILE_NAME)
        );
        let catalog =
            az_asset::read_asset_catalog(std::fs::File::open(catalog_path).unwrap()).unwrap();
        assert_eq!(catalog.entries().len(), 1);
        let entry = &catalog.entries()[0];
        assert_eq!(entry.path.as_str(), "cache/textures/rpc.dds");
        assert_eq!(
            entry.asset_id,
            AssetId::new(Uuid::from_bytes([0x91; 16]), 4)
        );
        assert_eq!(entry.asset_type, Uuid::from_bytes([0xb3; 16]));
        assert!(entry.dependencies.is_empty());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn publish_asset_catalog_writes_empty_catalog_and_reports_processing_status() {
        let fixture = fixture();
        let job = install_fixture_build_job(&fixture, fixture.builder_guid, "default");
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_fixture_builder(),
        );
        assert!(job.job_id > 0);

        let status = processor
            .processing_status(&AssetProcessingStatusRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                platform: DEFAULT_PLATFORM.to_string(),
            })
            .unwrap();
        assert_eq!(status.queued, 1);
        assert_eq!(status.active(), 1);

        let published = processor
            .publish_asset_catalog(&PublishAssetCatalogRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                platform: DEFAULT_PLATFORM.to_string(),
            })
            .wait()
            .unwrap();
        assert_eq!(published.entry_count, 0);
        let catalog_path = PathBuf::from(&published.catalog_path);
        assert_eq!(
            catalog_path,
            fixture
                .project_data_paths
                .product_cache_dir(DEFAULT_PLATFORM)
                .unwrap()
                .join(ASSET_CATALOG_FILE_NAME)
        );
        assert!(catalog_path.is_file());
        let catalog =
            az_asset::read_asset_catalog(std::fs::File::open(&catalog_path).unwrap()).unwrap();
        assert!(catalog.entries().is_empty());

        // A reuse path must not even touch the durable catalog output. A
        // sentinel makes that contract observable without timestamp sleeps.
        std::fs::write(&catalog_path, b"reuse-sentinel").unwrap();
        let republished = processor
            .publish_asset_catalog(&PublishAssetCatalogRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                platform: DEFAULT_PLATFORM.to_string(),
            })
            .wait()
            .unwrap();
        assert_eq!(republished.catalog_path, published.catalog_path);
        assert_eq!(republished.entry_count, published.entry_count);
        assert!(republished.reused);
        assert!(!published.reused);
        assert_eq!(
            std::fs::read(&republished.catalog_path).unwrap(),
            b"reuse-sentinel"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_projects_generated_rust_graph_sources_from_catalog_products() {
        let fixture = fixture();
        let project_data_paths = fixture.project_data_paths.clone();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let generated_root = project_data_paths.graphs_dir();
        std::fs::create_dir_all(&generated_root).unwrap();
        let stale_source = generated_root.join("stale.rs");
        std::fs::write(&stale_source, b"stale generated graph source").unwrap();

        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);

        let completed = rpc
            .processor()
            .complete_attempt(&CompleteAssetJobAttemptRequest {
                capability: capability(),
                asset_job_attempt_id: lease.leased.attempt_id,
                lease_owner: "worker-a".to_string(),
                grant_key: lease.grant_key,
                status: AttemptStatus::Succeeded,
                finished_unix_ms: 400,
                error_count: 0,
                warning_count: 0,
                product_manifest: Some(
                    try_product_manifest_handle_with_declared_product_format(
                        &staging_root,
                        "graphs/generated/saved.azgraph.generated.rs",
                        "graphs/generated/saved.rs",
                        GENERATED_SOURCE_BYTES,
                        blake3::hash(GENERATED_SOURCE_BYTES).as_bytes().to_vec(),
                        GENERATED_SOURCE_BYTES.len() as u64,
                        az_graph_builder::GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID.as_str(),
                        az_graph_builder::GENERATED_RUST_GRAPH_SOURCE_PRODUCT_FORMAT_VERSION,
                    )
                    .unwrap(),
                ),
            })
            .unwrap();

        assert!(completed);
        assert!(
            !stale_source.exists(),
            "generated graph source projection should be fully reconciled from current DB products"
        );
        assert_eq!(
            std::fs::read(generated_root.join("graphs/generated/saved.azgraph.generated.rs"))
                .unwrap(),
            GENERATED_SOURCE_BYTES
        );
        assert_eq!(
            std::fs::read(
                project_data_paths
                    .product_cache_dir(DEFAULT_PLATFORM)
                    .unwrap()
                    .join("graphs/generated/saved.azgraph.generated.rs")
            )
            .unwrap(),
            GENERATED_SOURCE_BYTES
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_does_not_reproject_generated_rust_graph_sources_for_unrelated_products() {
        let fixture = fixture();
        let project_data_paths = fixture.project_data_paths.clone();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let generated_root = project_data_paths.graphs_dir();
        std::fs::create_dir_all(&generated_root).unwrap();
        let sentinel = generated_root.join("sentinel.rs");
        std::fs::write(&sentinel, SENTINEL_BYTES).unwrap();

        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);

        assert!(
            rpc.processor()
                .complete_attempt(&CompleteAssetJobAttemptRequest {
                    capability: capability(),
                    asset_job_attempt_id: lease.leased.attempt_id,
                    lease_owner: "worker-a".to_string(),
                    grant_key: lease.grant_key,
                    status: AttemptStatus::Succeeded,
                    finished_unix_ms: 400,
                    error_count: 0,
                    warning_count: 0,
                    product_manifest: Some(product_manifest_handle_with_product_path(
                        &staging_root,
                        "textures/unrelated.dds",
                        "textures/unrelated.dds",
                    )),
                })
                .unwrap()
        );
        assert_eq!(
            std::fs::read(sentinel).unwrap(),
            SENTINEL_BYTES,
            "an unrelated completion must not reconcile the generated graph projection"
        );
    }

    /// Completes one build job whose manifest declares the generated-Rust
    /// product format, and asserts the projection reached the project.
    ///
    /// This is the state a later completion has to tear down, so a test that
    /// checks removal needs it to have landed first.
    fn assert_generated_rust_projection_lands(
        rpc: &Rc<AssetProcessorRpc>,
        local: &tokio::task::LocalSet,
        runtime: &tokio::runtime::Runtime,
        first_staging_root: &Path,
        generated_root: &Path,
    ) {
        let first_lease = local
            .block_on(
                runtime,
                rpc.lease_job(&LeaseAssetJobRequest {
                    capability: capability(),
                    lease_owner: "worker-a".to_string(),
                    lease_duration_ms: 400,
                    staging_root: Some(first_staging_root.to_string_lossy().into_owned()),
                }),
            )
            .unwrap();

        assert!(
            rpc.processor()
                .complete_attempt(&CompleteAssetJobAttemptRequest {
                    capability: capability(),
                    asset_job_attempt_id: first_lease.leased.attempt_id,
                    lease_owner: "worker-a".to_string(),
                    grant_key: first_lease.grant_key,
                    status: AttemptStatus::Succeeded,
                    finished_unix_ms: 400,
                    error_count: 0,
                    warning_count: 0,
                    product_manifest: Some(
                        try_product_manifest_handle_with_declared_product_format(
                            first_staging_root,
                            "graphs/generated/saved.azgraph.generated.rs",
                            "graphs/generated/saved.rs",
                            GENERATED_SOURCE_BYTES,
                            blake3::hash(GENERATED_SOURCE_BYTES).as_bytes().to_vec(),
                            GENERATED_SOURCE_BYTES.len() as u64,
                            az_graph_builder::GENERATED_RUST_GRAPH_SOURCE_FORMAT_ID.as_str(),
                            az_graph_builder::GENERATED_RUST_GRAPH_SOURCE_PRODUCT_FORMAT_VERSION,
                        )
                        .unwrap(),
                    ),
                })
                .unwrap()
        );
        assert!(
            generated_root
                .join("graphs/generated/saved.azgraph.generated.rs")
                .is_file()
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_reprojects_generated_rust_graph_sources_when_format_is_removed() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let first_staging_root = temp.path().join("staging").join("worker-a");
        let second_staging_root = temp.path().join("staging").join("worker-b");
        let generated_root = fixture.project_data_paths.graphs_dir();
        let writer = fixture.writer.clone();
        let workspace_id = fixture.workspace.workspace_id;
        let asset_id = fixture.asset.asset_id;
        let builder = fixture.builder_guid;
        let first_job =
            install_fixture_build_job(&fixture, builder, "compile-graph-runtime-product");
        let rpc = Rc::new(AssetProcessorRpc::new(
            grant_backed_processor_with_builder_registry(
                fixture.db,
                registry_with_fixture_builder(),
            ),
        ));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        assert_generated_rust_projection_lands(
            &rpc,
            &local,
            &runtime,
            &first_staging_root,
            &generated_root,
        );

        writer
            .apply_plan_delta(ApplyPlanDelta {
                workspace_pk: workspace_id,
                delta: PlanDelta {
                    retire_job_ids: vec![first_job.job_id],
                    replacements: vec![PlannedJob::build(
                        asset_id,
                        builder,
                        "compile-graph-runtime-product",
                        DEFAULT_PLATFORM,
                        Vec::new(),
                    )],
                    ..PlanDelta::default()
                },
            })
            .wait_blocking()
            .unwrap();
        let second_lease = local
            .block_on(
                &runtime,
                rpc.lease_job(&LeaseAssetJobRequest {
                    capability: capability(),
                    lease_owner: "worker-b".to_string(),
                    lease_duration_ms: 400,
                    staging_root: Some(second_staging_root.to_string_lossy().into_owned()),
                }),
            )
            .unwrap();
        assert!(
            rpc.processor()
                .complete_attempt(&CompleteAssetJobAttemptRequest {
                    capability: capability(),
                    asset_job_attempt_id: second_lease.leased.attempt_id,
                    lease_owner: "worker-b".to_string(),
                    grant_key: second_lease.grant_key,
                    status: AttemptStatus::Succeeded,
                    finished_unix_ms: 800,
                    error_count: 0,
                    warning_count: 0,
                    product_manifest: Some(product_manifest_handle_with_product_path(
                        &second_staging_root,
                        "graphs/generated/replacement.bin",
                        "graphs/generated/replacement.bin",
                    )),
                })
                .unwrap()
        );
        assert!(
            !generated_root.exists(),
            "removing the generated-Rust format from a logical job must remove its stale projection"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_reads_catalog_products_for_attached_workspace() {
        let fixture = fixture();
        let asset_guid = fixture.asset.guid;
        let builder_guid = fixture.builder_guid;
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);
        assert!(
            rpc.processor()
                .complete_attempt(&CompleteAssetJobAttemptRequest {
                    capability: capability(),
                    asset_job_attempt_id: lease.leased.attempt_id,
                    lease_owner: "worker-a".to_string(),
                    grant_key: lease.grant_key,
                    status: AttemptStatus::Succeeded,
                    finished_unix_ms: 400,
                    error_count: 0,
                    warning_count: 0,
                    product_manifest: Some(product_manifest_handle_with_product_path(
                        &staging_root,
                        "cache/textures/rpc.dds",
                        "textures/rpc.dds",
                    )),
                })
                .unwrap()
        );

        let inspected_job_id = rpc
            .processor()
            .inspect_job(&InspectJobRequest {
                capability: editor_read_capability(),
                selector: InspectJobSelector::Attempt(lease.leased.attempt_id),
            })
            .unwrap()
            .inspection
            .unwrap()
            .job
            .job_id;
        let catalog = rpc
            .processor()
            .catalog_products(&CatalogProductsRequest {
                capability: editor_read_capability(),
                platform: "pc".to_string(),
            })
            .unwrap();

        assert_eq!(catalog.entries.len(), 1);
        let entry = &catalog.entries[0];
        assert_eq!(entry.job_id, inspected_job_id);
        assert_eq!(entry.asset_guid, asset_guid);
        assert_eq!(entry.source_path, "textures/rpc.png");
        assert_eq!(entry.builder_guid, builder_guid);
        assert_eq!(entry.job_key, "default");
        assert_eq!(entry.platform, "pc");
        assert_eq!(entry.product_path, "cache/textures/rpc.dds");
        assert_eq!(entry.asset_type, Uuid::from_bytes([0xb3; 16]));
        assert_eq!(entry.sub_id, 4);
        assert_eq!(entry.product_format, "az.test.raw");
        assert_eq!(entry.product_format_version, 1);
        assert_eq!(
            entry.byte_length,
            i64::try_from(b"rpc product bytes".len()).unwrap()
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_catalog_products_serve_product_dependencies() {
        let fixture = fixture();
        let source_guid = fixture.asset.guid;
        let product_type = Uuid::from_bytes([0xb3; 16]);
        let dangling_guid = Uuid::from_bytes([0xd0; 16]);
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-dep");
        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);
        let manifest = try_product_manifest_handle_with_dependencies(
            &staging_root,
            "textures/rpc.dds",
            "textures/rpc.dds",
            b"hash-dep",
            blake3::hash(b"hash-dep").as_bytes().to_vec(),
            b"hash-dep".len() as u64,
            "az.test.raw",
            1,
            vec![
                ProductManifestProductDependency {
                    asset_guid: source_guid,
                    sub_id: 4,
                    flags: 0,
                },
                ProductManifestProductDependency {
                    asset_guid: dangling_guid,
                    sub_id: 9,
                    flags: 1,
                },
            ],
        )
        .unwrap();
        assert!(
            rpc.processor()
                .complete_attempt(&CompleteAssetJobAttemptRequest {
                    capability: capability(),
                    asset_job_attempt_id: lease.leased.attempt_id,
                    lease_owner: "worker-a".to_owned(),
                    grant_key: lease.grant_key,
                    status: AttemptStatus::Succeeded,
                    finished_unix_ms: 200,
                    error_count: 0,
                    warning_count: 0,
                    product_manifest: Some(manifest),
                })
                .unwrap()
        );

        let catalog = rpc
            .processor()
            .catalog_products(&CatalogProductsRequest {
                capability: editor_read_capability(),
                platform: "pc".to_string(),
            })
            .unwrap();

        assert_eq!(catalog.entries.len(), 1);
        let entry = &catalog.entries[0];
        assert_eq!(entry.dependencies.len(), 2);
        // Deterministic order by target guid: 0x91 sorts before 0xd0.
        assert_eq!(entry.dependencies[0].asset_guid, source_guid);
        assert_eq!(entry.dependencies[0].sub_id, 4);
        assert_eq!(
            entry.dependencies[0].asset_type,
            Some(product_type),
            "target type resolves from the shipping product set"
        );
        assert_eq!(entry.dependencies[0].hint, None);
        assert_eq!(entry.dependencies[1].asset_guid, dangling_guid);
        assert_eq!(entry.dependencies[1].sub_id, 9);
        assert_eq!(
            entry.dependencies[1].asset_type, None,
            "an unshipped target has no resolved type"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_reads_release_content_for_attached_workspace() {
        let fixture = fixture();
        let project_data_paths = fixture.project_data_paths.clone();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);
        assert!(
            rpc.processor()
                .complete_attempt(&CompleteAssetJobAttemptRequest {
                    capability: capability(),
                    asset_job_attempt_id: lease.leased.attempt_id,
                    lease_owner: "worker-a".to_string(),
                    grant_key: lease.grant_key,
                    status: AttemptStatus::Succeeded,
                    finished_unix_ms: 400,
                    error_count: 0,
                    warning_count: 0,
                    product_manifest: Some(product_manifest_handle_with_product_path(
                        &staging_root,
                        "cache/textures/rpc.dds",
                        "textures/rpc.dds",
                    )),
                })
                .unwrap()
        );

        let product = rpc
            .processor()
            .release_content(&ReleaseContentReadRequest {
                capability: editor_read_capability(),
                platform: "pc".to_string(),
                target: ReleaseContentTarget::ProductAsset {
                    asset_guid: Uuid::from_bytes([0x91; 16]),
                    sub_id: 4,
                },
            })
            .unwrap();
        let ReleaseContentReadResult::Product(product) = product else {
            panic!("expected release product result");
        };
        assert_eq!(product.asset_guid, Uuid::from_bytes([0x91; 16]));
        assert_eq!(product.sub_id, 4);
        assert_eq!(product.product_path, "cache/textures/rpc.dds");
        assert_eq!(product.product_format, "az.test.raw");
        assert_eq!(product.product_format_version, 1);
        assert_eq!(product.byte_length, b"rpc product bytes".len() as u64);
        assert_eq!(
            product.content_hash,
            blake3::hash(b"rpc product bytes").as_bytes().to_vec()
        );
        assert_eq!(product.payload.kind, SideChannelKind::MmapFile);
        assert_eq!(product.payload.capability, Some(editor_read_capability()));
        assert_eq!(
            std::fs::read(PathBuf::from(&product.payload.locator)).unwrap(),
            b"rpc product bytes"
        );

        write_test_asset_catalog(&project_data_paths);
        let catalog = rpc
            .processor()
            .release_content(&ReleaseContentReadRequest {
                capability: editor_read_capability(),
                platform: "pc".to_string(),
                target: ReleaseContentTarget::AssetCatalog,
            })
            .unwrap();
        let ReleaseContentReadResult::AssetCatalog(handle) = catalog else {
            panic!("expected release asset catalog result");
        };
        assert_eq!(handle.kind, SideChannelKind::MmapFile);
        assert_eq!(handle.capability, Some(editor_read_capability()));
        let catalog_path = PathBuf::from(&handle.locator);
        assert_eq!(
            catalog_path.file_name().and_then(|name| name.to_str()),
            Some(RELEASE_ASSET_CATALOG_FILE_NAME)
        );
        let catalog =
            az_asset::read_asset_catalog(std::fs::File::open(catalog_path).unwrap()).unwrap();
        assert_eq!(catalog.entries().len(), 1);
        let entry = &catalog.entries()[0];
        assert_eq!(entry.asset_id.guid, Uuid::from_bytes([0x91; 16]));
        assert_eq!(entry.asset_id.sub_id, 4);
        assert_eq!(entry.asset_type, Uuid::from_bytes([0xb3; 16]));
        assert_eq!(entry.product_format, "az.test.raw");
        assert_eq!(entry.product_format_version, 1);
        assert_eq!(entry.path.as_str(), "cache/textures/rpc.dds");
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_verifies_staged_product_content_before_commit() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("worker-a");
        let (rpc, lease, _fixture_temp) = lease_fixture_build_job(fixture, &staging_root);

        let error = rpc
            .processor()
            .complete_attempt(&CompleteAssetJobAttemptRequest {
                capability: capability(),
                asset_job_attempt_id: lease.leased.attempt_id,
                lease_owner: "worker-a".to_string(),
                grant_key: lease.grant_key,
                status: AttemptStatus::Succeeded,
                finished_unix_ms: 400,
                error_count: 0,
                warning_count: 0,
                product_manifest: Some(product_manifest_handle_with_declared_product(
                    &staging_root,
                    "cache/textures/rpc.dds",
                    "textures/rpc.dds",
                    b"actual product bytes",
                    vec![0xc4; 32],
                    "actual product bytes".len() as u64,
                )),
            })
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::StagedProductHashMismatch { .. }
        ));
        let inspection = rpc
            .processor()
            .inspect_job(&InspectJobRequest {
                capability: editor_read_capability(),
                selector: InspectJobSelector::Attempt(lease.leased.attempt_id),
            })
            .unwrap();
        assert!(inspection.inspection.unwrap().products.is_empty());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_expired_capabilities() {
        let fixture = fixture();
        let processor = grant_backed_processor(fixture.db);
        let expired = editor_read_capability().with_expires_unix_ms(1);

        let error = processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: expired,
                root_scope: AssetRootScope::All,
            })
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidCapability { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_reports_registered_asset_builders() {
        let fixture = fixture();
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );

        let catalog = processor
            .builder_catalog(&AssetBuilderCatalogRequest {
                capability: editor_read_capability(),
            })
            .unwrap();

        assert_eq!(catalog.builders.len(), 1);
        assert_eq!(catalog.builders[0].name, "az.test.prefab");
        assert_eq!(
            catalog.builders[0].builder_guid,
            uuid!("00000000-0000-0000-0000-00000000b001")
        );
        assert_eq!(catalog.builders[0].version, 1);
        assert_eq!(catalog.builders[0].patterns.len(), 1);
        assert_eq!(
            catalog.builders[0].patterns[0].kind,
            AssetBuilderPatternKind::Wildcard
        );
        assert_eq!(catalog.builders[0].patterns[0].pattern, "*.prefab.ron");
        assert_eq!(
            catalog.builders[0].source_schema_types,
            vec!["az.test.Prefab".to_string()]
        );
        assert!(
            catalog.source_schemas.iter().any(|schema| {
                schema.schema_type == "az.test.Prefab"
                    && matches!(
                        schema.authoring,
                        SourceSchemaAuthoring::ProjectDocument { .. }
                    )
            }),
            "builder catalog should expose registered source authoring workflows"
        );
        let file_schema = catalog
            .source_schemas
            .iter()
            .find(|schema| schema.schema_type == "az.test.FileSource")
            .expect("test file source schema is registered");
        assert_eq!(file_schema.file_templates.len(), 1);
        assert_eq!(
            file_schema.file_templates[0].source_path,
            "sources/created.ron"
        );
        let import_schema = catalog
            .source_schemas
            .iter()
            .find(|schema| schema.schema_type == "az.test.ImportFileSource")
            .expect("test import file source schema is registered");
        assert!(
            import_schema.file_templates.is_empty(),
            "import-only source schemas must not expose default-create templates"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_reports_registered_graph_source_schemas() {
        let processor = AssetProcessor::with_composed_builders(
            AssetDb::open_in_memory().unwrap(),
            default_capability_grants(),
            test_registries(),
        );

        let catalog = processor
            .builder_catalog(&AssetBuilderCatalogRequest {
                capability: editor_read_capability(),
            })
            .unwrap();

        let graph_builder = catalog
            .builders
            .iter()
            .find(|builder| builder.name == "az.graph.compiler")
            .expect("graph compiler builder is registered");
        assert_eq!(graph_builder.builder_guid, GRAPH_COMPILER_BUILDER_ID.0);
        assert!(
            graph_builder
                .source_schema_types
                .contains(&"az.asset_processor.tests.logic-graph".to_string())
        );
        assert!(
            graph_builder
                .source_schema_types
                .contains(&"az.asset_processor.tests.generated-graph".to_string()),
            "registered generated-code graph compilers should be claimed by the generic graph compiler"
        );
        let graph_schema = catalog
            .source_schemas
            .iter()
            .find(|schema| schema.schema_type == "az.asset_processor.tests.logic-graph")
            .expect("graph source schema is projected from graph type catalog");
        assert_eq!(graph_schema.label, "Asset Processor Test Logic Graph");
        assert!(matches!(
            &graph_schema.authoring,
            SourceSchemaAuthoring::File { workflow }
                if workflow.source_root == PROJECT_SOURCE_ROOT
                    && workflow.default_path_prefix == "graphs"
                    && workflow.extensions == vec!["azgraph.ron".to_string()]
                    && !workflow.can_create
                    && workflow.can_edit
        ));
        let generated_graph_schema = catalog
            .source_schemas
            .iter()
            .find(|schema| schema.schema_type == "az.asset_processor.tests.generated-graph")
            .expect("generated graph source schema is still projected for editor workflows");
        assert_eq!(
            generated_graph_schema.label,
            "Asset Processor Test Generated Graph"
        );
        assert!(matches!(
            &generated_graph_schema.authoring,
            SourceSchemaAuthoring::File { workflow }
                if workflow.source_root == PROJECT_SOURCE_ROOT
                    && workflow.default_path_prefix == "graphs/generated"
                    && workflow.extensions == vec!["azgraph.ron".to_string()]
                    && !workflow.can_create
                    && workflow.can_edit
        ));
    }

    #[test]
    fn processor_domain_rejects_source_templates_for_non_creatable_source_workflows() {
        let import_schema = SourceSchemaRegistration::for_source::<TestImportFileSourceFormat>()
            .with_import_file("imports", &["mtl"]);
        let import_template = az_asset_builder::SourceFileTemplateRegistration::for_source::<
            TestImportFileSourceFormat,
        >(test_file_source_template);

        let import_error = ensure_source_file_template_registrations_match_schemas(
            &[attributed(import_schema)],
            &[attributed(import_template)],
        )
        .unwrap_err();
        assert!(matches!(
            import_error,
            AssetProcessorError::InvalidBuilderCatalog { reason }
                if reason.contains("not default-creatable")
        ));

        let project_document_schema =
            SourceSchemaRegistration::for_source::<TestPrefabSourceFormat>()
                .with_creatable_document_schema("az.test.Prefab");
        let project_document_template =
            az_asset_builder::SourceFileTemplateRegistration::for_source::<TestPrefabSourceFormat>(
                test_file_source_template,
            );

        let project_document_error = ensure_source_file_template_registrations_match_schemas(
            &[attributed(project_document_schema)],
            &[attributed(project_document_template)],
        )
        .unwrap_err();
        assert!(matches!(
            project_document_error,
            AssetProcessorError::InvalidBuilderCatalog { reason }
                if reason.contains("project-document source schema")
        ));
    }

    #[test]
    fn processor_domain_rejects_invalid_source_template_candidates() {
        let file_schema = SourceSchemaRegistration::for_source::<TestFileSourceFormat>()
            .with_creatable_file("sources", &["ron"]);

        let wrong_extension_template =
            az_asset_builder::SourceFileTemplateRegistration::for_source::<TestFileSourceFormat>(
                test_file_source_template,
            )
            .with_candidates(wrong_extension_source_template_candidates);
        let wrong_extension_error = source_schema_to_proto(
            &attributed(file_schema),
            &[attributed(wrong_extension_template)],
        )
        .unwrap_err();
        assert!(matches!(
            wrong_extension_error,
            AssetProcessorError::InvalidBuilderCatalog { reason }
                if reason.contains("outside registered extensions")
        ));

        let duplicate_template = az_asset_builder::SourceFileTemplateRegistration::for_source::<
            TestFileSourceFormat,
        >(test_file_source_template)
        .with_candidates(duplicate_source_template_candidates);
        let duplicate_error =
            source_schema_to_proto(&attributed(file_schema), &[attributed(duplicate_template)])
                .unwrap_err();
        assert!(matches!(
            duplicate_error,
            AssetProcessorError::InvalidBuilderCatalog { reason }
                if reason.contains("duplicate source file template path")
        ));

        let unsafe_template = az_asset_builder::SourceFileTemplateRegistration::for_source::<
            TestFileSourceFormat,
        >(test_file_source_template)
        .with_candidates(unsafe_source_template_candidates);
        let unsafe_error =
            source_schema_to_proto(&attributed(file_schema), &[attributed(unsafe_template)])
                .unwrap_err();
        assert!(matches!(
            unsafe_error,
            AssetProcessorError::InvalidBuilderCatalog { reason }
                if reason.contains("non-canonical source path")
        ));
    }

    #[test]
    fn source_file_create_extension_wildcard_accepts_any_canonical_path() {
        validate_source_file_create_extension("imports/raw.anything", &["*"]).unwrap();
        validate_source_file_create_extension("imports/raw", &["*"]).unwrap();
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_builder_catalog_missing_source_schema_workflow() {
        let mut registry = BuildRuleRegistry::new();
        registry.register(material_builder_desc());
        let processor = AssetProcessor::with_builder_registry(
            AssetDb::open_in_memory().unwrap(),
            registry,
            default_capability_grants(),
            test_registries(),
        );

        let error = processor
            .builder_catalog(&AssetBuilderCatalogRequest {
                capability: editor_read_capability(),
            })
            .unwrap_err();

        assert!(
            matches!(error, AssetProcessorError::InvalidBuilderCatalog { .. }),
            "{error}"
        );
        assert!(
            error
                .to_string()
                .contains("without a source schema registration"),
            "{error}"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_requires_brokered_grant_token_when_configured() {
        let fixture = fixture();
        let valid = editor_read_capability().with_token_hash([0x77, 0x88]);
        let grants = CapabilityGrantSet::from_grants(vec![valid.clone()]);
        let project_data_paths = test_project_data_paths(&fixture.db);
        let processor = AssetProcessor::new(
            fixture.db,
            fixture.workspace.workspace_id,
            project_data_paths,
            grants,
        )
        .unwrap();

        processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: valid.clone(),
                root_scope: AssetRootScope::All,
            })
            .unwrap();

        let mut synthetic = valid.clone();
        synthetic.token_hash.clear();
        assert!(matches!(
            processor.workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: synthetic,
                root_scope: AssetRootScope::All,
            }),
            Err(AssetProcessorError::InvalidCapability { .. })
        ));

        let mut wrong_token = valid;
        wrong_token.token_hash = vec![0x99];
        assert!(matches!(
            processor.workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: wrong_token,
                root_scope: AssetRootScope::All,
            }),
            Err(AssetProcessorError::InvalidCapability { .. })
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_empty_brokered_grant_set() {
        let fixture = fixture();
        let project_data_paths = test_project_data_paths(&fixture.db);
        let processor = AssetProcessor::new(
            fixture.db,
            fixture.workspace.workspace_id,
            project_data_paths,
            CapabilityGrantSet::new(),
        )
        .unwrap();

        let error = processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
            })
            .unwrap_err();

        assert!(
            matches!(error, AssetProcessorError::InvalidCapability { reason }
                if reason.contains("capability grant set is empty"))
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_accepts_project_scoped_read_and_write_capabilities() {
        let fixture = fixture();
        let workspace_id = fixture.workspace.workspace_id;
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let source_path = "prefabs/unscoped.prefab.ron";
        let payload = "project-scoped prefab";
        upsert_saved_authored_source(
            &fixture.db,
            workspace_id,
            "local.asset_processor_rpc",
            source_path,
            "az.test.Prefab",
            payload,
        );
        let processor = grant_backed_processor(fixture.db);

        let snapshot = processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: unscoped_editor_read_capability(),
                root_scope: AssetRootScope::All,
            })
            .unwrap();
        assert_eq!(snapshot.snapshot.unwrap().workspace_id, workspace_id);

        let recorded = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: unscoped_project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: source_path.to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash: blake3::hash(payload.as_bytes()).as_bytes().to_vec(),
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();
        assert_eq!(recorded.entry.workspace_id, workspace_id);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_editor_capability_for_source_recording() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let processor = grant_backed_processor(fixture.db);

        let error = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/editor-record.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash: blake3::hash(b"editor record").as_bytes().to_vec(),
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidCapability { reason }
                if reason.contains("expected role ProjectHost")
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_non_uuid_request_session_ids() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let processor = grant_backed_processor(fixture.db);
        let content_hash = blake3::hash(b"request session").as_bytes().to_vec();
        let invalid_sessions = ["not-a-session-uuid".to_string(), Uuid::nil().to_string()];

        for session_id in invalid_sessions {
            let record_error = processor
                .record_source_asset(&SourceAssetRecordRequest {
                    capability: project_host_write_capability(),
                    session_id: session_id.clone(),
                    workspace_root_id: workspace_source_root_id,
                    owner_id: "local.asset_processor_rpc".to_string(),
                    source_path: "prefabs/bad-session.prefab.ron".to_string(),
                    schema_type: Some("az.test.Prefab".to_string()),
                    content_hash: content_hash.clone(),
                    changed_unix_ms: 1_000,
                    diagnostics_count: 0,
                })
                .wait()
                .unwrap_err();
            assert!(matches!(
                record_error,
                AssetProcessorError::InvalidSourceAssetRecord { reason }
                    if reason.contains("session id")
                        && (reason.contains("UUID") || reason.contains("nil UUID"))
            ));
        }
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_lists_attached_workspace_entries_with_job_activity() {
        let fixture = fixture();
        let staging = tempfile::tempdir().unwrap();
        let (rpc, leased, _fixture_temp) = lease_fixture_build_job(fixture, staging.path());

        let result = rpc
            .processor()
            .workspace_entry_page(&WorkspaceEntryPageRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
                after_entry_id: None,
                page_size: 64,
            })
            .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].source_path, "textures/rpc.png");
        assert_eq!(result.entries[0].diff, WorkspaceEntryDiff::Clean);
        assert_eq!(result.entries[0].jobs.len(), 1);
        assert_eq!(
            result.entries[0].jobs[0]
                .attempt
                .as_ref()
                .unwrap()
                .attempt_id,
            leased.leased.attempt_id
        );
        assert_eq!(result.next_after_entry_id, None);
    }

    #[test]
    fn workspace_entry_projection_uses_the_current_locator_for_job_activity() {
        let fixture = fixture();
        let job = install_fixture_build_job(&fixture, fixture.builder_guid, "default");
        let mut entry = fixture
            .db
            .entry_by_asset(fixture.workspace.workspace_id, fixture.asset.asset_id)
            .unwrap()
            .unwrap();
        entry.path = "textures/renamed-rpc.png".to_string();

        let entry = workspace_asset_entry_to_proto(&fixture.db, &fixture.asset, entry).unwrap();
        drop(fixture);

        assert_eq!(entry.source_path, "textures/renamed-rpc.png");
        assert_eq!(entry.jobs.len(), 1);
        assert_eq!(entry.jobs[0].job.job_id, job.job_id);
        assert_eq!(entry.jobs[0].job.source_path, "textures/renamed-rpc.png");
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn failed_build_job_is_replanned_for_an_unchanged_source() {
        let fixture = fixture();
        let workspace_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/retry-failed.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );
        let request = SourceAssetRecordRequest {
            capability: project_host_write_capability(),
            session_id: TEST_SESSION_ID.to_string(),
            workspace_root_id,
            owner_id: "local.asset_processor_rpc".to_string(),
            source_path: "prefabs/retry-failed.prefab.ron".to_string(),
            schema_type: Some("az.test.Prefab".to_string()),
            content_hash: blake3::hash(b"saved prefab").as_bytes().to_vec(),
            changed_unix_ms: 1_000,
            diagnostics_count: 0,
        };
        let first = processor.record_source_asset(&request).wait().unwrap();
        let first_job_id = build_job(&first.entry).job.job_id;
        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let staging = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let lease = local
            .block_on(
                &runtime,
                rpc.lease_job(&LeaseAssetJobRequest {
                    capability: capability(),
                    lease_owner: "worker-a".to_string(),
                    lease_duration_ms: 30_000,
                    staging_root: Some(path_string(staging.path())),
                }),
            )
            .unwrap();
        assert_eq!(lease.leased.job_key, "compile-prefab");
        assert!(
            rpc.processor()
                .complete_attempt(&CompleteAssetJobAttemptRequest {
                    capability: capability(),
                    asset_job_attempt_id: lease.leased.attempt_id,
                    lease_owner: "worker-a".to_string(),
                    grant_key: lease.grant_key,
                    status: AttemptStatus::Failed,
                    finished_unix_ms: 1_100,
                    error_count: 1,
                    warning_count: 0,
                    product_manifest: None,
                })
                .unwrap()
        );

        let retried = rpc
            .processor()
            .record_source_asset(&SourceAssetRecordRequest {
                changed_unix_ms: 1_200,
                ..request
            })
            .wait()
            .unwrap();
        let retry = build_job(&retried.entry);
        assert_eq!(retry.job.job_id, first_job_id);
        assert_eq!(retry.job.status, JobStatus::Queued);
        assert_eq!(retry.job.attempts, 0);
        assert!(retry.attempt.is_none());
    }
    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_zero_workspace_entry_page_size() {
        let fixture = fixture();
        let processor = grant_backed_processor(fixture.db);

        let error = processor
            .workspace_entry_page(&WorkspaceEntryPageRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
                after_entry_id: None,
                page_size: 0,
            })
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidWorkspaceEntryPageRequest { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_non_positive_workspace_entry_cursor() {
        let fixture = fixture();
        let processor = grant_backed_processor(fixture.db);

        let error = processor
            .workspace_entry_page(&WorkspaceEntryPageRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
                after_entry_id: Some(0),
                page_size: 64,
            })
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidWorkspaceEntryPageRequest { .. }
        ));

        let error = processor
            .workspace_entry_page(&WorkspaceEntryPageRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
                after_entry_id: Some(-1),
                page_size: 64,
            })
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidWorkspaceEntryPageRequest { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_projects_the_attached_workspace_snapshot() {
        let fixture = fixture();
        let processor = grant_backed_processor(fixture.db);

        let result = processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
            })
            .unwrap();

        let snapshot = result.snapshot.unwrap();
        assert_eq!(snapshot.workspace_id, fixture.workspace.workspace_id);
        assert_eq!(snapshot.project_id, "local.asset_processor_rpc");
        assert_eq!(
            snapshot.workspace_root,
            path_string(&fixture.workspace_root)
        );
        assert_eq!(snapshot.branch, "az/session/rpc");
        assert_eq!(snapshot.roots.len(), 1);
        assert_eq!(snapshot.roots[0].owner_id, "local.asset_processor_rpc");
        assert_eq!(
            snapshot.roots[0].source_root,
            path_string(&fixture.source_root)
        );
        assert_eq!(
            snapshot.roots[0].portable_key,
            "project:local.asset_processor_rpc:assets"
        );
    }

    /// A workspace with a project assets root and a gem assets root, each
    /// holding one recorded source.
    ///
    /// Two roots of different kinds is what makes a browser-scope query
    /// meaningful: the scope has to include both and keep them in root order.
    fn browser_scope_workspace() -> AssetDb {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let project_id = "local.asset_browser_scope";
        let workspace_root = "/wt/browser-scope";
        let workspace = writer
            .register_workspace(RegisterWorkspace {
                key: WorkspaceKey {
                    project: project_id.to_string(),
                    root: workspace_root.to_string(),
                    branch: "az/session/assets".to_string(),
                },
                now: 20,
            })
            .wait_blocking()
            .unwrap();
        let (project_root, project_policy) = writer
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: workspace.workspace_id,
                key: "project:local.asset_browser_scope:assets".to_string(),
                owner: project_id.to_string(),
                path: "/wt/browser-scope/assets".to_string(),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .unwrap();
        let (gem_root, gem_policy) = writer
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: workspace.workspace_id,
                key: "gem:azoth.physics:assets".to_string(),
                owner: "azoth.physics".to_string(),
                path: "/wt/browser-scope/gems/physics/assets".to_string(),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .unwrap();

        for (guid, policy, source_path, hash_byte) in [
            (
                Uuid::from_bytes([0x32; 16]),
                &project_policy,
                "gamedata/items.ron",
                "32",
            ),
            (
                Uuid::from_bytes([0x33; 16]),
                &gem_policy,
                "prefabs/rigid_body.prefab.ron",
                "33",
            ),
        ] {
            let content_hash = hash_byte.repeat(32).parse::<Digest>().unwrap();
            writer
                .apply_sweep_delta(ApplySweepDelta {
                    workspace_pk: workspace.workspace_id,
                    workspace_root_pk: policy.workspace_root_id,
                    records: vec![SweepRecord {
                        source: SweepEntry {
                            path: source_path.to_string(),
                            guid,
                            schema: Some("az.test.Source".to_string()),
                            digest: content_hash,
                            diff: DbDiff::Clean,
                            diagnostics: 0,
                            updated: 30,
                            src_bytes: 0,
                            src_mtime: 0,
                            meta_bytes: 0,
                            meta_mtime: 0,
                            observed: 30,
                            session: None,
                        },
                        planner: SweepPlannerJob {
                            key: ASSET_PLANNER_JOB_KEY.to_string(),
                            platform: DEFAULT_PLATFORM.to_string(),
                        },
                    }],
                    removals: Vec::new(),
                })
                .wait_blocking()
                .unwrap();
        }
        assert_ne!(project_root.root_id, gem_root.root_id);
        db
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_browser_scope_includes_project_and_gem_asset_roots() {
        let db = browser_scope_workspace();

        let processor = grant_backed_processor(db);
        let browser_snapshot = processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::BrowserAssets,
            })
            .unwrap()
            .snapshot
            .unwrap();
        let browser_keys = browser_snapshot
            .roots
            .iter()
            .map(|root| root.portable_key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            browser_keys,
            vec![
                "project:local.asset_browser_scope:assets",
                "gem:azoth.physics:assets"
            ]
        );

        let browser_status = processor
            .workspace_entry_page(&WorkspaceEntryPageRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::BrowserAssets,
                after_entry_id: None,
                page_size: 64,
            })
            .unwrap();
        let browser_paths = browser_status
            .entries
            .iter()
            .map(|entry| entry.source_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            browser_paths,
            vec!["gamedata/items.ron", "prefabs/rigid_body.prefab.ron"]
        );

        let full_snapshot = processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
            })
            .unwrap()
            .snapshot
            .unwrap();
        assert_eq!(full_snapshot.roots.len(), 2);

        let full_status = processor
            .workspace_entry_page(&WorkspaceEntryPageRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
                after_entry_id: None,
                page_size: 64,
            })
            .unwrap();
        assert_eq!(full_status.entries.len(), 2);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_session_scoped_reads_for_other_workspace() {
        let fixture = fixture();
        let processor = grant_backed_processor(fixture.db);
        let scoped_capability = scoped_editor_read_capability(Uuid::from_bytes([0x44; 16]));

        let snapshot_error = processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: scoped_capability.clone(),
                root_scope: AssetRootScope::All,
            })
            .unwrap_err();
        assert!(matches!(
            snapshot_error,
            AssetProcessorError::InvalidCapability { .. }
        ));

        let entries_error = processor
            .workspace_entry_page(&WorkspaceEntryPageRequest {
                capability: scoped_capability.clone(),
                root_scope: AssetRootScope::All,
                after_entry_id: None,
                page_size: 64,
            })
            .unwrap_err();
        assert!(matches!(
            entries_error,
            AssetProcessorError::InvalidCapability { .. }
        ));

        let inspect_error = processor
            .inspect_job(&InspectJobRequest {
                capability: scoped_capability.clone(),
                selector: InspectJobSelector::Attempt(1),
            })
            .unwrap_err();
        assert!(matches!(
            inspect_error,
            AssetProcessorError::InvalidCapability { .. }
        ));

        let products_error = processor
            .catalog_products(&CatalogProductsRequest {
                capability: scoped_capability,
                platform: DEFAULT_PLATFORM.to_owned(),
            })
            .unwrap_err();
        assert!(matches!(
            products_error,
            AssetProcessorError::InvalidCapability { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_session_scoped_write_for_other_session() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let processor = grant_backed_processor(fixture.db);
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let error = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: scoped_project_host_write_capability(Uuid::from_bytes([0x55; 16])),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/scoped.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidCapability { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_session_scoped_jobs_for_other_workspace() {
        let fixture = fixture();
        let project_data_paths = test_project_data_paths(&fixture.db);
        let processor = AssetProcessor::new(
            fixture.db,
            fixture.workspace.workspace_id,
            project_data_paths,
            CapabilityGrantSet::from_grants(vec![
                capability(),
                scoped_worker_capability(other_session_uuid()),
            ]),
        )
        .unwrap();
        let scoped_capability = scoped_worker_capability(other_session_uuid());
        let staging = tempfile::tempdir().unwrap();

        let lease_error = processor
            .validate_lease_admission(&LeaseAssetJobRequest {
                capability: scoped_capability.clone(),
                lease_owner: "worker-a".to_string(),
                lease_duration_ms: 400,
                staging_root: Some(path_string(staging.path())),
            })
            .unwrap_err();
        assert!(matches!(
            lease_error,
            AssetProcessorError::InvalidCapability { .. }
        ));

        let renew_error = processor
            .validate_renewal_admission(&RenewAssetJobLeaseRequest {
                capability: scoped_capability.clone(),
                asset_job_attempt_id: 1,
                lease_owner: "worker-a".to_string(),
                grant_key: Uuid::from_bytes([0x81; 16]),
            })
            .unwrap_err();
        assert!(matches!(
            renew_error,
            AssetProcessorError::InvalidCapability { .. }
        ));

        let complete_error = processor
            .validate_completion_admission(&CompleteAssetJobAttemptRequest {
                capability: scoped_capability,
                asset_job_attempt_id: 1,
                lease_owner: "worker-a".to_string(),
                grant_key: Uuid::from_bytes([0x82; 16]),
                status: AttemptStatus::Failed,
                finished_unix_ms: 300,
                error_count: 1,
                warning_count: 0,
                product_manifest: None,
            })
            .unwrap_err();
        assert!(matches!(
            complete_error,
            AssetProcessorError::InvalidCapability { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_creates_file_source_from_registered_template() {
        let fixture = fixture();
        let source_root = fixture.source_root.clone();
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_file_source_builder(),
        );

        let result = processor
            .create_source_file(&SourceFileCreateRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "sources/created.ron".to_string(),
                schema_type: "az.test.FileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::DefaultTemplate,
            })
            .wait()
            .unwrap();

        assert_eq!(result.record.entry.source_path, "sources/created.ron");
        assert_eq!(
            result.record.entry.schema_type.as_deref(),
            Some("az.test.FileSource")
        );
        assert_eq!(
            result.record.entry.content_hash,
            blake3::hash(b"created source\n").to_hex().to_string()
        );
        assert_eq!(
            build_job(&result.record.entry).job.owner,
            JobOwner::Build(uuid!("00000000-0000-0000-0000-00000000b009"))
        );
        assert!(
            !source_root.join("sources").join("created.ron").exists(),
            "source file creation stores the saved payload in the asset DB, not a fallback file"
        );

        let db = processor.db();
        let saved_payload = db
            .payload_for_source(
                processor.attached_workspace_id().unwrap(),
                result.record.entry.root_id,
                "sources/created.ron",
            )
            .unwrap()
            .expect("saved source payload checkpoint");
        assert_eq!(saved_payload.schema, "az.test.FileSource");
        drop(db);
        drop(processor);
        assert_eq!(
            saved_payload.checkpoint.as_deref(),
            Some(b"created source\n".as_slice())
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_moves_db_owned_source_file() {
        let fixture = fixture();
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_file_source_builder(),
        );
        let created = processor
            .create_source_file(&SourceFileCreateRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "sources/created.ron".to_string(),
                schema_type: "az.test.FileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::DefaultTemplate,
            })
            .wait()
            .unwrap();

        let moved = processor
            .move_source_file(&SourceFileMoveRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                from_source_path: "sources/created.ron".to_string(),
                to_source_path: "sources/new-name.ron".to_string(),
                changed_unix_ms: 2_000,
            })
            .wait()
            .unwrap();

        assert_eq!(moved.old_source_path, "sources/created.ron");
        assert_eq!(moved.record.asset_guid, created.record.asset_guid);
        assert_eq!(moved.record.entry.source_path, "sources/new-name.ron");
        assert!(
            processor
                .db()
                .payload_for_source(
                    processor.attached_workspace_id().unwrap(),
                    created.record.entry.root_id,
                    "sources/created.ron",
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(
            processor
                .db()
                .payload_for_source(
                    processor.attached_workspace_id().unwrap(),
                    moved.record.entry.root_id,
                    "sources/new-name.ron",
                )
                .unwrap()
                .unwrap()
                .checkpoint
                .as_deref(),
            Some(b"created source\n".as_slice())
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_deletes_db_owned_source_file() {
        let fixture = fixture();
        let root_pk = fixture.workspace_source_root.root_pk;
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_file_source_builder(),
        );
        processor
            .create_source_file(&SourceFileCreateRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "sources/created.ron".to_string(),
                schema_type: "az.test.FileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::DefaultTemplate,
            })
            .wait()
            .unwrap();

        let deleted = processor
            .delete_source_file(&SourceFileDeleteRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "sources/created.ron".to_string(),
                changed_unix_ms: 2_000,
            })
            .wait()
            .unwrap();

        assert_eq!(deleted.record.entry.source_path, "sources/created.ron");
        assert_eq!(deleted.record.entry.diff, WorkspaceEntryDiff::Deleted);
        assert!(
            processor
                .db()
                .payload_for_source(
                    processor.attached_workspace_id().unwrap(),
                    root_pk,
                    "sources/created.ron",
                )
                .unwrap()
                .is_none()
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_delete_cleans_physical_source_tombstone_outside_watched_root() {
        let fixture = fixture();
        let source_file = fixture.source_root.join("textures/rpc.png");
        let mutation_staging_root = fixture.workspace_root.join(".azoth-source-mutations");
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_file_source_builder(),
        );

        let deleted = processor
            .delete_source_file(&SourceFileDeleteRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "textures/rpc.png".to_string(),
                changed_unix_ms: 2_000,
            })
            .wait()
            .unwrap();

        assert_eq!(deleted.record.entry.diff, WorkspaceEntryDiff::Deleted);
        assert!(!source_file.exists());
        assert!(mutation_staging_root.is_dir());
        assert_eq!(fs::read_dir(mutation_staging_root).unwrap().count(), 0);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_delete_when_db_owned_source_has_unsaved_edits() {
        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let project = fixture.workspace.project.clone();
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_file_source_builder(),
        );
        let created = processor
            .create_source_file(&SourceFileCreateRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "sources/created.ron".to_string(),
                schema_type: "az.test.FileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::DefaultTemplate,
            })
            .wait()
            .unwrap();
        let result = processor
            .asset_db_writer
            .write_source_payload(WriteSourcePayload {
                workspace_pk,
                root_pk: created.record.entry.root_id,
                path: "sources/created.ron".to_string(),
                document: "sources/created.ron".to_string(),
                schema: "az.test.FileSource".to_string(),
                encoding: Encoding::Bytes,
                expected_revision: Some(1),
                revision: 2,
                saved: Some(1),
                digest: Digest::from(blake3::hash(b"dirty source\n")),
                payload: b"dirty source\n".to_vec(),
                checkpoint: CheckpointWrite::Preserve,
                session: Some(TEST_SESSION_ID.to_string()),
                project,
                now: 1_500,
            })
            .wait()
            .unwrap();
        assert!(matches!(result, WriteSourcePayloadResult::Written(_)));

        let error = processor
            .delete_source_file(&SourceFileDeleteRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "sources/created.ron".to_string(),
                changed_unix_ms: 2_000,
            })
            .wait()
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::SourceFileHasUnsavedEdits {
                operation: "delete",
                ..
            }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_creates_file_source_in_gem_source_root() {
        let fixture = fixture();
        let gem_root = fixture
            .workspace_root
            .join("gems")
            .join("asset-processor")
            .join("assets");
        std::fs::create_dir_all(&gem_root).unwrap();
        let (gem_root_identity, gem_workspace_source_root) = fixture
            .writer
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: fixture.workspace.workspace_id,
                key: TEST_GEM_SOURCE_ROOT.to_string(),
                owner: "az.test.asset-processor".to_string(),
                path: path_string(&gem_root),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .unwrap();
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_file_source_builder(),
        );

        let result = processor
            .create_source_file(&SourceFileCreateRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: TEST_GEM_SOURCE_ROOT.to_string(),
                source_path: "sources/created.ron".to_string(),
                schema_type: "az.test.GemFileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::DefaultTemplate,
            })
            .wait()
            .unwrap();

        assert_eq!(result.record.entry.root_id, gem_root_identity.root_id);
        assert_eq!(
            result.record.entry.schema_type.as_deref(),
            Some("az.test.GemFileSource")
        );
        let roots = processor
            .db()
            .workspace_roots(result.record.entry.workspace_id)
            .unwrap();
        let root = roots
            .iter()
            .find(|root| root.workspace_root_id == gem_workspace_source_root.workspace_root_id)
            .expect("created gem source root");
        assert_eq!(root.root_pk, gem_root_identity.root_id);
        let saved_payload = processor
            .db()
            .payload_for_source(
                processor.attached_workspace_id().unwrap(),
                result.record.entry.root_id,
                "sources/created.ron",
            )
            .unwrap()
            .expect("gem saved source payload checkpoint");
        assert_eq!(saved_payload.schema, "az.test.GemFileSource");
        drop(processor);
        assert_eq!(
            saved_payload.checkpoint.as_deref(),
            Some(b"created source\n".as_slice())
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_creates_file_source_from_payload_side_channel() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let payload_path = temp.path().join("manual.ron");
        std::fs::write(&payload_path, b"manual source\n").unwrap();
        let capability = project_host_write_capability();
        let payload = SideChannelHandle::staging_file(
            payload_path.to_string_lossy(),
            b"manual source\n".len() as u64,
            blake3::hash(b"manual source\n").as_bytes().to_vec(),
            std::env::consts::OS,
        )
        .with_capability(capability.clone());
        let processor = grant_backed_processor(fixture.db);

        let result = processor
            .create_source_file(&SourceFileCreateRequest {
                capability,
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "sources/manual.ron".to_string(),
                schema_type: "az.test.FileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::Payload(Box::new(payload)),
            })
            .wait()
            .unwrap();

        assert_eq!(result.record.entry.source_path, "sources/manual.ron");
        assert_eq!(
            result.record.entry.content_hash,
            blake3::hash(b"manual source\n").to_hex().to_string()
        );
        let db = processor.db();
        let saved_payload = db
            .payload_for_source(
                processor.attached_workspace_id().unwrap(),
                result.record.entry.root_id,
                "sources/manual.ron",
            )
            .unwrap()
            .expect("saved source payload checkpoint");
        assert_eq!(
            saved_payload.checkpoint.as_deref(),
            Some(b"manual source\n".as_slice())
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_creates_payload_source_from_published_worker_schema() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let payload_path = temp.path().join("remote.settings.ron");
        std::fs::write(&payload_path, b"published worker source\n").unwrap();
        let capability = project_host_write_capability();
        let payload = SideChannelHandle::staging_file(
            payload_path.to_string_lossy(),
            b"published worker source\n".len() as u64,
            blake3::hash(b"published worker source\n")
                .as_bytes()
                .to_vec(),
            std::env::consts::OS,
        )
        .with_capability(capability.clone());
        let processor = AssetProcessor::with_builder_registry_and_catalog(
            fixture.db,
            BuildRuleRegistry::new(),
            default_capability_grants(),
            test_registries(),
            Some(fixture.workspace.workspace_id),
            None,
            Some(AssetBuilderCatalogResult {
                builders: Vec::new(),
                source_schemas: vec![SourceSchemaDescriptor {
                    schema_type: "az.test.WorkerDeploymentProfile".to_string(),
                    owner: "az-test-worker".to_string(),
                    label: "Deployment Profile".to_string(),
                    category: "Deployment".to_string(),
                    authoring: SourceSchemaAuthoring::File {
                        workflow: SourceFileWorkflowDescriptor {
                            source_root: PROJECT_SOURCE_ROOT.to_string(),
                            default_path_prefix: "deployments".to_string(),
                            extensions: vec!["settings.ron".to_string()],
                            can_create: true,
                            can_edit: true,
                        },
                    },
                    file_templates: Vec::new(),
                }],
                product_formats: Vec::new(),
            }),
        );

        let result = processor
            .create_source_file(&SourceFileCreateRequest {
                capability,
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "deployments/remote.settings.ron".to_string(),
                schema_type: "az.test.WorkerDeploymentProfile".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::Payload(Box::new(payload)),
            })
            .wait()
            .unwrap();

        assert_eq!(
            result.record.entry.schema_type.as_deref(),
            Some("az.test.WorkerDeploymentProfile")
        );
        let planner = result
            .record
            .entry
            .jobs
            .iter()
            .find(|activity| activity.job.owner == JobOwner::Plan)
            .expect("published worker schema source planner job");
        assert_eq!(planner.job.key, ASSET_PLANNER_JOB_KEY);
        assert_eq!(planner.job.status, JobStatus::Queued);
        assert!(planner.attempt.is_none());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_imports_file_source_from_payload_side_channel() {
        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let payload_path = temp.path().join("material.mtl");
        std::fs::write(&payload_path, b"legacy material\n").unwrap();
        let capability = project_host_write_capability();
        let payload = SideChannelHandle::staging_file(
            payload_path.to_string_lossy(),
            b"legacy material\n".len() as u64,
            blake3::hash(b"legacy material\n").as_bytes().to_vec(),
            std::env::consts::OS,
        )
        .with_capability(capability.clone());
        let processor = grant_backed_processor(fixture.db);

        let result = processor
            .create_source_file(&SourceFileCreateRequest {
                capability,
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "imports/material.mtl".to_string(),
                schema_type: "az.test.ImportFileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::Payload(Box::new(payload)),
            })
            .wait()
            .unwrap();

        assert_eq!(result.record.entry.source_path, "imports/material.mtl");
        assert_eq!(
            result.record.entry.schema_type.as_deref(),
            Some("az.test.ImportFileSource")
        );
        assert_eq!(
            result.record.entry.content_hash,
            blake3::hash(b"legacy material\n").to_hex().to_string()
        );
        let saved_payload = processor
            .db()
            .payload_for_source(
                processor.attached_workspace_id().unwrap(),
                result.record.entry.root_id,
                "imports/material.mtl",
            )
            .unwrap()
            .expect("imported source payload checkpoint");
        assert_eq!(
            saved_payload.checkpoint.as_deref(),
            Some(b"legacy material\n".as_slice())
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_imports_binary_file_source_from_payload_side_channel() {
        const SOURCE_BYTES: &[u8] = b"\0\xffbinary material\n";

        let fixture = fixture();
        let temp = tempfile::tempdir().unwrap();
        let payload_path = temp.path().join("material.mtl");
        std::fs::write(&payload_path, SOURCE_BYTES).unwrap();
        let capability = project_host_write_capability();
        let payload = SideChannelHandle::staging_file(
            payload_path.to_string_lossy(),
            SOURCE_BYTES.len() as u64,
            blake3::hash(SOURCE_BYTES).as_bytes().to_vec(),
            std::env::consts::OS,
        )
        .with_capability(capability.clone());
        let processor = grant_backed_processor(fixture.db);

        let result = processor
            .create_source_file(&SourceFileCreateRequest {
                capability,
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "imports/binary-material.mtl".to_string(),
                schema_type: "az.test.ImportFileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::Payload(Box::new(payload)),
            })
            .wait()
            .unwrap();

        assert_eq!(
            result.record.entry.content_hash,
            blake3::hash(SOURCE_BYTES).to_hex().to_string()
        );
        let saved_payload = processor
            .db()
            .payload_for_source(
                processor.attached_workspace_id().unwrap(),
                result.record.entry.root_id,
                "imports/binary-material.mtl",
            )
            .unwrap()
            .expect("binary imported source payload checkpoint");
        assert_eq!(saved_payload.checkpoint.as_deref(), Some(SOURCE_BYTES));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_import_only_file_source_default_template() {
        let fixture = fixture();
        let root_pk = fixture.workspace_source_root.root_pk;
        let processor = grant_backed_processor(fixture.db);

        let error = processor
            .create_source_file(&SourceFileCreateRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "imports/material.mtl".to_string(),
                schema_type: "az.test.ImportFileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::DefaultTemplate,
            })
            .wait()
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::SourceFileCreateSchemaNotCreatable { .. }
        ));
        assert!(
            processor
                .db()
                .payload_for_source(
                    processor.attached_workspace_id().unwrap(),
                    root_pk,
                    "imports/material.mtl",
                )
                .unwrap()
                .is_none(),
            "failed default-template creation must not mutate source payload state"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_file_source_create_without_template_match() {
        let fixture = fixture();
        let processor = grant_backed_processor(fixture.db);

        let error = processor
            .create_source_file(&SourceFileCreateRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "sources/unknown.ron".to_string(),
                schema_type: "az.test.FileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::DefaultTemplate,
            })
            .wait()
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::SourceFileCreateTemplateUnavailable { .. }
        ));
        let entries = processor
            .db()
            .workspace_entry_page(fixture.workspace.workspace_id, None, 0, 64)
            .unwrap();
        assert!(
            entries
                .iter()
                .all(|entry| entry.source_path != "sources/unknown.ron"),
            "failed source creation must not mutate the workspace asset registry"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_file_source_create_schema_and_extension_mismatch() {
        let fixture = fixture();
        let processor = grant_backed_processor(fixture.db);

        let document_schema_error = processor
            .create_source_file(&SourceFileCreateRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "prefabs/created.prefab.ron".to_string(),
                schema_type: "az.test.Prefab".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::DefaultTemplate,
            })
            .wait()
            .unwrap_err();
        assert!(matches!(
            document_schema_error,
            AssetProcessorError::SourceFileCreateSchemaNotFileBacked { .. }
        ));

        let extension_error = processor
            .create_source_file(&SourceFileCreateRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "sources/created.txt".to_string(),
                schema_type: "az.test.FileSource".to_string(),
                changed_unix_ms: 1_000,
                content: SourceFileCreateContent::DefaultTemplate,
            })
            .wait()
            .unwrap_err();
        assert!(matches!(
            extension_error,
            AssetProcessorError::SourceFileCreateExtensionMismatch { .. }
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_records_saved_source_asset() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/saved.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor(fixture.db);
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/saved.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash: content_hash.clone(),
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();

        assert_eq!(result.entry.source_path, "prefabs/saved.prefab.ron");
        assert_eq!(result.entry.diff, WorkspaceEntryDiff::Added);
        assert_eq!(result.entry.content_hash, hex_lower(&content_hash));
        assert_eq!(result.entry.asset_guid, result.asset_guid);
        let snapshot = processor
            .workspace_snapshot(&WorkspaceSnapshotRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
            })
            .unwrap()
            .snapshot
            .unwrap();
        assert_eq!(snapshot.roots.len(), 1);
        assert_eq!(snapshot.roots[0].owner_id, "local.asset_processor_rpc");
        assert_eq!(
            snapshot.roots[0].source_root,
            path_string(&fixture.source_root)
        );
        assert_eq!(
            snapshot.roots[0].portable_key,
            "project:local.asset_processor_rpc:assets"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_requires_saved_payload_before_recording_source_asset() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let processor = grant_backed_processor(fixture.db);
        let source_path = "prefabs/missing-payload.prefab.ron";

        let error = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: source_path.to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash: blake3::hash(b"saved prefab").as_bytes().to_vec(),
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::AuthoredAssetMissingSavedPayload {
                workspace_id,
                source_path: actual_source_path,
            } if workspace_id == fixture.workspace.workspace_id
                && actual_source_path == source_path
        ));
        let entries = processor
            .db()
            .workspace_entry_page(fixture.workspace.workspace_id, None, 0, 64)
            .unwrap();
        assert!(
            entries.iter().all(|entry| entry.source_path != source_path),
            "recordSourceAsset must not mutate the registry before the DB saved payload is present"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_recorded_source_hash_mismatch_with_saved_payload() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let source_path = "prefabs/hash-mismatch.prefab.ron";
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            source_path,
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor(fixture.db);

        let error = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: source_path.to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash: blake3::hash(b"different payload").as_bytes().to_vec(),
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::AuthoredAssetRecordPayloadHashMismatch {
                workspace_id,
                source_path: actual_source_path,
                ..
            } if workspace_id == fixture.workspace.workspace_id
                && actual_source_path == source_path
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_non_canonical_recorded_source_asset_paths() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/saved.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let absolute = std::env::temp_dir().join("prefabs/saved.prefab.ron");
        let absolute = absolute.to_string_lossy();
        for source_path in [
            ".\\prefabs\\saved.prefab.ron",
            "prefabs/./saved.prefab.ron",
            "prefabs//saved.prefab.ron",
            " prefabs/saved.prefab.ron",
            "../prefabs/saved.prefab.ron",
            "/prefabs/saved.prefab.ron",
            absolute.as_ref(),
        ] {
            let error = processor
                .record_source_asset(&SourceAssetRecordRequest {
                    capability: project_host_write_capability(),
                    session_id: TEST_SESSION_ID.to_string(),
                    workspace_root_id: workspace_source_root_id,
                    owner_id: "local.asset_processor_rpc".to_string(),
                    source_path: source_path.to_string(),
                    schema_type: Some("az.test.Prefab".to_string()),
                    content_hash: content_hash.clone(),
                    changed_unix_ms: 1_000,
                    diagnostics_count: 0,
                })
                .wait()
                .unwrap_err();
            assert!(
                matches!(error, AssetProcessorError::InvalidSourceAssetRecord { .. }),
                "source path `{source_path}` should be rejected before DB mutation"
            );
        }

        let accepted = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/saved.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_001,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();

        assert_eq!(accepted.entry.source_path, "prefabs/saved.prefab.ron");
        assert_eq!(accepted.entry.diff, WorkspaceEntryDiff::Added);
        let entries = processor
            .db()
            .workspace_entry_page(fixture.workspace.workspace_id, None, 0, 64)
            .unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.source_path == "prefabs/saved.prefab.ron")
                .count(),
            1
        );
        drop(processor);
        assert_eq!(build_job(&accepted.entry).job.key, "compile-prefab");
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_empty_recorded_source_schema_type() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let processor = grant_backed_processor(fixture.db);
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        for schema_type in ["", " "] {
            let error = processor
                .record_source_asset(&SourceAssetRecordRequest {
                    capability: project_host_write_capability(),
                    session_id: TEST_SESSION_ID.to_string(),
                    workspace_root_id: workspace_source_root_id,
                    owner_id: "local.asset_processor_rpc".to_string(),
                    source_path: "prefabs/saved.prefab.ron".to_string(),
                    schema_type: Some(schema_type.to_string()),
                    content_hash: content_hash.clone(),
                    changed_unix_ms: 1_000,
                    diagnostics_count: 0,
                })
                .wait()
                .unwrap_err();

            assert!(matches!(
                error,
                AssetProcessorError::InvalidSourceAssetRecord { .. }
            ));
        }
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_source_asset_owner_mismatch() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/wrong-owner.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor(fixture.db);
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let error = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "azoth.other_owner".to_string(),
                source_path: "prefabs/wrong-owner.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidSourceAssetRecord { reason }
                if reason.contains("outside the attached workspace authority")
        ));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_enqueues_registered_builder_jobs_for_changed_source() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/saved.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/saved.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();

        let activity = build_job(&result.entry);
        assert_eq!(
            activity.job.owner,
            JobOwner::Build(uuid!("00000000-0000-0000-0000-00000000b001"))
        );
        assert_eq!(activity.job.key, "compile-prefab");
        assert_eq!(activity.job.platform, DEFAULT_PLATFORM);
        assert_eq!(
            activity.job.source_schema_type.as_deref(),
            Some("az.test.Prefab")
        );
        assert_eq!(activity.job.attempts, 0);
        assert_eq!(activity.job.status, JobStatus::Queued);
        assert!(
            activity.attempt.is_none(),
            "source payload handles and attempts must be worker-lease data, not queued-job data"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_enqueues_registered_graph_builder_for_saved_graph() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let source_path = "graphs/saved.azgraph.ron";
        let mut document = VisualGraphDocument::new("az.asset_processor.tests.logic-graph");
        let mut node = GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-00000000a501")),
            "az.asset_processor.tests.Print",
            1,
        );
        node.input_values.insert(
            NodePortId::new(1),
            az_core::ReflectedValueEnvelope::typed_ron("alloc::string::String", r#""queued""#),
        );
        document.nodes.push(node);
        let payload = encode_visual_graph_document_ron(&document).unwrap();
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            source_path,
            "az.asset_processor.tests.logic-graph",
            &payload,
        );
        let content_hash = blake3::hash(payload.as_bytes()).as_bytes().to_vec();
        let processor =
            grant_backed_processor_with_builder_registry(fixture.db, composed_build_rules());

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: source_path.to_string(),
                schema_type: Some("az.asset_processor.tests.logic-graph".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();

        let activity = build_job(&result.entry);
        assert_eq!(
            activity.job.owner,
            JobOwner::Build(GRAPH_COMPILER_BUILDER_ID.0)
        );
        assert_eq!(activity.job.key, GRAPH_COMPILER_JOB_KEY);
        assert_eq!(activity.job.platform, DEFAULT_PLATFORM);
        assert_eq!(
            activity.job.source_schema_type.as_deref(),
            Some("az.asset_processor.tests.logic-graph")
        );
        assert_eq!(activity.job.status, JobStatus::Queued);
        assert!(activity.attempt.is_none());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_enqueues_registered_generated_graph_backend_for_saved_graph() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let source_path = "graphs/generated/saved.azgraph.ron";
        let mut document = VisualGraphDocument::new("az.asset_processor.tests.generated-graph");
        document.nodes.push(GraphNode::new(
            GraphNodeId::new(uuid!("018f0c5a-0000-7000-8000-00000000a5f1")),
            "az.asset_processor.tests.Print",
            1,
        ));
        let payload = encode_visual_graph_document_ron(&document).unwrap();
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            source_path,
            "az.asset_processor.tests.generated-graph",
            &payload,
        );
        let content_hash = blake3::hash(payload.as_bytes()).as_bytes().to_vec();
        let processor =
            grant_backed_processor_with_builder_registry(fixture.db, composed_build_rules());

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: source_path.to_string(),
                schema_type: Some("az.asset_processor.tests.generated-graph".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();

        let activity = build_job(&result.entry);
        assert_eq!(
            activity.job.owner,
            JobOwner::Build(GRAPH_COMPILER_BUILDER_ID.0)
        );
        assert_eq!(activity.job.key, GRAPH_COMPILER_JOB_KEY);
        assert_eq!(activity.job.platform, DEFAULT_PLATFORM);
        assert_eq!(
            activity.job.source_schema_type.as_deref(),
            Some("az.asset_processor.tests.generated-graph")
        );
        assert_eq!(activity.job.status, JobStatus::Queued);
        assert!(activity.attempt.is_none());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_passes_source_schema_type_and_saved_bytes_to_builder_create_jobs() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/schema-aware.prefab.ron",
            "az.test.Prefab",
            "schema-aware prefab",
        );
        let mut registry = BuildRuleRegistry::new();
        registry.register(schema_aware_prefab_builder_desc());
        let processor = grant_backed_processor_with_builder_registry(fixture.db, registry);
        let content_hash = blake3::hash(b"schema-aware prefab").as_bytes().to_vec();

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/schema-aware.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();

        assert_eq!(
            build_job(&result.entry).job.source_schema_type.as_deref(),
            Some("az.test.Prefab")
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_uses_source_schema_filters_when_builder_patterns_overlap() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/shared.prefab.ron",
            "az.test.Material",
            "schema-filtered material",
        );
        let mut registry = BuildRuleRegistry::new();
        registry.register(prefab_builder_desc());
        registry.register(material_builder_desc());
        let processor = grant_backed_processor_with_builder_registry(fixture.db, registry);
        let content_hash = blake3::hash(b"schema-filtered material")
            .as_bytes()
            .to_vec();

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/shared.prefab.ron".to_string(),
                schema_type: Some("az.test.Material".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();

        let material_job = build_job(&result.entry);
        assert_eq!(
            material_job.job.owner,
            JobOwner::Build(uuid!("00000000-0000-0000-0000-00000000b002"))
        );
        assert_eq!(material_job.job.key, "compile-material");
        assert_eq!(
            material_job.job.source_schema_type.as_deref(),
            Some("az.test.Material")
        );
        assert!(result.entry.jobs.iter().all(|activity| {
            activity.job.owner != JobOwner::Build(uuid!("00000000-0000-0000-0000-00000000b001"))
        }));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_does_not_enqueue_builder_jobs_when_source_hash_is_current() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/saved.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();
        let request = SourceAssetRecordRequest {
            capability: project_host_write_capability(),
            session_id: TEST_SESSION_ID.to_string(),
            workspace_root_id: workspace_source_root_id,
            owner_id: "local.asset_processor_rpc".to_string(),
            source_path: "prefabs/saved.prefab.ron".to_string(),
            schema_type: Some("az.test.Prefab".to_string()),
            content_hash,
            changed_unix_ms: 1_000,
            diagnostics_count: 0,
        };

        let first = processor.record_source_asset(&request).wait().unwrap();
        let first_job_id = build_job(&first.entry).job.job_id;
        let second = processor
            .record_source_asset(&SourceAssetRecordRequest {
                changed_unix_ms: 1_001,
                ..request
            })
            .wait()
            .unwrap();
        let current_job = build_job(&second.entry);

        assert_eq!(second.entry.diff, WorkspaceEntryDiff::Clean);
        assert_eq!(current_job.job.job_id, first_job_id);
        assert_eq!(current_job.job.attempts, 0);
        assert!(current_job.attempt.is_none());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_reconcile_asset_sources_records_current_files() {
        let fixture = fixture();
        let source_path = "prefabs/scanned.prefab.ron";
        let source_bytes = write_test_prefab_source(&fixture.source_root, source_path);
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );

        let result = processor
            .reconcile_asset_sources(&ReconcileAssetSourcesRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                root_scope: AssetRootScope::All,
            })
            .unwrap();

        assert_eq!(result.source_root_count, 1);
        assert_eq!(result.recorded_source_asset_count, 1);

        let page = processor
            .workspace_entry_page(&WorkspaceEntryPageRequest {
                capability: editor_read_capability(),
                root_scope: AssetRootScope::All,
                after_entry_id: None,
                page_size: 64,
            })
            .unwrap();
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.source_path == source_path)
            .expect("reconciled authored source entry");
        assert_eq!(
            entry.content_hash,
            blake3::hash(&source_bytes).to_hex().to_string()
        );
        assert_eq!(entry.schema_type.as_deref(), Some("az.test.Prefab"));
        let job = entry
            .jobs
            .iter()
            .find(|activity| activity.job.owner == JobOwner::Plan)
            .expect("reconciled source should enqueue its distributed planner job");
        assert_eq!(job.job.key, ASSET_PLANNER_JOB_KEY);
        assert_eq!(job.job.status, JobStatus::Queued);
        assert!(job.attempt.is_none());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_reconcile_rejects_sidecar_identity_replacement() {
        let fixture = fixture();
        let source_path = "prefabs/scanned.prefab.ron";
        let source_bytes = write_test_prefab_source(&fixture.source_root, source_path);
        let source_file = fixture.source_root.join(source_path);
        let old_guid = Uuid::from_u128(0x1111);
        let preserved_guid = Uuid::from_u128(0x2222);
        let content_hash = Digest::from(blake3::hash(&source_bytes));
        fixture
            .writer
            .apply_sweep_delta(ApplySweepDelta {
                workspace_pk: fixture.workspace.workspace_id,
                workspace_root_pk: fixture.workspace_source_root.workspace_root_id,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: source_path.to_string(),
                        guid: old_guid,
                        schema: Some("az.test.Prefab".to_string()),
                        digest: content_hash,
                        diff: DbDiff::Clean,
                        diagnostics: 0,
                        updated: 1_000,
                        src_bytes: 0,
                        src_mtime: 0,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 1_000,
                        session: Some(TEST_SESSION_ID.to_string()),
                    },
                    planner: SweepPlannerJob {
                        key: ASSET_PLANNER_JOB_KEY.to_string(),
                        platform: DEFAULT_PLATFORM.to_string(),
                    },
                }],
                removals: Vec::new(),
            })
            .wait_blocking()
            .unwrap();
        std::fs::write(
            source_meta_sidecar_path(&source_file),
            serde_json::to_vec(&SourceAssetMeta::preserving(AssetId::new(
                preserved_guid,
                0,
            )))
            .unwrap(),
        )
        .unwrap();
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );

        let error = processor
            .reconcile_asset_sources(&ReconcileAssetSourcesRequest {
                capability: editor_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                root_scope: AssetRootScope::All,
            })
            .unwrap_err();
        assert!(matches!(error, AssetProcessorError::Repo(_)));
        let db = processor.db();
        let (identity, entry) = db
            .source_asset(
                fixture.workspace.workspace_id,
                fixture.workspace_source_root.root_pk,
                source_path,
            )
            .unwrap()
            .expect("the original stable identity remains at the locator");
        assert_eq!(identity.guid, old_guid);
        drop(db);
        assert_ne!(identity.guid, preserved_guid);
        assert_eq!(entry.diff, DbDiff::Clean);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_force_reprocess_enqueues_current_source() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        let ordinary_job = install_fixture_build_job(
            &fixture,
            uuid!("00000000-0000-0000-0000-00000000b001"),
            "ordinary-backlog",
        );
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/saved.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();
        let request = SourceAssetRecordRequest {
            capability: project_host_write_capability(),
            session_id: TEST_SESSION_ID.to_string(),
            workspace_root_id: workspace_source_root_id,
            owner_id: "local.asset_processor_rpc".to_string(),
            source_path: "prefabs/saved.prefab.ron".to_string(),
            schema_type: Some("az.test.Prefab".to_string()),
            content_hash,
            changed_unix_ms: 1_000,
            diagnostics_count: 0,
        };

        let recorded = processor.record_source_asset(&request).wait().unwrap();
        let recorded_job_id = build_job(&recorded.entry).job.job_id;
        let current = processor
            .record_source_asset(&SourceAssetRecordRequest {
                changed_unix_ms: 1_001,
                ..request
            })
            .wait()
            .unwrap();

        assert_eq!(build_job(&current.entry).job.job_id, recorded_job_id);

        let forced = processor
            .force_reprocess_asset(&ForceReprocessAssetRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: "prefabs/saved.prefab.ron".to_string(),
            })
            .wait()
            .unwrap();
        let forced_job = build_job(&forced.record.entry);

        assert_eq!(forced.enqueued_jobs, 1);
        assert_eq!(forced.record.entry.diff, WorkspaceEntryDiff::Clean);
        assert_eq!(
            forced_job.job.job_id, recorded_job_id,
            "force reprocess requeues the stable job identity"
        );
        let forced_asset_identity_pk = processor
            .db()
            .job_by_id(forced_job.job.job_id)
            .unwrap()
            .unwrap()
            .asset_pk;
        assert!(
            processor
                .prioritized_asset_identities
                .borrow()
                .contains(&forced_asset_identity_pk)
        );
        assert_eq!(forced_job.job.status, JobStatus::Queued);
        assert!(forced_job.attempt.is_none());

        let staging = tempfile::tempdir().unwrap();
        let rpc = Rc::new(AssetProcessorRpc::new(processor));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let leased = local
            .block_on(
                &runtime,
                rpc.lease_job(&LeaseAssetJobRequest {
                    capability: capability(),
                    lease_owner: "worker-priority".to_string(),
                    lease_duration_ms: 8_000,
                    staging_root: Some(path_string(staging.path())),
                }),
            )
            .unwrap()
            .leased;
        assert_eq!(leased.source_guid, forced.record.asset_guid);
        assert_eq!(
            rpc.processor()
                .db()
                .job_by_id(ordinary_job.job_id)
                .unwrap()
                .unwrap()
                .status,
            DbStatus::Queued,
        );
    }

    /// A worker catalog declaring both a catch-all legacy schema and a specific
    /// file-source schema for the same extension.
    ///
    /// The overlap is the point: the durable classification has to pick the
    /// specific one, and force-reprocess must not be able to talk it out of that.
    fn legacy_and_file_source_catalog(builders: &BuildRuleRegistry) -> AssetBuilderCatalogResult {
        let mut catalog = test_builder_catalog(builders);
        catalog.source_schemas = vec![
            SourceSchemaDescriptor {
                schema_type: "az.test.LegacyGeneric".to_string(),
                owner: "az-test-worker".to_string(),
                label: "Legacy Generic".to_string(),
                category: "Tests".to_string(),
                authoring: SourceSchemaAuthoring::File {
                    workflow: SourceFileWorkflowDescriptor {
                        source_root: PROJECT_SOURCE_ROOT.to_string(),
                        default_path_prefix: String::new(),
                        extensions: vec!["*".to_string()],
                        can_create: false,
                        can_edit: false,
                    },
                },
                file_templates: Vec::new(),
            },
            SourceSchemaDescriptor {
                schema_type: "az.test.FileSource".to_string(),
                owner: "az-test-worker".to_string(),
                label: "Crafting Station Database".to_string(),
                category: "Tests".to_string(),
                authoring: SourceSchemaAuthoring::File {
                    workflow: SourceFileWorkflowDescriptor {
                        source_root: PROJECT_SOURCE_ROOT.to_string(),
                        default_path_prefix: "crafting".to_string(),
                        extensions: vec!["craftstationdb.ron".to_string()],
                        can_create: true,
                        can_edit: true,
                    },
                },
                file_templates: Vec::new(),
            },
        ];
        catalog
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_force_reprocess_does_not_bypass_durable_source_classification() {
        let fixture = fixture();
        let source_path = "sharedassets/genericassets/craftingstations.craftstationdb.ron";
        let source_bytes = b"crafting station database";
        let source_file = fixture.source_root.join(source_path);
        std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
        std::fs::write(&source_file, source_bytes).unwrap();
        fixture
            .writer
            .apply_sweep_delta(ApplySweepDelta {
                workspace_pk: fixture.workspace.workspace_id,
                workspace_root_pk: fixture.workspace_source_root.workspace_root_id,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: source_path.to_string(),
                        guid: Uuid::from_u128(0x3333),
                        schema: Some("az.test.LegacyGeneric".to_string()),
                        digest: Digest::from(blake3::hash(source_bytes)),
                        diff: DbDiff::Clean,
                        diagnostics: 0,
                        updated: 1_000,
                        src_bytes: 0,
                        src_mtime: 0,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 1_000,
                        session: Some(TEST_SESSION_ID.to_string()),
                    },
                    planner: SweepPlannerJob {
                        key: ASSET_PLANNER_JOB_KEY.to_string(),
                        platform: DEFAULT_PLATFORM.to_string(),
                    },
                }],
                removals: Vec::new(),
            })
            .wait_blocking()
            .unwrap();
        let source_roots = vec![fixture_registered_source_root(&fixture)];
        let builders = registry_with_file_source_builder();
        let catalog = legacy_and_file_source_catalog(&builders);
        let processor = AssetProcessor::with_builder_registry_and_catalog(
            fixture.db,
            builders,
            default_capability_grants(),
            test_registries(),
            Some(fixture.workspace.workspace_id),
            None,
            Some(catalog),
        )
        .with_source_roots(source_roots);

        let error = processor
            .force_reprocess_asset(&ForceReprocessAssetRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                source_root: PROJECT_SOURCE_ROOT.to_string(),
                source_path: source_path.to_string(),
            })
            .wait()
            .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::ForceReprocessNoJobs { .. }
        ));
        let entry = processor
            .db()
            .workspace_entry_page(fixture.workspace.workspace_id, None, 0, 64)
            .unwrap()
            .into_iter()
            .find(|entry| entry.source_path == source_path)
            .expect("durable source remains recorded");
        assert_eq!(entry.schema.as_deref(), Some("az.test.LegacyGeneric"));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_retries_builder_create_jobs_after_recorded_source_failure() {
        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/flaky.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        FAIL_NEXT_PREFAB_CREATE_JOBS.store(true, Ordering::SeqCst);
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_flaky_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();
        let request = SourceAssetRecordRequest {
            capability: project_host_write_capability(),
            session_id: TEST_SESSION_ID.to_string(),
            workspace_root_id: workspace_source_root_id,
            owner_id: "local.asset_processor_rpc".to_string(),
            source_path: "prefabs/flaky.prefab.ron".to_string(),
            schema_type: Some("az.test.Prefab".to_string()),
            content_hash,
            changed_unix_ms: 1_000,
            diagnostics_count: 0,
        };

        let recorded = processor.record_source_asset(&request).wait().unwrap();
        assert_eq!(recorded.entry.diff, WorkspaceEntryDiff::Added);
        assert!(recorded.entry.jobs.is_empty());
        assert!(!FAIL_NEXT_PREFAB_CREATE_JOBS.load(Ordering::SeqCst));
        {
            let db = processor.db();
            let entries = db.workspace_entry_page(workspace_pk, None, 0, 64).unwrap();
            let entry = entries
                .iter()
                .find(|entry| entry.source_path == "prefabs/flaky.prefab.ron")
                .expect("failed create_jobs should not roll back source registry identity");
            assert_eq!(entry.diff, DbDiff::Added);
            assert!(
                db.jobs_for_asset(workspace_pk, entry.asset_pk)
                    .unwrap()
                    .is_empty(),
                "failed create_jobs should not leave a phantom queued attempt"
            );
            drop(db);
        }

        let retried = processor
            .record_source_asset(&SourceAssetRecordRequest {
                changed_unix_ms: 1_001,
                ..request
            })
            .wait()
            .unwrap();

        assert_eq!(retried.entry.diff, WorkspaceEntryDiff::Clean);
        let activity = build_job(&retried.entry);
        assert_eq!(activity.job.key, "compile-prefab");
        assert_eq!(activity.job.attempts, 0);
        assert_eq!(activity.job.status, JobStatus::Queued);
        assert!(activity.attempt.is_none());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_duplicate_builder_job_descriptors_before_enqueue() {
        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/duplicate-job.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_duplicate_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/duplicate-job.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();
        assert!(result.entry.jobs.is_empty());
        let error = replan_source_for_test(
            &processor,
            workspace_pk,
            result.entry.root_id,
            &result.entry.source_path,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidBuilderCreateJobs {
                builder_name: "az.test.prefab",
                source_path,
                reason,
            } if source_path == "prefabs/duplicate-job.prefab.ron"
                && reason.contains("duplicate job descriptor")
        ));
        let db = processor.db();
        let (_, entry) = db
            .source_asset(
                workspace_pk,
                result.entry.root_id,
                "prefabs/duplicate-job.prefab.ron",
            )
            .unwrap()
            .expect("source registry write should remain durable");
        assert!(
            db.jobs_for_asset(workspace_pk, entry.asset_pk)
                .unwrap()
                .is_empty(),
            "invalid builder descriptors must not enqueue durable jobs"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_records_builder_source_dependencies_with_job_plan() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/dependencies.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_dependency_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/dependencies.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();

        let activity = build_job(&result.entry);
        assert_eq!(activity.job.key, "compile-prefab");
        let job_id = activity.job.job_id;
        let workspace_id = activity.job.workspace_id;
        let db = processor.db();
        let planned = db.job_by_id(job_id).unwrap();
        let planned = planned.expect("planned dependency job");
        let dependencies = db
            .source_edges_for_asset(workspace_id, planned.asset_pk)
            .unwrap();
        assert_eq!(dependencies.len(), 3);
        assert_eq!(
            dependencies[0].builder,
            uuid!("00000000-0000-0000-0000-00000000b001")
        );
        assert_eq!(
            dependencies[0].target,
            DbTarget::path("materials/base.material.ron").unwrap()
        );
        assert_eq!(dependencies[0].relation, DbRelation::SourceToSource);
        assert_eq!(
            dependencies[1].target,
            DbTarget::Guid(uuid!("77777777-7777-7777-7777-777777777777"))
        );
        assert_eq!(dependencies[1].relation, DbRelation::SourceToSource);
        assert_eq!(
            dependencies[2].target,
            DbTarget::path("materials/base.material.ron").unwrap()
        );
        assert_eq!(dependencies[2].relation, DbRelation::JobToJob);
        drop(db);

        let result = processor
            .inspect_job(&InspectJobRequest {
                capability: editor_read_capability(),
                selector: InspectJobSelector::Job(job_id),
            })
            .unwrap();
        let inspection = result.inspection.expect("planned job inspection");
        assert_eq!(inspection.dependencies.len(), 1);
        assert_eq!(
            inspection.dependencies[0].target,
            JobDependencyTarget::Path("materials/base.material.ron".to_owned())
        );
        assert_eq!(inspection.dependencies[0].key, "compile-material");
        assert_eq!(inspection.dependencies[0].platform, "pc");
        assert_eq!(
            inspection.dependencies[0].kind,
            JobDependencyKind::Fingerprint
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_duplicate_builder_source_dependencies_before_enqueue() {
        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/duplicate-dependency.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_duplicate_dependency_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/duplicate-dependency.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();
        assert!(result.entry.jobs.is_empty());
        let error = replan_source_for_test(
            &processor,
            workspace_pk,
            result.entry.root_id,
            &result.entry.source_path,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidBuilderCreateJobs {
                builder_name: "az.test.prefab",
                source_path,
                reason,
            } if source_path == "prefabs/duplicate-dependency.prefab.ron"
                && reason.contains("duplicate source dependency")
        ));
        {
            let db = processor.db();
            let (_, entry) = db
                .source_asset(
                    workspace_pk,
                    result.entry.root_id,
                    "prefabs/duplicate-dependency.prefab.ron",
                )
                .unwrap()
                .expect("source registry write should remain durable");
            assert!(
                db.jobs_for_asset(workspace_pk, entry.asset_pk)
                    .unwrap()
                    .is_empty(),
                "invalid builder dependencies must not enqueue durable jobs"
            );
            assert!(
                db.source_edges_for_asset(workspace_pk, entry.asset_pk)
                    .unwrap()
                    .is_empty(),
                "invalid builder dependencies must not persist source dependency rows"
            );
            drop(db);
        }
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_duplicate_builder_job_dependencies_before_enqueue() {
        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/duplicate-job-dependency.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_duplicate_job_dependency_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/duplicate-job-dependency.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();
        assert!(result.entry.jobs.is_empty());
        let error = replan_source_for_test(
            &processor,
            workspace_pk,
            result.entry.root_id,
            &result.entry.source_path,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidBuilderCreateJobs {
                builder_name: "az.test.prefab",
                source_path,
                reason,
            } if source_path == "prefabs/duplicate-job-dependency.prefab.ron"
                && reason.contains("duplicate dependency")
        ));
        {
            let db = processor.db();
            let (_, entry) = db
                .source_asset(
                    workspace_pk,
                    result.entry.root_id,
                    "prefabs/duplicate-job-dependency.prefab.ron",
                )
                .unwrap()
                .expect("source registry write should remain durable");
            assert!(
                db.jobs_for_asset(workspace_pk, entry.asset_pk)
                    .unwrap()
                    .is_empty(),
                "invalid job dependencies must not enqueue durable jobs"
            );
            assert!(
                db.source_edges_for_asset(workspace_pk, entry.asset_pk)
                    .unwrap()
                    .is_empty(),
                "invalid job dependencies must not persist source dependency rows"
            );
            drop(db);
        }
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn processor_domain_rejects_invalid_builder_source_dependency_paths() {
        let fixture = fixture();
        let workspace_pk = fixture.workspace.workspace_id;
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/invalid-dependency.prefab.ron",
            "az.test.Prefab",
            "saved prefab",
        );
        let processor = grant_backed_processor_with_builder_registry(
            fixture.db,
            registry_with_invalid_dependency_prefab_builder(),
        );
        let content_hash = blake3::hash(b"saved prefab").as_bytes().to_vec();

        let result = processor
            .record_source_asset(&SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/invalid-dependency.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash,
                changed_unix_ms: 1_000,
                diagnostics_count: 0,
            })
            .wait()
            .unwrap();
        assert!(result.entry.jobs.is_empty());
        let error = replan_source_for_test(
            &processor,
            workspace_pk,
            result.entry.root_id,
            &result.entry.source_path,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::InvalidBuilderCreateJobs {
                builder_name: "az.test.prefab",
                source_path,
                reason,
            } if source_path == "prefabs/invalid-dependency.prefab.ron"
                && reason.contains("not an asset-db relative path")
        ));
        {
            let db = processor.db();
            let (_, entry) = db
                .source_asset(
                    workspace_pk,
                    result.entry.root_id,
                    "prefabs/invalid-dependency.prefab.ron",
                )
                .unwrap()
                .expect("source registry write should remain durable");
            assert!(
                db.jobs_for_asset(workspace_pk, entry.asset_pk)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                db.source_edges_for_asset(workspace_pk, entry.asset_pk)
                    .unwrap()
                    .is_empty()
            );
            drop(db);
        }
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn asset_processor_rpc_records_source_asset() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/rpc-saved.prefab.ron",
            "az.test.Prefab",
            "saved rpc prefab",
        );
        let rpc = Rc::new(AssetProcessorRpc::new(
            grant_backed_processor_with_builder_registry(
                fixture.db,
                registry_with_prefab_builder(),
            ),
        ));
        let client = AssetProcessorRpc::client_from_rc(&rpc);
        let content_hash = blake3::hash(b"saved rpc prefab").as_bytes().to_vec();

        let mut request = client.record_source_asset_request();
        (SourceAssetRecordRequest {
            capability: project_host_write_capability(),
            session_id: TEST_SESSION_ID.to_string(),
            workspace_root_id: workspace_source_root_id,
            owner_id: "local.asset_processor_rpc".to_string(),
            source_path: "prefabs/rpc-saved.prefab.ron".to_string(),
            schema_type: Some("az.test.Prefab".to_string()),
            content_hash: content_hash.clone(),
            changed_unix_ms: 1_000,
            diagnostics_count: 1,
        })
        .to_capnp(request.get().init_request())
        .unwrap();
        let response = block_on_test_runtime(request.send().promise).unwrap();
        let result =
            SourceAssetRecordResult::from_capnp(response.get().unwrap().get_result().unwrap())
                .unwrap();

        assert_eq!(result.entry.source_path, "prefabs/rpc-saved.prefab.ron");
        drop(rpc);
        assert_eq!(result.entry.diff, WorkspaceEntryDiff::Added);
        assert_eq!(result.entry.diagnostics_count, 1);
        assert_eq!(result.entry.content_hash, hex_lower(&content_hash));
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn asset_processor_rpc_publishes_recorded_source_asset_events() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/evented.prefab.ron",
            "az.test.Prefab",
            "evented prefab",
        );
        let rpc = Rc::new(AssetProcessorRpc::new(
            grant_backed_processor_with_builder_registry(
                fixture.db,
                registry_with_prefab_builder(),
            ),
        ));
        let client = AssetProcessorRpc::client_from_rc(&rpc);
        drop(rpc);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&runtime, async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let sink = capnp_rpc::new_client(TestAssetProcessorEventSink { tx });

            let mut subscribe = client.subscribe_events_request();
            {
                let mut params = subscribe.get();
                (AssetProcessorEventSubscriptionRequest {
                    capability: editor_read_capability(),
                })
                .to_capnp(params.reborrow().init_request())
                .unwrap();
                params.set_sink(sink);
            }
            let response = subscribe.send().promise.await.unwrap();
            let result = AssetProcessorEventSubscriptionResult::from_capnp(
                response.get().unwrap().get_result().unwrap(),
            )
            .unwrap();
            assert!(result.subscribed);

            let content_hash = blake3::hash(b"evented prefab").as_bytes().to_vec();
            let mut request = client.record_source_asset_request();
            (SourceAssetRecordRequest {
                capability: project_host_write_capability(),
                session_id: TEST_SESSION_ID.to_string(),
                workspace_root_id: workspace_source_root_id,
                owner_id: "local.asset_processor_rpc".to_string(),
                source_path: "prefabs/evented.prefab.ron".to_string(),
                schema_type: Some("az.test.Prefab".to_string()),
                content_hash: content_hash.clone(),
                changed_unix_ms: 1_250,
                diagnostics_count: 2,
            })
            .to_capnp(request.get().init_request())
            .unwrap();
            let response = request.send().promise.await.unwrap();
            let result =
                SourceAssetRecordResult::from_capnp(response.get().unwrap().get_result().unwrap())
                    .unwrap();

            let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .unwrap()
                .expect("asset processor event should be delivered");

            assert_eq!(event.seq, 1);
            assert_eq!(event.kind, AssetProcessorEventKind::SourceRecorded);
            assert_eq!(event.event_unix_ms, 1_250);
            assert_eq!(event.entry, result.entry);
            assert_eq!(event.entry.source_path, "prefabs/evented.prefab.ron");
            assert_eq!(event.entry.diagnostics_count, 2);
        });
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn asset_processor_rpc_prunes_only_the_subscriber_that_rejects_an_event() {
        let fixture = fixture();
        let workspace_source_root_id = fixture.workspace_source_root.workspace_root_id;
        upsert_saved_authored_source(
            &fixture.db,
            fixture.workspace.workspace_id,
            "local.asset_processor_rpc",
            "prefabs/pruned-subscriber.prefab.ron",
            "az.test.Prefab",
            "evented prefab",
        );
        let rpc = Rc::new(AssetProcessorRpc::new(
            grant_backed_processor_with_builder_registry(
                fixture.db,
                registry_with_prefab_builder(),
            ),
        ));
        let client = AssetProcessorRpc::client_from_rc(&rpc);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&runtime, async {
            let rejected_attempts = Rc::new(std::cell::Cell::new(0));
            let rejecting_sink = capnp_rpc::new_client(RejectingAssetProcessorEventSink {
                attempts: Rc::clone(&rejected_attempts),
            });
            let (healthy_tx, mut healthy_rx) = tokio::sync::mpsc::unbounded_channel();
            let healthy_sink =
                capnp_rpc::new_client(TestAssetProcessorEventSink { tx: healthy_tx });

            for sink in [rejecting_sink, healthy_sink] {
                let mut subscribe = client.subscribe_events_request();
                {
                    let mut params = subscribe.get();
                    (AssetProcessorEventSubscriptionRequest {
                        capability: editor_read_capability(),
                    })
                    .to_capnp(params.reborrow().init_request())
                    .unwrap();
                    params.set_sink(sink);
                }
                let response = subscribe.send().promise.await.unwrap();
                assert!(
                    AssetProcessorEventSubscriptionResult::from_capnp(
                        response.get().unwrap().get_result().unwrap(),
                    )
                    .unwrap()
                    .subscribed
                );
            }
            assert_eq!(rpc.event_subscribers.borrow().len(), 2);

            for (changed_unix_ms, diagnostics_count) in [(1_250, 1), (1_251, 2)] {
                let mut request = client.record_source_asset_request();
                (SourceAssetRecordRequest {
                    capability: project_host_write_capability(),
                    session_id: TEST_SESSION_ID.to_string(),
                    workspace_root_id: workspace_source_root_id,
                    owner_id: "local.asset_processor_rpc".to_string(),
                    source_path: "prefabs/pruned-subscriber.prefab.ron".to_string(),
                    schema_type: Some("az.test.Prefab".to_string()),
                    content_hash: blake3::hash(b"evented prefab").as_bytes().to_vec(),
                    changed_unix_ms,
                    diagnostics_count,
                })
                .to_capnp(request.get().init_request())
                .unwrap();
                request.send().promise.await.unwrap();

                let event =
                    tokio::time::timeout(std::time::Duration::from_secs(1), healthy_rx.recv())
                        .await
                        .unwrap()
                        .expect("healthy subscriber should receive every event");
                assert_eq!(event.event_unix_ms, changed_unix_ms);

                // Let the rejected update promise finish and prune its entry
                // before publishing the next event.
                tokio::task::yield_now().await;
            }

            assert_eq!(rejected_attempts.get(), 1);
            assert_eq!(rpc.event_subscribers.borrow().len(), 1);
            assert!(healthy_rx.try_recv().is_err());
        });
    }

    #[test]
    fn registers_session_asset_source_roots_from_project_and_gems() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");
        let workspace_root = normalize(temp.path()).join("workspace");
        let gem_root = workspace_root.join("gems").join("physics");

        let mut project = az_project::ProjectManifest::new(
            "local.asset_processor_sources",
            "Asset Processor Sources",
            "0.1.0",
        );
        project.asset_roots[0].path = PathBuf::from("project-assets");
        project.gems.push(az_project::ProjectGem {
            id: "azoth.physics".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            linkage: None,
            path: Some(PathBuf::from("gems").join("physics")),
        });
        az_project::write_project_manifest(&workspace_root, &project).unwrap();
        std::fs::create_dir_all(workspace_root.join("project-assets")).unwrap();

        let mut gem = az_project::GemManifest::new("azoth.physics", "Physics", "0.1.0");
        gem.contributions.push(az_project::GemContribution::assets(
            "assets",
            "gem-assets",
            [az_project::GemTargetRole::AssetProcessor],
        ));
        az_project::write_gem_manifest(&gem_root, &gem).unwrap();
        az_project::refresh_project_lock(&workspace_root).unwrap();
        std::fs::create_dir_all(gem_root.join("gem-assets")).unwrap();

        let registration = register_workspace_asset_source_roots_blocking(
            &db_path,
            "local.asset_processor_sources",
            Some("018f0c5a-0000-7000-8000-000000000444"),
            &workspace_root,
            "az/session/assets",
            123,
            test_registries(),
        )
        .unwrap();

        assert_eq!(registration.source_roots.len(), 2);
        assert!(registration.source_roots[0].workspace_root_id > 0);
        assert_eq!(
            registration.source_roots[0].owner_id,
            "local.asset_processor_sources"
        );
        assert_eq!(
            registration.source_roots[0].portable_key,
            "project:local.asset_processor_sources:assets"
        );
        assert_eq!(registration.source_roots[0].output_prefix, "");
        assert_eq!(
            registration.source_roots[1].portable_key,
            "gem:azoth.physics:assets"
        );
        assert_eq!(registration.source_roots[1].output_prefix, "");

        let db = AssetDb::open(&db_path).unwrap();
        let view = db
            .workspace_by_id(registration.workspace_id)
            .unwrap()
            .unwrap();
        assert_eq!(view.workspace_id, registration.workspace_id);
        let roots = db.workspace_roots(registration.workspace_id).unwrap();
        drop(db);
        assert_eq!(roots.len(), 2);
        assert_eq!(
            roots[0].path,
            path_string(&workspace_root.join("project-assets"))
        );
        assert_eq!(roots[1].path, path_string(&gem_root.join("gem-assets")));
        assert_eq!(
            registration.source_roots[0].root,
            workspace_root.join("project-assets")
        );
        assert_eq!(registration.source_roots[0].display_name, "Project Assets");
        assert_eq!(
            registration.source_roots[1].root,
            gem_root.join("gem-assets")
        );
        assert_eq!(registration.source_roots[1].display_name, "Physics Assets");
    }

    #[test]
    fn register_workspace_asset_source_roots_rejects_invalid_identity_before_db_open() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");
        let missing_workspace = temp.path().join("missing-workspace");

        let error = register_workspace_asset_source_roots(
            &db_path,
            "",
            &missing_workspace,
            "az/session/assets",
            123,
        )
        .unwrap_err();
        assert!(matches!(error, AssetProcessorError::ProjectIdRequired));

        let error = register_workspace_asset_source_roots(
            &db_path,
            "local.asset_processor_sources",
            &missing_workspace,
            "   ",
            123,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AssetProcessorError::WorkspaceBranchRequired
        ));
        assert!(
            !db_path.exists(),
            "invalid launch identity must fail before opening the asset DB"
        );
    }

    #[test]
    fn register_workspace_asset_source_roots_canonicalizes_workspace_and_source_roots() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");
        let workspace_root = temp.path().join("workspace");

        let mut project = az_project::ProjectManifest::new(
            "local.asset_processor_canonical",
            "Asset Processor Canonical",
            "0.1.0",
        );
        project.asset_roots[0].path = PathBuf::from("project-assets");
        az_project::write_project_manifest(&workspace_root, &project).unwrap();
        az_project::refresh_project_lock(&workspace_root).unwrap();
        std::fs::create_dir_all(workspace_root.join("project-assets")).unwrap();

        let registration = register_workspace_asset_source_roots_blocking(
            &db_path,
            "local.asset_processor_canonical",
            Some("018f0c5a-0000-7000-8000-000000000445"),
            workspace_root.join("."),
            "az/session/assets",
            123,
            test_registries(),
        )
        .unwrap();

        let db = AssetDb::open(&db_path).unwrap();
        let view = db
            .workspace_by_id(registration.workspace_id)
            .unwrap()
            .unwrap();
        assert_eq!(view.root, canonical_path_string(&workspace_root));
        let roots = db.workspace_roots(registration.workspace_id).unwrap();
        drop(db);
        assert_eq!(
            roots[0].path,
            canonical_path_string(&workspace_root.join("project-assets"))
        );
        assert_eq!(
            registration.source_roots[0].root,
            PathBuf::from(canonical_path_string(
                &workspace_root.join("project-assets")
            ))
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn register_workspace_asset_source_roots_reconciles_existing_registered_authored_sources() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");
        let workspace_root = temp.path().join("workspace");
        let source_path = "prefabs/existing.prefab.ron";
        let assets_root = workspace_root.join("assets");
        std::fs::create_dir_all(&assets_root).unwrap();
        let source_bytes = write_test_prefab_source(&assets_root, source_path);
        std::fs::write(
            assets_root.join("prefabs").join("notes.txt"),
            b"not an asset",
        )
        .unwrap();
        write_project_manifest_with_lock(
            &workspace_root,
            &az_project::ProjectManifest::new(
                "local.asset_processor_reconcile",
                "Asset Processor Reconcile",
                "0.1.0",
            ),
        );

        let registration = register_workspace_asset_source_roots_blocking(
            &db_path,
            "local.asset_processor_reconcile",
            Some("018f0c5a-0000-7000-8000-000000000449"),
            &workspace_root,
            "az/session/assets",
            1_234,
            test_registries(),
        )
        .unwrap();

        let db = AssetDb::open(&db_path).unwrap();
        let entries = db
            .workspace_entry_page(registration.workspace_id, None, 0, 10)
            .unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.source_path, source_path);
        assert_eq!(entry.digest, Digest::from(blake3::hash(&source_bytes)));
        assert_eq!(entry.diff, DbDiff::Added);
        assert_eq!(entry.schema.as_deref(), Some("az.test.Prefab"));
        assert!(db.asset_by_id(entry.asset_pk).unwrap().is_some());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn persistent_query_handle_reconcile_preserves_the_published_worker_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");
        let workspace_root = temp.path().join("workspace");
        let assets_root = workspace_root.join("assets");
        let source_path = "deployments/remote.settings.ron";
        std::fs::create_dir_all(assets_root.join("deployments")).unwrap();
        std::fs::write(assets_root.join(source_path), b"(profile: \"remote\")").unwrap();
        write_project_manifest_with_lock(
            &workspace_root,
            &az_project::ProjectManifest::new(
                "local.asset_processor_catalog_reconcile",
                "Asset Processor Catalog Reconcile",
                "0.1.0",
            ),
        );
        let session_id = TEST_SESSION_ID;
        let registered = open_registered_workspace_asset_db(
            &db_path,
            "local.asset_processor_catalog_reconcile",
            &workspace_root,
            "az/session/assets",
            1_234,
        )
        .unwrap();
        let (_db_path, database, _writer, source_roots, registration) = registered.into_parts();

        let processor = AssetProcessor::with_builder_registry_and_catalog(
            database,
            BuildRuleRegistry::new(),
            default_capability_grants(),
            engine_host_registries(),
            Some(registration.workspace_id),
            None,
            Some(AssetBuilderCatalogResult {
                builders: Vec::new(),
                source_schemas: vec![SourceSchemaDescriptor {
                    schema_type: "az.test.DeploymentProfile".to_string(),
                    owner: "az-asset-processor::tests".to_string(),
                    label: "Deployment Profile".to_string(),
                    category: "Deployment".to_string(),
                    authoring: SourceSchemaAuthoring::File {
                        workflow: SourceFileWorkflowDescriptor {
                            source_root: PROJECT_SOURCE_ROOT.to_string(),
                            default_path_prefix: "deployments".to_string(),
                            extensions: vec!["settings.ron".to_string()],
                            can_create: true,
                            can_edit: true,
                        },
                    },
                    file_templates: Vec::new(),
                }],
                product_formats: Vec::new(),
            }),
        )
        .with_source_roots(source_roots);
        let result = processor
            .reconcile_asset_sources(&ReconcileAssetSourcesRequest {
                capability: editor_write_capability(),
                session_id: session_id.to_string(),
                root_scope: AssetRootScope::All,
            })
            .unwrap();

        assert_eq!(result.recorded_source_asset_count, 1);
        let entries = processor
            .db()
            .workspace_entry_page(registration.workspace_id, None, 0, 10)
            .unwrap();
        assert_eq!(entries.len(), 1);
        drop(processor);
        assert_eq!(entries[0].source_path, source_path);
        assert_eq!(
            entries[0].schema.as_deref(),
            Some("az.test.DeploymentProfile")
        );
    }

    #[test]
    fn register_workspace_asset_source_roots_rejects_relative_workspace_root() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");

        let error = register_workspace_asset_source_roots(
            &db_path,
            "local.asset_processor_sources",
            "relative-workspace",
            "az/session/assets",
            123,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::WorkspaceRootNotAbsolute { workspace_root }
                if workspace_root == *"relative-workspace"
        ));
        assert!(!db_path.exists());
    }

    #[test]
    fn register_workspace_asset_source_roots_rejects_project_id_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");
        let workspace_root = temp.path().join("workspace");
        write_project_manifest_with_lock(
            &workspace_root,
            &az_project::ProjectManifest::new("local.actual", "Actual", "0.1.0"),
        );

        let error = register_workspace_asset_source_roots(
            &db_path,
            "local.expected",
            &workspace_root,
            "az/session/assets",
            123,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::ProjectManifestIdMismatch {
                expected,
                actual,
                ..
            } if expected == "local.expected" && actual == "local.actual"
        ));
        assert!(!db_path.exists());
    }

    #[test]
    fn register_workspace_asset_source_roots_rejects_missing_project_root_without_db_view() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");
        let missing_workspace = temp.path().join("missing-workspace");

        let error = register_workspace_asset_source_roots(
            &db_path,
            "local.missing_project_root",
            &missing_workspace,
            "az/session/assets",
            123,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AssetProcessorError::WorkspaceRootRead { workspace_root, .. }
                if workspace_root == missing_workspace
        ));
        assert!(!db_path.exists());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn register_workspace_asset_source_roots_registers_missing_gem_assets_as_empty_root() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");
        let workspace_root = normalize(temp.path()).join("workspace");
        let gem_root = workspace_root.join("gems").join("test-audio-system");

        let mut project = az_project::ProjectManifest::new(
            "local.missing_gem_assets",
            "Missing Gem Assets",
            "0.1.0",
        );
        project.gems.push(az_project::ProjectGem {
            id: "local.test-audio-system".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            linkage: None,
            path: Some(PathBuf::from("gems").join("test-audio-system")),
        });
        az_project::write_project_manifest(&workspace_root, &project).unwrap();
        std::fs::create_dir_all(workspace_root.join("assets")).unwrap();

        let mut gem =
            az_project::GemManifest::new("local.test-audio-system", "Test Audio", "0.1.0");
        gem.contributions.push(az_project::GemContribution::assets(
            "assets",
            "assets",
            [az_project::GemTargetRole::AssetProcessor],
        ));
        az_project::write_gem_manifest(&gem_root, &gem).unwrap();
        az_project::refresh_project_lock(&workspace_root).unwrap();

        let missing_assets = gem_root.join("assets");
        assert!(!missing_assets.exists());

        let registration = register_workspace_asset_source_roots_blocking(
            &db_path,
            "local.missing_gem_assets",
            Some("018f0c5a-0000-7000-8000-000000000558"),
            &workspace_root,
            "az/session/assets",
            123,
            test_registries(),
        )
        .unwrap();

        let gem_root = registration
            .source_roots
            .iter()
            .find(|root| root.portable_key == "gem:local.test-audio-system:assets")
            .expect("missing gem asset root should stay registered");
        assert_eq!(gem_root.owner_id, "local.test-audio-system");
        assert_eq!(gem_root.root, missing_assets);

        let db = AssetDb::open(&db_path).unwrap();
        let stored_roots = db.workspace_roots(registration.workspace_id).unwrap();
        assert!(stored_roots.iter().any(|root| {
            root.path == path_string(&missing_assets)
                && db
                    .root_by_id(root.root_pk)
                    .unwrap()
                    .is_some_and(|identity| identity.key == "gem:local.test-audio-system:assets")
        }));
        assert!(
            db.workspace_entry_page(registration.workspace_id, None, 0, 10)
                .unwrap()
                .is_empty(),
            "a missing auxiliary source root must reconcile as empty"
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn register_workspace_asset_source_roots_removes_stale_gem_roots() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assetdb.sqlite");
        let workspace_root = temp.path().join("workspace");
        let gem_root = workspace_root.join("gems").join("physics");

        let mut project =
            az_project::ProjectManifest::new("local.asset_root_replace", "Root Replace", "0.1.0");
        project.asset_roots[0].path = PathBuf::from("assets");
        project.gems.push(az_project::ProjectGem {
            id: "azoth.physics".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            linkage: None,
            path: Some(PathBuf::from("gems").join("physics")),
        });
        az_project::write_project_manifest(&workspace_root, &project).unwrap();
        std::fs::create_dir_all(workspace_root.join("assets")).unwrap();

        let mut gem = az_project::GemManifest::new("azoth.physics", "Physics", "0.1.0");
        gem.contributions.push(az_project::GemContribution::assets(
            "assets",
            "assets",
            [az_project::GemTargetRole::AssetProcessor],
        ));
        az_project::write_gem_manifest(&gem_root, &gem).unwrap();
        az_project::refresh_project_lock(&workspace_root).unwrap();
        std::fs::create_dir_all(gem_root.join("assets")).unwrap();

        let first = register_workspace_asset_source_roots_blocking(
            &db_path,
            "local.asset_root_replace",
            Some("018f0c5a-0000-7000-8000-000000000557"),
            &workspace_root,
            "az/session/assets",
            123,
            test_registries(),
        )
        .unwrap();
        assert_eq!(first.source_roots.len(), 2);

        project.gems.clear();
        write_project_manifest_with_lock(&workspace_root, &project);
        let second = register_workspace_asset_source_roots_blocking(
            &db_path,
            "local.asset_root_replace",
            Some("018f0c5a-0000-7000-8000-000000000557"),
            &workspace_root,
            "az/session/assets",
            124,
            test_registries(),
        )
        .unwrap();

        assert_eq!(second.workspace_id, first.workspace_id);
        assert_eq!(second.source_roots.len(), 1);
        assert!(
            second
                .source_roots
                .iter()
                .all(|root| root.portable_key != "gem:azoth.physics:assets")
        );

        let db = AssetDb::open(&db_path).unwrap();
        let stored_roots = db.workspace_roots(first.workspace_id).unwrap();
        assert_eq!(stored_roots.len(), 1);
        assert_eq!(
            db.root_by_id(stored_roots[0].root_pk).unwrap().unwrap().key,
            "project:local.asset_root_replace:assets"
        );
    }

    // capnp-rpc clients are single-threaded by construction and this runs inside
    // a `LocalSet`, so the future can only be `Send` if capnp-rpc grows a
    // thread-safe client.
    #[allow(clippy::future_not_send)]
    /// Drives one job all the way through the RPC surface: lease it, renew the
    /// lease, complete it with a product manifest, then read the result back.
    async fn assert_lease_renew_complete_and_inspect(
        client: &asset_capnp::asset_processor::Client,
        staging_root: &Path,
        job_key: &str,
    ) {
        let mut lease_request = client.lease_request();
        LeaseAssetJobRequest {
            capability: capability(),
            lease_owner: "worker-a".to_string(),
            lease_duration_ms: 30_000,
            staging_root: Some(path_string(staging_root)),
        }
        .to_capnp(lease_request.get().init_request())
        .unwrap();
        let lease_response = lease_request.send().promise.await.unwrap();
        let lease =
            LeaseAssetJobResult::from_capnp(lease_response.get().unwrap().get_result().unwrap())
                .unwrap();
        assert_eq!(lease.leased.job_key, job_key);

        let mut renew_request = client.renew_lease_request();
        RenewAssetJobLeaseRequest {
            capability: capability(),
            asset_job_attempt_id: lease.leased.attempt_id,
            lease_owner: "worker-a".to_string(),
            grant_key: lease.grant_key,
        }
        .to_capnp(renew_request.get().init_request())
        .unwrap();
        assert!(
            renew_request
                .send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_renewed()
        );

        let mut complete_request = client.complete_attempt_request();
        CompleteAssetJobAttemptRequest {
            capability: capability(),
            asset_job_attempt_id: lease.leased.attempt_id,
            lease_owner: "worker-a".to_string(),
            grant_key: lease.grant_key,
            status: AttemptStatus::Succeeded,
            finished_unix_ms: 400,
            error_count: 0,
            warning_count: 2,
            product_manifest: Some(product_manifest_handle(staging_root, "textures/rpc.dds")),
        }
        .to_capnp(complete_request.get().init_request())
        .unwrap();
        assert!(
            complete_request
                .send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_completed()
        );

        assert_attempt_and_catalog_report_the_product(client, lease.leased.attempt_id).await;
    }

    // capnp-rpc clients are single-threaded by construction and this runs inside
    // a `LocalSet`, so the future can only be `Send` if capnp-rpc grows a
    // thread-safe client.
    #[allow(clippy::future_not_send)]
    /// A completed attempt reports its product both through job inspection and
    /// through the platform product catalog.
    ///
    /// Those are two different durable reads of the same completion, so agreeing
    /// is what shows the commit reached both.
    async fn assert_attempt_and_catalog_report_the_product(
        client: &asset_capnp::asset_processor::Client,
        attempt_id: i64,
    ) {
        let mut inspect_request = client.inspect_job_request();
        InspectJobRequest {
            capability: editor_read_capability(),
            selector: InspectJobSelector::Attempt(attempt_id),
        }
        .to_capnp(inspect_request.get().init_request())
        .unwrap();
        let inspection = InspectJobResult::from_capnp(
            inspect_request
                .send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_result()
                .unwrap(),
        )
        .unwrap()
        .inspection
        .unwrap();
        assert_eq!(
            inspection.attempt.as_ref().unwrap().status,
            AttemptStatus::Succeeded
        );
        assert_eq!(inspection.attempt.as_ref().unwrap().warning_count, 2);
        assert_eq!(inspection.products.len(), 1);
        assert_eq!(inspection.products[0].path, "cache/textures/rpc.dds");
        assert_eq!(inspection.products[0].product_format, "az.test.raw");

        let mut catalog_request = client.catalog_products_request();
        CatalogProductsRequest {
            capability: editor_read_capability(),
            platform: "pc".to_owned(),
        }
        .to_capnp(catalog_request.get().init_request())
        .unwrap();
        let catalog_products = CatalogProductsResult::from_capnp(
            catalog_request
                .send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_result()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(catalog_products.entries.len(), 1);
        assert_eq!(
            catalog_products.entries[0].product_path,
            "cache/textures/rpc.dds"
        );
    }

    // capnp-rpc clients are single-threaded by construction and this runs inside
    // a `LocalSet`, so the future can only be `Send` if capnp-rpc grows a
    // thread-safe client.
    #[allow(clippy::future_not_send)]
    /// The completed product and the published catalog are both reachable
    /// through the release-content side channel.
    async fn assert_release_content_serves_product_and_catalog(
        client: &asset_capnp::asset_processor::Client,
    ) {
        let mut release_product_request = client.release_content_request();
        ReleaseContentReadRequest {
            capability: editor_read_capability(),
            platform: "pc".to_string(),
            target: ReleaseContentTarget::ProductAsset {
                asset_guid: Uuid::from_bytes([0x91; 16]),
                sub_id: 4,
            },
        }
        .to_capnp(release_product_request.get().init_request())
        .unwrap();
        let release_product = ReleaseContentReadResult::from_capnp(
            release_product_request
                .send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_result()
                .unwrap(),
        )
        .unwrap();
        let ReleaseContentReadResult::Product(product) = release_product else {
            panic!("expected release product result");
        };
        assert_eq!(
            std::fs::read(&product.payload.locator).unwrap(),
            b"rpc product bytes"
        );

        let mut release_catalog_request = client.release_content_request();
        ReleaseContentReadRequest {
            capability: editor_read_capability(),
            platform: "pc".to_string(),
            target: ReleaseContentTarget::AssetCatalog,
        }
        .to_capnp(release_catalog_request.get().init_request())
        .unwrap();
        let release_catalog = ReleaseContentReadResult::from_capnp(
            release_catalog_request
                .send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_result()
                .unwrap(),
        )
        .unwrap();
        let ReleaseContentReadResult::AssetCatalog(handle) = release_catalog else {
            panic!("expected release asset catalog result");
        };
        let catalog =
            az_asset::read_asset_catalog(std::fs::File::open(handle.locator).unwrap()).unwrap();
        assert_eq!(catalog.entries().len(), 1);
    }

    // capnp-rpc clients are single-threaded by construction and this runs inside
    // a `LocalSet`, so the future can only be `Send` if capnp-rpc grows a
    // thread-safe client.
    #[allow(clippy::future_not_send)]
    /// The builder catalog and the workspace entry page answer over RPC once a
    /// job has run, so an editor can see what the processor knows.
    async fn assert_builder_catalog_and_entry_page_are_visible(
        client: &asset_capnp::asset_processor::Client,
    ) {
        let mut builder_request = client.builder_catalog_request();
        AssetBuilderCatalogRequest {
            capability: editor_read_capability(),
        }
        .to_capnp(builder_request.get().init_request())
        .unwrap();
        let builders = AssetBuilderCatalogResult::from_capnp(
            builder_request
                .send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_result()
                .unwrap(),
        )
        .unwrap();
        assert!(!builders.builders.is_empty());

        let mut entries_request = client.workspace_entry_page_request();
        WorkspaceEntryPageRequest {
            capability: editor_read_capability(),
            root_scope: AssetRootScope::All,
            after_entry_id: None,
            page_size: 64,
        }
        .to_capnp(entries_request.get().init_request())
        .unwrap();
        let entries = WorkspaceEntryPageResult::from_capnp(
            entries_request
                .send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_result()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(entries.entries.len(), 1);
        assert_eq!(entries.entries[0].source_path, "textures/rpc.png");
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn asset_processor_rpc_routes_queue_lifecycle() {
        let fixture = fixture();
        let job = install_fixture_build_job(&fixture, fixture.builder_guid, "default");
        let rpc = Rc::new(AssetProcessorRpc::new(
            grant_backed_processor_with_builder_registry(
                fixture.db,
                registry_with_fixture_builder(),
            ),
        ));
        let client = AssetProcessorRpc::client_from_rc(&rpc);
        let temp = tempfile::tempdir().unwrap();
        let staging_root = temp.path().join("staging").join("attempt");
        write_test_asset_catalog(&fixture.project_data_paths);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&runtime, async {
            assert_lease_renew_complete_and_inspect(&client, &staging_root, &job.key).await;
            assert_release_content_serves_product_and_catalog(&client).await;
            assert_builder_catalog_and_entry_page_are_visible(&client).await;
        });
    }
}
