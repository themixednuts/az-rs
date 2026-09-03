//! Typed `AssetDB` repository and its single-writer command boundary.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use drizzle::core::expr::*;
use drizzle::sqlite::connection::SQLiteTransactionType;
use drizzle::sqlite::prelude::*;
use drizzle::sqlite::values::{SQLiteInsertValue, SQLiteUpdateValue};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::AssetDb;
use crate::connection::{AssetDbFutureExt, OpenError};
// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use crate::schema::*;
use crate::value::{
    Aliases, Coupling, Diff, Digest, Encoding, Exclusions, Registration, Relation, Status, Target,
    Work,
};

pub type RepoResult<T> = Result<T, RepoError>;

/// Per-Job facts folded into a dependents projection: the sorted set of product
/// paths each Job published, and the latest attempt id observed for each Job,
/// both keyed by Job id. The two maps are gathered in one chunked pass, so they
/// travel together.
type DependentJobFacts = (BTreeMap<i64, BTreeSet<String>>, BTreeMap<i64, i64>);

/// Keep generated `IN` predicates below `SQLite`'s adapter-independent bind
/// ceiling. Large fan-out reads are chunked here rather than delegated to AP.
const QUERY_BIND_BUDGET: usize = 512;

/// A durable writer transition that an in-process projection may consume after
/// its transaction commits.
///
/// These are facts about committed rows, not projection policy. Consumers own
/// their own freshness, retry, and filesystem state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostCommitEffect {
    CatalogInvalidated {
        workspace_pk: i64,
        platform: Option<String>,
    },
}

/// The outcome of advancing one post-commit-effect cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostCommitEffectDrain {
    Effects(Vec<PostCommitEffect>),
    /// The bounded retained history no longer reaches this cursor. A consumer
    /// must conservatively discard every projection it derived from effects.
    Gap,
}

const POST_COMMIT_EFFECT_RETENTION: usize = 256;

#[derive(Debug, Default)]
struct PostCommitEffectLog {
    next_revision: u64,
    retained: VecDeque<(u64, PostCommitEffect)>,
}

impl PostCommitEffectLog {
    fn record(&mut self, effect: PostCommitEffect) {
        let revision = self.next_revision;
        self.next_revision = self
            .next_revision
            .checked_add(1)
            .expect("post-commit effect revision overflow");
        self.retained.push_back((revision, effect));
        while self.retained.len() > POST_COMMIT_EFFECT_RETENTION {
            self.retained.pop_front();
        }
    }
}

/// A loss-detecting cursor over the process-local writer effect log.
///
/// A new subscriber begins at the current tail: initial projection state must
/// be derived from its authoritative read, not historical notifications.
pub struct PostCommitEffectSubscription {
    effects: Arc<Mutex<PostCommitEffectLog>>,
    next_revision: u64,
}

impl PostCommitEffectSubscription {
    fn new(effects: Arc<Mutex<PostCommitEffectLog>>) -> Self {
        let next_revision = effects
            .lock()
            .expect("post-commit effect log lock poisoned")
            .next_revision;
        Self {
            effects,
            next_revision,
        }
    }

    /// # Panics
    ///
    /// Panics if the post-commit effect log mutex is poisoned, which means a
    /// previous holder panicked while mutating the log.
    pub fn drain(&mut self) -> PostCommitEffectDrain {
        let effects = self
            .effects
            .lock()
            .expect("post-commit effect log lock poisoned");
        if effects
            .retained
            .front()
            .is_some_and(|(revision, _)| self.next_revision < *revision)
        {
            self.next_revision = effects.next_revision;
            return PostCommitEffectDrain::Gap;
        }
        let drained = effects
            .retained
            .iter()
            .filter(|(revision, _)| *revision >= self.next_revision)
            .map(|(_, effect)| effect.clone())
            .collect();
        self.next_revision = effects.next_revision;
        drop(effects);
        PostCommitEffectDrain::Effects(drained)
    }
}

#[derive(Debug, Error)]
pub enum RepoError {
    #[error("AssetDB storage operation failed: {0}")]
    Storage(String),
    #[error("AssetDB invariant failed: {0}")]
    Invariant(String),
    #[error("AssetDB writer is no longer available")]
    WriterStopped,
    #[error(transparent)]
    Open(#[from] OpenError),
}

// Passed directly to `Result::map_err`, which hands the error over by value, so this
// cannot take it by reference without wrapping every call site in a closure.
#[allow(clippy::needless_pass_by_value)]
fn storage(error: drizzle::error::DrizzleError) -> RepoError {
    RepoError::Storage(format!("{error:?}"))
}

trait DrizzleOptionalExt<T> {
    fn optional(self) -> Result<Option<T>, drizzle::error::DrizzleError>;
}

impl<T> DrizzleOptionalExt<T> for Result<T, drizzle::error::DrizzleError> {
    fn optional(self) -> Result<Option<T>, drizzle::error::DrizzleError> {
        match self {
            Ok(row) => Ok(Some(row)),
            Err(drizzle::error::DrizzleError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceKey {
    pub project: String,
    pub root: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterWorkspace {
    pub key: WorkspaceKey,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterWorkspaceRoot {
    pub workspace_pk: i64,
    pub key: String,
    pub owner: String,
    pub path: String,
    pub exclusions: Exclusions,
}

/// One desired portable-root binding in a workspace root-set replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRootRegistration {
    pub key: String,
    pub owner: String,
    pub path: String,
    pub exclusions: Exclusions,
}

/// Replaces the complete manifest-owned root set for one workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceWorkspaceRoots {
    pub workspace_pk: i64,
    pub roots: Vec<WorkspaceRootRegistration>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRootBinding {
    pub root: SelectRoots,
    pub policy: SelectWorkspaceRoots,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepEntry {
    pub path: String,
    pub guid: Uuid,
    pub schema: Option<String>,
    pub digest: Digest,
    pub diff: Diff,
    pub diagnostics: i64,
    pub updated: i64,
    pub src_bytes: i64,
    pub src_mtime: i64,
    pub meta_bytes: i64,
    pub meta_mtime: i64,
    pub observed: i64,
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepRemoval {
    pub path: String,
    pub observed: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepPlannerJob {
    pub key: String,
    pub platform: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepRecord {
    pub source: SweepEntry,
    pub planner: SweepPlannerJob,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplySweepDelta {
    pub workspace_pk: i64,
    pub workspace_root_pk: i64,
    pub records: Vec<SweepRecord>,
    pub removals: Vec<SweepRemoval>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepDeltaResult {
    pub inserted: u64,
    pub updated: u64,
    pub removed: u64,
    pub planned: u64,
    pub bound_job_edges: u64,
    pub bound_source_edges: u64,
    /// Stable Asset identities whose current source observation changed.
    /// The sweep owner uses this only for path-scoped dispatcher priority;
    /// root sweeps deliberately discard it.
    pub changed_assets: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuilderDescriptor {
    pub guid: Uuid,
    pub name: String,
    pub version: i64,
    pub digest: Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, SQLiteFromRow)]
struct SourceEdgeBuilderReference {
    #[column(SourceEdges::builder)]
    builder: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceBuilderCatalog {
    pub workspace_pk: i64,
    pub expected: Option<Digest>,
    pub replacement: Digest,
    pub builders: Vec<BuilderDescriptor>,
    pub plan_delta: PlanDelta,
    pub updated: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanDelta {
    pub retire_job_ids: Vec<i64>,
    pub retire_source_edge_ids: Vec<i64>,
    pub replacements: Vec<PlannedJob>,
    pub source_edges: Vec<SourceEdgeInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPlanDelta {
    pub workspace_pk: i64,
    pub delta: PlanDelta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEdgeInput {
    pub builder: Uuid,
    pub asset_pk: i64,
    pub depends_pk: Option<i64>,
    pub target: Target,
    pub relation: Relation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedJob {
    pub asset_pk: i64,
    pub kind: Work,
    pub builder: Option<Uuid>,
    pub key: String,
    pub platform: String,
    pub edges: Vec<JobEdgeInput>,
}

impl PlannedJob {
    #[must_use]
    pub fn plan(
        asset_pk: i64,
        key: impl Into<String>,
        platform: impl Into<String>,
        edges: Vec<JobEdgeInput>,
    ) -> Self {
        Self {
            asset_pk,
            kind: Work::Plan,
            builder: None,
            key: key.into(),
            platform: platform.into(),
            edges,
        }
    }

    #[must_use]
    pub fn build(
        asset_pk: i64,
        builder: Uuid,
        key: impl Into<String>,
        platform: impl Into<String>,
        edges: Vec<JobEdgeInput>,
    ) -> Self {
        Self {
            asset_pk,
            kind: Work::Build,
            builder: Some(builder),
            key: key.into(),
            platform: platform.into(),
            edges,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuilderCatalogReplaceOutcome {
    Unchanged,
    Replaced,
    Conflict { actual: Option<Digest> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReadyJob {
    pub job_id: i64,
    pub expected_attempts: i64,
    pub owner: String,
    /// The writer derives the persisted diagnostic expiry from this duration
    /// at its durable claim boundary, rather than trusting an RPC timestamp
    /// that may have waited behind other durable work.
    pub lease_duration_ms: u64,
    pub staging: String,
}

pub enum ClaimReadyJobResult {
    /// The joined claim projection is boxed because it is two orders of
    /// magnitude larger than `NoLongerClaimable`, and every caller moves the
    /// whole result through a channel before it ever inspects the variant.
    Claimed {
        context: Box<ClaimedJobContext>,
    },
    NoLongerClaimable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptFence {
    pub attempt_id: i64,
    pub owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbandonAttempts {
    pub attempts: Vec<AttemptFence>,
    pub finished: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AbandonAttemptsResult {
    pub requeued: Vec<i64>,
    pub exhausted: Vec<ExhaustedAttempt>,
    pub no_longer_owned: Vec<i64>,
}

pub const ATTEMPT_LIMIT_EXHAUSTED: &str = "asset_job_attempt_limit_exhausted";
pub const MAX_ASSET_JOB_ATTEMPTS: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExhaustedAttempt {
    pub job_id: i64,
    pub diagnostic: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductEdgeInput {
    pub guid: Uuid,
    pub sub_id: i64,
    pub flags: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductInput {
    pub asset_pk: i64,
    pub platform: String,
    pub sub_id: i64,
    pub path: String,
    pub kind: Uuid,
    pub format: String,
    pub version: i64,
    pub aliases: Aliases,
    pub registration: Registration,
    pub digest: Digest,
    pub bytes: i64,
    pub edges: Vec<ProductEdgeInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobEdgeInput {
    pub asset_pk: Option<i64>,
    pub target: Target,
    pub key: String,
    pub platform: String,
    pub coupling: Coupling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteAttempt {
    pub attempt_id: i64,
    pub owner: String,
    pub status: Status,
    pub finished: i64,
    pub errors: i64,
    pub warnings: i64,
    pub products: Vec<ProductInput>,
    /// Optional replacement for the owning Job's durable dependency edges.
    /// `None` preserves the plan-authored set; `Some` replaces it atomically.
    pub job_edges: Option<Vec<JobEdgeInput>>,
    /// Planner-only graph replacement committed with the Plan attempt.
    pub plan_delta: Option<PlanDelta>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteAttemptResult {
    Completed {
        job_id: i64,
        became_ready: Vec<i64>,
        /// Product formats atomically replaced by this completion.
        ///
        /// Post-commit projections use this durable fact to detect format
        /// removal without retaining a second product-history model.
        replaced_product_formats: BTreeSet<String>,
    },
    Abandoned {
        job_id: i64,
        retryable: bool,
        diagnostic: Option<&'static str>,
    },
    NoLongerOwned,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessingStatus {
    pub queued: u64,
    pub ready: u64,
    pub leased: u64,
    pub succeeded: u64,
    pub failed: u64,
}

impl ProcessingStatus {
    #[must_use]
    pub const fn active(self) -> bool {
        self.queued != 0 || self.leased != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogCursor {
    pub path: String,
    pub guid: Uuid,
    pub sub_id: i64,
}

pub struct CatalogPage {
    pub rows: Vec<SelectCatalog>,
    pub product_edges: Vec<CatalogProductEdge>,
    pub next: Option<CatalogCursor>,
}

pub struct CatalogProductEdge {
    pub edge: SelectProductEdges,
    pub target: Option<CatalogTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, SQLiteFromRow)]
pub struct CatalogTarget {
    #[column(Catalog::product_pk)]
    pub product_pk: i64,
    #[column(Catalog::job_pk)]
    pub job_pk: i64,
    #[column(Catalog::builder)]
    pub builder: Option<Uuid>,
    #[column(Catalog::guid)]
    pub guid: Uuid,
    #[column(Catalog::sub_id)]
    pub sub_id: i64,
    #[column(Catalog::source)]
    pub source: String,
    #[column(Catalog::schema)]
    pub schema: Option<String>,
    #[column(Catalog::kind)]
    pub kind: Uuid,
}

/// Canonical rows needed to stage one claimed job for a worker.
///
/// The scheduler owns the Job/Attempt transition; this projection only joins
/// stable source identity and optional saved authoring bytes after the claim.
pub struct ClaimedJobContext {
    /// Writer-thread monotonic instant captured immediately after the claim
    /// transaction has durably committed. This in-process value is the sole
    /// authority for dispatch lease deadlines.
    pub claimed_at: Instant,
    /// Writer-owned wall clock captured with the durable claim transition.
    /// It remains diagnostic/persisted data only; dispatch never translates
    /// it into a monotonic deadline.
    pub claimed_unix_ms: i64,
    pub job: SelectJobs,
    pub attempt: SelectAttempts,
    pub asset: SelectAssets,
    pub entry: SelectEntries,
    pub root: SelectRoots,
    pub workspace_root: SelectWorkspaceRoots,
    pub payload: Option<SelectPayloads>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobInspectionSelector {
    Job(i64),
    Attempt(i64),
}

pub struct InspectedProduct {
    pub product: SelectProducts,
    pub edges: Vec<SelectProductEdges>,
}

pub struct JobInspection {
    pub job: SelectJobs,
    pub attempt: Option<SelectAttempts>,
    pub asset: SelectAssets,
    pub entry: SelectEntries,
    pub workspace_root: SelectWorkspaceRoots,
    pub products: Vec<InspectedProduct>,
    pub edges: Vec<SelectJobEdges>,
}

pub const UNSATISFIABLE_DEPENDENCY: &str = "asset_job_unsatisfiable_dependency";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveIdleBlocked {
    pub workspace_pk: i64,
    pub job_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleFailedJob {
    pub job_id: i64,
    pub platform: String,
    pub diagnostic: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolveIdleBlockedResult {
    pub dropped_order_only_edges: Vec<i64>,
    pub failed_jobs: Vec<IdleFailedJob>,
    pub became_ready: Vec<i64>,
    pub unchanged: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsavedPayload {
    pub workspace: RecoveredWorkspace,
    pub root: RecoveredRoot,
    pub path: String,
    pub document: String,
    pub schema: String,
    pub encoding: Encoding,
    pub revision: i64,
    pub saved: Option<i64>,
    pub digest: Digest,
    pub bytes: i64,
    pub payload: Vec<u8>,
    pub checkpoint: Option<Vec<u8>>,
    pub session: Option<String>,
    pub project: String,
    pub deleted: bool,
    pub created: i64,
    pub updated: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveredWorkspace {
    pub key: WorkspaceKey,
    pub created: i64,
    pub updated: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveredRoot {
    pub key: String,
    pub owner: String,
    pub path: String,
    pub exclusions: Exclusions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportUnsavedPayload {
    pub payload: UnsavedPayload,
    pub expected: ExpectedPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedPayload {
    Absent,
    SavedAt { revision: i64, digest: Digest },
}

pub enum ImportRecoveredPayloadResult {
    Imported(SelectPayloads),
    AlreadyPresent(SelectPayloads),
    BaselineConflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteSourcePayload {
    pub workspace_pk: i64,
    pub root_pk: i64,
    pub path: String,
    pub document: String,
    pub schema: String,
    pub encoding: Encoding,
    pub expected_revision: Option<i64>,
    pub revision: i64,
    pub saved: Option<i64>,
    pub digest: Digest,
    pub payload: Vec<u8>,
    pub checkpoint: CheckpointWrite,
    pub session: Option<String>,
    pub project: String,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointWrite {
    /// Keep the checkpoint already stored for an existing payload row. This is
    /// rejected when creating a row because there is no prior image to keep.
    Preserve,
    /// Replace the saved checkpoint with this explicit immutable image.
    Replace(Vec<u8>),
    /// Remove the saved checkpoint. A command that also declares a saved
    /// revision cannot clear its corresponding saved image.
    Clear,
}

/// Durable outcome of the payload revision compare-and-set.
pub enum WriteSourcePayloadResult {
    /// The revision and requested checkpoint policy were committed.
    Written(SelectPayloads),
    /// A row exists, but its revision differs from `expected_revision`.
    Conflict(SelectPayloads),
    /// The command expected an existing revision, but no payload row exists.
    Missing,
    /// Workspace, root policy, or project identity did not form one authority.
    ScopeMismatch,
    /// The checkpoint policy cannot describe the requested saved state.
    InvalidCheckpoint,
}

/// One authored-source publication admitted by the `AssetDB` writer.
///
/// The payload revision and its source Asset/Path/Entry observation commit in
/// one transaction. Filesystem staging and compensation remain adapter-owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishAuthoredSource {
    pub payload: WriteSourcePayload,
    pub workspace_root_pk: i64,
    pub source: SweepEntry,
}

/// The three durable rows one authored publication commits together.
///
/// Asset identity, the workspace Entry projection of it, and the payload
/// revision are written inside a single transaction and are only ever handed
/// back as that one unit, so they are named together and boxed to keep the
/// refusal outcomes of `PublishAuthoredSourceResult` cheap to return.
pub struct PublishedAuthoredSource {
    pub asset: SelectAssets,
    pub entry: SelectEntries,
    pub payload: SelectPayloads,
}

pub enum PublishAuthoredSourceResult {
    Written(Box<PublishedAuthoredSource>),
    /// Boxed for the same reason as `Written`: a payload row carries the whole
    /// document and checkpoint blobs, dwarfing every refusal variant.
    Conflict(Box<SelectPayloads>),
    Missing,
    ScopeMismatch,
    InvalidCheckpoint,
    InvalidSourceProjection,
    LocatorConflict {
        asset: SelectAssets,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceStateToken {
    pub revision: Option<i64>,
    pub digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveSource {
    pub workspace_pk: i64,
    pub root_pk: i64,
    pub from: String,
    pub to: String,
    pub expected: SourceStateToken,
    pub now: i64,
}

/// The Asset identity and workspace Entry projection one relocation commits.
///
/// A move rewrites identity and locator inside one transaction, so both rows
/// only ever leave the writer together; naming them lets the success variant
/// be boxed and keeps the refusal variants pointer-sized.
pub struct MovedSource {
    pub asset: SelectAssets,
    pub entry: SelectEntries,
}

pub enum MoveSourceResult {
    Moved(Box<MovedSource>),
    Conflict,
    Unsaved,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteSource {
    pub workspace_pk: i64,
    pub root_pk: i64,
    pub path: String,
    pub expected: SourceStateToken,
    pub now: i64,
}

/// The Asset identity and workspace Entry projection one deletion retires.
///
/// The retirement of the locator and the identity row it belonged to commit in
/// one transaction, so both rows only ever leave the writer together; naming
/// them lets the success variant be boxed and keeps the refusals pointer-sized.
pub struct DeletedSource {
    pub asset: SelectAssets,
    pub entry: SelectEntries,
}

pub enum DeleteSourceResult {
    Deleted(Box<DeletedSource>),
    Conflict,
    Unsaved,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDependentsInput {
    pub workspace_pk: i64,
    pub asset_pk: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, SQLiteFromRow)]
pub struct SourceDependentSource {
    #[column(SourceEdges::source_edge_id)]
    pub edge_id: i64,
    #[column(SourceEdges::relation)]
    pub relation: Relation,
    #[column(SourceEdges::builder)]
    pub builder: Uuid,
    #[column(Entries::path)]
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SQLiteFromRow)]
struct SourceDependentJobRow {
    #[column(JobEdges::job_edge_id)]
    pub edge_id: i64,
    #[column(Entries::path)]
    pub source_path: String,
    #[column(Jobs::kind)]
    pub kind: Work,
    #[column(Jobs::builder)]
    pub builder: Option<Uuid>,
    #[column(Jobs::key)]
    pub job_key: String,
    #[column(Jobs::platform)]
    pub platform: String,
    #[column(JobEdges::key)]
    pub dependency_job_key: String,
    #[column(JobEdges::platform)]
    pub dependency_platform: String,
    #[column(JobEdges::coupling)]
    pub coupling: Coupling,
    #[column(Jobs::job_id)]
    pub job_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDependentJob {
    pub edge_id: i64,
    pub job_id: i64,
    pub source_path: String,
    pub kind: Work,
    pub builder: Option<Uuid>,
    pub job_key: String,
    pub platform: String,
    pub dependency_job_key: String,
    pub dependency_platform: String,
    pub coupling: Coupling,
    /// The latest durable execution of the owning Job. `None` means the Job
    /// has not been leased yet.
    pub latest_attempt_id: Option<i64>,
    pub product_paths: Vec<String>,
}

pub struct SourceDependents {
    pub sources: Vec<SourceDependentSource>,
    pub jobs: Vec<SourceDependentJob>,
}

#[derive(Debug, Clone, PartialEq, Eq, SQLiteFromRow)]
struct WorkspaceEntrySnapshotRow {
    #[column(Entries::entry_id)]
    entry_id: i64,
    #[column(Entries::workspace_pk)]
    workspace_pk: i64,
    #[column(Assets::asset_id)]
    asset_pk: i64,
    #[column(Assets::guid)]
    asset_guid: Uuid,
    #[column(Entries::root_pk)]
    root_pk: i64,
    #[column(WorkspaceRoots::path)]
    source_root: String,
    #[column(Entries::path)]
    source_path: String,
    #[column(Entries::schema)]
    schema: Option<String>,
    #[column(Entries::digest)]
    digest: Digest,
    #[column(Entries::diff)]
    diff: Diff,
    #[column(Entries::diagnostics)]
    diagnostics: i64,
    #[column(Entries::updated)]
    updated: i64,
    #[column(Entries::src_bytes)]
    src_bytes: i64,
    #[column(Entries::src_mtime)]
    src_mtime: i64,
    #[column(Entries::meta_bytes)]
    meta_bytes: i64,
    #[column(Entries::meta_mtime)]
    meta_mtime: i64,
}

impl WorkspaceEntrySnapshotRow {
    fn projection() -> Self {
        Self {
            entry_id: 0,
            workspace_pk: 0,
            asset_pk: 0,
            asset_guid: Uuid::nil(),
            root_pk: 0,
            source_root: String::new(),
            source_path: String::new(),
            schema: None,
            digest: Digest::from(blake3::hash(b"workspace-entry-projection")),
            diff: Diff::default(),
            diagnostics: 0,
            updated: 0,
            src_bytes: 0,
            src_mtime: 0,
            meta_bytes: 0,
            meta_mtime: 0,
        }
    }
}

pub struct WorkspaceEntrySnapshot {
    pub entry_id: i64,
    pub workspace_pk: i64,
    pub asset_pk: i64,
    pub asset_guid: Uuid,
    pub root_pk: i64,
    pub source_root: String,
    pub source_path: String,
    pub schema: Option<String>,
    pub digest: Digest,
    pub diff: Diff,
    pub diagnostics: i64,
    pub updated: i64,
    pub src_bytes: i64,
    pub src_mtime: i64,
    pub meta_bytes: i64,
    pub meta_mtime: i64,
    pub jobs: Vec<JobActivitySnapshot>,
}

pub struct JobActivitySnapshot {
    pub job: SelectJobs,
    pub attempt: Option<SelectAttempts>,
}

enum WriterCommand {
    RegisterWorkspace(
        RegisterWorkspace,
        oneshot::Sender<RepoResult<SelectWorkspaces>>,
    ),
    RegisterWorkspaceRoot(
        RegisterWorkspaceRoot,
        oneshot::Sender<RepoResult<(SelectRoots, SelectWorkspaceRoots)>>,
    ),
    ReplaceWorkspaceRoots(
        ReplaceWorkspaceRoots,
        oneshot::Sender<RepoResult<Vec<WorkspaceRootBinding>>>,
    ),
    ApplySweep(
        ApplySweepDelta,
        oneshot::Sender<RepoResult<SweepDeltaResult>>,
    ),
    ReplaceBuilders(
        ReplaceBuilderCatalog,
        oneshot::Sender<RepoResult<BuilderCatalogReplaceOutcome>>,
    ),
    ApplyPlan(ApplyPlanDelta, oneshot::Sender<RepoResult<()>>),
    Claim(
        ClaimReadyJob,
        oneshot::Sender<RepoResult<ClaimReadyJobResult>>,
    ),
    Abandon(
        AbandonAttempts,
        oneshot::Sender<RepoResult<AbandonAttemptsResult>>,
    ),
    Complete(
        CompleteAttempt,
        oneshot::Sender<RepoResult<CompleteAttemptResult>>,
    ),
    ResolveIdle(
        ResolveIdleBlocked,
        oneshot::Sender<RepoResult<ResolveIdleBlockedResult>>,
    ),
    ImportPayload(
        ImportUnsavedPayload,
        oneshot::Sender<RepoResult<ImportRecoveredPayloadResult>>,
    ),
    WritePayload(
        WriteSourcePayload,
        oneshot::Sender<RepoResult<WriteSourcePayloadResult>>,
    ),
    PublishAuthoredSource(
        PublishAuthoredSource,
        oneshot::Sender<RepoResult<PublishAuthoredSourceResult>>,
    ),
    MoveSource(MoveSource, oneshot::Sender<RepoResult<MoveSourceResult>>),
    DeleteSource(
        DeleteSource,
        oneshot::Sender<RepoResult<DeleteSourceResult>>,
    ),
    #[cfg(any(test, feature = "test-support"))]
    TestBarrier {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        reply: oneshot::Sender<RepoResult<()>>,
    },
}

struct WriterInner {
    sender: Mutex<Option<mpsc::Sender<WriterCommand>>>,
    join: Mutex<Option<JoinHandle<()>>>,
    effects: Arc<Mutex<PostCommitEffectLog>>,
}

impl Drop for WriterInner {
    fn drop(&mut self) {
        self.sender
            .get_mut()
            .expect("writer sender lock poisoned")
            .take();
        if let Some(join) = self
            .join
            .get_mut()
            .expect("writer join lock poisoned")
            .take()
        {
            let _ = join.join();
        }
    }
}

#[derive(Clone)]
pub struct AssetDbWriter {
    inner: Arc<WriterInner>,
}

/// Awaitable result of an already-enqueued writer command.
///
/// Dropping this value only abandons observation of the result; it does not
/// cancel or roll back the durable command. Synchronous maintenance and test
/// boundaries may opt into [`Self::wait_blocking`].
pub struct WriterReply<T> {
    receive: oneshot::Receiver<RepoResult<T>>,
}

impl<T> Future for WriterReply<T> {
    type Output = RepoResult<T>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.receive).poll(context) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(_)) => Poll::Ready(Err(RepoError::WriterStopped)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T> WriterReply<T> {
    /// # Errors
    ///
    /// Returns [`RepoError::WriterStopped`] if the writer task shut down before
    /// replying, plus whatever error the writer itself returned.
    pub fn wait_blocking(self) -> RepoResult<T> {
        self.receive
            .blocking_recv()
            .map_err(|_| RepoError::WriterStopped)?
    }
}

impl std::fmt::Debug for AssetDbWriter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AssetDbWriter")
            .finish_non_exhaustive()
    }
}

impl AssetDbWriter {
    pub(crate) fn has_other_handles(&self) -> bool {
        Arc::strong_count(&self.inner) != 1
    }

    fn start(db: &AssetDb) -> RepoResult<Self> {
        let writer_db = db.new_writer_handle()?;
        let (sender, receiver) = mpsc::channel();
        let effects = Arc::new(Mutex::new(PostCommitEffectLog::default()));
        let writer_effects = Arc::clone(&effects);
        let join = std::thread::Builder::new()
            .name("az-assetdb-writer".to_owned())
            .spawn(move || writer_loop(writer_db, receiver, writer_effects))
            .map_err(|error| RepoError::Invariant(format!("spawn AssetDB writer: {error}")))?;
        Ok(Self {
            inner: Arc::new(WriterInner {
                sender: Mutex::new(Some(sender)),
                join: Mutex::new(Some(join)),
                effects,
            }),
        })
    }

    /// Subscribe to committed repository facts without making a projection
    /// part of `AssetDB`'s connection state.
    #[must_use]
    pub fn subscribe_post_commit_effects(&self) -> PostCommitEffectSubscription {
        PostCommitEffectSubscription::new(Arc::clone(&self.inner.effects))
    }

    /// Deterministically inject a committed effect for a projection-owner
    /// test. Production effects are recorded only by the writer thread after
    /// their transaction succeeds.
    #[cfg(any(test, feature = "test-support"))]
    pub fn record_post_commit_effect_for_test(&self, effect: PostCommitEffect) {
        record_post_commit_effect(&self.inner.effects, effect);
    }

    fn send<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<RepoResult<T>>) -> WriterCommand,
    ) -> WriterReply<T> {
        let (reply, receive) = oneshot::channel();
        let command = build(reply);
        if let Some(sender) = self
            .inner
            .sender
            .lock()
            .expect("writer sender lock poisoned")
            .as_ref()
        {
            let _ = sender.send(command);
        }
        WriterReply { receive }
    }

    #[must_use]
    pub fn register_workspace(&self, input: RegisterWorkspace) -> WriterReply<SelectWorkspaces> {
        self.send(|reply| WriterCommand::RegisterWorkspace(input, reply))
    }

    #[must_use]
    pub fn register_workspace_root(
        &self,
        input: RegisterWorkspaceRoot,
    ) -> WriterReply<(SelectRoots, SelectWorkspaceRoots)> {
        self.send(|reply| WriterCommand::RegisterWorkspaceRoot(input, reply))
    }

    #[must_use]
    pub fn replace_workspace_roots(
        &self,
        input: ReplaceWorkspaceRoots,
    ) -> WriterReply<Vec<WorkspaceRootBinding>> {
        self.send(|reply| WriterCommand::ReplaceWorkspaceRoots(input, reply))
    }

    #[must_use]
    pub fn apply_sweep_delta(&self, input: ApplySweepDelta) -> WriterReply<SweepDeltaResult> {
        self.send(|reply| WriterCommand::ApplySweep(input, reply))
    }

    #[must_use]
    pub fn replace_builder_catalog(
        &self,
        input: ReplaceBuilderCatalog,
    ) -> WriterReply<BuilderCatalogReplaceOutcome> {
        self.send(|reply| WriterCommand::ReplaceBuilders(input, reply))
    }

    #[must_use]
    pub fn apply_plan_delta(&self, input: ApplyPlanDelta) -> WriterReply<()> {
        self.send(|reply| WriterCommand::ApplyPlan(input, reply))
    }

    #[must_use]
    pub fn claim_ready_job(&self, input: ClaimReadyJob) -> WriterReply<ClaimReadyJobResult> {
        self.send(|reply| WriterCommand::Claim(input, reply))
    }

    #[must_use]
    pub fn abandon_attempts(&self, input: AbandonAttempts) -> WriterReply<AbandonAttemptsResult> {
        self.send(|reply| WriterCommand::Abandon(input, reply))
    }

    #[must_use]
    pub fn complete_attempt(&self, input: CompleteAttempt) -> WriterReply<CompleteAttemptResult> {
        self.send(|reply| WriterCommand::Complete(input, reply))
    }

    #[must_use]
    pub fn resolve_idle_blocked(
        &self,
        input: ResolveIdleBlocked,
    ) -> WriterReply<ResolveIdleBlockedResult> {
        self.send(|reply| WriterCommand::ResolveIdle(input, reply))
    }

    #[must_use]
    pub fn import_unsaved_payload(
        &self,
        input: ImportUnsavedPayload,
    ) -> WriterReply<ImportRecoveredPayloadResult> {
        self.send(|reply| WriterCommand::ImportPayload(input, reply))
    }

    #[must_use]
    pub fn write_source_payload(
        &self,
        input: WriteSourcePayload,
    ) -> WriterReply<WriteSourcePayloadResult> {
        self.send(|reply| WriterCommand::WritePayload(input, reply))
    }

    #[must_use]
    pub fn publish_authored_source(
        &self,
        input: PublishAuthoredSource,
    ) -> WriterReply<PublishAuthoredSourceResult> {
        self.send(|reply| WriterCommand::PublishAuthoredSource(input, reply))
    }

    #[must_use]
    pub fn move_source(&self, input: MoveSource) -> WriterReply<MoveSourceResult> {
        self.send(|reply| WriterCommand::MoveSource(input, reply))
    }

    #[must_use]
    pub fn delete_source(&self, input: DeleteSource) -> WriterReply<DeleteSourceResult> {
        self.send(|reply| WriterCommand::DeleteSource(input, reply))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn test_barrier(
        &self,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    ) -> WriterReply<()> {
        self.send(|reply| WriterCommand::TestBarrier {
            entered,
            release,
            reply,
        })
    }
}

impl AssetDb {
    /// Test-support census used to bind an in-process processor to the single
    /// workspace created by its fixture. Production composition carries the
    /// registered workspace explicitly and never discovers authority this way.
    ///
    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the workspace query fails.
    #[cfg(any(test, feature = "test-support"))]
    pub fn workspaces_for_test(&self) -> RepoResult<Vec<SelectWorkspaces>> {
        self.drizzle
            .select(())
            .from(self.tables.workspaces)
            .order_by(asc(self.tables.workspaces.workspace_id))
            .all()
            .wait()
            .map_err(storage)
    }

    /// Obtain the composition-owned writer. Every handle sharing this database
    /// owner resolves the same writer instance.
    ///
    /// # Errors
    ///
    /// Returns [`RepoError::Invariant`] if the writer thread cannot be spawned.
    ///
    /// # Panics
    ///
    /// Panics if the writer cell is empty immediately after being initialized,
    /// which cannot happen: the `set` above either stores this candidate or
    /// loses a race to another thread that stored its own.
    pub fn writer(&self) -> RepoResult<AssetDbWriter> {
        if let Some(writer) = self.writer.get() {
            return Ok(writer.clone());
        }
        let candidate = AssetDbWriter::start(self)?;
        let _ = self.writer.set(candidate);
        Ok(self.writer.get().expect("writer initialized").clone())
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the workspace lookup query fails.
    pub fn workspace(&self, key: &WorkspaceKey) -> RepoResult<Option<SelectWorkspaces>> {
        let table = self.tables.workspaces;
        let workspace: Option<SelectWorkspaces> = self
            .drizzle
            .select(())
            .from(table)
            .r#where(and(
                eq(table.project, key.project.as_str()),
                and(
                    eq(table.root, key.root.as_str()),
                    eq(table.branch, key.branch.as_str()),
                ),
            ))
            .get()
            .wait()
            .optional()
            .map_err(storage)?;
        Ok(workspace)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the workspace-root query fails.
    pub fn workspace_roots(&self, workspace_pk: i64) -> RepoResult<Vec<SelectWorkspaceRoots>> {
        let table = self.tables.workspace_roots;
        self.drizzle
            .select(())
            .from(table)
            .r#where(eq(table.workspace_pk, workspace_pk))
            .order_by([asc(table.workspace_root_id)])
            .all()
            .wait()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Invariant`] if `workspace_root_pk` names no workspace
    /// root, and [`RepoError::Storage`] if either query fails.
    pub fn ordered_entries(&self, workspace_root_pk: i64) -> RepoResult<Vec<SelectEntries>> {
        let workspace_roots = self.tables.workspace_roots;
        let policy: Option<SelectWorkspaceRoots> = self
            .drizzle
            .select(())
            .from(workspace_roots)
            .r#where(eq(workspace_roots.workspace_root_id, workspace_root_pk))
            .get()
            .wait()
            .optional()
            .map_err(storage)?;
        let policy = policy
            .ok_or_else(|| RepoError::Invariant("workspace root does not exist".to_owned()))?;
        let table = self.tables.entries;
        self.drizzle
            .select(())
            .from(table)
            .r#where(and(
                eq(table.workspace_pk, policy.workspace_pk),
                eq(table.root_pk, policy.root_pk),
            ))
            .order_by([asc(table.path), asc(table.entry_id)])
            .all()
            .wait()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the ready-job query fails.
    pub fn ready_jobs(
        &self,
        workspace_pk: i64,
        kind: Work,
        after_job_id: i64,
        limit: u32,
    ) -> RepoResult<Vec<SelectJobs>> {
        let table = self.tables.jobs;
        self.drizzle
            .select(())
            .from(table)
            .r#where(and(
                eq(table.workspace_pk, workspace_pk),
                and(
                    eq(table.kind, kind),
                    and(
                        eq(table.status, Status::Queued),
                        and(eq(table.ready, true), gt(table.job_id, after_job_id)),
                    ),
                ),
            ))
            .order_by([asc(table.job_id)])
            .limit(i64::from(limit))
            .all()
            .wait()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the blocked-job page query fails.
    pub fn blocked_jobs_page(
        &self,
        workspace_pk: i64,
        after_job_id: i64,
        limit: u32,
    ) -> RepoResult<Vec<SelectJobs>> {
        let table = self.tables.jobs;
        self.drizzle
            .select(())
            .from(table)
            .r#where(and(
                eq(table.workspace_pk, workspace_pk),
                and(
                    eq(table.status, Status::Queued),
                    and(eq(table.ready, false), gt(table.job_id, after_job_id)),
                ),
            ))
            .order_by([asc(table.job_id)])
            .limit(i64::from(limit))
            .all()
            .wait()
            .map_err(storage)
    }

    /// Bounded startup recovery page. Runtime expiry is dispatcher-owned and
    /// submits the returned attempt/owner fences through `abandon_attempts`.
    ///
    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the expired-attempt query fails.
    pub fn expired_attempts(
        &self,
        expires_through: i64,
        after_attempt_id: i64,
        limit: u32,
    ) -> RepoResult<Vec<SelectAttempts>> {
        let attempts = self.tables.attempts;
        self.drizzle
            .select(())
            .from(attempts)
            .r#where(and(
                eq(attempts.status, Status::Leased),
                and(
                    lte(attempts.expires, Some(expires_through)),
                    gt(attempts.attempt_id, after_attempt_id),
                ),
            ))
            .order_by([asc(attempts.attempt_id)])
            .limit(i64::from(limit))
            .all()
            .wait()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if any of the five status counts fails.
    pub fn processing_status(
        &self,
        workspace_pk: i64,
        platform: Option<&str>,
    ) -> RepoResult<ProcessingStatus> {
        Ok(ProcessingStatus {
            queued: count_jobs(self, workspace_pk, platform, Status::Queued, None)?,
            ready: count_jobs(self, workspace_pk, platform, Status::Queued, Some(true))?,
            leased: count_jobs(self, workspace_pk, platform, Status::Leased, None)?,
            succeeded: count_jobs(self, workspace_pk, platform, Status::Succeeded, None)?,
            failed: count_jobs(self, workspace_pk, platform, Status::Failed, None)?,
        })
    }

    /// Resolves the asset identity behind every product-edge target, paged under the
    /// query bind budget so one catalog page cannot exceed it.
    fn catalog_target_identities(
        &self,
        product_edges: &[SelectProductEdges],
        workspace_pk: i64,
        platform: &str,
    ) -> RepoResult<BTreeMap<(Uuid, i64), CatalogTarget>> {
        let table = self.tables.catalog;
        let mut targets_by_identity = BTreeMap::new();
        let target_guids = product_edges
            .iter()
            .map(|edge| edge.guid)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        for guid_page in target_guids.chunks(QUERY_BIND_BUDGET) {
            let targets: Vec<CatalogTarget> = self
                .drizzle
                .select(CatalogTarget {
                    product_pk: 0,
                    job_pk: 0,
                    builder: None,
                    guid: Uuid::nil(),
                    sub_id: 0,
                    source: String::new(),
                    schema: None,
                    kind: Uuid::nil(),
                })
                .from(table)
                .r#where(and(
                    and(
                        eq(table.workspace_pk, workspace_pk),
                        eq(table.platform, platform),
                    ),
                    in_array(table.guid, guid_page.to_vec()),
                ))
                .all()
                .wait()
                .map_err(storage)?;
            for target in targets {
                targets_by_identity.insert((target.guid, target.sub_id), target);
            }
        }
        Ok(targets_by_identity)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the catalog page query fails.
    pub fn catalog_page(
        &self,
        workspace_pk: i64,
        platform: &str,
        after: Option<&CatalogCursor>,
        limit: u32,
    ) -> RepoResult<CatalogPage> {
        let table = self.tables.catalog;
        let rows: Vec<SelectCatalog> = if let Some(after) = after {
            self.drizzle
                .select(())
                .from(table)
                .r#where(and(
                    and(
                        eq(table.workspace_pk, workspace_pk),
                        eq(table.platform, platform),
                    ),
                    or(
                        gt(table.path, after.path.as_str()),
                        and(
                            eq(table.path, after.path.as_str()),
                            or(
                                gt(table.guid, after.guid),
                                and(eq(table.guid, after.guid), gt(table.sub_id, after.sub_id)),
                            ),
                        ),
                    ),
                ))
                .order_by([asc(table.path), asc(table.guid), asc(table.sub_id)])
                .limit(i64::from(limit))
                .all()
                .wait()
                .map_err(storage)?
        } else {
            self.drizzle
                .select(())
                .from(table)
                .r#where(and(
                    eq(table.workspace_pk, workspace_pk),
                    eq(table.platform, platform),
                ))
                .order_by([asc(table.path), asc(table.guid), asc(table.sub_id)])
                .limit(i64::from(limit))
                .all()
                .wait()
                .map_err(storage)?
        };
        let next = rows.last().map(|row| CatalogCursor {
            path: row.path.clone(),
            guid: row.guid,
            sub_id: row.sub_id,
        });
        let product_edges: Vec<SelectProductEdges> = if rows.is_empty() {
            Vec::new()
        } else {
            let product_ids = rows.iter().map(|row| row.product_pk).collect::<Vec<_>>();
            self.drizzle
                .select(())
                .from(self.tables.product_edges)
                .r#where(in_array(self.tables.product_edges.product_pk, product_ids))
                .order_by([
                    asc(self.tables.product_edges.product_pk),
                    asc(self.tables.product_edges.product_edge_id),
                ])
                .all()
                .wait()
                .map_err(storage)?
        };
        let targets_by_identity =
            self.catalog_target_identities(&product_edges, workspace_pk, platform)?;
        let product_edges = product_edges
            .into_iter()
            .map(|edge| CatalogProductEdge {
                target: targets_by_identity.get(&(edge.guid, edge.sub_id)).cloned(),
                edge,
            })
            .collect();
        Ok(CatalogPage {
            rows,
            product_edges,
            next,
        })
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the catalog count query fails.
    pub fn catalog_count(&self, workspace_pk: i64, platform: &str) -> RepoResult<u64> {
        let table = self.tables.catalog;
        let count = self
            .drizzle
            .select(count(table.product_pk))
            .from(table)
            .r#where(and(
                eq(table.workspace_pk, workspace_pk),
                eq(table.platform, platform),
            ))
            .get::<i64, _, _>()
            .wait()
            .map_err(storage)?;
        Ok(count.cast_unsigned())
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the workspace lookup query fails.
    pub fn workspace_by_id(&self, workspace_pk: i64) -> RepoResult<Option<SelectWorkspaces>> {
        self.drizzle
            .select(())
            .from(self.tables.workspaces)
            .r#where(eq(self.tables.workspaces.workspace_id, workspace_pk))
            .get()
            .wait()
            .optional()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the workspace-root lookup query fails.
    pub fn workspace_root_by_id(
        &self,
        workspace_root_pk: i64,
    ) -> RepoResult<Option<SelectWorkspaceRoots>> {
        self.drizzle
            .select(())
            .from(self.tables.workspace_roots)
            .r#where(eq(
                self.tables.workspace_roots.workspace_root_id,
                workspace_root_pk,
            ))
            .get()
            .wait()
            .optional()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the workspace-root lookup query fails.
    pub fn workspace_root_for_root(
        &self,
        workspace_pk: i64,
        root_pk: i64,
    ) -> RepoResult<Option<SelectWorkspaceRoots>> {
        self.drizzle
            .select(())
            .from(self.tables.workspace_roots)
            .r#where(and(
                eq(self.tables.workspace_roots.workspace_pk, workspace_pk),
                eq(self.tables.workspace_roots.root_pk, root_pk),
            ))
            .get()
            .wait()
            .optional()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the root lookup query fails.
    pub fn root_by_id(&self, root_pk: i64) -> RepoResult<Option<SelectRoots>> {
        self.drizzle
            .select(())
            .from(self.tables.roots)
            .r#where(eq(self.tables.roots.root_id, root_pk))
            .get()
            .wait()
            .optional()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the asset lookup query fails.
    pub fn asset_by_id(&self, asset_pk: i64) -> RepoResult<Option<SelectAssets>> {
        self.drizzle
            .select(())
            .from(self.tables.assets)
            .r#where(eq(self.tables.assets.asset_id, asset_pk))
            .get()
            .wait()
            .optional()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the workspace-entry lookup query fails.
    pub fn entry_by_asset(
        &self,
        workspace_pk: i64,
        asset_pk: i64,
    ) -> RepoResult<Option<SelectEntries>> {
        self.drizzle
            .select(())
            .from(self.tables.entries)
            .r#where(and(
                eq(self.tables.entries.workspace_pk, workspace_pk),
                eq(self.tables.entries.asset_pk, asset_pk),
            ))
            .get()
            .wait()
            .optional()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns any error [`Self::asset_by_id`] returns, and
    /// [`RepoError::Storage`] if the source lookup fails.
    pub fn source_asset(
        &self,
        workspace_pk: i64,
        root_pk: i64,
        path: &str,
    ) -> RepoResult<Option<(SelectAssets, SelectEntries)>> {
        let entry: Option<SelectEntries> = self
            .drizzle
            .select(())
            .from(self.tables.entries)
            .r#where(and(
                eq(self.tables.entries.workspace_pk, workspace_pk),
                and(
                    eq(self.tables.entries.root_pk, root_pk),
                    eq(self.tables.entries.path, path),
                ),
            ))
            .get()
            .wait()
            .optional()
            .map_err(storage)?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        if entry.diff == Diff::Deleted {
            return Ok(None);
        }
        let asset = self.asset_by_id(entry.asset_pk)?;
        Ok(asset.map(|asset| (asset, entry)))
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the payload lookup query fails.
    pub fn payload_for_source(
        &self,
        workspace_pk: i64,
        root_pk: i64,
        path: &str,
    ) -> RepoResult<Option<SelectPayloads>> {
        self.drizzle
            .select(())
            .from(self.tables.payloads)
            .r#where(and(
                eq(self.tables.payloads.deleted, false),
                and(
                    eq(self.tables.payloads.workspace_pk, workspace_pk),
                    and(
                        eq(self.tables.payloads.root_pk, root_pk),
                        eq(self.tables.payloads.path, path),
                    ),
                ),
            ))
            .get()
            .wait()
            .optional()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the attempt lookup query fails.
    pub fn attempt_by_id(&self, attempt_id: i64) -> RepoResult<Option<SelectAttempts>> {
        self.drizzle
            .select(())
            .from(self.tables.attempts)
            .r#where(eq(self.tables.attempts.attempt_id, attempt_id))
            .get()
            .wait()
            .optional()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the job lookup query fails.
    pub fn job_by_id(&self, job_id: i64) -> RepoResult<Option<SelectJobs>> {
        self.drizzle
            .select(())
            .from(self.tables.jobs)
            .r#where(eq(self.tables.jobs.job_id, job_id))
            .get()
            .wait()
            .optional()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the job query fails.
    pub fn jobs_for_asset(&self, workspace_pk: i64, asset_pk: i64) -> RepoResult<Vec<SelectJobs>> {
        self.drizzle
            .select(())
            .from(self.tables.jobs)
            .r#where(and(
                eq(self.tables.jobs.workspace_pk, workspace_pk),
                eq(self.tables.jobs.asset_pk, asset_pk),
            ))
            .order_by([asc(self.tables.jobs.job_id)])
            .all()
            .wait()
            .map_err(storage)
    }

    /// # Errors
    ///
    /// Returns any error [`Self::entry_by_asset`] returns, and
    /// [`RepoError::Storage`] if the source-edge query fails.
    pub fn source_edges_for_asset(
        &self,
        workspace_pk: i64,
        asset_pk: i64,
    ) -> RepoResult<Vec<SelectSourceEdges>> {
        if self.entry_by_asset(workspace_pk, asset_pk)?.is_none() {
            return Ok(Vec::new());
        }
        self.drizzle
            .select(())
            .from(self.tables.source_edges)
            .r#where(and(
                eq(self.tables.source_edges.workspace_pk, workspace_pk),
                eq(self.tables.source_edges.asset_pk, asset_pk),
            ))
            .order_by([asc(self.tables.source_edges.source_edge_id)])
            .all()
            .wait()
            .map_err(storage)
    }

    /// Groups product edges by their owning product, paged under the query bind
    /// budget so one inspection cannot exceed it.
    fn product_edges_by_product(
        &self,
        rows: &[SelectProducts],
    ) -> RepoResult<BTreeMap<i64, Vec<SelectProductEdges>>> {
        let mut product_edges = BTreeMap::<i64, Vec<SelectProductEdges>>::new();
        let product_ids = rows
            .iter()
            .map(|product| product.product_id)
            .collect::<Vec<_>>();
        for product_ids in product_ids.chunks(QUERY_BIND_BUDGET) {
            let edges: Vec<SelectProductEdges> = self
                .drizzle
                .select(())
                .from(self.tables.product_edges)
                .r#where(in_array(
                    self.tables.product_edges.product_pk,
                    product_ids.to_vec(),
                ))
                .order_by([
                    asc(self.tables.product_edges.product_pk),
                    asc(self.tables.product_edges.product_edge_id),
                ])
                .all()
                .wait()
                .map_err(storage)?;
            for edge in edges {
                product_edges.entry(edge.product_pk).or_default().push(edge);
            }
        }
        Ok(product_edges)
    }

    /// Resolves the job and, for an attempt selector, its attempt row.
    fn inspect_job_selection(
        &self,
        workspace_pk: i64,
        selector: JobInspectionSelector,
    ) -> RepoResult<Option<(SelectJobs, Option<SelectAttempts>)>> {
        Ok(match selector {
            JobInspectionSelector::Job(job_id) => {
                let job: Option<SelectJobs> = self
                    .drizzle
                    .select(())
                    .from(self.tables.jobs)
                    .r#where(and(
                        eq(self.tables.jobs.job_id, job_id),
                        eq(self.tables.jobs.workspace_pk, workspace_pk),
                    ))
                    .get()
                    .wait()
                    .optional()
                    .map_err(storage)?;
                let Some(job) = job else {
                    return Ok(None);
                };
                let attempt = self
                    .drizzle
                    .select(())
                    .from(self.tables.attempts)
                    .r#where(eq(self.tables.attempts.job_pk, job_id))
                    .order_by([desc(self.tables.attempts.ordinal)])
                    .limit(1)
                    .get()
                    .wait()
                    .optional()
                    .map_err(storage)?;
                Some((job, attempt))
            }
            JobInspectionSelector::Attempt(attempt_id) => {
                let attempt: Option<SelectAttempts> = self
                    .drizzle
                    .select(())
                    .from(self.tables.attempts)
                    .inner_join((
                        self.tables.jobs,
                        eq(self.tables.jobs.job_id, self.tables.attempts.job_pk),
                    ))
                    .r#where(and(
                        eq(self.tables.attempts.attempt_id, attempt_id),
                        eq(self.tables.jobs.workspace_pk, workspace_pk),
                    ))
                    .get()
                    .wait()
                    .optional()
                    .map_err(storage)?;
                let Some(attempt) = attempt else {
                    return Ok(None);
                };
                let job: Option<SelectJobs> = self
                    .drizzle
                    .select(())
                    .from(self.tables.jobs)
                    .r#where(and(
                        eq(self.tables.jobs.job_id, attempt.job_pk),
                        eq(self.tables.jobs.workspace_pk, workspace_pk),
                    ))
                    .get()
                    .wait()
                    .optional()
                    .map_err(storage)?;
                let job = job.ok_or_else(|| {
                    RepoError::Invariant("inspection attempt has no scoped owning Job".to_owned())
                })?;
                Some((job, Some(attempt)))
            }
        })
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Invariant`] if the attempt has no owning job, the job
    /// has no owning asset, no workspace entry, or no workspace-root policy, and
    /// [`RepoError::Storage`] if any of those queries fails.
    pub fn inspect_job(
        &self,
        workspace_pk: i64,
        selector: JobInspectionSelector,
    ) -> RepoResult<Option<JobInspection>> {
        let Some((job, attempt)) = self.inspect_job_selection(workspace_pk, selector)? else {
            return Ok(None);
        };
        let asset = self
            .asset_by_id(job.asset_pk)?
            .ok_or_else(|| RepoError::Invariant("inspection Job has no owning Asset".to_owned()))?;
        let entry = self
            .entry_by_asset(job.workspace_pk, job.asset_pk)?
            .ok_or_else(|| {
                RepoError::Invariant("inspection Job has no workspace Entry".to_owned())
            })?;
        let workspace_root: Option<SelectWorkspaceRoots> = self
            .drizzle
            .select(())
            .from(self.tables.workspace_roots)
            .r#where(and(
                eq(self.tables.workspace_roots.workspace_pk, job.workspace_pk),
                eq(self.tables.workspace_roots.root_pk, entry.root_pk),
            ))
            .get()
            .wait()
            .optional()
            .map_err(storage)?;
        let workspace_root = workspace_root.ok_or_else(|| {
            RepoError::Invariant("inspection Job has no workspace-root policy".to_owned())
        })?;
        let rows: Vec<SelectProducts> = self
            .drizzle
            .select(())
            .from(self.tables.products)
            .r#where(eq(self.tables.products.job_pk, job.job_id))
            .order_by([asc(self.tables.products.product_id)])
            .all()
            .wait()
            .map_err(storage)?;
        let mut product_edges = self.product_edges_by_product(&rows)?;
        let products = rows
            .into_iter()
            .map(|product| InspectedProduct {
                edges: product_edges
                    .remove(&product.product_id)
                    .unwrap_or_default(),
                product,
            })
            .collect();
        let edges = self
            .drizzle
            .select(())
            .from(self.tables.job_edges)
            .r#where(eq(self.tables.job_edges.job_pk, job.job_id))
            .order_by([asc(self.tables.job_edges.job_edge_id)])
            .all()
            .wait()
            .map_err(storage)?;
        Ok(Some(JobInspection {
            job,
            attempt,
            asset,
            entry,
            workspace_root,
            products,
            edges,
        }))
    }

    /// Folds each dependent job's product paths and latest attempt, paged under the
    /// query bind budget so one dependents query cannot exceed it.
    fn dependent_job_facts(&self, job_ids: &[i64]) -> RepoResult<DependentJobFacts> {
        let mut product_paths: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
        let mut latest_attempts = BTreeMap::new();
        for job_id_page in job_ids.chunks(QUERY_BIND_BUDGET) {
            let products: Vec<(i64, String)> = self
                .drizzle
                .select((self.tables.products.job_pk, self.tables.products.path))
                .from(self.tables.products)
                .r#where(in_array(self.tables.products.job_pk, job_id_page.to_vec()))
                .order_by([
                    asc(self.tables.products.job_pk),
                    asc(self.tables.products.path),
                ])
                .all()
                .wait()
                .map_err(storage)?;
            for (job_id, path) in products {
                product_paths.entry(job_id).or_default().insert(path);
            }
            let attempts: Vec<(i64, i64)> = self
                .drizzle
                .select((self.tables.attempts.job_pk, self.tables.attempts.attempt_id))
                .from(self.tables.attempts)
                .r#where(in_array(self.tables.attempts.job_pk, job_id_page.to_vec()))
                .order_by([
                    asc(self.tables.attempts.job_pk),
                    desc(self.tables.attempts.ordinal),
                ])
                .all()
                .wait()
                .map_err(storage)?;
            for (job_id, attempt_id) in attempts {
                latest_attempts.entry(job_id).or_insert(attempt_id);
            }
        }
        Ok((product_paths, latest_attempts))
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Invariant`] if a dependent row is missing the asset or
    /// workspace row it references, and [`RepoError::Storage`] if any query fails.
    pub fn source_dependents(&self, input: &SourceDependentsInput) -> RepoResult<SourceDependents> {
        if self
            .entry_by_asset(input.workspace_pk, input.asset_pk)?
            .is_none()
        {
            return Err(RepoError::Invariant(
                "dependent lookup asset does not belong to the workspace".to_owned(),
            ));
        }
        let sources = self
            .drizzle
            .select(SourceDependentSource {
                edge_id: 0,
                relation: Relation::default(),
                builder: Uuid::nil(),
                source_path: String::new(),
            })
            .from(self.tables.source_edges)
            .inner_join((
                self.tables.entries,
                and(
                    eq(
                        self.tables.entries.asset_pk,
                        self.tables.source_edges.asset_pk,
                    ),
                    eq(self.tables.entries.workspace_pk, input.workspace_pk),
                ),
            ))
            .r#where(and(
                eq(self.tables.source_edges.workspace_pk, input.workspace_pk),
                eq(self.tables.source_edges.depends_pk, Some(input.asset_pk)),
            ))
            .order_by([asc(self.tables.source_edges.source_edge_id)])
            .all()
            .wait()
            .map_err(storage)?;
        let job_rows: Vec<SourceDependentJobRow> = self
            .drizzle
            .select(SourceDependentJobRow {
                edge_id: 0,
                source_path: String::new(),
                kind: Work::default(),
                builder: None,
                job_key: String::new(),
                platform: String::new(),
                dependency_job_key: String::new(),
                dependency_platform: String::new(),
                coupling: Coupling::default(),
                job_id: 0,
            })
            .from(self.tables.job_edges)
            .inner_join((
                self.tables.jobs,
                and(
                    eq(self.tables.jobs.job_id, self.tables.job_edges.job_pk),
                    eq(self.tables.jobs.workspace_pk, input.workspace_pk),
                ),
            ))
            .inner_join((
                self.tables.entries,
                and(
                    eq(self.tables.entries.asset_pk, self.tables.jobs.asset_pk),
                    eq(self.tables.entries.workspace_pk, input.workspace_pk),
                ),
            ))
            .r#where(eq(self.tables.job_edges.asset_pk, Some(input.asset_pk)))
            .order_by([asc(self.tables.job_edges.job_edge_id)])
            .all()
            .wait()
            .map_err(storage)?;
        let job_ids = job_rows.iter().map(|row| row.job_id).collect::<Vec<_>>();
        let (mut product_paths, mut latest_attempts) = self.dependent_job_facts(&job_ids)?;
        let jobs = job_rows
            .into_iter()
            .map(|row| SourceDependentJob {
                edge_id: row.edge_id,
                job_id: row.job_id,
                source_path: row.source_path,
                kind: row.kind,
                builder: row.builder,
                job_key: row.job_key,
                platform: row.platform,
                dependency_job_key: row.dependency_job_key,
                dependency_platform: row.dependency_platform,
                coupling: row.coupling,
                latest_attempt_id: latest_attempts.remove(&row.job_id),
                product_paths: product_paths
                    .remove(&row.job_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            })
            .collect();
        Ok(SourceDependents { sources, jobs })
    }

    /// Return every current Job for the selected Assets plus its latest
    /// durable Attempt, if one exists. Queued jobs remain visible without
    /// inventing an Attempt row.
    ///
    /// # Errors
    ///
    /// Returns [`RepoError::Storage`] if the job-activity query fails.
    pub fn job_activities_for_assets(
        &self,
        workspace_pk: i64,
        asset_ids: &[i64],
    ) -> RepoResult<BTreeMap<i64, Vec<JobActivitySnapshot>>> {
        let mut jobs: Vec<SelectJobs> = Vec::new();
        for asset_page in asset_ids.chunks(QUERY_BIND_BUDGET) {
            jobs.extend(
                self.drizzle
                    .select(())
                    .from(self.tables.jobs)
                    .r#where(and(
                        eq(self.tables.jobs.workspace_pk, workspace_pk),
                        in_array(self.tables.jobs.asset_pk, asset_page.to_vec()),
                    ))
                    .order_by([asc(self.tables.jobs.asset_pk), asc(self.tables.jobs.job_id)])
                    .all()
                    .wait()
                    .map_err(storage)?,
            );
        }
        let job_ids = jobs.iter().map(|job| job.job_id).collect::<Vec<_>>();
        let mut latest_attempts = BTreeMap::new();
        for job_page in job_ids.chunks(QUERY_BIND_BUDGET) {
            let attempts: Vec<SelectAttempts> = self
                .drizzle
                .select(())
                .from(self.tables.attempts)
                .r#where(in_array(self.tables.attempts.job_pk, job_page.to_vec()))
                .order_by([
                    asc(self.tables.attempts.job_pk),
                    desc(self.tables.attempts.ordinal),
                ])
                .all()
                .wait()
                .map_err(storage)?;
            for attempt in attempts {
                latest_attempts.entry(attempt.job_pk).or_insert(attempt);
            }
        }
        let mut by_asset = BTreeMap::<i64, Vec<JobActivitySnapshot>>::new();
        for job in jobs {
            let attempt = latest_attempts.remove(&job.job_id);
            by_asset
                .entry(job.asset_pk)
                .or_default()
                .push(JobActivitySnapshot { job, attempt });
        }
        Ok(by_asset)
    }

    /// # Errors
    ///
    /// Returns any error [`Self::job_activities_for_assets`] returns, and
    /// [`RepoError::Storage`] if the entry-page query fails.
    pub fn workspace_entry_page(
        &self,
        workspace_pk: i64,
        root_pks: Option<&[i64]>,
        after_entry_id: i64,
        limit: u32,
    ) -> RepoResult<Vec<WorkspaceEntrySnapshot>> {
        if root_pks.is_some_and(<[i64]>::is_empty) {
            return Ok(Vec::new());
        }
        let rows: Vec<WorkspaceEntrySnapshotRow> = if let Some(root_pks) = root_pks {
            let mut rows = Vec::new();
            let root_pks = root_pks.iter().copied().collect::<BTreeSet<_>>();
            let root_pks = root_pks.into_iter().collect::<Vec<_>>();
            for root_page in root_pks.chunks(QUERY_BIND_BUDGET) {
                rows.extend(
                    self.drizzle
                        .select(WorkspaceEntrySnapshotRow::projection())
                        .from(self.tables.entries)
                        .inner_join((
                            self.tables.assets,
                            eq(self.tables.assets.asset_id, self.tables.entries.asset_pk),
                        ))
                        .inner_join((
                            self.tables.workspace_roots,
                            and(
                                eq(self.tables.workspace_roots.workspace_pk, workspace_pk),
                                eq(
                                    self.tables.workspace_roots.root_pk,
                                    self.tables.entries.root_pk,
                                ),
                            ),
                        ))
                        .r#where(and(
                            eq(self.tables.entries.workspace_pk, workspace_pk),
                            and(
                                in_array(self.tables.entries.root_pk, root_page.to_vec()),
                                gt(self.tables.entries.entry_id, after_entry_id),
                            ),
                        ))
                        .order_by([asc(self.tables.entries.entry_id)])
                        .limit(i64::from(limit))
                        .all()
                        .wait()
                        .map_err(storage)?,
                );
            }
            rows.sort_by_key(|row| row.entry_id);
            rows.truncate(limit as usize);
            rows
        } else {
            self.drizzle
                .select(WorkspaceEntrySnapshotRow::projection())
                .from(self.tables.entries)
                .inner_join((
                    self.tables.assets,
                    eq(self.tables.assets.asset_id, self.tables.entries.asset_pk),
                ))
                .inner_join((
                    self.tables.workspace_roots,
                    and(
                        eq(self.tables.workspace_roots.workspace_pk, workspace_pk),
                        eq(
                            self.tables.workspace_roots.root_pk,
                            self.tables.entries.root_pk,
                        ),
                    ),
                ))
                .r#where(and(
                    eq(self.tables.entries.workspace_pk, workspace_pk),
                    gt(self.tables.entries.entry_id, after_entry_id),
                ))
                .order_by([asc(self.tables.entries.entry_id)])
                .limit(i64::from(limit))
                .all()
                .wait()
                .map_err(storage)?
        };
        let asset_ids = rows.iter().map(|row| row.asset_pk).collect::<Vec<_>>();
        let mut jobs = self.job_activities_for_assets(workspace_pk, &asset_ids)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let job_activities = jobs.remove(&row.asset_pk).unwrap_or_default();
                WorkspaceEntrySnapshot {
                    entry_id: row.entry_id,
                    workspace_pk: row.workspace_pk,
                    asset_pk: row.asset_pk,
                    asset_guid: row.asset_guid,
                    root_pk: row.root_pk,
                    source_root: row.source_root,
                    source_path: row.source_path,
                    schema: row.schema,
                    digest: row.digest,
                    diff: row.diff,
                    diagnostics: row.diagnostics,
                    updated: row.updated,
                    src_bytes: row.src_bytes,
                    src_mtime: row.src_mtime,
                    meta_bytes: row.meta_bytes,
                    meta_mtime: row.meta_mtime,
                    jobs: job_activities,
                }
            })
            .collect())
    }

    /// # Errors
    ///
    /// Returns [`RepoError::Invariant`] if a payload references a missing
    /// workspace, and [`RepoError::Storage`] if any query fails.
    pub fn export_unsaved_payloads(&self) -> RepoResult<Vec<UnsavedPayload>> {
        let table = self.tables.payloads;
        let rows: Vec<SelectPayloads> = self
            .drizzle
            .select(())
            .from(table)
            .r#where(or(is_null(table.saved), ne(table.saved, table.revision)))
            .order_by([asc(table.payload_id)])
            .all()
            .wait()
            .map_err(storage)?;
        let mut exported = Vec::with_capacity(rows.len());
        for row in rows {
            let workspace: Option<SelectWorkspaces> = self
                .drizzle
                .select(())
                .from(self.tables.workspaces)
                .r#where(eq(self.tables.workspaces.workspace_id, row.workspace_pk))
                .get()
                .wait()
                .optional()
                .map_err(storage)?;
            let workspace = workspace
                .ok_or_else(|| RepoError::Invariant("payload workspace is missing".to_owned()))?;
            let root: Option<SelectRoots> = self
                .drizzle
                .select(())
                .from(self.tables.roots)
                .r#where(eq(self.tables.roots.root_id, row.root_pk))
                .get()
                .wait()
                .optional()
                .map_err(storage)?;
            let root =
                root.ok_or_else(|| RepoError::Invariant("payload root is missing".to_owned()))?;
            let policy: Option<SelectWorkspaceRoots> = self
                .drizzle
                .select(())
                .from(self.tables.workspace_roots)
                .r#where(and(
                    eq(self.tables.workspace_roots.workspace_pk, row.workspace_pk),
                    eq(self.tables.workspace_roots.root_pk, row.root_pk),
                ))
                .get()
                .wait()
                .optional()
                .map_err(storage)?;
            let policy = policy.ok_or_else(|| {
                RepoError::Invariant("payload workspace-root policy is missing".to_owned())
            })?;
            exported.push(UnsavedPayload {
                workspace: RecoveredWorkspace {
                    key: WorkspaceKey {
                        project: workspace.project,
                        root: workspace.root,
                        branch: workspace.branch,
                    },
                    created: workspace.created,
                    updated: workspace.updated,
                },
                root: RecoveredRoot {
                    key: root.key,
                    owner: policy.owner,
                    path: policy.path,
                    exclusions: policy.exclusions,
                },
                path: row.path,
                document: row.document,
                schema: row.schema,
                encoding: row.encoding,
                revision: row.revision,
                saved: row.saved,
                digest: row.digest,
                bytes: row.bytes,
                payload: row.payload,
                checkpoint: row.checkpoint,
                session: row.session,
                project: row.project,
                deleted: row.deleted,
                created: row.created,
                updated: row.updated,
            });
        }
        Ok(exported)
    }
}

fn count_jobs(
    db: &AssetDb,
    workspace_pk: i64,
    platform: Option<&str>,
    status: Status,
    ready: Option<bool>,
) -> RepoResult<u64> {
    let jobs = db.tables.jobs;
    let count: i64 = match (platform, ready) {
        (Some(platform), Some(ready)) => db
            .drizzle
            .select(count(jobs.job_id))
            .from(jobs)
            .r#where(and(
                eq(jobs.workspace_pk, workspace_pk),
                and(
                    eq(jobs.platform, platform),
                    and(eq(jobs.status, status), eq(jobs.ready, ready)),
                ),
            ))
            .get::<i64, _, _>()
            .wait(),
        (Some(platform), None) => db
            .drizzle
            .select(count(jobs.job_id))
            .from(jobs)
            .r#where(and(
                eq(jobs.workspace_pk, workspace_pk),
                and(eq(jobs.platform, platform), eq(jobs.status, status)),
            ))
            .get::<i64, _, _>()
            .wait(),
        (None, Some(ready)) => db
            .drizzle
            .select(count(jobs.job_id))
            .from(jobs)
            .r#where(and(
                eq(jobs.workspace_pk, workspace_pk),
                and(eq(jobs.status, status), eq(jobs.ready, ready)),
            ))
            .get::<i64, _, _>()
            .wait(),
        (None, None) => db
            .drizzle
            .select(count(jobs.job_id))
            .from(jobs)
            .r#where(and(
                eq(jobs.workspace_pk, workspace_pk),
                eq(jobs.status, status),
            ))
            .get::<i64, _, _>()
            .wait(),
    }
    .map_err(storage)?;
    Ok(count.cast_unsigned())
}

fn record_post_commit_effect(effects: &Arc<Mutex<PostCommitEffectLog>>, effect: PostCommitEffect) {
    effects
        .lock()
        .expect("post-commit effect log lock poisoned")
        .record(effect);
}

fn record_catalog_invalidation(
    effects: &Arc<Mutex<PostCommitEffectLog>>,
    workspace_pk: i64,
    platform: Option<String>,
) {
    record_post_commit_effect(
        effects,
        PostCommitEffect::CatalogInvalidated {
            workspace_pk,
            platform,
        },
    );
}

// Thread entry point: the database handle is moved into the writer thread and must be
// owned for its whole life, so it cannot be taken by reference.
/// Handles the writer commands that follow job resolution: idle repair, payload and
/// source-file mutations, and the test barrier.
fn handle_late_writer_command(
    db: &AssetDb,
    command: WriterCommand,
    effects: &Arc<Mutex<PostCommitEffectLog>>,
) {
    match command {
        WriterCommand::ResolveIdle(input, reply) => {
            let workspace_pk = input.workspace_pk;
            let result = resolve_idle_blocked(db, input);
            if let Ok(result) = &result {
                for failed in &result.failed_jobs {
                    record_catalog_invalidation(
                        effects,
                        workspace_pk,
                        Some(failed.platform.clone()),
                    );
                }
                if !result.failed_jobs.is_empty()
                    || !result.dropped_order_only_edges.is_empty()
                    || !result.became_ready.is_empty()
                {
                    db.publish_processing_change();
                }
            }
            let _ = reply.send(result);
        }
        WriterCommand::ImportPayload(input, reply) => {
            let _ = reply.send(import_payload(db, input));
        }
        WriterCommand::WritePayload(input, reply) => {
            let _ = reply.send(write_source_payload(db, input));
        }
        WriterCommand::PublishAuthoredSource(input, reply) => {
            let workspace_pk = input.payload.workspace_pk;
            let result = publish_authored_source(db, input);
            if matches!(&result, Ok(PublishAuthoredSourceResult::Written(_))) {
                record_catalog_invalidation(effects, workspace_pk, None);
                db.publish_processing_change();
            }
            let _ = reply.send(result);
        }
        WriterCommand::MoveSource(input, reply) => {
            let workspace_pk = input.workspace_pk;
            let result = move_source(db, input);
            if matches!(&result, Ok(MoveSourceResult::Moved(_))) {
                record_catalog_invalidation(effects, workspace_pk, None);
                db.publish_processing_change();
            }
            let _ = reply.send(result);
        }
        WriterCommand::DeleteSource(input, reply) => {
            let workspace_pk = input.workspace_pk;
            let result = delete_source(db, &input);
            if matches!(&result, Ok(DeleteSourceResult::Deleted(_))) {
                record_catalog_invalidation(effects, workspace_pk, None);
                db.publish_processing_change();
            }
            let _ = reply.send(result);
        }
        #[cfg(any(test, feature = "test-support"))]
        WriterCommand::TestBarrier {
            entered,
            release,
            reply,
        } => {
            let _ = entered.send(());
            let result = release.recv().map_err(|_| RepoError::WriterStopped);
            let _ = reply.send(result);
        }
        _ => unreachable!("writer_loop routes only late commands here"),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn writer_loop(
    db: AssetDb,
    receiver: mpsc::Receiver<WriterCommand>,
    effects: Arc<Mutex<PostCommitEffectLog>>,
) {
    while let Ok(command) = receiver.recv() {
        match command {
            WriterCommand::RegisterWorkspace(input, reply) => {
                let _ = reply.send(register_workspace(&db, input));
            }
            WriterCommand::RegisterWorkspaceRoot(input, reply) => {
                let _ = reply.send(register_workspace_root(&db, input));
            }
            WriterCommand::ReplaceWorkspaceRoots(input, reply) => {
                let _ = reply.send(replace_workspace_roots(&db, input));
            }
            WriterCommand::ApplySweep(input, reply) => {
                let workspace_pk = input.workspace_pk;
                let result = apply_sweep_delta(&db, input);
                if result
                    .as_ref()
                    .is_ok_and(|result| *result != SweepDeltaResult::default())
                {
                    record_catalog_invalidation(&effects, workspace_pk, None);
                    db.publish_processing_change();
                }
                let _ = reply.send(result);
            }
            WriterCommand::ReplaceBuilders(input, reply) => {
                let workspace_pk = input.workspace_pk;
                let result = replace_builder_catalog(&db, input);
                if matches!(&result, Ok(BuilderCatalogReplaceOutcome::Replaced)) {
                    record_catalog_invalidation(&effects, workspace_pk, None);
                    db.publish_processing_change();
                }
                let _ = reply.send(result);
            }
            WriterCommand::ApplyPlan(input, reply) => {
                let workspace_pk = input.workspace_pk;
                let result = apply_plan_delta(&db, input);
                if result.is_ok() {
                    record_catalog_invalidation(&effects, workspace_pk, None);
                    db.publish_processing_change();
                }
                let _ = reply.send(result);
            }
            WriterCommand::Claim(input, reply) => {
                let result = claim_ready_job(&db, input);
                if matches!(result, Ok(ClaimReadyJobResult::Claimed { .. })) {
                    db.publish_processing_change();
                }
                let _ = reply.send(result);
            }
            WriterCommand::Abandon(input, reply) => {
                let result = abandon_attempts(&db, input).map(|(result, invalidations)| {
                    for (workspace_pk, platform) in invalidations {
                        record_catalog_invalidation(&effects, workspace_pk, Some(platform));
                    }
                    result
                });
                if result
                    .as_ref()
                    .is_ok_and(|result| !result.requeued.is_empty() || !result.exhausted.is_empty())
                {
                    db.publish_processing_change();
                }
                let _ = reply.send(result);
            }
            WriterCommand::Complete(input, reply) => {
                let result = complete_attempt(&db, input).map(|(result, invalidation)| {
                    if let Some((workspace_pk, platform)) = invalidation {
                        record_catalog_invalidation(&effects, workspace_pk, Some(platform));
                    }
                    result
                });
                if matches!(
                    &result,
                    Ok(CompleteAttemptResult::Completed { .. }
                        | CompleteAttemptResult::Abandoned { .. })
                ) {
                    db.publish_processing_change();
                }
                let _ = reply.send(result);
            }
            other => handle_late_writer_command(&db, other, &effects),
        }
    }
}

fn register_workspace(db: &AssetDb, input: RegisterWorkspace) -> RepoResult<SelectWorkspaces> {
    let table = db.tables.workspaces;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            if let Some(existing) = tx
                .select(())
                .from(table)
                .r#where(and(
                    eq(table.project, input.key.project.as_str()),
                    and(
                        eq(table.root, input.key.root.as_str()),
                        eq(table.branch, input.key.branch.as_str()),
                    ),
                ))
                .get()
                .await
                .optional()?
            {
                return Ok(existing);
            }
            tx.insert(table)
                .values([InsertWorkspaces::new(
                    input.key.project,
                    input.key.root,
                    input.key.branch,
                    input.now,
                    input.now,
                )])
                .returning(())
                .get()
                .await
        })
        .wait()
        .map_err(storage)
}

fn register_workspace_root(
    db: &AssetDb,
    input: RegisterWorkspaceRoot,
) -> RepoResult<(SelectRoots, SelectWorkspaceRoots)> {
    let roots = db.tables.roots;
    let workspace_roots = db.tables.workspace_roots;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            let existing_root: Option<SelectRoots> = tx
                .select(())
                .from(roots)
                .r#where(eq(roots.key, input.key.as_str()))
                .get()
                .await
                .optional()?;
            let root: SelectRoots = if let Some(root) = existing_root {
                root
            } else {
                tx.insert(roots)
                    .values([InsertRoots::new(input.key)])
                    .returning(())
                    .get()
                    .await?
            };
            let current: Option<SelectWorkspaceRoots> = tx
                .select(())
                .from(workspace_roots)
                .r#where(and(
                    eq(workspace_roots.workspace_pk, input.workspace_pk),
                    eq(workspace_roots.root_pk, root.root_id),
                ))
                .get()
                .await
                .optional()?;
            let policy = if let Some(current) = current {
                tx.update(workspace_roots)
                    .set(
                        UpdateWorkspaceRoots::default()
                            .with_owner(input.owner)
                            .with_path(input.path)
                            .with_exclusions(input.exclusions),
                    )
                    .r#where(eq(
                        workspace_roots.workspace_root_id,
                        current.workspace_root_id,
                    ))
                    .returning(())
                    .get()
                    .await?
            } else {
                tx.insert(workspace_roots)
                    .values([InsertWorkspaceRoots::new(
                        input.workspace_pk,
                        root.root_id,
                        input.owner,
                        input.path,
                        input.exclusions,
                    )])
                    .returning(())
                    .get()
                    .await?
            };
            Ok((root, policy))
        })
        .wait()
        .map_err(storage)
}

/// Rebinds a workspace's source roots inside an open transaction.
async fn replace_workspace_roots_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: ReplaceWorkspaceRoots,
) -> Result<Vec<WorkspaceRootBinding>, drizzle::error::DrizzleError> {
    let mut desired_root_ids = BTreeSet::new();
    let mut bindings = Vec::with_capacity(input.roots.len());
    for desired in input.roots {
        let root: SelectRoots = match tx
            .select(())
            .from(tables.roots)
            .r#where(eq(tables.roots.key, desired.key.as_str()))
            .get()
            .await
            .optional()?
        {
            Some(root) => root,
            None => {
                tx.insert(tables.roots)
                    .values([InsertRoots::new(desired.key)])
                    .returning(())
                    .get()
                    .await?
            }
        };
        desired_root_ids.insert(root.root_id);
        let current: Option<SelectWorkspaceRoots> = tx
            .select(())
            .from(tables.workspace_roots)
            .r#where(and(
                eq(tables.workspace_roots.workspace_pk, input.workspace_pk),
                eq(tables.workspace_roots.root_pk, root.root_id),
            ))
            .get()
            .await
            .optional()?;
        let policy = match current {
            Some(current) => {
                tx.update(tables.workspace_roots)
                    .set(
                        UpdateWorkspaceRoots::default()
                            .with_owner(desired.owner)
                            .with_path(desired.path)
                            .with_exclusions(desired.exclusions),
                    )
                    .r#where(eq(
                        tables.workspace_roots.workspace_root_id,
                        current.workspace_root_id,
                    ))
                    .returning(())
                    .get()
                    .await?
            }
            None => {
                tx.insert(tables.workspace_roots)
                    .values([InsertWorkspaceRoots::new(
                        input.workspace_pk,
                        root.root_id,
                        desired.owner,
                        desired.path,
                        desired.exclusions,
                    )])
                    .returning(())
                    .get()
                    .await?
            }
        };
        bindings.push(WorkspaceRootBinding { root, policy });
    }

    let current: Vec<SelectWorkspaceRoots> = tx
        .select(())
        .from(tables.workspace_roots)
        .r#where(eq(tables.workspace_roots.workspace_pk, input.workspace_pk))
        .all()
        .await?;
    for stale in current
        .into_iter()
        .filter(|row| !desired_root_ids.contains(&row.root_pk))
    {
        tx.delete(tables.workspace_roots)
            .r#where(eq(
                tables.workspace_roots.workspace_root_id,
                stale.workspace_root_id,
            ))
            .execute()
            .await?;
    }
    Ok(bindings)
}

fn replace_workspace_roots(
    db: &AssetDb,
    input: ReplaceWorkspaceRoots,
) -> RepoResult<Vec<WorkspaceRootBinding>> {
    if input.roots.is_empty() {
        return Err(RepoError::Invariant(
            "a workspace root set must contain at least one root".to_owned(),
        ));
    }
    let mut keys = BTreeSet::new();
    if input.roots.iter().any(|root| !keys.insert(&root.key)) {
        return Err(RepoError::Invariant(
            "a workspace root set cannot contain duplicate portable keys".to_owned(),
        ));
    }

    let tables = db.tables;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            replace_workspace_roots_in_tx(tx, tables, input).await
        })
        .wait()
        .map_err(storage)
}

/// Applies one sweep removal: marks the workspace entry deleted and retires the
/// asset identity if no other workspace still observes it.
async fn apply_sweep_removal(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    root_pk: i64,
    removal: SweepRemoval,
    result: &mut SweepDeltaResult,
) -> Result<(), drizzle::error::DrizzleError> {
    let entry: Option<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(and(
            eq(tables.entries.workspace_pk, workspace_pk),
            and(
                eq(tables.entries.root_pk, root_pk),
                eq(tables.entries.path, removal.path.as_str()),
            ),
        ))
        .get()
        .await
        .optional()?;
    if let Some(entry) = entry {
        tx.update(tables.entries)
            .set(
                UpdateEntries::default()
                    .with_diff(Diff::Deleted)
                    .with_observed(removal.observed),
            )
            .r#where(eq(tables.entries.entry_id, entry.entry_id))
            .execute()
            .await?;
        let asset: Option<SelectAssets> = tx
            .select(())
            .from(tables.assets)
            .r#where(eq(tables.assets.asset_id, entry.asset_pk))
            .get()
            .await
            .optional()?;
        let asset = asset.ok_or(drizzle::error::DrizzleError::NotFound)?;
        result.changed_assets.push(asset.asset_id);
        retire_unobserved_asset_in_tx(tx, tables, asset.asset_id, removal.observed).await?;
        close_workspace_asset_path(tx, tables, workspace_pk, asset.asset_id, removal.observed)
            .await?;
        let candidates: Vec<SelectJobEdges> = tx
            .select(())
            .from(tables.job_edges)
            .r#where(eq(tables.job_edges.asset_pk, Some(asset.asset_id)))
            .all()
            .await?;
        let mut affected = Vec::new();
        for edge in candidates {
            let owner: Option<SelectJobs> = tx
                .select(())
                .from(tables.jobs)
                .r#where(and(
                    eq(tables.jobs.job_id, edge.job_pk),
                    eq(tables.jobs.workspace_pk, workspace_pk),
                ))
                .get()
                .await
                .optional()?;
            if owner.is_some() {
                tx.update(tables.job_edges)
                    .set(UpdateJobEdges::default().with_asset_pk(SQLiteUpdateValue::Null))
                    .r#where(eq(tables.job_edges.job_edge_id, edge.job_edge_id))
                    .execute()
                    .await?;
                affected.push(edge.job_pk);
            }
        }
        tx.update(tables.source_edges)
            .set(UpdateSourceEdges::default().with_depends_pk(SQLiteUpdateValue::Null))
            .r#where(and(
                eq(tables.source_edges.workspace_pk, workspace_pk),
                eq(tables.source_edges.depends_pk, Some(asset.asset_id)),
            ))
            .execute()
            .await?;
        tx.delete(tables.jobs)
            .r#where(and(
                eq(tables.jobs.workspace_pk, workspace_pk),
                eq(tables.jobs.asset_pk, asset.asset_id),
            ))
            .execute()
            .await?;
        recompute_job_ids(tx, tables, affected).await?;
        result.removed += 1;
    }
    Ok(())
}

fn apply_sweep_delta(db: &AssetDb, input: ApplySweepDelta) -> RepoResult<SweepDeltaResult> {
    if input.records.is_empty() && input.removals.is_empty() {
        return Ok(SweepDeltaResult::default());
    }
    let tables = db.tables;
    let workspace_pk = input.workspace_pk;
    let mut context = db.transaction_context();
    let result = context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            let policy: Option<SelectWorkspaceRoots> = tx
                .select(())
                .from(tables.workspace_roots)
                .r#where(eq(
                    tables.workspace_roots.workspace_root_id,
                    input.workspace_root_pk,
                ))
                .get()
                .await
                .optional()?;
            let policy = policy.ok_or(drizzle::error::DrizzleError::NotFound)?;
            if policy.workspace_pk != workspace_pk {
                return Err(drizzle::error::DrizzleError::Other(
                    "sweep workspace root belongs to another workspace".into(),
                ));
            }
            let root_pk = policy.root_pk;
            let mut result = SweepDeltaResult::default();
            for record in input.records {
                apply_sweep_record(tx, tables, workspace_pk, root_pk, record, &mut result).await?;
            }
            for removal in input.removals {
                apply_sweep_removal(tx, tables, workspace_pk, root_pk, removal, &mut result)
                    .await?;
            }
            result.changed_assets.sort_unstable();
            result.changed_assets.dedup();
            Ok(result)
        })
        .wait()
        .map_err(storage)?;
    Ok(result)
}

/// Refreshes one swept asset's workspace entry and replaces its planner job,
/// rebinding the job and source edges that hang off it.
#[expect(
    clippy::too_many_arguments,
    reason = "the arguments are the open transaction and its schema handle, the two scope keys the Entries statement filters on, the identity and observation rows whose columns it writes, the planner Job that replaces the plan, and the caller's delta accumulator; grouping them would bag an open transaction, three different borrow lifetimes, and an out-parameter under one name that describes nothing"
)]
async fn refresh_sweep_entry_and_planner(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    root_pk: i64,
    asset: &SelectAssets,
    source: &SweepEntry,
    planner: SweepPlannerJob,
    result: &mut SweepDeltaResult,
) -> Result<(), drizzle::error::DrizzleError> {
    let entry: Option<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(and(
            eq(tables.entries.workspace_pk, workspace_pk),
            eq(tables.entries.asset_pk, asset.asset_id),
        ))
        .get()
        .await
        .optional()?;
    if let Some(entry) = entry {
        tx.update(tables.entries)
            .set(
                UpdateEntries::default()
                    .with_asset_pk(asset.asset_id)
                    .with_root_pk(root_pk)
                    .with_path(source.path.clone())
                    .with_schema(
                        source
                            .schema
                            .clone()
                            .map_or(SQLiteUpdateValue::Null, Into::into),
                    )
                    .with_digest(source.digest)
                    .with_diff(source.diff)
                    .with_diagnostics(source.diagnostics)
                    .with_updated(source.updated)
                    .with_src_bytes(source.src_bytes)
                    .with_src_mtime(source.src_mtime)
                    .with_meta_bytes(source.meta_bytes)
                    .with_meta_mtime(source.meta_mtime)
                    .with_observed(source.observed),
            )
            .r#where(eq(tables.entries.entry_id, entry.entry_id))
            .execute()
            .await?;
    } else {
        tx.insert(tables.entries)
            .values([InsertEntries::new(
                workspace_pk,
                asset.asset_id,
                root_pk,
                source.path.clone(),
                source.digest,
                source.diff,
                source.updated,
                source.src_bytes,
                source.src_mtime,
                source.meta_bytes,
                source.meta_mtime,
                source.observed,
            )
            .with_schema(
                source
                    .schema
                    .clone()
                    .map_or(SQLiteInsertValue::Null, Into::into),
            )
            .with_diagnostics(source.diagnostics)])
            .execute()
            .await?;
    }
    replace_sweep_planner_job(tx, tables, workspace_pk, asset.asset_id, planner).await?;
    result.planned += 1;
    result.bound_job_edges +=
        bind_authored_job_edges(tx, tables, workspace_pk, asset, &source.path).await?;
    result.bound_source_edges +=
        bind_authored_source_edges(tx, tables, workspace_pk, asset, &source.path).await?;
    Ok(())
}

/// Applies one sweep record: resolves or creates the asset identity, refreshes its
/// workspace entry, and replaces the planner job plus its bound edges.
async fn apply_sweep_record(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    root_pk: i64,
    record: SweepRecord,
    result: &mut SweepDeltaResult,
) -> Result<(), drizzle::error::DrizzleError> {
    let SweepRecord { source, planner } = record;
    let at_locator =
        workspace_asset_at_locator(tx, tables, workspace_pk, root_pk, &source.path).await?;
    if at_locator
        .as_ref()
        .is_some_and(|(_, existing)| existing.guid != source.guid)
    {
        return Err(drizzle::error::DrizzleError::Other(
            format!(
                "source locator {} resolved to a different stable asset identity",
                source.path
            )
            .into(),
        ));
    }
    let by_identity: Option<SelectAssets> = tx
        .select(())
        .from(tables.assets)
        .r#where(eq(tables.assets.guid, source.guid))
        .get()
        .await
        .optional()?;
    let asset = if let Some(existing) = by_identity.or_else(|| at_locator.map(|(_, asset)| asset)) {
        result.updated += 1;
        maintain_path_history(
            tx,
            tables,
            workspace_pk,
            existing.asset_id,
            root_pk,
            &source.path,
            source.digest,
            source.session.clone(),
            source.updated,
        )
        .await?;
        tx.update(tables.assets)
            .set(
                UpdateAssets::default()
                    .with_deleted(false)
                    .with_updated(source.updated),
            )
            .r#where(eq(tables.assets.asset_id, existing.asset_id))
            .returning(())
            .get()
            .await?
    } else {
        result.inserted += 1;
        let inserted: SelectAssets = tx
            .insert(tables.assets)
            .values([InsertAssets::new(
                source.guid,
                source.updated,
                source.updated,
            )])
            .returning(())
            .get()
            .await?;
        tx.insert(tables.paths)
            .values([InsertPaths::new(
                workspace_pk,
                inserted.asset_id,
                root_pk,
                source.path.clone(),
                source.digest,
                source.updated,
            )
            .with_session(
                source
                    .session
                    .clone()
                    .map_or(SQLiteInsertValue::Null, Into::into),
            )])
            .execute()
            .await?;
        inserted
    };
    result.changed_assets.push(asset.asset_id);
    refresh_sweep_entry_and_planner(
        tx,
        tables,
        workspace_pk,
        root_pk,
        &asset,
        &source,
        planner,
        result,
    )
    .await?;
    Ok(())
}

async fn replace_sweep_planner_job(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    asset_pk: i64,
    planner: SweepPlannerJob,
) -> Result<(), drizzle::error::DrizzleError> {
    let retired: Vec<SelectJobs> = tx
        .select(())
        .from(tables.jobs)
        .r#where(and(
            eq(tables.jobs.workspace_pk, workspace_pk),
            and(
                eq(tables.jobs.asset_pk, asset_pk),
                eq(tables.jobs.kind, Work::Plan),
            ),
        ))
        .all()
        .await?;
    let mut readiness_ids = Vec::new();
    for job in &retired {
        let dependents: Vec<SelectJobEdges> = tx
            .select(())
            .from(tables.job_edges)
            .r#where(and(
                eq(tables.job_edges.asset_pk, Some(asset_pk)),
                and(
                    eq(tables.job_edges.key, job.key.as_str()),
                    eq(tables.job_edges.platform, job.platform.as_str()),
                ),
            ))
            .all()
            .await?;
        readiness_ids.extend(dependents.into_iter().map(|edge| edge.job_pk));
    }
    tx.delete(tables.jobs)
        .r#where(and(
            eq(tables.jobs.workspace_pk, workspace_pk),
            and(
                eq(tables.jobs.asset_pk, asset_pk),
                eq(tables.jobs.kind, Work::Plan),
            ),
        ))
        .execute()
        .await?;
    tx.delete(tables.source_edges)
        .r#where(and(
            eq(tables.source_edges.workspace_pk, workspace_pk),
            eq(tables.source_edges.asset_pk, asset_pk),
        ))
        .execute()
        .await?;
    let inserted: SelectJobs = tx
        .insert(tables.jobs)
        .values([InsertJobs::new(
            workspace_pk,
            asset_pk,
            Work::Plan,
            planner.key,
            planner.platform,
            Status::Queued,
        )])
        .returning(())
        .get()
        .await?;
    readiness_ids.push(inserted.job_id);
    recompute_job_ids(tx, tables, readiness_ids)
        .await
        .map(|_| ())
}

async fn workspace_asset_at_locator(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    root_pk: i64,
    path: &str,
) -> Result<Option<(SelectEntries, SelectAssets)>, drizzle::error::DrizzleError> {
    let entry: Option<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(and(
            eq(tables.entries.workspace_pk, workspace_pk),
            and(
                eq(tables.entries.root_pk, root_pk),
                eq(tables.entries.path, path),
            ),
        ))
        .get()
        .await
        .optional()?;
    let Some(entry) = entry else {
        return Ok(None);
    };
    let asset: Option<SelectAssets> = tx
        .select(())
        .from(tables.assets)
        .r#where(eq(tables.assets.asset_id, entry.asset_pk))
        .get()
        .await
        .optional()?;
    Ok(Some((
        entry,
        asset.ok_or(drizzle::error::DrizzleError::NotFound)?,
    )))
}

async fn close_workspace_asset_path(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    asset_pk: i64,
    now: i64,
) -> Result<(), drizzle::error::DrizzleError> {
    tx.update(tables.paths)
        .set(UpdatePaths::default().with_to(now))
        .r#where(and(
            eq(tables.paths.workspace_pk, workspace_pk),
            and(
                eq(tables.paths.asset_pk, asset_pk),
                is_null(tables.paths.to),
            ),
        ))
        .execute()
        .await?;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "every argument after the transaction and its schema handle is one column of the Paths row being closed and reopened - workspace_pk, asset_pk, root_pk, path, digest, session, and the `from`/`to` timestamp; a params struct would just re-bag that row under a second name"
)]
async fn maintain_path_history(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    asset_pk: i64,
    root_pk: i64,
    path: &str,
    digest: Digest,
    session: Option<String>,
    now: i64,
) -> Result<(), drizzle::error::DrizzleError> {
    let current: Option<SelectPaths> = tx
        .select(())
        .from(tables.paths)
        .r#where(and(
            eq(tables.paths.workspace_pk, workspace_pk),
            and(
                eq(tables.paths.asset_pk, asset_pk),
                is_null(tables.paths.to),
            ),
        ))
        .get()
        .await
        .optional()?;
    if current
        .as_ref()
        .is_some_and(|current| current.root_pk == root_pk && current.path == path)
    {
        return Ok(());
    }
    close_workspace_asset_path(tx, tables, workspace_pk, asset_pk, now).await?;
    tx.insert(tables.paths)
        .values([InsertPaths::new(
            workspace_pk,
            asset_pk,
            root_pk,
            path.to_owned(),
            digest,
            now,
        )
        .with_session(session.map_or(SQLiteInsertValue::Null, Into::into))])
        .execute()
        .await?;
    Ok(())
}

async fn bind_authored_job_edges(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    asset: &SelectAssets,
    workspace_path: &str,
) -> Result<u64, drizzle::error::DrizzleError> {
    let mut total = 0;
    let path_target = Target::path(workspace_path)
        .map_err(|error| drizzle::error::DrizzleError::ConversionError(error.to_string().into()))?;
    for target in [Target::Guid(asset.guid), path_target] {
        let edges: Vec<(i64, i64)> = tx
            .select((tables.job_edges.job_edge_id, tables.job_edges.job_pk))
            .from(tables.job_edges)
            .inner_join((
                tables.jobs,
                and(
                    eq(tables.jobs.job_id, tables.job_edges.job_pk),
                    eq(tables.jobs.workspace_pk, workspace_pk),
                ),
            ))
            .r#where(and(
                is_null(tables.job_edges.asset_pk),
                eq(tables.job_edges.target, target),
            ))
            .all()
            .await?;
        for (edge_id, job_id) in edges {
            total += tx
                .update(tables.job_edges)
                .set(UpdateJobEdges::default().with_asset_pk(asset.asset_id))
                .r#where(eq(tables.job_edges.job_edge_id, edge_id))
                .execute()
                .await? as u64;
            recompute_job_readiness_in_tx(tx, tables, job_id).await?;
        }
    }
    Ok(total)
}

async fn bind_authored_source_edges(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    asset: &SelectAssets,
    workspace_path: &str,
) -> Result<u64, drizzle::error::DrizzleError> {
    let mut total = 0;
    let path_target = Target::path(workspace_path)
        .map_err(|error| drizzle::error::DrizzleError::ConversionError(error.to_string().into()))?;
    for target in [Target::Guid(asset.guid), path_target] {
        let candidates: Vec<i64> = tx
            .select(tables.source_edges.source_edge_id)
            .from(tables.source_edges)
            .inner_join((
                tables.entries,
                and(
                    eq(tables.entries.asset_pk, tables.source_edges.asset_pk),
                    eq(tables.entries.workspace_pk, workspace_pk),
                ),
            ))
            .r#where(and(
                eq(tables.source_edges.workspace_pk, workspace_pk),
                and(
                    is_null(tables.source_edges.depends_pk),
                    eq(tables.source_edges.target, target),
                ),
            ))
            .all()
            .await?;
        for edge_id in candidates {
            total += tx
                .update(tables.source_edges)
                .set(UpdateSourceEdges::default().with_depends_pk(asset.asset_id))
                .r#where(and(
                    eq(tables.source_edges.workspace_pk, workspace_pk),
                    eq(tables.source_edges.source_edge_id, edge_id),
                ))
                .execute()
                .await? as u64;
        }
    }
    Ok(total)
}

fn target_identifies_asset(target: &Target, asset: &SelectAssets, workspace_path: &str) -> bool {
    match target {
        Target::Guid(guid) => *guid == asset.guid,
        Target::Path(path) => path.as_str() == workspace_path,
    }
}

async fn validate_bound_dependency(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    asset_pk: i64,
    target: &Target,
) -> Result<(), drizzle::error::DrizzleError> {
    let membership: Option<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(and(
            eq(tables.entries.workspace_pk, workspace_pk),
            eq(tables.entries.asset_pk, asset_pk),
        ))
        .get()
        .await
        .optional()?;
    let Some(membership) = membership else {
        return Err(drizzle::error::DrizzleError::Other(
            "bound dependency target belongs to another workspace".into(),
        ));
    };
    let asset: Option<SelectAssets> = tx
        .select(())
        .from(tables.assets)
        .r#where(eq(tables.assets.asset_id, asset_pk))
        .get()
        .await
        .optional()?;
    let asset = asset.ok_or(drizzle::error::DrizzleError::NotFound)?;
    if !target_identifies_asset(target, &asset, &membership.path) {
        return Err(drizzle::error::DrizzleError::Other(
            "authored dependency target disagrees with its bound asset".into(),
        ));
    }
    Ok(())
}

fn apply_plan_delta(db: &AssetDb, input: ApplyPlanDelta) -> RepoResult<()> {
    let tables = db.tables;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            apply_plan_delta_in_tx(tx, tables, input.workspace_pk, input.delta).await
        })
        .wait()
        .map_err(storage)
}

fn validate_planned_job_builder(
    registered_builders: &BTreeSet<Uuid>,
    replacement: &PlannedJob,
) -> Result<(), drizzle::error::DrizzleError> {
    match (replacement.kind, replacement.builder) {
        (Work::Plan, None) => Ok(()),
        (Work::Build, Some(builder)) => {
            validate_registered_builder(registered_builders, builder, "planned Build job")
        }
        _ => Err(drizzle::error::DrizzleError::Other(
            "planned job kind and builder ownership disagree".into(),
        )),
    }
}

async fn registered_builder_guids(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
) -> Result<BTreeSet<Uuid>, drizzle::error::DrizzleError> {
    let builders: Vec<SelectBuilders> = tx
        .select(())
        .from(tables.builders)
        .r#where(eq(tables.builders.workspace_pk, workspace_pk))
        .all()
        .await?;
    Ok(builders.into_iter().map(|builder| builder.guid).collect())
}

fn validate_registered_builder(
    registered_builders: &BTreeSet<Uuid>,
    builder: Uuid,
    owner: &'static str,
) -> Result<(), drizzle::error::DrizzleError> {
    if !registered_builders.contains(&builder) {
        return Err(drizzle::error::DrizzleError::Other(
            format!("{owner} references builder {builder} outside the workspace catalog").into(),
        ));
    }
    Ok(())
}

async fn validate_workspace_builder_closure(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    registered_builders: &BTreeSet<Uuid>,
) -> Result<(), drizzle::error::DrizzleError> {
    let jobs: Vec<SelectJobs> = tx
        .select(())
        .from(tables.jobs)
        .r#where(eq(tables.jobs.workspace_pk, workspace_pk))
        .all()
        .await?;
    for job in jobs {
        if let Some(builder) = job.builder {
            validate_registered_builder(registered_builders, builder, "surviving Build job")?;
        }
    }
    let source_edges: Vec<SourceEdgeBuilderReference> = tx
        .select(SourceEdgeBuilderReference {
            builder: Uuid::nil(),
        })
        .from(tables.source_edges)
        .r#where(eq(tables.source_edges.workspace_pk, workspace_pk))
        .all()
        .await?;
    for edge in source_edges {
        validate_registered_builder(
            registered_builders,
            edge.builder,
            "surviving source-analysis edge",
        )?;
    }
    Ok(())
}

/// Retires the source edges a plan delta drops and binds the ones it adds.
async fn apply_plan_delta_source_edges(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    retire_source_edge_ids: Vec<i64>,
    source_edges: Vec<SourceEdgeInput>,
    registered_builders: &BTreeSet<Uuid>,
) -> Result<(), drizzle::error::DrizzleError> {
    for source_edge_id in retire_source_edge_ids {
        let edge: Option<SelectSourceEdges> = tx
            .select(())
            .from(tables.source_edges)
            .r#where(and(
                eq(tables.source_edges.workspace_pk, workspace_pk),
                eq(tables.source_edges.source_edge_id, source_edge_id),
            ))
            .get()
            .await
            .optional()?;
        if edge.is_some() {
            tx.delete(tables.source_edges)
                .r#where(and(
                    eq(tables.source_edges.workspace_pk, workspace_pk),
                    eq(tables.source_edges.source_edge_id, source_edge_id),
                ))
                .execute()
                .await?;
        }
    }
    for edge in source_edges {
        validate_registered_builder(registered_builders, edge.builder, "source-analysis edge")?;
        let membership: Option<SelectEntries> = tx
            .select(())
            .from(tables.entries)
            .r#where(and(
                eq(tables.entries.workspace_pk, workspace_pk),
                eq(tables.entries.asset_pk, edge.asset_pk),
            ))
            .get()
            .await
            .optional()?;
        if membership.is_none() {
            return Err(drizzle::error::DrizzleError::Other(
                "source-analysis edge belongs to another workspace".into(),
            ));
        }
        if let Some(depends_pk) = edge.depends_pk {
            validate_bound_dependency(tx, tables, workspace_pk, depends_pk, &edge.target).await?;
        }
        tx.insert(tables.source_edges)
            .values([InsertSourceEdges::new(
                workspace_pk,
                edge.builder,
                edge.asset_pk,
                edge.target,
                edge.relation,
            )
            .with_depends_pk(edge.depends_pk.map_or(SQLiteInsertValue::Null, Into::into))])
            .execute()
            .await?;
    }
    Ok(())
}

/// Retires the jobs a plan delta drops, collecting the ids whose readiness the
/// caller must recompute.
async fn retire_plan_delta_jobs(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    retire_job_ids: Vec<i64>,
    replacements: &mut Vec<PlannedJob>,
    readiness_ids: &mut Vec<i64>,
) -> Result<(), drizzle::error::DrizzleError> {
    for job_id in retire_job_ids {
        let retired: Option<SelectJobs> = tx
            .select(())
            .from(tables.jobs)
            .r#where(and(
                eq(tables.jobs.workspace_pk, workspace_pk),
                eq(tables.jobs.job_id, job_id),
            ))
            .get()
            .await
            .optional()?;
        if let Some(retired) = &retired {
            let dependents: Vec<SelectJobEdges> = tx
                .select(())
                .from(tables.job_edges)
                .r#where(and(
                    eq(tables.job_edges.asset_pk, Some(retired.asset_pk)),
                    and(
                        eq(tables.job_edges.key, retired.key.as_str()),
                        eq(tables.job_edges.platform, retired.platform.as_str()),
                    ),
                ))
                .all()
                .await?;
            readiness_ids.extend(dependents.into_iter().map(|edge| edge.job_pk));
        }
        if let Some(retired) = retired
            && let Some(position) = replacements
                .iter()
                .position(|replacement| same_logical_job(&retired, replacement))
        {
            let replacement = replacements.remove(position);
            tx.delete(tables.attempts)
                .r#where(eq(tables.attempts.job_pk, retired.job_id))
                .execute()
                .await?;
            tx.delete(tables.job_edges)
                .r#where(eq(tables.job_edges.job_pk, retired.job_id))
                .execute()
                .await?;
            tx.update(tables.jobs)
                .set(
                    UpdateJobs::default()
                        .with_status(Status::Queued)
                        .with_ready(false)
                        .with_attempts(0),
                )
                .r#where(eq(tables.jobs.job_id, retired.job_id))
                .execute()
                .await?;
            insert_planned_job_edges(tx, tables, workspace_pk, retired.job_id, replacement.edges)
                .await?;
            readiness_ids.push(retired.job_id);
            continue;
        }
        tx.delete(tables.jobs)
            .r#where(and(
                eq(tables.jobs.workspace_pk, workspace_pk),
                eq(tables.jobs.job_id, job_id),
            ))
            .execute()
            .await?;
    }
    Ok(())
}

async fn apply_plan_delta_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    delta: PlanDelta,
) -> Result<(), drizzle::error::DrizzleError> {
    let registered_builders = registered_builder_guids(tx, tables, workspace_pk).await?;
    for replacement in &delta.replacements {
        validate_planned_job_builder(&registered_builders, replacement)?;
        let membership: Option<SelectEntries> = tx
            .select(())
            .from(tables.entries)
            .r#where(and(
                eq(tables.entries.workspace_pk, workspace_pk),
                eq(tables.entries.asset_pk, replacement.asset_pk),
            ))
            .get()
            .await
            .optional()?;
        if membership.is_none() {
            return Err(drizzle::error::DrizzleError::Other(
                "planned job source asset belongs to another workspace".into(),
            ));
        }
    }
    let PlanDelta {
        retire_job_ids,
        retire_source_edge_ids,
        mut replacements,
        source_edges,
    } = delta;
    let mut readiness_ids = Vec::new();
    retire_plan_delta_jobs(
        tx,
        tables,
        workspace_pk,
        retire_job_ids,
        &mut replacements,
        &mut readiness_ids,
    )
    .await?;
    for replacement in replacements {
        let inserted: SelectJobs = tx
            .insert(tables.jobs)
            .values([InsertJobs::new(
                workspace_pk,
                replacement.asset_pk,
                replacement.kind,
                replacement.key,
                replacement.platform,
                Status::Queued,
            )
            .with_builder(
                replacement
                    .builder
                    .map_or(SQLiteInsertValue::Null, Into::into),
            )])
            .returning(())
            .get()
            .await?;
        insert_planned_job_edges(tx, tables, workspace_pk, inserted.job_id, replacement.edges)
            .await?;
        readiness_ids.push(inserted.job_id);
    }
    apply_plan_delta_source_edges(
        tx,
        tables,
        workspace_pk,
        retire_source_edge_ids,
        source_edges,
        &registered_builders,
    )
    .await?;
    recompute_job_ids(tx, tables, readiness_ids).await?;
    Ok(())
}

fn same_logical_job(job: &SelectJobs, replacement: &PlannedJob) -> bool {
    job.asset_pk == replacement.asset_pk
        && job.kind == replacement.kind
        && job.builder == replacement.builder
        && job.key == replacement.key
        && job.platform == replacement.platform
}

async fn insert_planned_job_edges(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    job_id: i64,
    edges: Vec<JobEdgeInput>,
) -> Result<(), drizzle::error::DrizzleError> {
    for edge in edges {
        if let Some(asset_pk) = edge.asset_pk {
            validate_bound_dependency(tx, tables, workspace_pk, asset_pk, &edge.target).await?;
        }
        tx.insert(tables.job_edges)
            .values([InsertJobEdges::new(
                job_id,
                edge.target,
                edge.key,
                edge.platform,
                edge.coupling,
            )
            .with_asset_pk(edge.asset_pk.map_or(SQLiteInsertValue::Null, Into::into))])
            .execute()
            .await?;
    }
    Ok(())
}

/// Inserts the replacement jobs a catalog plan delta carries, returning their ids
/// so the caller can recompute readiness for them.
async fn insert_catalog_plan_replacements(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    replacements: Vec<PlannedJob>,
    registered_builders: &BTreeSet<Uuid>,
) -> Result<Vec<i64>, drizzle::error::DrizzleError> {
    let mut replacement_ids = Vec::new();
    for replacement in replacements {
        validate_planned_job_builder(registered_builders, &replacement)?;
        let source_membership: Option<SelectEntries> = tx
            .select(())
            .from(tables.entries)
            .r#where(and(
                eq(tables.entries.workspace_pk, workspace_pk),
                eq(tables.entries.asset_pk, replacement.asset_pk),
            ))
            .get()
            .await
            .optional()?;
        if source_membership.is_none() {
            return Err(drizzle::error::DrizzleError::Other(
                "planned job source asset belongs to another workspace".into(),
            ));
        }
        let inserted: SelectJobs = tx
            .insert(tables.jobs)
            .values([InsertJobs::new(
                workspace_pk,
                replacement.asset_pk,
                replacement.kind,
                replacement.key,
                replacement.platform,
                Status::Queued,
            )
            .with_builder(
                replacement
                    .builder
                    .map_or(SQLiteInsertValue::Null, Into::into),
            )])
            .returning(())
            .get()
            .await?;
        for edge in replacement.edges {
            if let Some(asset_pk) = edge.asset_pk {
                validate_bound_dependency(tx, tables, workspace_pk, asset_pk, &edge.target).await?;
            }
            tx.insert(tables.job_edges)
                .values([InsertJobEdges::new(
                    inserted.job_id,
                    edge.target,
                    edge.key,
                    edge.platform,
                    edge.coupling,
                )
                .with_asset_pk(edge.asset_pk.map_or(SQLiteInsertValue::Null, Into::into))])
                .execute()
                .await?;
        }
        replacement_ids.push(inserted.job_id);
    }
    Ok(replacement_ids)
}

/// Retires the jobs a catalog plan delta drops, collecting the ids whose readiness
/// the caller must recompute.
async fn retire_catalog_plan_jobs(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    retire_job_ids: Vec<i64>,
    readiness_ids: &mut Vec<i64>,
) -> Result<(), drizzle::error::DrizzleError> {
    for job_id in retire_job_ids {
        let retired: Option<SelectJobs> = tx
            .select(())
            .from(tables.jobs)
            .r#where(and(
                eq(tables.jobs.workspace_pk, workspace_pk),
                eq(tables.jobs.job_id, job_id),
            ))
            .get()
            .await
            .optional()?;
        if let Some(retired) = retired {
            let dependents: Vec<SelectJobEdges> = tx
                .select(())
                .from(tables.job_edges)
                .r#where(and(
                    eq(tables.job_edges.asset_pk, Some(retired.asset_pk)),
                    and(
                        eq(tables.job_edges.key, retired.key.as_str()),
                        eq(tables.job_edges.platform, retired.platform.as_str()),
                    ),
                ))
                .all()
                .await?;
            readiness_ids.extend(dependents.into_iter().map(|edge| edge.job_pk));
        }
        tx.delete(tables.jobs)
            .r#where(and(
                eq(tables.jobs.workspace_pk, workspace_pk),
                eq(tables.jobs.job_id, job_id),
            ))
            .execute()
            .await?;
    }
    Ok(())
}

/// Applies the plan delta that rides along with a builder-catalog replacement.
async fn apply_catalog_plan_delta_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    plan_delta: PlanDelta,
) -> Result<(), drizzle::error::DrizzleError> {
    let registered_builders = registered_builder_guids(tx, tables, workspace_pk).await?;
    let mut readiness_ids = Vec::new();
    retire_catalog_plan_jobs(
        tx,
        tables,
        workspace_pk,
        plan_delta.retire_job_ids,
        &mut readiness_ids,
    )
    .await?;
    let replacement_ids = insert_catalog_plan_replacements(
        tx,
        tables,
        workspace_pk,
        plan_delta.replacements,
        &registered_builders,
    )
    .await?;
    readiness_ids.extend(replacement_ids);
    recompute_job_ids(tx, tables, readiness_ids).await?;
    for source_edge_id in plan_delta.retire_source_edge_ids {
        let edge: Option<SelectSourceEdges> = tx
            .select(())
            .from(tables.source_edges)
            .r#where(and(
                eq(tables.source_edges.workspace_pk, workspace_pk),
                eq(tables.source_edges.source_edge_id, source_edge_id),
            ))
            .get()
            .await
            .optional()?;
        if edge.is_some() {
            tx.delete(tables.source_edges)
                .r#where(and(
                    eq(tables.source_edges.workspace_pk, workspace_pk),
                    eq(tables.source_edges.source_edge_id, source_edge_id),
                ))
                .execute()
                .await?;
        }
    }
    for edge in plan_delta.source_edges {
        validate_registered_builder(&registered_builders, edge.builder, "source-analysis edge")?;
        let membership: Option<SelectEntries> = tx
            .select(())
            .from(tables.entries)
            .r#where(and(
                eq(tables.entries.workspace_pk, workspace_pk),
                eq(tables.entries.asset_pk, edge.asset_pk),
            ))
            .get()
            .await
            .optional()?;
        if membership.is_none() {
            return Err(drizzle::error::DrizzleError::Other(
                "source-analysis edge belongs to an asset outside the catalog workspace".into(),
            ));
        }
        if let Some(depends_pk) = edge.depends_pk {
            validate_bound_dependency(tx, tables, workspace_pk, depends_pk, &edge.target).await?;
        }
        tx.insert(tables.source_edges)
            .values([InsertSourceEdges::new(
                workspace_pk,
                edge.builder,
                edge.asset_pk,
                edge.target,
                edge.relation,
            )
            .with_depends_pk(edge.depends_pk.map_or(SQLiteInsertValue::Null, Into::into))])
            .execute()
            .await?;
    }
    validate_workspace_builder_closure(tx, tables, workspace_pk, &registered_builders).await?;
    Ok(())
}

/// Replaces one workspace's builder catalog inside an open transaction.
async fn replace_builder_catalog_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: ReplaceBuilderCatalog,
) -> Result<BuilderCatalogReplaceOutcome, drizzle::error::DrizzleError> {
    let workspace: Option<SelectWorkspaces> = tx
        .select(())
        .from(tables.workspaces)
        .r#where(eq(tables.workspaces.workspace_id, input.workspace_pk))
        .get()
        .await
        .optional()?;
    let workspace = workspace.ok_or(drizzle::error::DrizzleError::NotFound)?;
    if workspace.builders == Some(input.replacement) {
        if !input.plan_delta.retire_job_ids.is_empty()
            || !input.plan_delta.retire_source_edge_ids.is_empty()
            || !input.plan_delta.replacements.is_empty()
            || !input.plan_delta.source_edges.is_empty()
        {
            return Err(drizzle::error::DrizzleError::Other(
                "unchanged builder catalog carried a non-empty plan delta".into(),
            ));
        }
        return Ok(BuilderCatalogReplaceOutcome::Unchanged);
    }
    if workspace.builders != input.expected {
        return Ok(BuilderCatalogReplaceOutcome::Conflict {
            actual: workspace.builders,
        });
    }
    tx.delete(tables.builders)
        .r#where(eq(tables.builders.workspace_pk, input.workspace_pk))
        .execute()
        .await?;
    for builder in input.builders {
        tx.insert(tables.builders)
            .values([InsertBuilders::new(
                input.workspace_pk,
                builder.guid,
                builder.name,
                builder.version,
                builder.digest,
            )])
            .execute()
            .await?;
    }
    apply_catalog_plan_delta_in_tx(tx, tables, input.workspace_pk, input.plan_delta).await?;
    tx.update(tables.workspaces)
        .set(
            UpdateWorkspaces::default()
                .with_builders(input.replacement)
                .with_updated(input.updated),
        )
        .r#where(eq(tables.workspaces.workspace_id, input.workspace_pk))
        .execute()
        .await?;
    Ok(BuilderCatalogReplaceOutcome::Replaced)
}

fn replace_builder_catalog(
    db: &AssetDb,
    input: ReplaceBuilderCatalog,
) -> RepoResult<BuilderCatalogReplaceOutcome> {
    let tables = db.tables;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            replace_builder_catalog_in_tx(tx, tables, input).await
        })
        .wait()
        .map_err(storage)
}

fn writer_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

struct ClaimedJobTransactionContext {
    claimed_unix_ms: i64,
    job: SelectJobs,
    attempt: SelectAttempts,
    asset: SelectAssets,
    entry: SelectEntries,
    root: SelectRoots,
    workspace_root: SelectWorkspaceRoots,
    payload: Option<SelectPayloads>,
}

impl ClaimedJobTransactionContext {
    fn into_claimed_context(self, claimed_at: Instant) -> ClaimedJobContext {
        ClaimedJobContext {
            claimed_at,
            claimed_unix_ms: self.claimed_unix_ms,
            job: self.job,
            attempt: self.attempt,
            asset: self.asset,
            entry: self.entry,
            root: self.root,
            workspace_root: self.workspace_root,
            payload: self.payload,
        }
    }
}

/// Reads the job a claim names and returns it only if it is still claimable:
/// queued, ready, and on the attempt count the caller compared against.
async fn claimable_job(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: &ClaimReadyJob,
) -> Result<Option<SelectJobs>, drizzle::error::DrizzleError> {
    let job: Option<SelectJobs> = tx
        .select(())
        .from(tables.jobs)
        .r#where(eq(tables.jobs.job_id, input.job_id))
        .get()
        .await
        .optional()?;
    let Some(job) = job else {
        return Ok(None);
    };
    if job.status != Status::Queued || !job.ready || job.attempts != input.expected_attempts {
        return Ok(None);
    }
    Ok(Some(job))
}

/// Claims one ready job inside an open transaction.
async fn claim_ready_job_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: ClaimReadyJob,
    lease_duration_ms: i64,
) -> Result<Option<ClaimedJobTransactionContext>, drizzle::error::DrizzleError> {
    let Some(job) = claimable_job(tx, tables, &input).await? else {
        return Ok(None);
    };
    // This is the logical durable-claim boundary: SQLite has granted
    // the immediate transaction and the compare-and-set still holds.
    // Any queueing or lock wait before it cannot consume this lease.
    let claimed_unix_ms = writer_unix_ms();
    let diagnostic_expires = claimed_unix_ms.saturating_add(lease_duration_ms);
    let ordinal = job.attempts + 1;
    let job: SelectJobs = tx
        .update(tables.jobs)
        .set(
            UpdateJobs::default()
                .with_status(Status::Leased)
                .with_attempts(ordinal),
        )
        .r#where(and(
            eq(tables.jobs.job_id, input.job_id),
            and(
                eq(tables.jobs.status, Status::Queued),
                and(
                    eq(tables.jobs.ready, true),
                    eq(tables.jobs.attempts, input.expected_attempts),
                ),
            ),
        ))
        .returning(())
        .get()
        .await?;
    let attempt: SelectAttempts = tx
        .insert(tables.attempts)
        .values([InsertAttempts::new(job.job_id, ordinal, Status::Leased)
            .with_owner(input.owner)
            .with_expires(diagnostic_expires)
            .with_staging(input.staging)])
        .returning(())
        .get()
        .await?;
    let asset: SelectAssets = tx
        .select(())
        .from(tables.assets)
        .r#where(eq(tables.assets.asset_id, job.asset_pk))
        .get()
        .await?;
    let entry: SelectEntries = tx
        .select(())
        .from(tables.entries)
        .r#where(and(
            eq(tables.entries.workspace_pk, job.workspace_pk),
            eq(tables.entries.asset_pk, job.asset_pk),
        ))
        .get()
        .await?;
    let root: SelectRoots = tx
        .select(())
        .from(tables.roots)
        .r#where(eq(tables.roots.root_id, entry.root_pk))
        .get()
        .await?;
    let workspace_root: SelectWorkspaceRoots = tx
        .select(())
        .from(tables.workspace_roots)
        .r#where(and(
            eq(tables.workspace_roots.workspace_pk, job.workspace_pk),
            eq(tables.workspace_roots.root_pk, entry.root_pk),
        ))
        .get()
        .await?;
    let payload: Option<SelectPayloads> = tx
        .select(())
        .from(tables.payloads)
        .r#where(and(
            eq(tables.payloads.workspace_pk, job.workspace_pk),
            and(
                eq(tables.payloads.root_pk, entry.root_pk),
                eq(tables.payloads.path, entry.path.as_str()),
            ),
        ))
        .get()
        .await
        .optional()?;
    Ok(Some(ClaimedJobTransactionContext {
        claimed_unix_ms,
        job,
        attempt,
        asset,
        entry,
        root,
        workspace_root,
        payload,
    }))
}

fn claim_ready_job(db: &AssetDb, input: ClaimReadyJob) -> RepoResult<ClaimReadyJobResult> {
    // The duration crosses the RPC boundary, but neither writer clock does.
    // The diagnostic clock is persisted inside the transaction; the monotonic
    // clock is captured by this writer thread only after `.wait()` reports the
    // durable commit.
    let lease_duration_ms = i64::try_from(input.lease_duration_ms)
        .map_err(|_| RepoError::Invariant("lease duration exceeds i64 milliseconds".to_string()))?;
    let tables = db.tables;
    let mut context = db.transaction_context();
    let claimed = context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            claim_ready_job_in_tx(tx, tables, input, lease_duration_ms).await
        })
        .wait();
    let claimed_at = Instant::now();
    Ok(claimed
        .map_err(storage)?
        .map_or(ClaimReadyJobResult::NoLongerClaimable, |context| {
            ClaimReadyJobResult::Claimed {
                context: Box::new(context.into_claimed_context(claimed_at)),
            }
        }))
}

fn abandon_attempts(
    db: &AssetDb,
    input: AbandonAttempts,
) -> RepoResult<(AbandonAttemptsResult, Vec<(i64, String)>)> {
    let tables = db.tables;
    let mut context = db.transaction_context();
    let (result, invalidations) = context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            let mut result = AbandonAttemptsResult::default();
            let mut invalidations = Vec::new();
            for fence in input.attempts {
                let attempt: Option<SelectAttempts> = tx
                    .select(())
                    .from(tables.attempts)
                    .r#where(eq(tables.attempts.attempt_id, fence.attempt_id))
                    .get()
                    .await
                    .optional()?;
                let Some(attempt) = attempt else {
                    result.no_longer_owned.push(fence.attempt_id);
                    continue;
                };
                if attempt.status != Status::Leased
                    || attempt.owner.as_deref() != Some(&fence.owner)
                {
                    result.no_longer_owned.push(fence.attempt_id);
                    continue;
                }
                let job: Option<SelectJobs> = tx
                    .select(())
                    .from(tables.jobs)
                    .r#where(eq(tables.jobs.job_id, attempt.job_pk))
                    .get()
                    .await
                    .optional()?;
                let job = job.ok_or(drizzle::error::DrizzleError::NotFound)?;
                if job.status != Status::Leased || job.attempts != attempt.ordinal {
                    result.no_longer_owned.push(fence.attempt_id);
                    continue;
                }
                tx.update(tables.attempts)
                    .set(
                        UpdateAttempts::default()
                            .with_status(Status::Abandoned)
                            .with_finished(input.finished)
                            .with_expires(SQLiteUpdateValue::Null),
                    )
                    .r#where(eq(tables.attempts.attempt_id, attempt.attempt_id))
                    .execute()
                    .await?;
                if job.attempts < MAX_ASSET_JOB_ATTEMPTS {
                    tx.update(tables.jobs)
                        .set(UpdateJobs::default().with_status(Status::Queued))
                        .r#where(eq(tables.jobs.job_id, job.job_id))
                        .execute()
                        .await?;
                    result.requeued.push(job.job_id);
                } else {
                    tx.update(tables.jobs)
                        .set(UpdateJobs::default().with_status(Status::Failed))
                        .r#where(eq(tables.jobs.job_id, job.job_id))
                        .execute()
                        .await?;
                    result.exhausted.push(ExhaustedAttempt {
                        job_id: job.job_id,
                        diagnostic: ATTEMPT_LIMIT_EXHAUSTED,
                    });
                    invalidations.push((job.workspace_pk, job.platform));
                }
            }
            Ok((result, invalidations))
        })
        .wait()
        .map_err(storage)?;
    Ok((result, invalidations))
}

/// Admits one attempt completion: the attempt must still be leased by this owner,
/// its job must still be on that attempt, and the payload must match the job kind.
/// `Err` is a contract violation; `Ok(Err(..))` is the refusal to hand back.
#[allow(clippy::type_complexity)]
async fn admit_attempt_completion(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: &CompleteAttempt,
) -> Result<
    Result<(SelectAttempts, SelectJobs), (CompleteAttemptResult, Option<(i64, String)>)>,
    drizzle::error::DrizzleError,
> {
    let attempt: Option<SelectAttempts> = tx
        .select(())
        .from(tables.attempts)
        .r#where(eq(tables.attempts.attempt_id, input.attempt_id))
        .get()
        .await
        .optional()?;
    let Some(attempt) = attempt else {
        return Ok(Err((CompleteAttemptResult::NoLongerOwned, None)));
    };
    if attempt.status != Status::Leased || attempt.owner.as_deref() != Some(&input.owner) {
        return Ok(Err((CompleteAttemptResult::NoLongerOwned, None)));
    }
    let job: Option<SelectJobs> = tx
        .select(())
        .from(tables.jobs)
        .r#where(eq(tables.jobs.job_id, attempt.job_pk))
        .get()
        .await
        .optional()?;
    let job = job.ok_or(drizzle::error::DrizzleError::NotFound)?;
    if job.status != Status::Leased || job.attempts != attempt.ordinal {
        return Ok(Err((CompleteAttemptResult::NoLongerOwned, None)));
    }
    match (job.kind, input.status, input.plan_delta.is_some()) {
        (Work::Plan, Status::Succeeded, true)
            if input.products.is_empty() && input.job_edges.is_none() => {}
        (Work::Build, Status::Succeeded, false)
        | (_, Status::Failed | Status::Abandoned, false) => {}
        _ => {
            return Err(drizzle::error::DrizzleError::Other(
                "completion payload does not match its Plan or Build job kind".into(),
            ));
        }
    }
    Ok(Ok((attempt, job)))
}

/// Records the products a successful attempt produced, checking each one belongs to
/// the completing job.
async fn record_completed_products(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    products: Vec<ProductInput>,
    job_edges: Option<Vec<JobEdgeInput>>,
    job: &SelectJobs,
) -> Result<(), drizzle::error::DrizzleError> {
    for product in products {
        if product.asset_pk != job.asset_pk || product.platform != job.platform {
            return Err(drizzle::error::DrizzleError::Other(
                "completed product identity must match its owning job".into(),
            ));
        }
        let inserted: SelectProducts = tx
            .insert(tables.products)
            .values([InsertProducts::new(
                job.workspace_pk,
                product.asset_pk,
                product.platform,
                product.sub_id,
                job.job_id,
                product.path,
                product.kind,
                product.format,
                product.version,
                product.aliases,
                product.registration,
                product.digest,
                product.bytes,
            )])
            .returning(())
            .get()
            .await?;
        for edge in product.edges {
            tx.insert(tables.product_edges)
                .values([InsertProductEdges::new(
                    inserted.product_id,
                    edge.guid,
                    edge.sub_id,
                    edge.flags,
                )])
                .execute()
                .await?;
        }
    }
    if let Some(job_edges) = job_edges {
        tx.delete(tables.job_edges)
            .r#where(eq(tables.job_edges.job_pk, job.job_id))
            .execute()
            .await?;
        for edge in job_edges {
            if let Some(asset_pk) = edge.asset_pk {
                validate_bound_dependency(tx, tables, job.workspace_pk, asset_pk, &edge.target)
                    .await?;
            }
            tx.insert(tables.job_edges)
                .values([InsertJobEdges::new(
                    job.job_id,
                    edge.target,
                    edge.key,
                    edge.platform,
                    edge.coupling,
                )
                .with_asset_pk(edge.asset_pk.map_or(SQLiteInsertValue::Null, Into::into))])
                .execute()
                .await?;
        }
    }
    Ok(())
}

/// Commits one attempt completion inside an open transaction, returning the result and any catalog invalidation.
async fn complete_attempt_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: CompleteAttempt,
) -> Result<(CompleteAttemptResult, Option<(i64, String)>), drizzle::error::DrizzleError> {
    let (attempt, job) = match admit_attempt_completion(tx, tables, &input).await? {
        Ok(pair) => pair,
        Err(refusal) => return Ok(refusal),
    };
    if input.status == Status::Abandoned {
        tx.update(tables.attempts)
            .set(
                UpdateAttempts::default()
                    .with_status(Status::Abandoned)
                    .with_finished(input.finished)
                    .with_expires(SQLiteUpdateValue::Null),
            )
            .r#where(eq(tables.attempts.attempt_id, attempt.attempt_id))
            .execute()
            .await?;
        let retryable = job.attempts < MAX_ASSET_JOB_ATTEMPTS;
        tx.update(tables.jobs)
            .set(UpdateJobs::default().with_status(if retryable {
                Status::Queued
            } else {
                Status::Failed
            }))
            .r#where(eq(tables.jobs.job_id, job.job_id))
            .execute()
            .await?;
        let invalidation = Some((job.workspace_pk, job.platform.clone()));
        return Ok((
            CompleteAttemptResult::Abandoned {
                job_id: job.job_id,
                retryable,
                diagnostic: (!retryable).then_some(ATTEMPT_LIMIT_EXHAUSTED),
            },
            invalidation,
        ));
    }
    tx.update(tables.attempts)
        .set(
            UpdateAttempts::default()
                .with_status(input.status)
                .with_finished(input.finished)
                .with_expires(SQLiteUpdateValue::Null)
                .with_errors(input.errors)
                .with_warnings(input.warnings),
        )
        .r#where(eq(tables.attempts.attempt_id, attempt.attempt_id))
        .execute()
        .await?;
    let replaced_product_formats: Vec<(String,)> = tx
        .select((tables.products.format,))
        .from(tables.products)
        .r#where(eq(tables.products.job_pk, job.job_id))
        .all()
        .await?;
    let replaced_product_formats = replaced_product_formats
        .into_iter()
        .map(|(format,)| format)
        .collect();
    tx.delete(tables.products)
        .r#where(eq(tables.products.job_pk, job.job_id))
        .execute()
        .await?;
    if input.status == Status::Succeeded {
        record_completed_products(tx, tables, input.products, input.job_edges, &job).await?;
    }
    tx.update(tables.jobs)
        .set(UpdateJobs::default().with_status(input.status))
        .r#where(eq(tables.jobs.job_id, job.job_id))
        .execute()
        .await?;
    if let Some(delta) = input.plan_delta {
        apply_plan_delta_in_tx(tx, tables, job.workspace_pk, delta).await?;
    }
    let became_ready = recompute_dependents(tx, tables, &job).await?;
    let invalidation = Some((job.workspace_pk, job.platform));
    Ok((
        CompleteAttemptResult::Completed {
            job_id: job.job_id,
            became_ready,
            replaced_product_formats,
        },
        invalidation,
    ))
}

fn complete_attempt(
    db: &AssetDb,
    input: CompleteAttempt,
) -> RepoResult<(CompleteAttemptResult, Option<(i64, String)>)> {
    if !input.status.can_complete_from_worker() && input.status != Status::Abandoned {
        return Err(RepoError::Invariant(
            "worker completion status must be Succeeded, Failed, or Abandoned".to_owned(),
        ));
    }
    if input.status != Status::Succeeded
        && (!input.products.is_empty() || input.job_edges.is_some() || input.plan_delta.is_some())
    {
        return Err(RepoError::Invariant(
            "failed completion cannot publish products or dependency edges".to_owned(),
        ));
    }
    let tables = db.tables;
    let mut context = db.transaction_context();
    let (result, invalidation) = context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            complete_attempt_in_tx(tx, tables, input).await
        })
        .wait()
        .map_err(storage)?;
    Ok((result, invalidation))
}

async fn recompute_dependents(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    completed: &SelectJobs,
) -> Result<Vec<i64>, drizzle::error::DrizzleError> {
    let edges: Vec<SelectJobEdges> = tx
        .select(())
        .from(tables.job_edges)
        .r#where(and(
            eq(tables.job_edges.asset_pk, Some(completed.asset_pk)),
            and(
                eq(tables.job_edges.key, completed.key.as_str()),
                eq(tables.job_edges.platform, completed.platform.as_str()),
            ),
        ))
        .all()
        .await?;
    let mut became_ready = Vec::new();
    for job_id in edges.into_iter().map(|edge| edge.job_pk) {
        if recompute_job_readiness_in_tx(tx, tables, job_id).await? {
            became_ready.push(job_id);
        }
    }
    Ok(became_ready)
}

async fn recompute_job_ids(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    job_ids: impl IntoIterator<Item = i64>,
) -> Result<Vec<i64>, drizzle::error::DrizzleError> {
    let mut became_ready = Vec::new();
    for job_id in job_ids {
        if recompute_job_readiness_in_tx(tx, tables, job_id).await? {
            became_ready.push(job_id);
        }
    }
    Ok(became_ready)
}

/// The sole writer of `Jobs.ready`: evaluate the complete dependency
/// predicate from `JobEdges` and exact indexed Job targets in one transaction.
async fn recompute_job_readiness_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    job_id: i64,
) -> Result<bool, drizzle::error::DrizzleError> {
    let job: Option<SelectJobs> = tx
        .select(())
        .from(tables.jobs)
        .r#where(eq(tables.jobs.job_id, job_id))
        .get()
        .await
        .optional()?;
    let Some(job) = job else { return Ok(false) };
    if job.status != Status::Queued {
        return Ok(false);
    }
    let all_edges: Vec<SelectJobEdges> = tx
        .select(())
        .from(tables.job_edges)
        .r#where(eq(tables.job_edges.job_pk, job.job_id))
        .all()
        .await?;
    let mut ready = true;
    for dependency in all_edges {
        let Some(asset_pk) = dependency.asset_pk else {
            ready = false;
            break;
        };
        let matches: Vec<SelectJobs> = tx
            .select(())
            .from(tables.jobs)
            .r#where(and(
                eq(tables.jobs.workspace_pk, job.workspace_pk),
                and(
                    eq(tables.jobs.asset_pk, asset_pk),
                    and(
                        eq(tables.jobs.key, dependency.key.as_str()),
                        eq(tables.jobs.platform, dependency.platform.as_str()),
                    ),
                ),
            ))
            .all()
            .await?;
        if matches.is_empty()
            || matches
                .iter()
                .any(|dependency| dependency.status != Status::Succeeded)
        {
            ready = false;
            break;
        }
    }
    if ready != job.ready {
        tx.update(tables.jobs)
            .set(UpdateJobs::default().with_ready(ready))
            .r#where(eq(tables.jobs.job_id, job.job_id))
            .execute()
            .await?;
        return Ok(ready);
    }
    Ok(false)
}

fn resolve_idle_blocked(
    db: &AssetDb,
    input: ResolveIdleBlocked,
) -> RepoResult<ResolveIdleBlockedResult> {
    let tables = db.tables;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            let mut result = ResolveIdleBlockedResult::default();
            for job_id in input.job_ids {
                let job: Option<SelectJobs> = tx
                    .select(())
                    .from(tables.jobs)
                    .r#where(and(
                        eq(tables.jobs.workspace_pk, input.workspace_pk),
                        eq(tables.jobs.job_id, job_id),
                    ))
                    .get()
                    .await
                    .optional()?;
                let Some(job) = job else {
                    result.unchanged.push(job_id);
                    continue;
                };
                if job.status != Status::Queued || job.ready {
                    result.unchanged.push(job_id);
                    continue;
                }
                let edges: Vec<SelectJobEdges> = tx
                    .select(())
                    .from(tables.job_edges)
                    .r#where(eq(tables.job_edges.job_pk, job_id))
                    .all()
                    .await?;
                let mut fail_job = false;
                let mut dropped = false;
                for edge in edges {
                    let unsatisfiable = if let Some(asset_pk) = edge.asset_pk {
                        let matches: Vec<SelectJobs> = tx
                            .select(())
                            .from(tables.jobs)
                            .r#where(and(
                                eq(tables.jobs.workspace_pk, job.workspace_pk),
                                and(
                                    eq(tables.jobs.asset_pk, asset_pk),
                                    and(
                                        eq(tables.jobs.key, edge.key.as_str()),
                                        eq(tables.jobs.platform, edge.platform.as_str()),
                                    ),
                                ),
                            ))
                            .all()
                            .await?;
                        matches.is_empty()
                            || matches
                                .iter()
                                .any(|dependency| dependency.status == Status::Failed)
                    } else {
                        true
                    };
                    if !unsatisfiable {
                        continue;
                    }
                    if edge.coupling == Coupling::OrderOnly {
                        tx.delete(tables.job_edges)
                            .r#where(eq(tables.job_edges.job_edge_id, edge.job_edge_id))
                            .execute()
                            .await?;
                        result.dropped_order_only_edges.push(edge.job_edge_id);
                        dropped = true;
                    } else {
                        fail_job = true;
                    }
                }
                if fail_job {
                    tx.update(tables.jobs)
                        .set(UpdateJobs::default().with_status(Status::Failed))
                        .r#where(and(
                            eq(tables.jobs.job_id, job_id),
                            and(
                                eq(tables.jobs.status, Status::Queued),
                                eq(tables.jobs.ready, false),
                            ),
                        ))
                        .execute()
                        .await?;
                    result.failed_jobs.push(IdleFailedJob {
                        job_id,
                        platform: job.platform,
                        diagnostic: UNSATISFIABLE_DEPENDENCY,
                    });
                } else if recompute_job_readiness_in_tx(tx, tables, job_id).await? {
                    result.became_ready.push(job_id);
                } else if !dropped {
                    result.unchanged.push(job_id);
                }
            }
            Ok(result)
        })
        .wait()
        .map_err(storage)
}

/// Updates the payload row a recovered import lands on, refusing when the row on
/// disk does not match the baseline the import expected.
async fn update_imported_payload_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: ImportUnsavedPayload,
    workspace: &SelectWorkspaces,
    root: &SelectRoots,
    current: SelectPayloads,
) -> Result<ImportRecoveredPayloadResult, drizzle::error::DrizzleError> {
    let table = tables.payloads;
    if recovered_payload_matches(
        &current,
        &input.payload,
        workspace.workspace_id,
        root.root_id,
    ) {
        return Ok(ImportRecoveredPayloadResult::AlreadyPresent(current));
    }
    let ExpectedPayload::SavedAt { revision, digest } = input.expected else {
        return Ok(ImportRecoveredPayloadResult::BaselineConflict);
    };
    if current.revision != revision || current.saved != Some(revision) || current.digest != digest {
        return Ok(ImportRecoveredPayloadResult::BaselineConflict);
    }
    let imported = tx
        .update(table)
        .set(
            UpdatePayloads::default()
                .with_root_pk(root.root_id)
                .with_path(input.payload.path)
                .with_schema(input.payload.schema)
                .with_encoding(input.payload.encoding)
                .with_revision(input.payload.revision)
                .with_saved(
                    input
                        .payload
                        .saved
                        .map_or(SQLiteUpdateValue::Null, Into::into),
                )
                .with_digest(input.payload.digest)
                .with_bytes(input.payload.bytes)
                .with_payload(input.payload.payload)
                .with_checkpoint(
                    input
                        .payload
                        .checkpoint
                        .map_or(SQLiteUpdateValue::Null, Into::into),
                )
                .with_session(
                    input
                        .payload
                        .session
                        .map_or(SQLiteUpdateValue::Null, Into::into),
                )
                .with_project(input.payload.project)
                .with_deleted(input.payload.deleted)
                .with_updated(input.payload.updated),
        )
        .r#where(eq(table.payload_id, current.payload_id))
        .returning(())
        .get()
        .await?;
    Ok(ImportRecoveredPayloadResult::Imported(imported))
}

/// Writes one recovered payload row once its workspace and root are resolved.
async fn write_imported_payload_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: ImportUnsavedPayload,
    workspace: &SelectWorkspaces,
    root: &SelectRoots,
) -> Result<ImportRecoveredPayloadResult, drizzle::error::DrizzleError> {
    let table = tables.payloads;
    let current: Option<SelectPayloads> = tx
        .select(())
        .from(table)
        .r#where(and(
            eq(table.workspace_pk, workspace.workspace_id),
            eq(table.document, input.payload.document.as_str()),
        ))
        .get()
        .await
        .optional()?;
    if let Some(current) = current {
        update_imported_payload_in_tx(tx, tables, input, workspace, root, current).await
    } else {
        if input.expected != ExpectedPayload::Absent {
            return Ok(ImportRecoveredPayloadResult::BaselineConflict);
        }
        let imported = tx
            .insert(table)
            .values([InsertPayloads::new(
                workspace.workspace_id,
                root.root_id,
                input.payload.path,
                input.payload.document,
                input.payload.schema,
                input.payload.encoding,
                input.payload.revision,
                input.payload.digest,
                input.payload.bytes,
                input.payload.payload,
                input.payload.project,
                input.payload.created,
                input.payload.updated,
            )
            .with_saved(
                input
                    .payload
                    .saved
                    .map_or(SQLiteInsertValue::Null, Into::into),
            )
            .with_checkpoint(
                input
                    .payload
                    .checkpoint
                    .map_or(SQLiteInsertValue::Null, Into::into),
            )
            .with_session(
                input
                    .payload
                    .session
                    .map_or(SQLiteInsertValue::Null, Into::into),
            )
            .with_deleted(input.payload.deleted)])
            .returning(())
            .get()
            .await?;
        Ok(ImportRecoveredPayloadResult::Imported(imported))
    }
}

/// Checks that a source-payload write targets a workspace and root this project
/// owns, and that its checkpoint is coherent. `Some(..)` is the refusal to return.
async fn source_payload_scope_refusal(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: &WriteSourcePayload,
    checkpoint: &CheckpointWrite,
) -> Result<Option<WriteSourcePayloadResult>, drizzle::error::DrizzleError> {
    let workspace: Option<SelectWorkspaces> = tx
        .select(())
        .from(tables.workspaces)
        .r#where(eq(tables.workspaces.workspace_id, input.workspace_pk))
        .get()
        .await
        .optional()?;
    let Some(workspace) = workspace else {
        return Ok(Some(WriteSourcePayloadResult::ScopeMismatch));
    };
    let policy: Option<SelectWorkspaceRoots> = tx
        .select(())
        .from(tables.workspace_roots)
        .r#where(and(
            eq(tables.workspace_roots.workspace_pk, input.workspace_pk),
            eq(tables.workspace_roots.root_pk, input.root_pk),
        ))
        .get()
        .await
        .optional()?;
    if policy.is_none() || input.project != workspace.project {
        return Ok(Some(WriteSourcePayloadResult::ScopeMismatch));
    }
    if input.saved.is_some() && matches!(checkpoint, CheckpointWrite::Clear) {
        return Ok(Some(WriteSourcePayloadResult::InvalidCheckpoint));
    }
    Ok(None)
}

/// Imports one recovered payload inside an open transaction.
async fn import_payload_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: ImportUnsavedPayload,
) -> Result<ImportRecoveredPayloadResult, drizzle::error::DrizzleError> {
    let workspaces = tables.workspaces;
    let roots = tables.roots;
    let workspace_roots = tables.workspace_roots;
    let existing_workspace: Option<SelectWorkspaces> = tx
        .select(())
        .from(workspaces)
        .r#where(and(
            eq(
                workspaces.project,
                input.payload.workspace.key.project.as_str(),
            ),
            and(
                eq(workspaces.root, input.payload.workspace.key.root.as_str()),
                eq(
                    workspaces.branch,
                    input.payload.workspace.key.branch.as_str(),
                ),
            ),
        ))
        .get()
        .await
        .optional()?;
    let workspace: SelectWorkspaces = if let Some(workspace) = existing_workspace {
        if workspace.builders.is_some()
            || workspace.created != input.payload.workspace.created
            || workspace.updated != input.payload.workspace.updated
        {
            return Ok(ImportRecoveredPayloadResult::BaselineConflict);
        }
        workspace
    } else {
        tx.insert(workspaces)
            .values([InsertWorkspaces::new(
                input.payload.workspace.key.project.clone(),
                input.payload.workspace.key.root.clone(),
                input.payload.workspace.key.branch.clone(),
                input.payload.workspace.created,
                input.payload.workspace.updated,
            )])
            .returning(())
            .get()
            .await?
    };
    let existing_root: Option<SelectRoots> = tx
        .select(())
        .from(roots)
        .r#where(eq(roots.key, input.payload.root.key.as_str()))
        .get()
        .await
        .optional()?;
    let root: SelectRoots = if let Some(root) = existing_root {
        root
    } else {
        tx.insert(roots)
            .values([InsertRoots::new(input.payload.root.key.clone())])
            .returning(())
            .get()
            .await?
    };
    let policy: Option<SelectWorkspaceRoots> = tx
        .select(())
        .from(workspace_roots)
        .r#where(and(
            eq(workspace_roots.workspace_pk, workspace.workspace_id),
            eq(workspace_roots.root_pk, root.root_id),
        ))
        .get()
        .await
        .optional()?;
    if let Some(policy) = policy {
        if policy.owner != input.payload.root.owner
            || policy.path != input.payload.root.path
            || policy.exclusions != input.payload.root.exclusions
        {
            return Ok(ImportRecoveredPayloadResult::BaselineConflict);
        }
    } else {
        tx.insert(workspace_roots)
            .values([InsertWorkspaceRoots::new(
                workspace.workspace_id,
                root.root_id,
                input.payload.root.owner.clone(),
                input.payload.root.path.clone(),
                input.payload.root.exclusions.clone(),
            )])
            .execute()
            .await?;
    }
    write_imported_payload_in_tx(tx, tables, input, &workspace, &root).await
}

fn import_payload(
    db: &AssetDb,
    input: ImportUnsavedPayload,
) -> RepoResult<ImportRecoveredPayloadResult> {
    let tables = db.tables;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            import_payload_in_tx(tx, tables, input).await
        })
        .wait()
        .map_err(storage)
}

fn recovered_payload_matches(
    current: &SelectPayloads,
    recovered: &UnsavedPayload,
    workspace_pk: i64,
    root_pk: i64,
) -> bool {
    current.workspace_pk == workspace_pk
        && current.root_pk == root_pk
        && current.path == recovered.path
        && current.document == recovered.document
        && current.schema == recovered.schema
        && current.encoding == recovered.encoding
        && current.revision == recovered.revision
        && current.saved == recovered.saved
        && current.digest == recovered.digest
        && current.bytes == recovered.bytes
        && current.payload == recovered.payload
        && current.checkpoint == recovered.checkpoint
        && current.session == recovered.session
        && current.project == recovered.project
        && current.deleted == recovered.deleted
        && current.created == recovered.created
        && current.updated == recovered.updated
}

fn write_source_payload(
    db: &AssetDb,
    input: WriteSourcePayload,
) -> RepoResult<WriteSourcePayloadResult> {
    let tables = db.tables;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            write_source_payload_in_tx(tx, tables, input).await
        })
        .wait()
        .map_err(storage)
}

async fn write_source_payload_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: WriteSourcePayload,
) -> Result<WriteSourcePayloadResult, drizzle::error::DrizzleError> {
    let checkpoint = input.checkpoint.clone();
    if let Some(refusal) = source_payload_scope_refusal(tx, tables, &input, &checkpoint).await? {
        return Ok(refusal);
    }
    let current: Option<SelectPayloads> = tx
        .select(())
        .from(tables.payloads)
        .r#where(and(
            eq(tables.payloads.workspace_pk, input.workspace_pk),
            eq(tables.payloads.document, input.document.as_str()),
        ))
        .get()
        .await
        .optional()?;
    if let Some(current) = current {
        if input.expected_revision != Some(current.revision) {
            return Ok(WriteSourcePayloadResult::Conflict(current));
        }
        if matches!(&checkpoint, CheckpointWrite::Preserve) && input.saved != current.saved {
            return Ok(WriteSourcePayloadResult::InvalidCheckpoint);
        }
        let update = UpdatePayloads::default()
            .with_root_pk(input.root_pk)
            .with_path(input.path)
            .with_schema(input.schema)
            .with_encoding(input.encoding)
            .with_revision(input.revision)
            .with_saved(input.saved.map_or(SQLiteUpdateValue::Null, Into::into))
            .with_digest(input.digest)
            .with_bytes(i64::try_from(input.payload.len()).unwrap_or(i64::MAX))
            .with_payload(input.payload)
            .with_session(input.session.map_or(SQLiteUpdateValue::Null, Into::into))
            .with_project(input.project)
            .with_deleted(false)
            .with_updated(input.now);
        let update = match checkpoint {
            CheckpointWrite::Preserve => update,
            CheckpointWrite::Replace(value) => update.with_checkpoint(value),
            CheckpointWrite::Clear => update.with_checkpoint(SQLiteUpdateValue::Null),
        };
        let written = tx
            .update(tables.payloads)
            .set(update)
            .r#where(and(
                eq(tables.payloads.payload_id, current.payload_id),
                eq(tables.payloads.revision, current.revision),
            ))
            .returning(())
            .get()
            .await?;
        Ok(WriteSourcePayloadResult::Written(written))
    } else {
        if input.expected_revision.is_some() {
            return Ok(WriteSourcePayloadResult::Missing);
        }
        if matches!(&checkpoint, CheckpointWrite::Preserve) {
            return Ok(WriteSourcePayloadResult::InvalidCheckpoint);
        }
        let insert = InsertPayloads::new(
            input.workspace_pk,
            input.root_pk,
            input.path,
            input.document,
            input.schema,
            input.encoding,
            input.revision,
            input.digest,
            i64::try_from(input.payload.len()).unwrap_or(i64::MAX),
            input.payload,
            input.project,
            input.now,
            input.now,
        )
        .with_saved(input.saved.map_or(SQLiteInsertValue::Null, Into::into))
        .with_session(input.session.map_or(SQLiteInsertValue::Null, Into::into));
        let written = match checkpoint {
            CheckpointWrite::Replace(value) => {
                tx.insert(tables.payloads)
                    .values([insert.with_checkpoint(value)])
                    .returning(())
                    .get()
                    .await?
            }
            CheckpointWrite::Clear => {
                tx.insert(tables.payloads)
                    .values([insert])
                    .returning(())
                    .get()
                    .await?
            }
            CheckpointWrite::Preserve => unreachable!("validated above"),
        };
        Ok(WriteSourcePayloadResult::Written(written))
    }
}

fn publish_authored_source(
    db: &AssetDb,
    input: PublishAuthoredSource,
) -> RepoResult<PublishAuthoredSourceResult> {
    let tables = db.tables;
    let workspace_pk = input.payload.workspace_pk;
    let root_pk = input.payload.root_pk;
    let workspace_root_pk = input.workspace_root_pk;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            if input.source.path != input.payload.path
                || input.source.digest != input.payload.digest
            {
                return Ok(PublishAuthoredSourceResult::InvalidSourceProjection);
            }
            let policy: Option<SelectWorkspaceRoots> = tx
                .select(())
                .from(tables.workspace_roots)
                .r#where(eq(
                    tables.workspace_roots.workspace_root_id,
                    workspace_root_pk,
                ))
                .get()
                .await
                .optional()?;
            if !policy.is_some_and(|policy| {
                policy.workspace_pk == workspace_pk && policy.root_pk == root_pk
            }) {
                return Ok(PublishAuthoredSourceResult::ScopeMismatch);
            }
            let occupied =
                workspace_asset_at_locator(tx, tables, workspace_pk, root_pk, &input.source.path)
                    .await?;
            if let Some((_, asset)) = occupied
                && asset.guid != input.source.guid
            {
                return Ok(PublishAuthoredSourceResult::LocatorConflict { asset });
            }
            let payload = match write_source_payload_in_tx(tx, tables, input.payload).await? {
                WriteSourcePayloadResult::Written(payload) => payload,
                WriteSourcePayloadResult::Conflict(row) => {
                    return Ok(PublishAuthoredSourceResult::Conflict(Box::new(row)));
                }
                WriteSourcePayloadResult::Missing => {
                    return Ok(PublishAuthoredSourceResult::Missing);
                }
                WriteSourcePayloadResult::ScopeMismatch => {
                    return Ok(PublishAuthoredSourceResult::ScopeMismatch);
                }
                WriteSourcePayloadResult::InvalidCheckpoint => {
                    return Ok(PublishAuthoredSourceResult::InvalidCheckpoint);
                }
            };
            let (asset, entry) =
                upsert_sweep_record_in_tx(tx, tables, workspace_pk, root_pk, input.source).await?;
            let published = PublishedAuthoredSource {
                asset,
                entry,
                payload,
            };
            Ok(PublishAuthoredSourceResult::Written(Box::new(published)))
        })
        .wait()
        .map_err(storage)
}

/// Upserts the workspace entry a swept asset resolves to, returning the durable row.
async fn upsert_sweep_entry_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    root_pk: i64,
    asset: &SelectAssets,
    record: SweepEntry,
) -> Result<SelectEntries, drizzle::error::DrizzleError> {
    let current: Option<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(and(
            eq(tables.entries.workspace_pk, workspace_pk),
            eq(tables.entries.asset_pk, asset.asset_id),
        ))
        .get()
        .await
        .optional()?;
    let entry: SelectEntries = if let Some(entry) = current {
        tx.update(tables.entries)
            .set(
                UpdateEntries::default()
                    .with_root_pk(root_pk)
                    .with_path(record.path.clone())
                    .with_schema(
                        record
                            .schema
                            .clone()
                            .map_or(SQLiteUpdateValue::Null, Into::into),
                    )
                    .with_digest(record.digest)
                    .with_diff(record.diff)
                    .with_diagnostics(record.diagnostics)
                    .with_updated(record.updated)
                    .with_src_bytes(record.src_bytes)
                    .with_src_mtime(record.src_mtime)
                    .with_meta_bytes(record.meta_bytes)
                    .with_meta_mtime(record.meta_mtime)
                    .with_observed(record.observed),
            )
            .r#where(eq(tables.entries.entry_id, entry.entry_id))
            .returning(())
            .get()
            .await?
    } else {
        tx.insert(tables.entries)
            .values([InsertEntries::new(
                workspace_pk,
                asset.asset_id,
                root_pk,
                record.path,
                record.digest,
                record.diff,
                record.updated,
                record.src_bytes,
                record.src_mtime,
                record.meta_bytes,
                record.meta_mtime,
                record.observed,
            )
            .with_schema(record.schema.map_or(SQLiteInsertValue::Null, Into::into))
            .with_diagnostics(record.diagnostics)])
            .returning(())
            .get()
            .await?
    };
    Ok(entry)
}

async fn upsert_sweep_record_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    workspace_pk: i64,
    root_pk: i64,
    record: SweepEntry,
) -> Result<(SelectAssets, SelectEntries), drizzle::error::DrizzleError> {
    let at_locator =
        workspace_asset_at_locator(tx, tables, workspace_pk, root_pk, &record.path).await?;
    if at_locator
        .as_ref()
        .is_some_and(|(_, existing)| existing.guid != record.guid)
    {
        return Err(drizzle::error::DrizzleError::Other(
            format!(
                "source locator {} resolved to a different stable asset identity",
                record.path
            )
            .into(),
        ));
    }
    let by_identity: Option<SelectAssets> = tx
        .select(())
        .from(tables.assets)
        .r#where(eq(tables.assets.guid, record.guid))
        .get()
        .await
        .optional()?;
    let asset = if let Some(existing) = by_identity.or_else(|| at_locator.map(|(_, asset)| asset)) {
        maintain_path_history(
            tx,
            tables,
            workspace_pk,
            existing.asset_id,
            root_pk,
            &record.path,
            record.digest,
            record.session.clone(),
            record.updated,
        )
        .await?;
        tx.update(tables.assets)
            .set(
                UpdateAssets::default()
                    .with_deleted(false)
                    .with_updated(record.updated),
            )
            .r#where(eq(tables.assets.asset_id, existing.asset_id))
            .returning(())
            .get()
            .await?
    } else {
        let inserted: SelectAssets = tx
            .insert(tables.assets)
            .values([InsertAssets::new(
                record.guid,
                record.updated,
                record.updated,
            )])
            .returning(())
            .get()
            .await?;
        tx.insert(tables.paths)
            .values([InsertPaths::new(
                workspace_pk,
                inserted.asset_id,
                root_pk,
                record.path.clone(),
                record.digest,
                record.updated,
            )
            .with_session(
                record
                    .session
                    .clone()
                    .map_or(SQLiteInsertValue::Null, Into::into),
            )])
            .execute()
            .await?;
        inserted
    };
    let entry = upsert_sweep_entry_in_tx(tx, tables, workspace_pk, root_pk, &asset, record).await?;
    bind_authored_job_edges(tx, tables, workspace_pk, &asset, &entry.path).await?;
    bind_authored_source_edges(tx, tables, workspace_pk, &asset, &entry.path).await?;
    Ok((asset, entry))
}

enum SourceStateAssessment {
    Matches,
    Conflict,
    Unsaved,
}

async fn assess_source_state(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    entry: &SelectEntries,
    expected: SourceStateToken,
) -> Result<SourceStateAssessment, drizzle::error::DrizzleError> {
    if entry.digest != expected.digest {
        return Ok(SourceStateAssessment::Conflict);
    }
    let payload: Option<SelectPayloads> = tx
        .select(())
        .from(tables.payloads)
        .r#where(and(
            eq(tables.payloads.workspace_pk, entry.workspace_pk),
            and(
                eq(tables.payloads.root_pk, entry.root_pk),
                eq(tables.payloads.path, entry.path.as_str()),
            ),
        ))
        .get()
        .await
        .optional()?;
    if payload.as_ref().map(|payload| payload.revision) != expected.revision {
        return Ok(SourceStateAssessment::Conflict);
    }
    if payload.as_ref().is_some_and(|payload| {
        payload.saved != Some(payload.revision) || payload.checkpoint.is_none()
    }) {
        return Ok(SourceStateAssessment::Unsaved);
    }
    Ok(SourceStateAssessment::Matches)
}

/// Moves one source path inside an open transaction.
async fn move_source_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: MoveSource,
) -> Result<MoveSourceResult, drizzle::error::DrizzleError> {
    let entry: Option<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(and(
            eq(tables.entries.workspace_pk, input.workspace_pk),
            and(
                eq(tables.entries.root_pk, input.root_pk),
                eq(tables.entries.path, input.from.as_str()),
            ),
        ))
        .get()
        .await
        .optional()?;
    let Some(entry) = entry else {
        return Ok(MoveSourceResult::NotFound);
    };
    match assess_source_state(tx, tables, &entry, input.expected).await? {
        SourceStateAssessment::Matches => {}
        SourceStateAssessment::Conflict => return Ok(MoveSourceResult::Conflict),
        SourceStateAssessment::Unsaved => return Ok(MoveSourceResult::Unsaved),
    }
    let collision: Option<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(and(
            eq(tables.entries.workspace_pk, input.workspace_pk),
            and(
                eq(tables.entries.root_pk, input.root_pk),
                eq(tables.entries.path, input.to.as_str()),
            ),
        ))
        .get()
        .await
        .optional()?;
    if collision.is_some() {
        return Ok(MoveSourceResult::Conflict);
    }
    let asset: Option<SelectAssets> = tx
        .select(())
        .from(tables.assets)
        .r#where(eq(tables.assets.asset_id, entry.asset_pk))
        .get()
        .await
        .optional()?;
    let asset = asset.ok_or(drizzle::error::DrizzleError::NotFound)?;
    maintain_path_history(
        tx,
        tables,
        input.workspace_pk,
        asset.asset_id,
        input.root_pk,
        &input.to,
        entry.digest,
        None,
        input.now,
    )
    .await?;
    let asset: SelectAssets = tx
        .update(tables.assets)
        .set(UpdateAssets::default().with_updated(input.now))
        .r#where(eq(tables.assets.asset_id, asset.asset_id))
        .returning(())
        .get()
        .await?;
    let entry: SelectEntries = tx
        .update(tables.entries)
        .set(
            UpdateEntries::default()
                .with_root_pk(input.root_pk)
                .with_path(input.to.clone())
                .with_diff(Diff::Modified)
                .with_updated(input.now),
        )
        .r#where(eq(tables.entries.entry_id, entry.entry_id))
        .returning(())
        .get()
        .await?;
    tx.update(tables.payloads)
        .set(
            UpdatePayloads::default()
                .with_root_pk(input.root_pk)
                .with_path(input.to)
                .with_updated(input.now),
        )
        .r#where(and(
            eq(tables.payloads.workspace_pk, input.workspace_pk),
            and(
                eq(tables.payloads.root_pk, entry.root_pk),
                eq(tables.payloads.path, input.from.as_str()),
            ),
        ))
        .execute()
        .await?;
    Ok(MoveSourceResult::Moved(Box::new(MovedSource {
        asset,
        entry,
    })))
}

fn move_source(db: &AssetDb, input: MoveSource) -> RepoResult<MoveSourceResult> {
    let tables = db.tables;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            move_source_in_tx(tx, tables, input).await
        })
        .wait()
        .map_err(storage)
}

/// Marks an asset identity deleted once no workspace entry still observes it.
async fn retire_unobserved_asset_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    asset_id: i64,
    observed: i64,
) -> Result<(), drizzle::error::DrizzleError> {
    let remaining_entries: Vec<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(eq(tables.entries.asset_pk, asset_id))
        .all()
        .await?;
    let globally_unobserved = remaining_entries
        .iter()
        .all(|entry| entry.diff == Diff::Deleted);
    if globally_unobserved {
        tx.update(tables.assets)
            .set(
                UpdateAssets::default()
                    .with_deleted(true)
                    .with_updated(observed),
            )
            .r#where(eq(tables.assets.asset_id, asset_id))
            .execute()
            .await?;
    }
    Ok(())
}

/// Recomputes readiness for every job that depended on a deleted source.
async fn requeue_dependents_of_deleted_source(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    asset_id: i64,
    workspace_pk: i64,
) -> Result<(), drizzle::error::DrizzleError> {
    let dependent_edges: Vec<SelectJobEdges> = tx
        .select(())
        .from(tables.job_edges)
        .r#where(eq(tables.job_edges.asset_pk, Some(asset_id)))
        .all()
        .await?;
    let mut affected = Vec::new();
    for edge in dependent_edges {
        let owner: Option<SelectJobs> = tx
            .select(())
            .from(tables.jobs)
            .r#where(and(
                eq(tables.jobs.job_id, edge.job_pk),
                eq(tables.jobs.workspace_pk, workspace_pk),
            ))
            .get()
            .await
            .optional()?;
        if owner.is_some() {
            tx.update(tables.job_edges)
                .set(UpdateJobEdges::default().with_asset_pk(SQLiteUpdateValue::Null))
                .r#where(eq(tables.job_edges.job_edge_id, edge.job_edge_id))
                .execute()
                .await?;
            affected.push(edge.job_pk);
        }
    }
    tx.update(tables.source_edges)
        .set(UpdateSourceEdges::default().with_depends_pk(SQLiteUpdateValue::Null))
        .r#where(and(
            eq(tables.source_edges.workspace_pk, workspace_pk),
            eq(tables.source_edges.depends_pk, Some(asset_id)),
        ))
        .execute()
        .await?;
    tx.delete(tables.jobs)
        .r#where(and(
            eq(tables.jobs.workspace_pk, workspace_pk),
            eq(tables.jobs.asset_pk, asset_id),
        ))
        .execute()
        .await?;
    recompute_job_ids(tx, tables, affected).await?;
    Ok(())
}

/// Deletes one source and its dependent rows inside an open transaction.
async fn delete_source_in_tx(
    tx: &drizzle::sqlite::turso::Transaction<'_, AssetSchema>,
    tables: AssetSchema,
    input: &DeleteSource,
) -> Result<DeleteSourceResult, drizzle::error::DrizzleError> {
    let entry: Option<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(and(
            eq(tables.entries.workspace_pk, input.workspace_pk),
            and(
                eq(tables.entries.root_pk, input.root_pk),
                eq(tables.entries.path, input.path.as_str()),
            ),
        ))
        .get()
        .await
        .optional()?;
    let Some(entry) = entry else {
        return Ok(DeleteSourceResult::NotFound);
    };
    match assess_source_state(tx, tables, &entry, input.expected).await? {
        SourceStateAssessment::Matches => {}
        SourceStateAssessment::Conflict => return Ok(DeleteSourceResult::Conflict),
        SourceStateAssessment::Unsaved => return Ok(DeleteSourceResult::Unsaved),
    }
    let asset: Option<SelectAssets> = tx
        .select(())
        .from(tables.assets)
        .r#where(eq(tables.assets.asset_id, entry.asset_pk))
        .get()
        .await
        .optional()?;
    let asset = asset.ok_or(drizzle::error::DrizzleError::NotFound)?;
    let entry: SelectEntries = tx
        .update(tables.entries)
        .set(
            UpdateEntries::default()
                .with_diff(Diff::Deleted)
                .with_observed(input.now)
                .with_updated(input.now),
        )
        .r#where(eq(tables.entries.entry_id, entry.entry_id))
        .returning(())
        .get()
        .await?;
    let remaining_entries: Vec<SelectEntries> = tx
        .select(())
        .from(tables.entries)
        .r#where(eq(tables.entries.asset_pk, asset.asset_id))
        .all()
        .await?;
    let globally_unobserved = remaining_entries
        .iter()
        .all(|entry| entry.diff == Diff::Deleted);
    let asset = if globally_unobserved {
        tx.update(tables.assets)
            .set(
                UpdateAssets::default()
                    .with_deleted(true)
                    .with_updated(input.now),
            )
            .r#where(eq(tables.assets.asset_id, asset.asset_id))
            .returning(())
            .get()
            .await?
    } else {
        asset
    };
    close_workspace_asset_path(tx, tables, input.workspace_pk, asset.asset_id, input.now).await?;
    tx.update(tables.payloads)
        .set(
            UpdatePayloads::default()
                .with_deleted(true)
                .with_updated(input.now),
        )
        .r#where(and(
            eq(tables.payloads.workspace_pk, input.workspace_pk),
            and(
                eq(tables.payloads.root_pk, input.root_pk),
                eq(tables.payloads.path, input.path.as_str()),
            ),
        ))
        .execute()
        .await?;
    requeue_dependents_of_deleted_source(tx, tables, asset.asset_id, input.workspace_pk).await?;
    Ok(DeleteSourceResult::Deleted(Box::new(DeletedSource {
        asset,
        entry,
    })))
}

fn delete_source(db: &AssetDb, input: &DeleteSource) -> RepoResult<DeleteSourceResult> {
    let tables = db.tables;
    let mut context = db.transaction_context();
    context
        .transaction(SQLiteTransactionType::Immediate, async |tx| {
            delete_source_in_tx(tx, tables, input).await
        })
        .wait()
        .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! test_blocking_writer_calls {
        ($(($blocking:ident, $method:ident, $input:ty, $output:ty)),+ $(,)?) => {
            trait TestAssetDbWriterBlockingExt {
                $(fn $blocking(&self, input: $input) -> RepoResult<$output>;)+
            }

            impl TestAssetDbWriterBlockingExt for AssetDbWriter {
                $(
                    fn $blocking(&self, input: $input) -> RepoResult<$output> {
                        self.$method(input).wait_blocking()
                    }
                )+
            }
        };
    }

    test_blocking_writer_calls!(
        (
            register_workspace_blocking,
            register_workspace,
            RegisterWorkspace,
            SelectWorkspaces
        ),
        (
            register_workspace_root_blocking,
            register_workspace_root,
            RegisterWorkspaceRoot,
            (SelectRoots, SelectWorkspaceRoots)
        ),
        (
            replace_workspace_roots_blocking,
            replace_workspace_roots,
            ReplaceWorkspaceRoots,
            Vec<WorkspaceRootBinding>
        ),
        (
            apply_sweep_delta_blocking,
            apply_sweep_delta,
            ApplySweepDelta,
            SweepDeltaResult
        ),
        (
            replace_builder_catalog_blocking,
            replace_builder_catalog,
            ReplaceBuilderCatalog,
            BuilderCatalogReplaceOutcome
        ),
        (
            apply_plan_delta_blocking,
            apply_plan_delta,
            ApplyPlanDelta,
            ()
        ),
        (
            claim_ready_job_blocking,
            claim_ready_job,
            ClaimReadyJob,
            ClaimReadyJobResult
        ),
        (
            abandon_attempts_blocking,
            abandon_attempts,
            AbandonAttempts,
            AbandonAttemptsResult
        ),
        (
            complete_attempt_blocking,
            complete_attempt,
            CompleteAttempt,
            CompleteAttemptResult
        ),
        (
            resolve_idle_blocked_blocking,
            resolve_idle_blocked,
            ResolveIdleBlocked,
            ResolveIdleBlockedResult
        ),
        (
            import_unsaved_payload_blocking,
            import_unsaved_payload,
            ImportUnsavedPayload,
            ImportRecoveredPayloadResult
        ),
        (
            write_source_payload_blocking,
            write_source_payload,
            WriteSourcePayload,
            WriteSourcePayloadResult
        ),
        (
            publish_authored_source_blocking,
            publish_authored_source,
            PublishAuthoredSource,
            PublishAuthoredSourceResult
        ),
        (
            move_source_blocking,
            move_source,
            MoveSource,
            MoveSourceResult
        ),
        (
            delete_source_blocking,
            delete_source,
            DeleteSource,
            DeleteSourceResult
        ),
    );

    #[test]
    fn optional_row_boundary_maps_only_not_found() {
        let absent: Result<i64, _> = Err(drizzle::error::DrizzleError::NotFound);
        assert_eq!(absent.optional().unwrap(), None);

        let present: Result<i64, drizzle::error::DrizzleError> = Ok(7);
        assert_eq!(present.optional().unwrap(), Some(7));

        let failure: Result<i64, _> = Err(drizzle::error::DrizzleError::Other("boom".into()));
        assert!(matches!(
            failure.optional(),
            Err(drizzle::error::DrizzleError::Other(_))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn writer_reply_wait_does_not_block_other_local_work() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        drop(db);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let operation = tokio::spawn({
            let writer = writer.clone();
            async move { writer.test_barrier(entered_tx, release_rx).await }
        });
        tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
            .await
            .unwrap();

        let progressed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let marker = tokio::spawn({
            let progressed = Arc::clone(&progressed);
            async move {
                tokio::task::yield_now().await;
                progressed.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
        marker.await.unwrap();
        assert!(progressed.load(std::sync::atomic::Ordering::SeqCst));

        release_tx.send(()).unwrap();
        operation.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    #[should_panic(expected = "Cannot block the current thread from within a runtime")]
    async fn blocking_writer_wait_fails_fast_inside_async_runtime() {
        let db = AssetDb::open_in_memory().unwrap();
        db.writer()
            .unwrap()
            .register_workspace(RegisterWorkspace {
                key: workspace_key("wrong-boundary"),
                now: 1,
            })
            .wait_blocking()
            .unwrap();
    }

    fn digest(label: &str) -> Digest {
        blake3::hash(label.as_bytes()).into()
    }

    fn workspace_key(label: &str) -> WorkspaceKey {
        WorkspaceKey {
            project: label.into(),
            root: std::env::temp_dir()
                .join(label)
                .to_string_lossy()
                .into_owned(),
            branch: "main".into(),
        }
    }

    fn register_test_workspace(
        writer: &AssetDbWriter,
        label: &str,
    ) -> (SelectWorkspaces, SelectWorkspaceRoots) {
        let workspace = writer
            .register_workspace_blocking(RegisterWorkspace {
                key: workspace_key(label),
                now: 1,
            })
            .unwrap();
        let (_, root) = writer
            .register_workspace_root_blocking(RegisterWorkspaceRoot {
                workspace_pk: workspace.workspace_id,
                key: label.into(),
                owner: label.into(),
                path: "assets".into(),
                exclusions: Exclusions::default(),
            })
            .unwrap();
        (workspace, root)
    }

    fn test_builder_descriptor(guid: Uuid, name: &str) -> BuilderDescriptor {
        BuilderDescriptor {
            guid,
            name: name.into(),
            version: 1,
            digest: digest(name),
        }
    }

    fn observe_asset(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        root: &SelectWorkspaceRoots,
        path: &str,
        guid: Uuid,
    ) -> SelectAssets {
        observe_asset_with_projection(db, writer, workspace, root, path, guid, None, digest(path))
    }

    #[allow(clippy::too_many_arguments)]
    fn observe_asset_with_projection(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        root: &SelectWorkspaceRoots,
        path: &str,
        guid: Uuid,
        schema: Option<&str>,
        source_digest: Digest,
    ) -> SelectAssets {
        writer
            .apply_sweep_delta_blocking(ApplySweepDelta {
                workspace_pk: workspace.workspace_id,
                workspace_root_pk: root.workspace_root_id,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: path.into(),
                        guid,
                        schema: schema.map(str::to_owned),
                        digest: source_digest,
                        diff: Diff::Clean,
                        diagnostics: 0,
                        updated: 2,
                        src_bytes: 1,
                        src_mtime: 2,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 2,
                        session: None,
                    },
                    planner: SweepPlannerJob {
                        key: "azoth.asset-planner".into(),
                        platform: "pc".into(),
                    },
                }],
                removals: Vec::new(),
            })
            .unwrap();
        let asset: Option<SelectAssets> = db
            .drizzle
            .select(())
            .from(db.tables.assets)
            .r#where(eq(db.tables.assets.guid, guid))
            .get()
            .wait()
            .optional()
            .unwrap();
        asset.unwrap()
    }

    #[test]
    fn empty_sweep_delta_is_a_true_noop() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        drop(db);
        let result = writer
            .apply_sweep_delta_blocking(ApplySweepDelta {
                workspace_pk: 1,
                workspace_root_pk: 1,
                records: Vec::new(),
                removals: Vec::new(),
            })
            .unwrap();
        assert_eq!(result, SweepDeltaResult::default());
    }

    /// A sweep that would collide two identities on one locator must fail whole and
    /// leave no rolled-back jobs behind.
    fn assert_conflicting_sweep_rolls_back_whole(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        root: &SelectWorkspaceRoots,
    ) {
        let first_guid = Uuid::new_v4();
        let failed = writer.apply_sweep_delta_blocking(ApplySweepDelta {
            workspace_pk: workspace.workspace_id,
            workspace_root_pk: root.workspace_root_id,
            records: vec![
                SweepRecord {
                    source: authored_sweep("rolled-back.ron", first_guid, digest("first")),
                    planner: SweepPlannerJob {
                        key: "azoth.asset-planner".into(),
                        platform: "pc".into(),
                    },
                },
                SweepRecord {
                    source: authored_sweep("source.ron", Uuid::new_v4(), digest("collision")),
                    planner: SweepPlannerJob {
                        key: "azoth.asset-planner".into(),
                        platform: "pc".into(),
                    },
                },
            ],
            removals: Vec::new(),
        });
        assert!(failed.is_err());
        assert!(
            db.source_asset(workspace.workspace_id, root.root_pk, "rolled-back.ron")
                .unwrap()
                .is_none()
        );
        let rolled_back_jobs: Vec<(SelectJobs, SelectAssets)> = db
            .drizzle
            .select(())
            .from(db.tables.jobs)
            .inner_join((
                db.tables.assets,
                eq(db.tables.jobs.asset_pk, db.tables.assets.asset_id),
            ))
            .r#where(eq(db.tables.assets.guid, first_guid))
            .all()
            .wait()
            .unwrap();
        assert!(rolled_back_jobs.is_empty());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn sweep_commits_entry_and_unique_plan_together_without_retiring_build_jobs() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "sweep-plan");
        let guid = Uuid::new_v4();
        let asset = observe_asset(&db, &writer, &workspace, &root, "source.ron", guid);
        let builder = Uuid::new_v4();
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace.workspace_id,
                expected: None,
                replacement: digest("sweep-plan-catalog"),
                builders: vec![test_builder_descriptor(builder, "compile")],
                plan_delta: PlanDelta::default(),
                updated: 2,
            })
            .unwrap();
        writer
            .apply_plan_delta_blocking(ApplyPlanDelta {
                workspace_pk: workspace.workspace_id,
                delta: PlanDelta {
                    replacements: vec![PlannedJob {
                        asset_pk: asset.asset_id,
                        kind: Work::Build,
                        builder: Some(builder),
                        key: "compile".into(),
                        platform: "pc".into(),
                        edges: vec![JobEdgeInput {
                            asset_pk: Some(asset.asset_id),
                            target: Target::Guid(guid),
                            key: "azoth.asset-planner".into(),
                            platform: "pc".into(),
                            coupling: Coupling::Order,
                        }],
                    }],
                    ..PlanDelta::default()
                },
            })
            .unwrap();

        let result = writer
            .apply_sweep_delta_blocking(ApplySweepDelta {
                workspace_pk: workspace.workspace_id,
                workspace_root_pk: root.workspace_root_id,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: "source.ron".into(),
                        guid,
                        schema: Some("Source".into()),
                        digest: digest("changed"),
                        diff: Diff::Modified,
                        diagnostics: 0,
                        updated: 3,
                        src_bytes: 7,
                        src_mtime: 3,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 3,
                        session: None,
                    },
                    planner: SweepPlannerJob {
                        key: "azoth.asset-planner".into(),
                        platform: "pc".into(),
                    },
                }],
                removals: Vec::new(),
            })
            .unwrap();
        assert_eq!(result.updated, 1);
        assert_eq!(result.planned, 1);
        let entry = db
            .source_asset(workspace.workspace_id, root.root_pk, "source.ron")
            .unwrap()
            .unwrap()
            .1;
        assert_eq!(entry.digest, digest("changed"));
        let jobs = db
            .jobs_for_asset(workspace.workspace_id, asset.asset_id)
            .unwrap();
        assert_eq!(jobs.iter().filter(|job| job.kind == Work::Plan).count(), 1);
        assert_eq!(jobs.iter().filter(|job| job.kind == Work::Build).count(), 1);
        assert!(
            !jobs
                .iter()
                .find(|job| job.kind == Work::Build)
                .unwrap()
                .ready
        );

        assert_conflicting_sweep_rolls_back_whole(&db, &writer, &workspace, &root);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn workspace_and_root_registration_are_idempotent() {
        let db = AssetDb::open_in_memory().unwrap();
        let input = RegisterWorkspace {
            key: WorkspaceKey {
                project: "project".into(),
                root: std::env::temp_dir()
                    .join("assetdb-project")
                    .to_string_lossy()
                    .into_owned(),
                branch: "main".into(),
            },
            now: 1,
        };
        let writer = db.writer().unwrap();
        let first = writer.register_workspace_blocking(input.clone()).unwrap();
        let second = writer.register_workspace_blocking(input).unwrap();
        assert_eq!(first.workspace_id, second.workspace_id);

        let root = RegisterWorkspaceRoot {
            workspace_pk: first.workspace_id,
            key: "project".into(),
            owner: "project".into(),
            path: "assets".into(),
            exclusions: Exclusions::default(),
        };
        let first_root = writer
            .register_workspace_root_blocking(root.clone())
            .unwrap();
        let second_root = writer.register_workspace_root_blocking(root).unwrap();
        assert_eq!(first_root.0.root_id, second_root.0.root_id);
        assert_eq!(db.workspace_roots(first.workspace_id).unwrap().len(), 1);
    }

    #[test]
    fn workspace_root_set_replacement_is_atomic_and_removes_stale_bindings() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let workspace = writer
            .register_workspace_blocking(RegisterWorkspace {
                key: workspace_key("replace-roots"),
                now: 1,
            })
            .unwrap();
        let registration = |key: &str, path: &str| WorkspaceRootRegistration {
            key: key.to_owned(),
            owner: "project".to_owned(),
            path: path.to_owned(),
            exclusions: Exclusions::default(),
        };

        let first = writer
            .replace_workspace_roots_blocking(ReplaceWorkspaceRoots {
                workspace_pk: workspace.workspace_id,
                roots: vec![
                    registration("project", "assets"),
                    registration("gem:physics", "gems/physics/assets"),
                ],
            })
            .unwrap();
        assert_eq!(first.len(), 2);

        let second = writer
            .replace_workspace_roots_blocking(ReplaceWorkspaceRoots {
                workspace_pk: workspace.workspace_id,
                roots: vec![registration("project", "authoring")],
            })
            .unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].root.key, "project");
        assert_eq!(second[0].policy.path, "authoring");
        let stored = db.workspace_roots(workspace.workspace_id).unwrap();
        drop(db);
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].workspace_root_id,
            second[0].policy.workspace_root_id
        );
    }

    #[test]
    fn source_payload_write_fences_workspace_root_project_and_missing_revision() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "payload-scope");
        let (_, foreign_root) = register_test_workspace(&writer, "payload-foreign");
        let input = WriteSourcePayload {
            workspace_pk: workspace.workspace_id,
            root_pk: root.root_pk,
            path: "source.ron".into(),
            document: "source.ron".into(),
            schema: "Source".into(),
            encoding: Encoding::Bytes,
            expected_revision: None,
            revision: 1,
            saved: Some(1),
            digest: digest("payload"),
            payload: b"payload".to_vec(),
            checkpoint: CheckpointWrite::Replace(b"payload".to_vec()),
            session: None,
            project: workspace.project.clone(),
            now: 2,
        };

        let mut mismatched_root = input.clone();
        mismatched_root.root_pk = foreign_root.root_pk;
        assert!(matches!(
            writer
                .write_source_payload_blocking(mismatched_root)
                .unwrap(),
            WriteSourcePayloadResult::ScopeMismatch
        ));
        assert!(
            db.payload_for_source(workspace.workspace_id, root.root_pk, "source.ron")
                .unwrap()
                .is_none()
        );

        let mut mismatched_project = input.clone();
        mismatched_project.project = "another-project".into();
        assert!(matches!(
            writer
                .write_source_payload_blocking(mismatched_project)
                .unwrap(),
            WriteSourcePayloadResult::ScopeMismatch
        ));

        let mut missing = input.clone();
        missing.expected_revision = Some(0);
        assert!(matches!(
            writer.write_source_payload_blocking(missing).unwrap(),
            WriteSourcePayloadResult::Missing
        ));
        let WriteSourcePayloadResult::Written(written) =
            writer.write_source_payload_blocking(input.clone()).unwrap()
        else {
            panic!("expected initial payload write")
        };
        assert_eq!(written.checkpoint.as_deref(), Some(b"payload".as_slice()));

        let mut draft = input.clone();
        draft.expected_revision = Some(1);
        draft.revision = 2;
        draft.saved = Some(1);
        draft.payload = b"draft".to_vec();
        draft.digest = digest("draft");
        draft.checkpoint = CheckpointWrite::Preserve;
        let WriteSourcePayloadResult::Written(written) =
            writer.write_source_payload_blocking(draft).unwrap()
        else {
            panic!("expected draft payload write")
        };
        assert_eq!(written.saved, Some(1));
        assert_eq!(written.checkpoint.as_deref(), Some(b"payload".as_slice()));

        let mut invalid_save = input;
        invalid_save.expected_revision = Some(2);
        invalid_save.revision = 2;
        invalid_save.saved = Some(2);
        invalid_save.checkpoint = CheckpointWrite::Preserve;
        assert!(matches!(
            writer.write_source_payload_blocking(invalid_save).unwrap(),
            WriteSourcePayloadResult::InvalidCheckpoint
        ));
        let durable = db
            .payload_for_source(workspace.workspace_id, root.root_pk, "source.ron")
            .unwrap()
            .unwrap();
        assert_eq!(durable.saved, Some(1));
        drop(db);
        assert_eq!(durable.checkpoint.as_deref(), Some(b"payload".as_slice()));
    }

    fn authored_payload(
        workspace: &SelectWorkspaces,
        root: &SelectWorkspaceRoots,
        path: &str,
        expected_revision: Option<i64>,
        revision: i64,
        bytes: &[u8],
    ) -> WriteSourcePayload {
        WriteSourcePayload {
            workspace_pk: workspace.workspace_id,
            root_pk: root.root_pk,
            path: path.to_owned(),
            document: path.to_owned(),
            schema: "Source".to_owned(),
            encoding: Encoding::Bytes,
            expected_revision,
            revision,
            saved: Some(revision),
            digest: digest(std::str::from_utf8(bytes).unwrap()),
            payload: bytes.to_vec(),
            checkpoint: CheckpointWrite::Replace(bytes.to_vec()),
            session: None,
            project: workspace.project.clone(),
            now: revision + 10,
        }
    }

    fn authored_sweep(path: &str, guid: Uuid, digest: Digest) -> SweepEntry {
        SweepEntry {
            path: path.to_owned(),
            guid,
            schema: Some("Source".to_owned()),
            digest,
            diff: Diff::Clean,
            diagnostics: 0,
            updated: 11,
            src_bytes: 7,
            src_mtime: 11,
            meta_bytes: 0,
            meta_mtime: 0,
            observed: 11,
            session: None,
        }
    }

    /// Compare-and-set refusals for authored payloads, lifted out for length.
    fn authored_publication_rejects_stale_cas_writes(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        root: &SelectWorkspaceRoots,
    ) {
        let existing = authored_payload(workspace, root, "cas.ron", None, 1, b"base");
        assert!(matches!(
            writer.write_source_payload_blocking(existing).unwrap(),
            WriteSourcePayloadResult::Written(_)
        ));
        let conflict_payload = authored_payload(workspace, root, "cas.ron", Some(0), 2, b"changed");
        let conflict_digest = conflict_payload.digest;
        assert!(matches!(
            writer
                .publish_authored_source_blocking(PublishAuthoredSource {
                    workspace_root_pk: root.workspace_root_id,
                    source: authored_sweep("cas.ron", Uuid::new_v4(), conflict_digest),
                    payload: conflict_payload,
                })
                .unwrap(),
            PublishAuthoredSourceResult::Conflict(_)
        ));
        let durable = db
            .payload_for_source(workspace.workspace_id, root.root_pk, "cas.ron")
            .unwrap()
            .unwrap();
        assert_eq!(durable.revision, 1);
        assert_eq!(durable.payload, b"base");
        assert!(
            db.source_asset(workspace.workspace_id, root.root_pk, "cas.ron")
                .unwrap()
                .is_none()
        );

        let success_payload = authored_payload(workspace, root, "success.ron", None, 1, b"success");
        let success_guid = Uuid::new_v4();
        let success_digest = success_payload.digest;
        let PublishAuthoredSourceResult::Written(written) = writer
            .publish_authored_source_blocking(PublishAuthoredSource {
                workspace_root_pk: root.workspace_root_id,
                source: authored_sweep("success.ron", success_guid, success_digest),
                payload: success_payload,
            })
            .unwrap()
        else {
            panic!("expected successful authored publication")
        };
        let PublishedAuthoredSource {
            asset,
            entry,
            payload,
        } = *written;
        let (read_asset, read_entry) = db
            .source_asset(workspace.workspace_id, root.root_pk, "success.ron")
            .unwrap()
            .unwrap();
        let read_payload = db
            .payload_for_source(workspace.workspace_id, root.root_pk, "success.ron")
            .unwrap()
            .unwrap();
        assert!(
            asset.asset_id == read_asset.asset_id
                && asset.guid == read_asset.guid
                && asset.deleted == read_asset.deleted
                && asset.created == read_asset.created
                && asset.updated == read_asset.updated
        );
        assert!(
            entry.entry_id == read_entry.entry_id
                && entry.workspace_pk == read_entry.workspace_pk
                && entry.asset_pk == read_entry.asset_pk
                && entry.root_pk == read_entry.root_pk
                && entry.path == read_entry.path
                && entry.schema == read_entry.schema
                && entry.digest == read_entry.digest
                && entry.diff == read_entry.diff
                && entry.diagnostics == read_entry.diagnostics
                && entry.updated == read_entry.updated
                && entry.src_bytes == read_entry.src_bytes
                && entry.src_mtime == read_entry.src_mtime
                && entry.meta_bytes == read_entry.meta_bytes
                && entry.meta_mtime == read_entry.meta_mtime
                && entry.observed == read_entry.observed
        );
        assert!(
            payload.payload_id == read_payload.payload_id
                && payload.workspace_pk == read_payload.workspace_pk
                && payload.root_pk == read_payload.root_pk
                && payload.path == read_payload.path
                && payload.document == read_payload.document
                && payload.schema == read_payload.schema
                && payload.encoding == read_payload.encoding
                && payload.revision == read_payload.revision
                && payload.saved == read_payload.saved
                && payload.digest == read_payload.digest
                && payload.bytes == read_payload.bytes
                && payload.payload == read_payload.payload
                && payload.checkpoint == read_payload.checkpoint
                && payload.session == read_payload.session
                && payload.project == read_payload.project
                && payload.deleted == read_payload.deleted
                && payload.created == read_payload.created
                && payload.updated == read_payload.updated
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn authored_publication_is_atomic_across_projection_locator_and_payload_fences() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "publish-atomic");

        let mismatch_payload = authored_payload(&workspace, &root, "mismatch.ron", None, 1, b"one");
        let mismatched = PublishAuthoredSource {
            workspace_root_pk: root.workspace_root_id,
            source: authored_sweep("other.ron", Uuid::new_v4(), mismatch_payload.digest),
            payload: mismatch_payload,
        };
        assert!(matches!(
            writer.publish_authored_source_blocking(mismatched).unwrap(),
            PublishAuthoredSourceResult::InvalidSourceProjection
        ));
        assert!(
            db.payload_for_source(workspace.workspace_id, root.root_pk, "mismatch.ron")
                .unwrap()
                .is_none()
        );
        assert!(
            db.source_asset(workspace.workspace_id, root.root_pk, "mismatch.ron")
                .unwrap()
                .is_none()
        );

        let occupied = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "occupied.ron",
            Uuid::new_v4(),
        );
        let locator_payload =
            authored_payload(&workspace, &root, "occupied.ron", None, 1, b"occupied");
        let locator_digest = locator_payload.digest;
        assert!(matches!(
            writer
                .publish_authored_source_blocking(PublishAuthoredSource {
                    workspace_root_pk: root.workspace_root_id,
                    source: authored_sweep("occupied.ron", Uuid::new_v4(), locator_digest),
                    payload: locator_payload,
                })
                .unwrap(),
            PublishAuthoredSourceResult::LocatorConflict { asset }
                if asset.asset_id == occupied.asset_id
        ));
        assert!(
            db.payload_for_source(workspace.workspace_id, root.root_pk, "occupied.ron")
                .unwrap()
                .is_none()
        );

        authored_publication_rejects_stale_cas_writes(&db, &writer, &workspace, &root);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn draft_payload_blocks_move_and_delete_inside_writer_transaction() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "draft-source");
        let asset = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "source.ron",
            Uuid::new_v4(),
        );
        let base = WriteSourcePayload {
            workspace_pk: workspace.workspace_id,
            root_pk: root.root_pk,
            path: "source.ron".into(),
            document: "source.ron".into(),
            schema: "Source".into(),
            encoding: Encoding::Bytes,
            expected_revision: None,
            revision: 1,
            saved: Some(1),
            digest: digest("source.ron"),
            payload: b"source.ron".to_vec(),
            checkpoint: CheckpointWrite::Replace(b"source.ron".to_vec()),
            session: None,
            project: workspace.project.clone(),
            now: 3,
        };
        assert!(matches!(
            writer.write_source_payload_blocking(base.clone()).unwrap(),
            WriteSourcePayloadResult::Written(_)
        ));
        let mut draft = base;
        draft.expected_revision = Some(1);
        draft.revision = 2;
        draft.saved = Some(1);
        draft.digest = digest("draft-source");
        draft.payload = b"draft-source".to_vec();
        draft.checkpoint = CheckpointWrite::Preserve;
        assert!(matches!(
            writer.write_source_payload_blocking(draft).unwrap(),
            WriteSourcePayloadResult::Written(_)
        ));
        let expected = SourceStateToken {
            revision: Some(2),
            digest: digest("source.ron"),
        };
        assert!(matches!(
            writer
                .move_source_blocking(MoveSource {
                    workspace_pk: workspace.workspace_id,
                    root_pk: root.root_pk,
                    from: "source.ron".into(),
                    to: "moved.ron".into(),
                    expected,
                    now: 4,
                })
                .unwrap(),
            MoveSourceResult::Unsaved
        ));
        assert!(matches!(
            writer
                .delete_source_blocking(DeleteSource {
                    workspace_pk: workspace.workspace_id,
                    root_pk: root.root_pk,
                    path: "source.ron".into(),
                    expected,
                    now: 4,
                })
                .unwrap(),
            DeleteSourceResult::Unsaved
        ));
        assert_eq!(
            db.entry_by_asset(workspace.workspace_id, asset.asset_id)
                .unwrap()
                .unwrap()
                .path,
            "source.ron"
        );
        assert!(!db.asset_by_id(asset.asset_id).unwrap().unwrap().deleted);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn attempt_abandonment_retries_twice_then_fails_the_job() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "attempts");
        let asset = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "source.ron",
            Uuid::new_v4(),
        );
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace.workspace_id,
                expected: None,
                replacement: digest("catalog"),
                builders: Vec::new(),
                plan_delta: PlanDelta {
                    replacements: vec![PlannedJob {
                        asset_pk: asset.asset_id,
                        kind: Work::Plan,
                        builder: None,
                        key: "plan".into(),
                        platform: "pc".into(),
                        edges: Vec::new(),
                    }],
                    ..PlanDelta::default()
                },
                updated: 3,
            })
            .unwrap();
        let job = db
            .ready_jobs(workspace.workspace_id, Work::Plan, 0, 1)
            .unwrap()
            .pop()
            .unwrap();
        for expected_attempts in 0..MAX_ASSET_JOB_ATTEMPTS {
            let claimed = writer
                .claim_ready_job_blocking(ClaimReadyJob {
                    job_id: job.job_id,
                    expected_attempts,
                    owner: "worker".into(),
                    lease_duration_ms: 100,
                    staging: "stage".into(),
                })
                .unwrap();
            let ClaimReadyJobResult::Claimed { context } = claimed else {
                panic!("expected claim")
            };
            assert_eq!(
                context.attempt.expires,
                Some(context.claimed_unix_ms.saturating_add(100)),
                "the writer derives diagnostic expiry from its claim timestamp"
            );
            let result = writer
                .abandon_attempts_blocking(AbandonAttempts {
                    attempts: vec![AttemptFence {
                        attempt_id: context.attempt.attempt_id,
                        owner: "worker".into(),
                    }],
                    finished: 4 + expected_attempts,
                })
                .unwrap();
            if expected_attempts + 1 < MAX_ASSET_JOB_ATTEMPTS {
                assert_eq!(result.requeued, vec![job.job_id]);
            } else {
                assert_eq!(result.exhausted[0].job_id, job.job_id);
                assert_eq!(result.exhausted[0].diagnostic, ATTEMPT_LIMIT_EXHAUSTED);
            }
        }
        let durable: Option<SelectJobs> = db
            .drizzle
            .select(())
            .from(db.tables.jobs)
            .r#where(eq(db.tables.jobs.job_id, job.job_id))
            .get()
            .wait()
            .optional()
            .unwrap();
        assert_eq!(durable.unwrap().status, Status::Failed);
    }

    /// Retiring a source edge must remove it atomically, leaving no partially bound
    /// row behind.
    fn assert_source_edge_retires_atomically(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace_a: &SelectWorkspaces,
        edge: &SelectSourceEdges,
        builder_a: Uuid,
    ) {
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace_a.workspace_id,
                expected: Some(digest("catalog-a1")),
                replacement: digest("catalog-a2"),
                builders: vec![BuilderDescriptor {
                    guid: builder_a,
                    name: "builder-a".into(),
                    version: 1,
                    digest: digest("builder-a"),
                }],
                plan_delta: PlanDelta {
                    retire_source_edge_ids: vec![edge.source_edge_id],
                    ..PlanDelta::default()
                },
                updated: 4,
            })
            .unwrap();
        let retired: Option<SelectSourceEdges> = db
            .drizzle
            .select(())
            .from(db.tables.source_edges)
            .r#where(eq(
                db.tables.source_edges.source_edge_id,
                edge.source_edge_id,
            ))
            .get()
            .wait()
            .optional()
            .unwrap();
        assert!(retired.is_none());
    }

    /// The harness every shared-identity assertion is seeded from: the database
    /// and the writer under test, plus the two workspaces that observe the one
    /// shared asset identity the assertion is about.
    ///
    /// These four are what makes an assertion "cross-workspace" at all - a claim
    /// about one workspace's view is only meaningful against the other's - so
    /// they are established once per test and read together by every helper.
    /// Rows that belong to a single assertion (roots, edges, digests, payload
    /// bytes) stay explicit arguments.
    #[derive(Clone, Copy)]
    struct SharedIdentityHarness<'a> {
        db: &'a AssetDb,
        writer: &'a AssetDbWriter,
        workspace_a: &'a SelectWorkspaces,
        workspace_b: &'a SelectWorkspaces,
    }

    /// A source edge bound in workspace B must not leak into workspace A's dependent
    /// view of the same shared asset identity.
    fn assert_source_edges_do_not_leak_across_workspaces(
        harness: SharedIdentityHarness<'_>,
        root_b: &SelectWorkspaceRoots,
        dependency: &SelectAssets,
        builder_b: Uuid,
        dependency_guid: Uuid,
    ) {
        let SharedIdentityHarness {
            db,
            writer,
            workspace_a,
            workspace_b,
        } = harness;
        let workspace_b_source = observe_asset(
            db,
            writer,
            workspace_b,
            root_b,
            "consumer.ron",
            Uuid::new_v4(),
        );
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace_b.workspace_id,
                expected: None,
                replacement: digest("catalog-b1"),
                builders: vec![BuilderDescriptor {
                    guid: builder_b,
                    name: "builder-b".into(),
                    version: 1,
                    digest: digest("builder-b"),
                }],
                plan_delta: PlanDelta {
                    source_edges: vec![SourceEdgeInput {
                        builder: builder_b,
                        asset_pk: workspace_b_source.asset_id,
                        depends_pk: Some(dependency.asset_id),
                        target: Target::Guid(dependency_guid),
                        relation: Relation::SourceToSource,
                    }],
                    ..PlanDelta::default()
                },
                updated: 3,
            })
            .unwrap();
        let dependents = db
            .source_dependents(&SourceDependentsInput {
                workspace_pk: workspace_a.workspace_id,
                asset_pk: dependency.asset_id,
            })
            .unwrap();
        assert_eq!(dependents.sources.len(), 1);
        assert_eq!(dependents.sources[0].source_path, "source.ron");
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one end-to-end atomicity scenario: two workspaces are set up, cross-bound, then retired in a single transaction, and splitting it would separate the setup from the atomicity it exists to prove"
    )]
    fn source_edges_bind_only_inside_their_workspace_and_retire_atomically() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace_a, root_a) = register_test_workspace(&writer, "source-a");
        let (workspace_b, root_b) = register_test_workspace(&writer, "source-b");
        let source = observe_asset(
            &db,
            &writer,
            &workspace_a,
            &root_a,
            "source.ron",
            Uuid::new_v4(),
        );
        let bystander = observe_asset(
            &db,
            &writer,
            &workspace_b,
            &root_b,
            "later.ron",
            Uuid::new_v4(),
        );
        let builder_a = Uuid::new_v4();
        let builder_b = Uuid::new_v4();
        let descriptor = |guid, name: &str| BuilderDescriptor {
            guid,
            name: name.into(),
            version: 1,
            digest: digest(name),
        };
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace_a.workspace_id,
                expected: None,
                replacement: digest("catalog-a1"),
                builders: vec![descriptor(builder_a, "builder-a")],
                plan_delta: PlanDelta {
                    source_edges: vec![SourceEdgeInput {
                        builder: builder_a,
                        asset_pk: source.asset_id,
                        depends_pk: None,
                        target: Target::path("later.ron").unwrap(),
                        relation: Relation::SourceToSource,
                    }],
                    ..PlanDelta::default()
                },
                updated: 3,
            })
            .unwrap();
        let edge: Option<SelectSourceEdges> = db
            .drizzle
            .select(())
            .from(db.tables.source_edges)
            .get()
            .wait()
            .optional()
            .unwrap();
        let edge = edge.unwrap();
        assert_eq!(edge.depends_pk, None);
        assert_ne!(edge.depends_pk, Some(bystander.asset_id));

        let dependency_guid = bystander.guid;
        let dependency = observe_asset(
            &db,
            &writer,
            &workspace_a,
            &root_a,
            "later.ron",
            dependency_guid,
        );
        let bound: Option<SelectSourceEdges> = db
            .drizzle
            .select(())
            .from(db.tables.source_edges)
            .r#where(eq(
                db.tables.source_edges.source_edge_id,
                edge.source_edge_id,
            ))
            .get()
            .wait()
            .optional()
            .unwrap();
        assert_eq!(bound.unwrap().depends_pk, Some(dependency.asset_id));

        let workspace_b_dependency = observe_asset(
            &db,
            &writer,
            &workspace_b,
            &root_b,
            "later.ron",
            dependency_guid,
        );
        assert_eq!(workspace_b_dependency.asset_id, dependency.asset_id);
        assert_source_edges_do_not_leak_across_workspaces(
            SharedIdentityHarness {
                db: &db,
                writer: &writer,
                workspace_a: &workspace_a,
                workspace_b: &workspace_b,
            },
            &root_b,
            &dependency,
            builder_b,
            dependency_guid,
        );
        assert_source_edge_retires_atomically(&db, &writer, &workspace_a, &edge, builder_a);
    }

    /// Retiring a source edge in one workspace must leave the other workspace's edge
    /// on the same shared asset identity untouched.
    fn assert_source_edge_retirement_is_workspace_scoped(
        harness: SharedIdentityHarness<'_>,
        asset_a: &SelectAssets,
        edge_a: &SelectSourceEdges,
        edge_b: &SelectSourceEdges,
        builder_a: Uuid,
        builder_b: Uuid,
    ) {
        let SharedIdentityHarness {
            db,
            writer,
            workspace_a,
            workspace_b,
        } = harness;
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace_a.workspace_id,
                expected: Some(digest("builder-a-catalog")),
                replacement: digest("builder-a-catalog-next"),
                builders: vec![BuilderDescriptor {
                    guid: builder_a,
                    name: "builder-a".into(),
                    version: 1,
                    digest: digest("builder-a"),
                }],
                plan_delta: PlanDelta {
                    retire_source_edge_ids: vec![edge_a.source_edge_id],
                    ..PlanDelta::default()
                },
                updated: 4,
            })
            .unwrap();

        assert!(
            db.source_edges_for_asset(workspace_a.workspace_id, asset_a.asset_id)
                .unwrap()
                .is_empty()
        );
        let surviving_b = db
            .source_edges_for_asset(workspace_b.workspace_id, asset_a.asset_id)
            .unwrap();
        assert_eq!(surviving_b.len(), 1);
        assert_eq!(surviving_b[0].source_edge_id, edge_b.source_edge_id);
        assert_eq!(surviving_b[0].workspace_pk, workspace_b.workspace_id);
        assert_eq!(surviving_b[0].builder, builder_b);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn source_edges_are_owned_by_workspace_when_asset_identity_is_shared() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace_a, root_a) = register_test_workspace(&writer, "shared-edge-a");
        let (workspace_b, root_b) = register_test_workspace(&writer, "shared-edge-b");
        let shared_guid = Uuid::new_v4();
        let asset_a = observe_asset(&db, &writer, &workspace_a, &root_a, "a.ron", shared_guid);
        let asset_b = observe_asset(&db, &writer, &workspace_b, &root_b, "b.ron", shared_guid);
        assert_eq!(asset_a.asset_id, asset_b.asset_id);

        let builder_a = Uuid::new_v4();
        let builder_b = Uuid::new_v4();
        let descriptor = |guid, name: &str| BuilderDescriptor {
            guid,
            name: name.into(),
            version: 1,
            digest: digest(name),
        };
        for (workspace, builder, name, target_path) in [
            (&workspace_a, builder_a, "builder-a", "dependency-a.ron"),
            (&workspace_b, builder_b, "builder-b", "dependency-b.ron"),
        ] {
            writer
                .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                    workspace_pk: workspace.workspace_id,
                    expected: None,
                    replacement: digest(&format!("{name}-catalog")),
                    builders: vec![descriptor(builder, name)],
                    plan_delta: PlanDelta {
                        source_edges: vec![SourceEdgeInput {
                            builder,
                            asset_pk: asset_a.asset_id,
                            depends_pk: None,
                            target: Target::path(target_path).unwrap(),
                            relation: Relation::SourceToSource,
                        }],
                        ..PlanDelta::default()
                    },
                    updated: 3,
                })
                .unwrap();
        }

        let dependency_guid = Uuid::new_v4();
        let dependency_a = observe_asset(
            &db,
            &writer,
            &workspace_a,
            &root_a,
            "dependency-a.ron",
            dependency_guid,
        );
        let dependency_b = observe_asset(
            &db,
            &writer,
            &workspace_b,
            &root_b,
            "dependency-b.ron",
            dependency_guid,
        );
        assert_eq!(dependency_a.asset_id, dependency_b.asset_id);

        let edge_a = db
            .source_edges_for_asset(workspace_a.workspace_id, asset_a.asset_id)
            .unwrap()
            .pop()
            .unwrap();
        let edge_b = db
            .source_edges_for_asset(workspace_b.workspace_id, asset_a.asset_id)
            .unwrap()
            .pop()
            .unwrap();
        assert_ne!(edge_a.source_edge_id, edge_b.source_edge_id);
        assert_eq!(edge_a.workspace_pk, workspace_a.workspace_id);
        assert_eq!(edge_b.workspace_pk, workspace_b.workspace_id);
        assert_eq!(edge_a.depends_pk, Some(dependency_a.asset_id));
        assert_eq!(edge_b.depends_pk, Some(dependency_b.asset_id));
        let dependents_a = db
            .source_dependents(&SourceDependentsInput {
                workspace_pk: workspace_a.workspace_id,
                asset_pk: dependency_a.asset_id,
            })
            .unwrap();
        let dependents_b = db
            .source_dependents(&SourceDependentsInput {
                workspace_pk: workspace_b.workspace_id,
                asset_pk: dependency_b.asset_id,
            })
            .unwrap();
        assert_eq!(dependents_a.sources[0].source_path, "a.ron");
        assert_eq!(dependents_b.sources[0].source_path, "b.ron");

        assert_source_edge_retirement_is_workspace_scoped(
            SharedIdentityHarness {
                db: &db,
                writer: &writer,
                workspace_a: &workspace_a,
                workspace_b: &workspace_b,
            },
            &asset_a,
            &edge_a,
            &edge_b,
            builder_a,
            builder_b,
        );
    }

    /// Each workspace must claim and inspect the shared asset through its own locator
    /// and payload, never the other workspace's.
    fn assert_each_workspace_sees_its_own_locator(
        harness: SharedIdentityHarness<'_>,
        digest_a: Digest,
        digest_b: Digest,
        bytes_a: &[u8],
        bytes_b: &[u8],
    ) {
        let SharedIdentityHarness {
            db,
            writer,
            workspace_a,
            workspace_b,
        } = harness;
        for (workspace, expected_path, expected_schema, expected_digest, expected_bytes) in [
            (&workspace_a, "a.ron", "test.SourceA", digest_a, bytes_a),
            (&workspace_b, "b.ron", "test.SourceB", digest_b, bytes_b),
        ] {
            let job = db
                .ready_jobs(workspace.workspace_id, Work::Plan, 0, 1)
                .unwrap()
                .pop()
                .unwrap();
            let claimed = writer
                .claim_ready_job_blocking(ClaimReadyJob {
                    job_id: job.job_id,
                    expected_attempts: 0,
                    owner: format!("worker-{}", workspace.workspace_id),
                    lease_duration_ms: 100,
                    staging: format!("staging/{}", workspace.workspace_id),
                })
                .unwrap();
            let ClaimReadyJobResult::Claimed { context } = claimed else {
                panic!("workspace plan job must remain claimable");
            };
            assert_eq!(context.entry.path, expected_path);
            assert_eq!(context.entry.schema.as_deref(), Some(expected_schema));
            assert_eq!(context.entry.digest, expected_digest);
            assert_eq!(context.workspace_root.workspace_pk, workspace.workspace_id);
            assert_eq!(
                context
                    .payload
                    .as_ref()
                    .unwrap()
                    .checkpoint
                    .as_ref()
                    .unwrap(),
                expected_bytes
            );
            let inspection = db
                .inspect_job(
                    workspace.workspace_id,
                    JobInspectionSelector::Job(context.job.job_id),
                )
                .unwrap()
                .unwrap();
            assert_eq!(inspection.entry.path, expected_path);
            assert_eq!(inspection.entry.schema.as_deref(), Some(expected_schema));
            assert_eq!(inspection.entry.digest, expected_digest);
            assert_eq!(
                inspection.workspace_root.workspace_pk,
                workspace.workspace_id
            );
        }
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn shared_asset_claim_and_inspection_use_each_workspace_locator_and_payload() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace_a, root_a) = register_test_workspace(&writer, "shared-claim-a");
        let (workspace_b, root_b) = register_test_workspace(&writer, "shared-claim-b");
        let guid = Uuid::new_v4();
        let bytes_a = b"workspace-a".as_slice();
        let bytes_b = b"workspace-b".as_slice();
        let digest_a = Digest::from(blake3::hash(bytes_a));
        let digest_b = Digest::from(blake3::hash(bytes_b));
        let asset_a = observe_asset_with_projection(
            &db,
            &writer,
            &workspace_a,
            &root_a,
            "a.ron",
            guid,
            Some("test.SourceA"),
            digest_a,
        );
        let asset_b = observe_asset_with_projection(
            &db,
            &writer,
            &workspace_b,
            &root_b,
            "b.ron",
            guid,
            Some("test.SourceB"),
            digest_b,
        );
        assert_eq!(asset_a.asset_id, asset_b.asset_id);

        for (workspace, root, path, schema, bytes, source_digest) in [
            (
                &workspace_a,
                &root_a,
                "a.ron",
                "test.SourceA",
                bytes_a,
                digest_a,
            ),
            (
                &workspace_b,
                &root_b,
                "b.ron",
                "test.SourceB",
                bytes_b,
                digest_b,
            ),
        ] {
            let written = writer
                .write_source_payload_blocking(WriteSourcePayload {
                    workspace_pk: workspace.workspace_id,
                    root_pk: root.root_pk,
                    path: path.into(),
                    document: path.into(),
                    schema: schema.into(),
                    encoding: Encoding::Bytes,
                    expected_revision: None,
                    revision: 1,
                    saved: Some(1),
                    digest: source_digest,
                    payload: bytes.to_vec(),
                    checkpoint: CheckpointWrite::Replace(bytes.to_vec()),
                    session: None,
                    project: workspace.project.clone(),
                    now: 3,
                })
                .unwrap();
            assert!(matches!(written, WriteSourcePayloadResult::Written(_)));
        }

        assert_each_workspace_sees_its_own_locator(
            SharedIdentityHarness {
                db: &db,
                writer: &writer,
                workspace_a: &workspace_a,
                workspace_b: &workspace_b,
            },
            digest_a,
            digest_b,
            bytes_a,
            bytes_b,
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn claim_rolls_back_when_authoritative_context_is_incomplete() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "claim-context-rollback");
        let asset = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "source.ron",
            Uuid::new_v4(),
        );
        let job = db
            .ready_jobs(workspace.workspace_id, Work::Plan, 0, 1)
            .unwrap()
            .pop()
            .expect("observed source has a ready Plan Job");

        db.drizzle
            .delete(db.tables.workspace_roots)
            .r#where(and(
                eq(
                    db.tables.workspace_roots.workspace_pk,
                    workspace.workspace_id,
                ),
                eq(db.tables.workspace_roots.root_pk, root.root_pk),
            ))
            .execute()
            .wait()
            .expect("inject missing workspace-root context");

        let claim = writer.claim_ready_job_blocking(ClaimReadyJob {
            job_id: job.job_id,
            expected_attempts: 0,
            owner: "worker".into(),
            lease_duration_ms: 100,
            staging: "stage".into(),
        });
        assert!(claim.is_err());

        let job_after = db.job_by_id(job.job_id).unwrap().unwrap();
        assert_eq!(job_after.asset_pk, asset.asset_id);
        assert_eq!(job_after.status, Status::Queued);
        assert!(job_after.ready);
        assert_eq!(job_after.attempts, 0);
        let attempts: Vec<SelectAttempts> = db
            .drizzle
            .select(())
            .from(db.tables.attempts)
            .r#where(eq(db.tables.attempts.job_pk, job.job_id))
            .all()
            .wait()
            .unwrap();
        assert!(attempts.is_empty());
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn deleting_one_shared_guid_entry_leaves_the_other_workspace_claimable() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace_a, root_a) = register_test_workspace(&writer, "shared-delete-a");
        let (workspace_b, root_b) = register_test_workspace(&writer, "shared-delete-b");
        let guid = Uuid::new_v4();
        let asset_a = observe_asset(&db, &writer, &workspace_a, &root_a, "a.ron", guid);
        let asset_b = observe_asset(&db, &writer, &workspace_b, &root_b, "b.ron", guid);
        assert_eq!(asset_a.asset_id, asset_b.asset_id);

        let deleted = writer
            .delete_source_blocking(DeleteSource {
                workspace_pk: workspace_a.workspace_id,
                root_pk: root_a.root_pk,
                path: "a.ron".into(),
                expected: SourceStateToken {
                    revision: None,
                    digest: digest("a.ron"),
                },
                now: 3,
            })
            .unwrap();
        assert!(matches!(deleted, DeleteSourceResult::Deleted(_)));
        assert!(
            db.source_asset(workspace_a.workspace_id, root_a.root_pk, "a.ron")
                .unwrap()
                .is_none()
        );
        let (remaining_asset, remaining_entry) = db
            .source_asset(workspace_b.workspace_id, root_b.root_pk, "b.ron")
            .unwrap()
            .expect("workspace B retains its source observation");
        assert_eq!(remaining_asset.asset_id, asset_b.asset_id);
        assert_eq!(remaining_entry.path, "b.ron");
        assert!(!remaining_asset.deleted);

        let ready = db
            .ready_jobs(workspace_b.workspace_id, Work::Plan, 0, 1)
            .unwrap()
            .pop()
            .expect("workspace B planner remains ready");
        let claimed = writer
            .claim_ready_job_blocking(ClaimReadyJob {
                job_id: ready.job_id,
                expected_attempts: 0,
                owner: "worker-b".into(),
                lease_duration_ms: 100,
                staging: "staging/b".into(),
            })
            .unwrap();
        let ClaimReadyJobResult::Claimed { context } = claimed else {
            panic!("workspace B planner must remain claimable")
        };
        assert_eq!(context.entry.path, "b.ron");
        let inspection = db
            .inspect_job(
                workspace_b.workspace_id,
                JobInspectionSelector::Job(context.job.job_id),
            )
            .unwrap()
            .unwrap();
        assert_eq!(inspection.entry.path, "b.ron");
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn workspace_locator_identity_is_independent_across_workspaces() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let workspace_a = writer
            .register_workspace_blocking(RegisterWorkspace {
                key: workspace_key("locator-a"),
                now: 1,
            })
            .unwrap();
        let workspace_b = writer
            .register_workspace_blocking(RegisterWorkspace {
                key: workspace_key("locator-b"),
                now: 1,
            })
            .unwrap();
        let (_, root_a) = writer
            .register_workspace_root_blocking(RegisterWorkspaceRoot {
                workspace_pk: workspace_a.workspace_id,
                key: "shared-portable-root".into(),
                owner: "a".into(),
                path: "assets".into(),
                exclusions: Exclusions::default(),
            })
            .unwrap();
        let (_, root_b) = writer
            .register_workspace_root_blocking(RegisterWorkspaceRoot {
                workspace_pk: workspace_b.workspace_id,
                key: "shared-portable-root".into(),
                owner: "b".into(),
                path: "assets".into(),
                exclusions: Exclusions::default(),
            })
            .unwrap();
        assert_eq!(root_a.root_pk, root_b.root_pk);
        let guid_a = Uuid::new_v4();
        let guid_b = Uuid::new_v4();
        let asset_a = observe_asset(&db, &writer, &workspace_a, &root_a, "same.ron", guid_a);
        let asset_b = observe_asset(&db, &writer, &workspace_b, &root_b, "same.ron", guid_b);
        assert_ne!(asset_a.asset_id, asset_b.asset_id);

        let collision = writer.apply_sweep_delta_blocking(ApplySweepDelta {
            workspace_pk: workspace_a.workspace_id,
            workspace_root_pk: root_a.workspace_root_id,
            records: vec![SweepRecord {
                source: SweepEntry {
                    path: "same.ron".into(),
                    guid: Uuid::new_v4(),
                    schema: None,
                    digest: digest("conflict"),
                    diff: Diff::Clean,
                    diagnostics: 0,
                    updated: 3,
                    src_bytes: 1,
                    src_mtime: 3,
                    meta_bytes: 0,
                    meta_mtime: 0,
                    observed: 3,
                    session: None,
                },
                planner: SweepPlannerJob {
                    key: "azoth.asset-planner".into(),
                    platform: "pc".into(),
                },
            }],
            removals: Vec::new(),
        });
        assert!(collision.is_err());
        assert_eq!(
            db.source_asset(workspace_a.workspace_id, root_a.root_pk, "same.ron")
                .unwrap()
                .unwrap()
                .0
                .guid,
            guid_a
        );
        assert_eq!(
            db.source_asset(workspace_b.workspace_id, root_b.root_pk, "same.ron")
                .unwrap()
                .unwrap()
                .0
                .guid,
            guid_b
        );
    }

    #[test]
    fn path_history_is_open_and_closed_per_workspace_asset() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace_a, root_a) = register_test_workspace(&writer, "history-a");
        let (workspace_b, root_b) = register_test_workspace(&writer, "history-b");
        let guid = Uuid::new_v4();
        let asset = observe_asset(&db, &writer, &workspace_a, &root_a, "a-old.ron", guid);
        observe_asset(&db, &writer, &workspace_b, &root_b, "b.ron", guid);

        let moved = writer
            .move_source_blocking(MoveSource {
                workspace_pk: workspace_a.workspace_id,
                root_pk: root_a.root_pk,
                from: "a-old.ron".into(),
                to: "a-new.ron".into(),
                expected: SourceStateToken {
                    revision: None,
                    digest: digest("a-old.ron"),
                },
                now: 3,
            })
            .unwrap();
        assert!(matches!(moved, MoveSourceResult::Moved(_)));
        let open_after_move: Vec<SelectPaths> = db
            .drizzle
            .select(())
            .from(db.tables.paths)
            .r#where(and(
                eq(db.tables.paths.asset_pk, asset.asset_id),
                is_null(db.tables.paths.to),
            ))
            .order_by([asc(db.tables.paths.workspace_pk)])
            .all()
            .wait()
            .unwrap();
        assert_eq!(open_after_move.len(), 2);
        assert_eq!(open_after_move[0].workspace_pk, workspace_a.workspace_id);
        assert_eq!(open_after_move[0].path, "a-new.ron");
        assert_eq!(open_after_move[1].workspace_pk, workspace_b.workspace_id);
        assert_eq!(open_after_move[1].path, "b.ron");

        writer
            .delete_source_blocking(DeleteSource {
                workspace_pk: workspace_a.workspace_id,
                root_pk: root_a.root_pk,
                path: "a-new.ron".into(),
                expected: SourceStateToken {
                    revision: None,
                    digest: digest("a-old.ron"),
                },
                now: 4,
            })
            .unwrap();
        let open_after_delete: Vec<SelectPaths> = db
            .drizzle
            .select(())
            .from(db.tables.paths)
            .r#where(and(
                eq(db.tables.paths.asset_pk, asset.asset_id),
                is_null(db.tables.paths.to),
            ))
            .all()
            .wait()
            .unwrap();
        assert_eq!(open_after_delete.len(), 1);
        drop(db);
        assert_eq!(open_after_delete[0].workspace_pk, workspace_b.workspace_id);
        assert_eq!(open_after_delete[0].path, "b.ron");
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn dependent_projection_pages_above_bind_budget_and_folds_job_facts() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "dependent-page");
        let target = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "target.ron",
            Uuid::new_v4(),
        );
        let owner = observe_asset(&db, &writer, &workspace, &root, "owner.ron", Uuid::new_v4());
        let count = QUERY_BIND_BUDGET + 1;
        let jobs: Vec<SelectJobs> = db
            .drizzle
            .insert(db.tables.jobs)
            .values((0..count).map(|index| {
                InsertJobs::new(
                    workspace.workspace_id,
                    owner.asset_id,
                    Work::Plan,
                    format!("job-{index:04}"),
                    "pc",
                    Status::Succeeded,
                )
            }))
            .returning(())
            .all()
            .wait()
            .unwrap();
        db.drizzle
            .insert(db.tables.job_edges)
            .values(jobs.iter().map(|job| {
                InsertJobEdges::new(
                    job.job_id,
                    Target::Guid(target.guid),
                    "dependency",
                    "pc",
                    Coupling::Order,
                )
                .with_asset_pk(target.asset_id)
            }))
            .execute()
            .wait()
            .unwrap();
        db.drizzle
            .insert(db.tables.attempts)
            .values(
                jobs.iter()
                    .map(|job| InsertAttempts::new(job.job_id, 1, Status::Succeeded)),
            )
            .execute()
            .wait()
            .unwrap();
        let attempts: Vec<SelectAttempts> = db
            .drizzle
            .select(())
            .from(db.tables.attempts)
            .order_by([asc(db.tables.attempts.job_pk)])
            .all()
            .wait()
            .unwrap();
        let mut products = Vec::with_capacity(count * 3);
        for (index, job) in jobs.iter().enumerate() {
            for (offset, path) in [(0, "z.product"), (1, "a.product"), (2, "a.product")] {
                products.push(InsertProducts::new(
                    workspace.workspace_id,
                    owner.asset_id,
                    "pc",
                    i64::try_from(index * 3 + offset).unwrap(),
                    job.job_id,
                    path,
                    Uuid::new_v4(),
                    "test",
                    1,
                    Aliases::default(),
                    Registration::Registered,
                    digest(&format!("{index}-{offset}")),
                    1,
                ));
            }
        }
        db.drizzle
            .insert(db.tables.products)
            .values(products)
            .execute()
            .wait()
            .unwrap();

        let dependents = db
            .source_dependents(&SourceDependentsInput {
                workspace_pk: workspace.workspace_id,
                asset_pk: target.asset_id,
            })
            .unwrap();

        assert_eq!(dependents.jobs.len(), count);
        for (dependent, attempt) in dependents.jobs.iter().zip(attempts) {
            assert_eq!(dependent.latest_attempt_id, Some(attempt.attempt_id));
            assert_eq!(dependent.product_paths, ["a.product", "z.product"]);
        }
    }

    #[test]
    fn workspace_entry_jobs_keep_plan_and_build_with_optional_attempts() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "latest-build-attempt");
        let asset = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "source.ron",
            Uuid::new_v4(),
        );
        // Observation owns the durable Plan job. A Jobs-primary entry retains it
        // alongside independently scheduled Build work.
        let planner = db
            .jobs_for_asset(workspace.workspace_id, asset.asset_id)
            .unwrap()
            .into_iter()
            .find(|job| job.kind == Work::Plan)
            .expect("sweep observation owns a Plan job");
        let builder = Uuid::new_v4();
        let build: SelectJobs = db
            .drizzle
            .insert(db.tables.jobs)
            .values([InsertJobs::new(
                workspace.workspace_id,
                asset.asset_id,
                Work::Build,
                "build",
                "pc",
                Status::Succeeded,
            )
            .with_builder(builder)])
            .returning(())
            .get()
            .wait()
            .unwrap();
        let build_attempt: SelectAttempts = db
            .drizzle
            .insert(db.tables.attempts)
            .values([InsertAttempts::new(build.job_id, 1, Status::Succeeded)])
            .returning(())
            .get()
            .wait()
            .unwrap();
        let entries = db
            .workspace_entry_page(
                workspace.workspace_id,
                Some(&[root.root_pk, root.root_pk]),
                0,
                1,
            )
            .unwrap();
        assert_eq!(entries.len(), 1);
        drop(db);
        assert_eq!(entries[0].jobs.len(), 2);
        let build_activity = entries[0]
            .jobs
            .iter()
            .find(|activity| activity.job.job_id == build.job_id)
            .unwrap();
        assert_eq!(
            build_activity
                .attempt
                .as_ref()
                .map(|attempt| attempt.attempt_id),
            Some(build_attempt.attempt_id),
        );
        let planner_activity = entries[0]
            .jobs
            .iter()
            .find(|activity| activity.job.job_id == planner.job_id)
            .unwrap();
        assert_eq!(planner_activity.attempt, None);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn workspace_entry_page_pages_and_deduplicates_root_filters() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let workspace = writer
            .register_workspace_blocking(RegisterWorkspace {
                key: workspace_key("root-page"),
                now: 1,
            })
            .unwrap();
        let root_count = QUERY_BIND_BUDGET + 1;
        let mut roots = Vec::with_capacity(root_count);
        for first in (0..root_count).step_by(128) {
            let end = (first + 128).min(root_count);
            let mut page: Vec<SelectRoots> = db
                .drizzle
                .insert(db.tables.roots)
                .values((first..end).map(|index| InsertRoots::new(format!("root-{index:04}"))))
                .returning(())
                .all()
                .wait()
                .unwrap();
            roots.append(&mut page);
        }
        let mut policies = Vec::with_capacity(root_count);
        for root_page in roots.chunks(96) {
            let mut page: Vec<SelectWorkspaceRoots> = db
                .drizzle
                .insert(db.tables.workspace_roots)
                .values(root_page.iter().map(|root| {
                    InsertWorkspaceRoots::new(
                        workspace.workspace_id,
                        root.root_id,
                        "owner".to_owned(),
                        format!("assets/{}", root.key),
                        Exclusions::default(),
                    )
                }))
                .returning(())
                .all()
                .wait()
                .unwrap();
            policies.append(&mut page);
        }

        // Insert in an order that disagrees with root-id page order. The
        // merged result must still honor the global Entry cursor and limit.
        observe_asset(
            &db,
            &writer,
            &workspace,
            &policies[root_count - 1],
            "first.ron",
            Uuid::new_v4(),
        );
        observe_asset(
            &db,
            &writer,
            &workspace,
            &policies[0],
            "second.ron",
            Uuid::new_v4(),
        );
        observe_asset(
            &db,
            &writer,
            &workspace,
            &policies[QUERY_BIND_BUDGET - 1],
            "third.ron",
            Uuid::new_v4(),
        );
        let mut filter = policies
            .iter()
            .map(|policy| policy.root_pk)
            .collect::<Vec<_>>();
        filter.extend([policies[0].root_pk, policies[root_count - 1].root_pk]);

        let page = db
            .workspace_entry_page(workspace.workspace_id, Some(&filter), 0, 2)
            .unwrap();

        assert_eq!(page.len(), 2);
        assert!(page[0].entry_id < page[1].entry_id);
        assert_eq!(page[0].source_path, "first.ron");
        assert_eq!(page[1].source_path, "second.ron");
    }

    /// Once every dependency is satisfied, idle resolution must repair the consumer
    /// job and report it as newly ready.
    fn assert_idle_resolution_repairs_satisfied_consumer(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        consumer_job: &SelectJobs,
        dependencies: &[&SelectJobs],
    ) {
        db.drizzle
            .update(db.tables.jobs)
            .set(UpdateJobs::default().with_status(Status::Succeeded))
            .r#where(eq(db.tables.jobs.job_id, dependencies[1].job_id))
            .execute()
            .wait()
            .unwrap();
        db.drizzle
            .update(db.tables.jobs)
            .set(
                UpdateJobs::default()
                    .with_status(Status::Queued)
                    .with_ready(false),
            )
            .r#where(eq(db.tables.jobs.job_id, consumer_job.job_id))
            .execute()
            .wait()
            .unwrap();
        let repaired = writer
            .resolve_idle_blocked_blocking(ResolveIdleBlocked {
                workspace_pk: workspace.workspace_id,
                job_ids: vec![consumer_job.job_id],
            })
            .unwrap();
        assert_eq!(repaired.became_ready, vec![consumer_job.job_id]);
    }

    /// A consumer whose dependencies end in mixed terminal states must be failed with
    /// the unsatisfiable-dependency diagnostic.
    fn assert_idle_resolution_fails_mixed_terminal_consumer(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        consumer_job: &SelectJobs,
        dependencies: &[&SelectJobs],
    ) {
        db.drizzle
            .update(db.tables.jobs)
            .set(UpdateJobs::default().with_status(Status::Succeeded))
            .r#where(eq(db.tables.jobs.job_id, dependencies[0].job_id))
            .execute()
            .wait()
            .unwrap();
        db.drizzle
            .update(db.tables.jobs)
            .set(UpdateJobs::default().with_status(Status::Failed))
            .r#where(eq(db.tables.jobs.job_id, dependencies[1].job_id))
            .execute()
            .wait()
            .unwrap();
        let failed = writer
            .resolve_idle_blocked_blocking(ResolveIdleBlocked {
                workspace_pk: workspace.workspace_id,
                job_ids: vec![consumer_job.job_id],
            })
            .unwrap();
        assert_eq!(failed.failed_jobs[0].job_id, consumer_job.job_id);
        assert_eq!(failed.failed_jobs[0].diagnostic, UNSATISFIABLE_DEPENDENCY);
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn idle_resolution_repairs_satisfied_jobs_and_fails_mixed_terminal_dependencies() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "idle");
        let dependency = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "dependency.ron",
            Uuid::new_v4(),
        );
        let consumer = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "consumer.ron",
            Uuid::new_v4(),
        );
        let first_builder = Uuid::new_v4();
        let second_builder = Uuid::new_v4();
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace.workspace_id,
                expected: None,
                replacement: digest("idle-catalog"),
                builders: vec![
                    test_builder_descriptor(first_builder, "idle-first"),
                    test_builder_descriptor(second_builder, "idle-second"),
                ],
                plan_delta: PlanDelta {
                    replacements: vec![
                        PlannedJob {
                            asset_pk: dependency.asset_id,
                            kind: Work::Build,
                            builder: Some(first_builder),
                            key: "dependency".into(),
                            platform: "pc".into(),
                            edges: Vec::new(),
                        },
                        PlannedJob {
                            asset_pk: dependency.asset_id,
                            kind: Work::Build,
                            builder: Some(second_builder),
                            key: "dependency".into(),
                            platform: "pc".into(),
                            edges: Vec::new(),
                        },
                        PlannedJob {
                            asset_pk: consumer.asset_id,
                            kind: Work::Plan,
                            builder: None,
                            key: "consumer".into(),
                            platform: "pc".into(),
                            edges: vec![JobEdgeInput {
                                asset_pk: Some(dependency.asset_id),
                                target: Target::Guid(dependency.guid),
                                key: "dependency".into(),
                                platform: "pc".into(),
                                coupling: Coupling::Order,
                            }],
                        },
                    ],
                    ..PlanDelta::default()
                },
                updated: 3,
            })
            .unwrap();
        let jobs: Vec<SelectJobs> = db
            .drizzle
            .select(())
            .from(db.tables.jobs)
            .r#where(eq(db.tables.jobs.workspace_pk, workspace.workspace_id))
            .all()
            .wait()
            .unwrap();
        let consumer_job = jobs.iter().find(|job| job.key == "consumer").unwrap();
        let dependencies: Vec<&SelectJobs> =
            jobs.iter().filter(|job| job.key == "dependency").collect();
        assert_idle_resolution_fails_mixed_terminal_consumer(
            &db,
            &writer,
            &workspace,
            consumer_job,
            &dependencies,
        );
        assert_idle_resolution_repairs_satisfied_consumer(
            &db,
            &writer,
            &workspace,
            consumer_job,
            &dependencies,
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn strict_status_domains_reject_impossible_job_and_attempt_states() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "status-domain");
        let asset = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "source.ron",
            Uuid::new_v4(),
        );
        let rejected_job = db
            .drizzle
            .insert(db.tables.jobs)
            .values([InsertJobs::new(
                workspace.workspace_id,
                asset.asset_id,
                Work::Plan,
                "illegal",
                "pc",
                Status::Abandoned,
            )])
            .execute()
            .wait();
        assert!(rejected_job.is_err());

        let job: SelectJobs = db
            .drizzle
            .insert(db.tables.jobs)
            .values([InsertJobs::new(
                workspace.workspace_id,
                asset.asset_id,
                Work::Plan,
                "legal",
                "pc",
                Status::Queued,
            )])
            .returning(())
            .get()
            .wait()
            .unwrap();
        let rejected_attempt = db
            .drizzle
            .insert(db.tables.attempts)
            .values([InsertAttempts::new(job.job_id, 1, Status::Queued)])
            .execute()
            .wait();
        assert!(rejected_attempt.is_err());
    }

    #[test]
    fn recovery_manifest_round_trips_natural_parent_identity_and_hex_digest() {
        let recovered = UnsavedPayload {
            workspace: RecoveredWorkspace {
                key: WorkspaceKey {
                    project: "project".into(),
                    root: std::env::temp_dir()
                        .join("assetdb-recovered-project")
                        .to_string_lossy()
                        .into_owned(),
                    branch: "main".into(),
                },
                created: 1,
                updated: 2,
            },
            root: RecoveredRoot {
                key: "project".into(),
                owner: "project".into(),
                path: "assets".into(),
                exclusions: Exclusions::default(),
            },
            path: "tables/game.datasheet".into(),
            document: "game".into(),
            schema: "GameData".into(),
            encoding: Encoding::Bytes,
            revision: 4,
            saved: Some(3),
            digest: digest("unsaved"),
            bytes: 3,
            payload: vec![1, 2, 3],
            checkpoint: Some(vec![1, 2]),
            session: Some("session".into()),
            project: "project".into(),
            deleted: false,
            created: 1,
            updated: 4,
        };
        let encoded = serde_json::to_string(&recovered).unwrap();
        assert!(encoded.contains(&recovered.digest.to_hex()));
        assert_eq!(
            serde_json::from_str::<UnsavedPayload>(&encoded).unwrap(),
            recovered
        );
    }

    /// Removing the last observation of an asset must mark the identity deleted,
    /// unbind its source edges, and close its open path rows.
    fn assert_removal_retires_identity_and_paths(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace_b: &SelectWorkspaces,
        root_b: &SelectWorkspaceRoots,
        original: &SelectAssets,
    ) {
        writer
            .apply_sweep_delta_blocking(ApplySweepDelta {
                workspace_pk: workspace_b.workspace_id,
                workspace_root_pk: root_b.workspace_root_id,
                records: Vec::new(),
                removals: vec![SweepRemoval {
                    path: "new.ron".into(),
                    observed: 5,
                }],
            })
            .unwrap();
        let deleted: Option<SelectAssets> = db
            .drizzle
            .select(())
            .from(db.tables.assets)
            .r#where(eq(db.tables.assets.asset_id, original.asset_id))
            .get()
            .wait()
            .optional()
            .unwrap();
        assert!(deleted.unwrap().deleted);
        let unbound_source_edge: Option<SelectSourceEdges> = db
            .drizzle
            .select(())
            .from(db.tables.source_edges)
            .get()
            .wait()
            .optional()
            .unwrap();
        assert_eq!(unbound_source_edge.unwrap().depends_pk, None);
        let open_paths: Vec<SelectPaths> = db
            .drizzle
            .select(())
            .from(db.tables.paths)
            .r#where(and(
                eq(db.tables.paths.asset_pk, original.asset_id),
                is_null(db.tables.paths.to),
            ))
            .all()
            .wait()
            .unwrap();
        assert!(open_paths.is_empty());
    }

    /// After a workspace-scoped removal, that workspace's job and source edges must
    /// read back unbound.
    fn assert_edges_read_back_unbound(db: &AssetDb, consumer_job_id: i64) {
        let unbound_job_edge: Option<SelectJobEdges> = db
            .drizzle
            .select(())
            .from(db.tables.job_edges)
            .r#where(eq(db.tables.job_edges.job_pk, consumer_job_id))
            .get()
            .wait()
            .optional()
            .unwrap();
        assert_eq!(unbound_job_edge.unwrap().asset_pk, None);
        let unbound_source_edge: Option<SelectSourceEdges> = db
            .drizzle
            .select(())
            .from(db.tables.source_edges)
            .get()
            .wait()
            .optional()
            .unwrap();
        assert_eq!(unbound_source_edge.unwrap().depends_pk, None);
    }

    /// Re-observing the moved path in the bystander workspace must keep the shared
    /// identity alive after the other workspace removes it.
    fn assert_bystander_reobservation_keeps_identity_live(
        harness: SharedIdentityHarness<'_>,
        root_a: &SelectWorkspaceRoots,
        root_b: &SelectWorkspaceRoots,
        original: &SelectAssets,
        consumer: &SelectAssets,
        guid: Uuid,
        consumer_job_id: i64,
    ) {
        let SharedIdentityHarness {
            db,
            writer,
            workspace_a,
            workspace_b,
        } = harness;
        observe_asset(db, writer, workspace_b, root_b, "new.ron", guid);
        writer
            .apply_sweep_delta_blocking(ApplySweepDelta {
                workspace_pk: workspace_a.workspace_id,
                workspace_root_pk: root_a.workspace_root_id,
                records: Vec::new(),
                removals: vec![SweepRemoval {
                    path: "new.ron".into(),
                    observed: 4,
                }],
            })
            .unwrap();
        let still_live: Option<SelectAssets> = db
            .drizzle
            .select(())
            .from(db.tables.assets)
            .r#where(eq(db.tables.assets.asset_id, original.asset_id))
            .get()
            .wait()
            .optional()
            .unwrap();
        assert!(!still_live.unwrap().deleted);
        let jobs_after_first_removal: Vec<SelectJobs> = db
            .drizzle
            .select(())
            .from(db.tables.jobs)
            .r#where(eq(db.tables.jobs.workspace_pk, workspace_a.workspace_id))
            .all()
            .wait()
            .unwrap();
        assert_eq!(jobs_after_first_removal.len(), 2);
        assert!(
            jobs_after_first_removal
                .iter()
                .all(|job| job.asset_pk == consumer.asset_id)
        );
        assert_eq!(
            jobs_after_first_removal
                .iter()
                .filter(|job| job.kind == Work::Plan)
                .count(),
            2
        );
        assert!(
            !jobs_after_first_removal
                .iter()
                .find(|job| job.job_id == consumer_job_id)
                .unwrap()
                .ready
        );
        assert_edges_read_back_unbound(db, consumer_job_id);
    }

    /// A workspace-scoped removal must unbind only that workspace's job and source
    /// edges, leaving the bystander workspace's bindings in place.
    fn assert_removal_unbinds_only_its_workspace(
        harness: SharedIdentityHarness<'_>,
        root_a: &SelectWorkspaceRoots,
        root_b: &SelectWorkspaceRoots,
        original: &SelectAssets,
        consumer: &SelectAssets,
        guid: Uuid,
    ) {
        let SharedIdentityHarness {
            db,
            writer,
            workspace_a,
            ..
        } = harness;
        let source_edge_builder = Uuid::new_v4();
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace_a.workspace_id,
                expected: None,
                replacement: digest("move-catalog"),
                builders: vec![test_builder_descriptor(
                    source_edge_builder,
                    "move-source-analysis",
                )],
                plan_delta: PlanDelta {
                    replacements: vec![
                        PlannedJob {
                            asset_pk: original.asset_id,
                            kind: Work::Plan,
                            builder: None,
                            key: "source".into(),
                            platform: "pc".into(),
                            edges: Vec::new(),
                        },
                        PlannedJob {
                            asset_pk: consumer.asset_id,
                            kind: Work::Plan,
                            builder: None,
                            key: "consumer".into(),
                            platform: "pc".into(),
                            edges: vec![JobEdgeInput {
                                asset_pk: Some(original.asset_id),
                                target: Target::Guid(guid),
                                key: "source".into(),
                                platform: "pc".into(),
                                coupling: Coupling::Order,
                            }],
                        },
                    ],
                    source_edges: vec![SourceEdgeInput {
                        builder: source_edge_builder,
                        asset_pk: consumer.asset_id,
                        depends_pk: Some(original.asset_id),
                        target: Target::Guid(guid),
                        relation: Relation::SourceToSource,
                    }],
                    ..PlanDelta::default()
                },
                updated: 3,
            })
            .unwrap();
        let jobs_before: Vec<SelectJobs> = db
            .drizzle
            .select(())
            .from(db.tables.jobs)
            .r#where(eq(db.tables.jobs.workspace_pk, workspace_a.workspace_id))
            .all()
            .wait()
            .unwrap();
        let consumer_job_id = jobs_before
            .iter()
            .find(|job| job.asset_pk == consumer.asset_id && job.key == "consumer")
            .unwrap()
            .job_id;

        assert_bystander_reobservation_keeps_identity_live(
            harness,
            root_a,
            root_b,
            original,
            consumer,
            guid,
            consumer_job_id,
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn sweep_move_preserves_identity_and_removal_respects_workspace_bystanders() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace_a, root_a) = register_test_workspace(&writer, "move-a");
        let (workspace_b, _) = register_test_workspace(&writer, "move-b");
        let (_, root_b) = writer
            .register_workspace_root_blocking(RegisterWorkspaceRoot {
                workspace_pk: workspace_b.workspace_id,
                key: "move-a".into(),
                owner: "move-b".into(),
                path: "assets".into(),
                exclusions: Exclusions::default(),
            })
            .unwrap();
        let guid = Uuid::new_v4();
        let original = observe_asset(&db, &writer, &workspace_a, &root_a, "old.ron", guid);
        let moved = observe_asset(&db, &writer, &workspace_a, &root_a, "new.ron", guid);
        assert_eq!(moved.asset_id, original.asset_id);
        let paths: Vec<SelectPaths> = db
            .drizzle
            .select(())
            .from(db.tables.paths)
            .r#where(eq(db.tables.paths.asset_pk, original.asset_id))
            .order_by([asc(db.tables.paths.path_id)])
            .all()
            .wait()
            .unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].to.is_some());
        assert!(paths[1].to.is_none());
        let duplicate_open = db
            .drizzle
            .insert(db.tables.paths)
            .values([InsertPaths::new(
                workspace_a.workspace_id,
                original.asset_id,
                root_a.root_pk,
                "duplicate.ron",
                digest("duplicate"),
                3,
            )])
            .execute()
            .wait();
        assert!(duplicate_open.is_err());

        let consumer = observe_asset(
            &db,
            &writer,
            &workspace_a,
            &root_a,
            "consumer.ron",
            Uuid::new_v4(),
        );
        assert_removal_unbinds_only_its_workspace(
            SharedIdentityHarness {
                db: &db,
                writer: &writer,
                workspace_a: &workspace_a,
                workspace_b: &workspace_b,
            },
            &root_a,
            &root_b,
            &original,
            &consumer,
            guid,
        );
        assert_removal_retires_identity_and_paths(&db, &writer, &workspace_b, &root_b, &original);
    }

    /// The one seeded catalog vertical that every catalog-refusal assertion is
    /// written against: the database and writer under test, the workspace whose
    /// first catalog was published, the source and dependency identities that
    /// catalog planned, the build Job it produced, and the builder GUID plus the
    /// stored catalog digest the next replacement has to fence against.
    ///
    /// A refusal is only meaningful relative to the state that was published
    /// before it, so this whole set is seeded once and then threaded unchanged
    /// through the chain of assertions; each assertion adds only the row it
    /// introduces itself.
    #[derive(Clone, Copy)]
    struct CatalogVertical<'a> {
        db: &'a AssetDb,
        writer: &'a AssetDbWriter,
        workspace: &'a SelectWorkspaces,
        source: &'a SelectAssets,
        dependency: &'a SelectAssets,
        job: &'a SelectJobs,
        build_builder: Uuid,
        first_digest: Digest,
    }

    /// A completion whose product target names another builder is refused, and the
    /// attempt it came from stays leased rather than being half-committed.
    fn assert_completion_rejects_foreign_product_target(
        vertical: CatalogVertical<'_>,
        foreign_asset: &SelectAssets,
    ) {
        let CatalogVertical {
            db,
            writer,
            workspace,
            source,
            job,
            build_builder,
            first_digest,
            ..
        } = vertical;
        let second_digest = digest("catalog-v2");
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace.workspace_id,
                expected: Some(first_digest),
                replacement: second_digest,
                builders: vec![BuilderDescriptor {
                    guid: build_builder,
                    name: "catalog builder".into(),
                    version: 2,
                    digest: digest("catalog-builder-v2"),
                }],
                plan_delta: PlanDelta {
                    replacements: vec![PlannedJob {
                        asset_pk: source.asset_id,
                        kind: Work::Build,
                        builder: Some(build_builder),
                        key: "ownership".into(),
                        platform: "pc".into(),
                        edges: Vec::new(),
                    }],
                    ..PlanDelta::default()
                },
                updated: 7,
            })
            .unwrap();
        let ownership_job = db
            .ready_jobs(workspace.workspace_id, Work::Build, job.job_id, 10)
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.key == "ownership")
            .unwrap();
        let claimed = writer
            .claim_ready_job_blocking(ClaimReadyJob {
                job_id: ownership_job.job_id,
                expected_attempts: 0,
                owner: "worker".into(),
                lease_duration_ms: 100,
                staging: "stage".into(),
            })
            .unwrap();
        let ClaimReadyJobResult::Claimed { context } = claimed else {
            panic!("expected ownership claim")
        };
        let rejected = writer.complete_attempt_blocking(CompleteAttempt {
            attempt_id: context.attempt.attempt_id,
            owner: "worker".into(),
            status: Status::Succeeded,
            finished: 8,
            errors: 0,
            warnings: 0,
            products: vec![ProductInput {
                asset_pk: foreign_asset.asset_id,
                platform: "pc".into(),
                sub_id: 0,
                path: "foreign.bin".into(),
                kind: Uuid::new_v4(),
                format: "bin".into(),
                version: 1,
                aliases: Aliases::default(),
                registration: Registration::Registered,
                digest: digest("foreign-product"),
                bytes: 1,
                edges: Vec::new(),
            }],
            job_edges: Some(Vec::new()),
            plan_delta: None,
        });
        assert!(rejected.is_err());
        let attempt_after_rejection: Option<SelectAttempts> = db
            .drizzle
            .select(())
            .from(db.tables.attempts)
            .r#where(eq(
                db.tables.attempts.attempt_id,
                context.attempt.attempt_id,
            ))
            .get()
            .wait()
            .optional()
            .unwrap();
        assert_eq!(attempt_after_rejection.unwrap().status, Status::Leased);
    }

    /// A catalog replacement is refused when it drops a builder that still owns jobs,
    /// and when its expected digest does not match the stored one.
    fn assert_catalog_replacement_refusals(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        source: &SelectAssets,
        dependency: &SelectAssets,
        foreign_asset: &SelectAssets,
        first_digest: Digest,
    ) {
        let invalid_catalog = writer.replace_builder_catalog_blocking(ReplaceBuilderCatalog {
            workspace_pk: workspace.workspace_id,
            expected: Some(first_digest),
            replacement: digest("invalid-catalog"),
            builders: Vec::new(),
            plan_delta: PlanDelta {
                replacements: vec![PlannedJob {
                    asset_pk: foreign_asset.asset_id,
                    kind: Work::Plan,
                    builder: None,
                    key: "foreign".into(),
                    platform: "pc".into(),
                    edges: Vec::new(),
                }],
                ..PlanDelta::default()
            },
            updated: 6,
        });
        assert!(invalid_catalog.is_err());
        let mismatched_target = writer.replace_builder_catalog_blocking(ReplaceBuilderCatalog {
            workspace_pk: workspace.workspace_id,
            expected: Some(first_digest),
            replacement: digest("mismatched-target"),
            builders: Vec::new(),
            plan_delta: PlanDelta {
                replacements: vec![PlannedJob {
                    asset_pk: source.asset_id,
                    kind: Work::Plan,
                    builder: None,
                    key: "mismatched".into(),
                    platform: "pc".into(),
                    edges: vec![JobEdgeInput {
                        asset_pk: Some(dependency.asset_id),
                        target: Target::Guid(Uuid::new_v4()),
                        key: "dependency".into(),
                        platform: "pc".into(),
                        coupling: Coupling::Order,
                    }],
                }],
                ..PlanDelta::default()
            },
            updated: 6,
        });
        assert!(mismatched_target.is_err());
        assert_eq!(
            db.workspace(&workspace_key("catalog"))
                .unwrap()
                .unwrap()
                .builders,
            Some(first_digest)
        );
    }

    /// A replacement that drops a builder must also retire the source edges that
    /// builder owns; omitting them is refused and leaves the catalog unchanged.
    fn assert_replacement_requires_source_edge_retirement(
        vertical: CatalogVertical<'_>,
        replacement_descriptor: &BuilderDescriptor,
    ) {
        let CatalogVertical {
            db,
            writer,
            workspace,
            source,
            dependency,
            job,
            build_builder,
            first_digest,
        } = vertical;
        let omitted_source_edge_retirement =
            writer.replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace.workspace_id,
                expected: Some(first_digest),
                replacement: digest("catalog-omitted-source-edge-retirement"),
                builders: vec![replacement_descriptor.clone()],
                plan_delta: PlanDelta {
                    retire_job_ids: vec![job.job_id],
                    ..PlanDelta::default()
                },
                updated: 6,
            });
        assert!(omitted_source_edge_retirement.is_err());
        assert_eq!(
            db.jobs_for_asset(workspace.workspace_id, source.asset_id)
                .unwrap()
                .iter()
                .find(|candidate| candidate.job_id == job.job_id)
                .unwrap()
                .builder,
            Some(build_builder)
        );
        assert_eq!(
            db.source_edges_for_asset(workspace.workspace_id, source.asset_id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.workspace(&workspace_key("catalog"))
                .unwrap()
                .unwrap()
                .builders,
            Some(first_digest)
        );
        let stored_builders: Vec<SelectBuilders> = db
            .drizzle
            .select(())
            .from(db.tables.builders)
            .r#where(eq(db.tables.builders.workspace_pk, workspace.workspace_id))
            .all()
            .wait()
            .unwrap();
        assert_eq!(stored_builders.len(), 1);
        assert_eq!(stored_builders[0].guid, build_builder);

        let (other_workspace, other_root) = register_test_workspace(writer, "catalog-other");
        let foreign_asset = observe_asset(
            db,
            writer,
            &other_workspace,
            &other_root,
            "foreign.ron",
            Uuid::new_v4(),
        );
        assert_catalog_replacement_refusals(
            db,
            writer,
            workspace,
            source,
            dependency,
            &foreign_asset,
            first_digest,
        );
        assert_completion_rejects_foreign_product_target(vertical, &foreign_asset);
    }

    /// A catalog replacement must retire the build jobs and source edges it drops;
    /// omitting either retirement is refused and leaves the stored catalog intact.
    fn assert_catalog_replacement_requires_retirements(vertical: CatalogVertical<'_>) {
        let CatalogVertical {
            writer,
            workspace,
            source,
            dependency,
            build_builder,
            first_digest,
            ..
        } = vertical;
        let replacement_builder = Uuid::new_v4();
        let replacement_descriptor = BuilderDescriptor {
            guid: replacement_builder,
            name: "replacement builder".into(),
            version: 1,
            digest: digest("replacement-builder"),
        };
        let omitted_build_retirement =
            writer.replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace.workspace_id,
                expected: Some(first_digest),
                replacement: digest("catalog-omitted-build-retirement"),
                builders: vec![replacement_descriptor.clone()],
                plan_delta: PlanDelta::default(),
                updated: 6,
            });
        assert!(omitted_build_retirement.is_err());
        writer
            .apply_plan_delta_blocking(ApplyPlanDelta {
                workspace_pk: workspace.workspace_id,
                delta: PlanDelta {
                    source_edges: vec![SourceEdgeInput {
                        builder: build_builder,
                        asset_pk: source.asset_id,
                        depends_pk: Some(dependency.asset_id),
                        target: Target::Guid(dependency.guid),
                        relation: Relation::SourceToSource,
                    }],
                    ..PlanDelta::default()
                },
            })
            .unwrap();
        assert_replacement_requires_source_edge_retirement(vertical, &replacement_descriptor);
    }

    /// Catalog and plan-delta writes must refuse builders the workspace never stored,
    /// and must not leave partial state behind when they do.
    fn assert_catalog_refuses_unknown_builders(vertical: CatalogVertical<'_>) {
        let CatalogVertical {
            db,
            writer,
            workspace,
            source,
            build_builder,
            first_digest,
            ..
        } = vertical;
        let missing_builder = Uuid::new_v4();
        let missing_builder_update =
            writer.replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace.workspace_id,
                expected: Some(first_digest),
                replacement: digest("catalog-missing-builder"),
                builders: Vec::new(),
                plan_delta: PlanDelta {
                    replacements: vec![PlannedJob::build(
                        source.asset_id,
                        missing_builder,
                        "missing-builder",
                        "pc",
                        Vec::new(),
                    )],
                    ..PlanDelta::default()
                },
                updated: 6,
            });
        assert!(missing_builder_update.is_err());
        assert_eq!(
            db.workspace(&workspace_key("catalog"))
                .unwrap()
                .unwrap()
                .builders,
            Some(first_digest)
        );
        let stored_builders: Vec<SelectBuilders> = db
            .drizzle
            .select(())
            .from(db.tables.builders)
            .r#where(eq(db.tables.builders.workspace_pk, workspace.workspace_id))
            .all()
            .wait()
            .unwrap();
        assert_eq!(stored_builders.len(), 1);
        assert_eq!(stored_builders[0].guid, build_builder);
        assert!(
            db.jobs_for_asset(workspace.workspace_id, source.asset_id)
                .unwrap()
                .iter()
                .all(|job| job.key != "missing-builder")
        );
        let missing_stored_builder = writer.apply_plan_delta_blocking(ApplyPlanDelta {
            workspace_pk: workspace.workspace_id,
            delta: PlanDelta {
                replacements: vec![PlannedJob::build(
                    source.asset_id,
                    missing_builder,
                    "missing-stored-builder",
                    "pc",
                    Vec::new(),
                )],
                ..PlanDelta::default()
            },
        });
        assert!(missing_stored_builder.is_err());
        assert!(
            db.jobs_for_asset(workspace.workspace_id, source.asset_id)
                .unwrap()
                .iter()
                .all(|job| job.key != "missing-stored-builder")
        );
        assert_catalog_replacement_requires_retirements(vertical);
    }

    /// Completing the plan attempt must publish its catalog rows and flip the
    /// dependent job to ready in the same durable commit.
    fn assert_completion_publishes_catalog_and_readies_dependent(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        source: &SelectAssets,
        dependency: &SelectAssets,
        job: &SelectJobs,
        dependent_job_id: i64,
    ) {
        let claimed = writer
            .claim_ready_job_blocking(ClaimReadyJob {
                job_id: job.job_id,
                expected_attempts: 0,
                owner: "worker".into(),
                lease_duration_ms: 100,
                staging: "stage".into(),
            })
            .unwrap();
        let ClaimReadyJobResult::Claimed { context } = claimed else {
            panic!("expected claim")
        };
        let completion = writer
            .complete_attempt_blocking(CompleteAttempt {
                attempt_id: context.attempt.attempt_id,
                owner: "worker".into(),
                status: Status::Succeeded,
                finished: 5,
                errors: 0,
                warnings: 0,
                products: vec![
                    ProductInput {
                        asset_pk: source.asset_id,
                        platform: "pc".into(),
                        sub_id: 1,
                        path: "products/a.bin".into(),
                        kind: Uuid::new_v4(),
                        format: "bin".into(),
                        version: 1,
                        aliases: Aliases::default(),
                        registration: Registration::Registered,
                        digest: digest("product-a"),
                        bytes: 10,
                        edges: vec![ProductEdgeInput {
                            guid: dependency.guid,
                            sub_id: 0,
                            flags: 1,
                        }],
                    },
                    ProductInput {
                        asset_pk: source.asset_id,
                        platform: "pc".into(),
                        sub_id: 2,
                        path: "products/b.bin".into(),
                        kind: Uuid::new_v4(),
                        format: "bin".into(),
                        version: 1,
                        aliases: Aliases::default(),
                        registration: Registration::Registered,
                        digest: digest("product-b"),
                        bytes: 20,
                        edges: Vec::new(),
                    },
                ],
                job_edges: Some(vec![JobEdgeInput {
                    asset_pk: Some(dependency.asset_id),
                    target: Target::Guid(dependency.guid),
                    key: "dependency".into(),
                    platform: "pc".into(),
                    coupling: Coupling::Order,
                }]),
                plan_delta: None,
            })
            .unwrap();
        let CompleteAttemptResult::Completed { became_ready, .. } = completion else {
            panic!("expected completed attempt")
        };
        assert_eq!(became_ready, vec![dependent_job_id]);
        let dependent_after: Option<SelectJobs> = db
            .drizzle
            .select(())
            .from(db.tables.jobs)
            .r#where(eq(db.tables.jobs.job_id, dependent_job_id))
            .get()
            .wait()
            .optional()
            .unwrap();
        assert!(dependent_after.unwrap().ready);
        assert_eq!(db.catalog_count(workspace.workspace_id, "pc").unwrap(), 2);
    }

    /// The keyset catalog page must hand back one row per product, never repeat a
    /// product across pages, and agree with the processing-status counts.
    fn assert_catalog_keyset_page_and_status(db: &AssetDb, workspace: &SelectWorkspaces) {
        let first = db
            .catalog_page(workspace.workspace_id, "pc", None, 1)
            .unwrap();
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.product_edges.len(), 1);
        let second = db
            .catalog_page(workspace.workspace_id, "pc", first.next.as_ref(), 1)
            .unwrap();
        assert_eq!(second.rows.len(), 1);
        assert_ne!(first.rows[0].product_pk, second.rows[0].product_pk);
        let status = db
            .processing_status(workspace.workspace_id, Some("pc"))
            .unwrap();
        assert_eq!(status.succeeded, 1);
        assert_eq!(status.queued, 3);
        assert_eq!(status.ready, 3);
    }

    /// Publishing the first builder catalog must be accepted as a compare-and-set
    /// against the empty baseline, and must plan the jobs it declares.
    fn assert_first_catalog_publish_plans_jobs(
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        source: &SelectAssets,
        dependency: &SelectAssets,
        build_builder: Uuid,
        first_digest: Digest,
    ) {
        assert_eq!(
            writer
                .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                    workspace_pk: workspace.workspace_id,
                    expected: None,
                    replacement: first_digest,
                    builders: vec![BuilderDescriptor {
                        guid: build_builder,
                        name: "catalog builder".into(),
                        version: 1,
                        digest: digest("catalog-builder-v1"),
                    }],
                    plan_delta: PlanDelta {
                        replacements: vec![
                            PlannedJob::build(
                                source.asset_id,
                                build_builder,
                                "build",
                                "pc",
                                Vec::new(),
                            ),
                            PlannedJob::plan(
                                dependency.asset_id,
                                "dependent",
                                "pc",
                                vec![JobEdgeInput {
                                    asset_pk: Some(source.asset_id),
                                    target: Target::Guid(source.guid),
                                    key: "build".into(),
                                    platform: "pc".into(),
                                    coupling: Coupling::Fingerprint,
                                }],
                            ),
                        ],
                        ..PlanDelta::default()
                    },
                    updated: 3,
                })
                .unwrap(),
            BuilderCatalogReplaceOutcome::Replaced
        );
        assert_eq!(
            writer
                .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                    workspace_pk: workspace.workspace_id,
                    expected: None,
                    replacement: digest("conflicting"),
                    builders: Vec::new(),
                    plan_delta: PlanDelta::default(),
                    updated: 4,
                })
                .unwrap(),
            BuilderCatalogReplaceOutcome::Conflict {
                actual: Some(first_digest)
            }
        );
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn catalog_cas_completion_and_keyset_page_are_one_typed_vertical() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "catalog");
        let source = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "source.ron",
            Uuid::new_v4(),
        );
        let dependency = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "dependency.ron",
            Uuid::new_v4(),
        );
        let build_builder = Uuid::new_v4();
        let first_digest = digest("catalog-v1");
        assert_first_catalog_publish_plans_jobs(
            &writer,
            &workspace,
            &source,
            &dependency,
            build_builder,
            first_digest,
        );
        let job = db
            .ready_jobs(workspace.workspace_id, Work::Build, 0, 1)
            .unwrap()
            .pop()
            .unwrap();
        let dependent_before: Option<SelectJobs> = db
            .drizzle
            .select(())
            .from(db.tables.jobs)
            .r#where(and(
                eq(db.tables.jobs.workspace_pk, workspace.workspace_id),
                eq(db.tables.jobs.key, "dependent"),
            ))
            .get()
            .wait()
            .optional()
            .unwrap();
        let dependent_job_id = dependent_before.as_ref().unwrap().job_id;
        assert!(!dependent_before.unwrap().ready);
        assert_completion_publishes_catalog_and_readies_dependent(
            &db,
            &writer,
            &workspace,
            &source,
            &dependency,
            &job,
            dependent_job_id,
        );
        assert_catalog_keyset_page_and_status(&db, &workspace);
        assert_catalog_refuses_unknown_builders(CatalogVertical {
            db: &db,
            writer: &writer,
            workspace: &workspace,
            source: &source,
            dependency: &dependency,
            job: &job,
            build_builder,
            first_digest,
        });
    }

    /// Seeds the products and product edges the shared-target page test reads back.
    fn seed_shared_product_targets(
        db: &AssetDb,
        writer: &AssetDbWriter,
        workspace: &SelectWorkspaces,
        source_a: &SelectAssets,
        source_b: &SelectAssets,
        target: &SelectAssets,
        target_kind: Uuid,
    ) {
        for (asset, key, path, sub_id, kind, edges) in [
            (
                &target,
                "target",
                "products/target.bin",
                7,
                target_kind,
                Vec::new(),
            ),
            (
                &source_a,
                "source-a",
                "products/source-a.bin",
                1,
                Uuid::new_v4(),
                vec![ProductEdgeInput {
                    guid: target.guid,
                    sub_id: 7,
                    flags: 1,
                }],
            ),
            (
                &source_b,
                "source-b",
                "products/source-b.bin",
                2,
                Uuid::new_v4(),
                vec![ProductEdgeInput {
                    guid: target.guid,
                    sub_id: 7,
                    flags: 2,
                }],
            ),
        ] {
            let job = db
                .jobs_for_asset(workspace.workspace_id, asset.asset_id)
                .unwrap()
                .into_iter()
                .find(|job| job.key == key)
                .unwrap();
            let claimed = writer
                .claim_ready_job_blocking(ClaimReadyJob {
                    job_id: job.job_id,
                    expected_attempts: 0,
                    owner: format!("worker-{key}"),
                    lease_duration_ms: 100,
                    staging: format!("staging/{key}"),
                })
                .unwrap();
            let ClaimReadyJobResult::Claimed { context } = claimed else {
                panic!("{key} must be claimable")
            };
            writer
                .complete_attempt_blocking(CompleteAttempt {
                    attempt_id: context.attempt.attempt_id,
                    owner: format!("worker-{key}"),
                    status: Status::Succeeded,
                    finished: 5,
                    errors: 0,
                    warnings: 0,
                    products: vec![ProductInput {
                        asset_pk: asset.asset_id,
                        platform: "pc".to_string(),
                        sub_id,
                        path: path.to_string(),
                        kind,
                        format: "bin".to_string(),
                        version: 1,
                        aliases: Aliases::default(),
                        registration: Registration::Registered,
                        digest: digest(path),
                        bytes: 10,
                        edges,
                    }],
                    job_edges: None,
                    plan_delta: None,
                })
                .unwrap();
        }
    }

    #[test]
    fn catalog_page_resolves_every_edge_that_shares_one_product_target() {
        let db = AssetDb::open_in_memory().unwrap();
        let writer = db.writer().unwrap();
        let (workspace, root) = register_test_workspace(&writer, "shared-catalog-target");
        let source_a = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "source-a.ron",
            Uuid::new_v4(),
        );
        let source_b = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "source-b.ron",
            Uuid::new_v4(),
        );
        let target = observe_asset(
            &db,
            &writer,
            &workspace,
            &root,
            "target.ron",
            Uuid::new_v4(),
        );
        let builder = Uuid::new_v4();
        writer
            .replace_builder_catalog_blocking(ReplaceBuilderCatalog {
                workspace_pk: workspace.workspace_id,
                expected: None,
                replacement: digest("shared-catalog-target"),
                builders: vec![BuilderDescriptor {
                    guid: builder,
                    name: "shared target builder".to_string(),
                    version: 1,
                    digest: digest("shared target builder"),
                }],
                plan_delta: PlanDelta {
                    replacements: vec![
                        PlannedJob::build(source_a.asset_id, builder, "source-a", "pc", Vec::new()),
                        PlannedJob::build(source_b.asset_id, builder, "source-b", "pc", Vec::new()),
                        PlannedJob::build(target.asset_id, builder, "target", "pc", Vec::new()),
                    ],
                    ..PlanDelta::default()
                },
                updated: 3,
            })
            .unwrap();

        let target_kind = Uuid::new_v4();
        seed_shared_product_targets(
            &db,
            &writer,
            &workspace,
            &source_a,
            &source_b,
            &target,
            target_kind,
        );
        let page = db
            .catalog_page(workspace.workspace_id, "pc", None, 10)
            .unwrap();
        assert_eq!(page.product_edges.len(), 2);
        drop(db);
        for dependency in page.product_edges {
            let resolved = dependency.target.expect("shared target product");
            assert_eq!(resolved.guid, target.guid);
            assert_eq!(resolved.sub_id, 7);
            assert_eq!(resolved.kind, target_kind);
        }
    }

    // Test-only: the fixture deliberately lives for the whole test, since it owns the
    // temp directory and database the assertions run against, and the values under test
    // borrow from it. There is no cross-thread contention here for an early drop to
    // relieve.
    #[allow(clippy::significant_drop_tightening)]
    #[test]
    fn recovery_import_remaps_parents_and_is_idempotent_but_cas_fenced() {
        let payload = UnsavedPayload {
            workspace: RecoveredWorkspace {
                key: WorkspaceKey {
                    project: "recovery".into(),
                    root: std::env::temp_dir()
                        .join("assetdb-recovery-project")
                        .to_string_lossy()
                        .into_owned(),
                    branch: "main".into(),
                },
                created: 1,
                updated: 2,
            },
            root: RecoveredRoot {
                key: "portable-root".into(),
                owner: "project".into(),
                path: "assets".into(),
                exclusions: Exclusions::default(),
            },
            path: "source.ron".into(),
            document: "source.ron".into(),
            schema: "schema".into(),
            encoding: Encoding::Ron,
            revision: 2,
            saved: Some(1),
            digest: digest("unsaved"),
            bytes: 7,
            payload: b"unsaved".to_vec(),
            checkpoint: Some(b"saved".to_vec()),
            session: Some("session".into()),
            project: "recovery".into(),
            deleted: false,
            created: 1,
            updated: 2,
        };
        let source = AssetDb::open_in_memory().unwrap();
        let source_writer = source.writer().unwrap();
        assert!(matches!(
            source_writer
                .import_unsaved_payload_blocking(ImportUnsavedPayload {
                    payload: payload.clone(),
                    expected: ExpectedPayload::Absent,
                })
                .unwrap(),
            ImportRecoveredPayloadResult::Imported(_)
        ));
        let exported = source.export_unsaved_payloads().unwrap();
        drop(source);
        assert_eq!(exported, vec![payload.clone()]);

        let restored = AssetDb::open_in_memory().unwrap();
        let restored_writer = restored.writer().unwrap();
        let (unrelated_workspace, unrelated_root) =
            register_test_workspace(&restored_writer, "unrelated");
        assert_eq!(unrelated_workspace.workspace_id, 1);
        assert_eq!(unrelated_root.root_pk, 1);
        assert!(matches!(
            restored_writer
                .import_unsaved_payload_blocking(ImportUnsavedPayload {
                    payload: exported[0].clone(),
                    expected: ExpectedPayload::Absent,
                })
                .unwrap(),
            ImportRecoveredPayloadResult::Imported(_)
        ));
        assert!(matches!(
            restored_writer
                .import_unsaved_payload_blocking(ImportUnsavedPayload {
                    payload: payload.clone(),
                    expected: ExpectedPayload::Absent,
                })
                .unwrap(),
            ImportRecoveredPayloadResult::AlreadyPresent(_)
        ));
        let mut conflict = payload.clone();
        conflict.payload.push(9);
        assert!(matches!(
            restored_writer
                .import_unsaved_payload_blocking(ImportUnsavedPayload {
                    payload: conflict,
                    expected: ExpectedPayload::SavedAt {
                        revision: payload.revision,
                        digest: payload.digest,
                    },
                })
                .unwrap(),
            ImportRecoveredPayloadResult::BaselineConflict
        ));
        let workspace = restored.workspace(&payload.workspace.key).unwrap().unwrap();
        assert_ne!(workspace.workspace_id, unrelated_workspace.workspace_id);
        assert_eq!(workspace.builders, None);
        assert_eq!(restored.export_unsaved_payloads().unwrap(), vec![payload]);
    }
}
