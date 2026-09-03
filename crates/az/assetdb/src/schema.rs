//! Durable `AssetDB` schema.
//!
//! The database is a regenerable workspace projection with one exception:
//! unsaved editor payloads. Jobs are the current scheduler entity; Attempts
//! are bounded execution history; Products and dependency edges belong to the
//! Job. Drizzle migrations are the only schema-version authority.

use drizzle::sqlite::prelude::*;
use uuid::Uuid;

use crate::value::{
    Aliases, Coupling, Diff, Digest, Encoding, Exclusions, Registration, Relation, Status, Target,
    Work,
};

/// Portable source-root identity. Manifest policy belongs to WorkspaceRoots.
#[SQLiteTable(STRICT)]
pub struct Roots {
    #[column(primary)]
    pub root_id: i64,
    #[column(unique)]
    pub key: String,
}

/// One project workspace/branch projection.
#[SQLiteTable(STRICT)]
pub struct Workspaces {
    #[column(primary)]
    pub workspace_id: i64,
    pub project: String,
    pub root: String,
    pub branch: String,
    /// Digest of the worker builder catalog used to derive this graph.
    #[column(blob)]
    pub builders: Option<Digest>,
    pub created: i64,
    pub updated: i64,
}

/// Manifest-derived policy for one portable root in one workspace.
#[SQLiteTable(STRICT)]
pub struct WorkspaceRoots {
    #[column(primary)]
    pub workspace_root_id: i64,
    #[column(references = Workspaces::workspace_id, on_delete = CASCADE)]
    pub workspace_pk: i64,
    #[column(references = Roots::root_id, on_delete = RESTRICT)]
    pub root_pk: i64,
    pub owner: String,
    pub path: String,
    #[column(JSON)]
    pub exclusions: Exclusions,
}

/// Stable asset identity shared by every workspace that observes the GUID.
#[SQLiteTable(STRICT)]
pub struct Assets {
    #[column(primary)]
    pub asset_id: i64,
    #[column(blob, unique)]
    pub guid: Uuid,
    /// Derived global lifecycle: true only when every Entry is deleted.
    #[column(default = false)]
    pub deleted: bool,
    pub created: i64,
    pub updated: i64,
}

/// Historical source locators for one workspace's observation of an Asset.
#[SQLiteTable(STRICT)]
pub struct Paths {
    #[column(primary)]
    pub path_id: i64,
    #[column(references = Workspaces::workspace_id, on_delete = CASCADE)]
    pub workspace_pk: i64,
    #[column(references = Assets::asset_id, on_delete = CASCADE)]
    pub asset_pk: i64,
    #[column(references = Roots::root_id, on_delete = RESTRICT)]
    pub root_pk: i64,
    pub path: String,
    #[column(blob)]
    pub digest: Digest,
    pub session: Option<String>,
    pub from: i64,
    pub to: Option<i64>,
}

/// One workspace's current observation of an Asset.
#[SQLiteTable(STRICT)]
pub struct Entries {
    #[column(primary)]
    pub entry_id: i64,
    #[column(references = Workspaces::workspace_id, on_delete = CASCADE)]
    pub workspace_pk: i64,
    #[column(references = Assets::asset_id, on_delete = CASCADE)]
    pub asset_pk: i64,
    #[column(references = Roots::root_id, on_delete = RESTRICT)]
    pub root_pk: i64,
    pub path: String,
    pub schema: Option<String>,
    #[column(blob)]
    pub digest: Digest,
    #[column(integer, ENUM)]
    pub diff: Diff,
    #[column(default = 0)]
    pub diagnostics: i64,
    pub updated: i64,
    pub src_bytes: i64,
    pub src_mtime: i64,
    pub meta_bytes: i64,
    pub meta_mtime: i64,
    pub observed: i64,
}

/// Editable payload authority for both reflected RON documents and file-backed
/// source bytes. `payload` may contain unsaved state; `checkpoint` is the
/// explicit-save image consumed by workers.
#[SQLiteTable(STRICT)]
pub struct Payloads {
    #[column(primary)]
    pub payload_id: i64,
    #[column(references = Workspaces::workspace_id, on_delete = CASCADE)]
    pub workspace_pk: i64,
    #[column(references = Roots::root_id, on_delete = RESTRICT)]
    pub root_pk: i64,
    pub path: String,
    pub document: String,
    pub schema: String,
    #[column(integer, ENUM)]
    pub encoding: Encoding,
    pub revision: i64,
    pub saved: Option<i64>,
    #[column(blob)]
    pub digest: Digest,
    pub bytes: i64,
    pub payload: Vec<u8>,
    pub checkpoint: Option<Vec<u8>>,
    pub session: Option<String>,
    pub project: String,
    #[column(default = false)]
    pub deleted: bool,
    pub created: i64,
    pub updated: i64,
}

/// Project-linked builder descriptors for one workspace.
#[SQLiteTable(STRICT)]
pub struct Builders {
    #[column(primary)]
    pub builder_id: i64,
    #[column(references = Workspaces::workspace_id, on_delete = CASCADE)]
    pub workspace_pk: i64,
    #[column(blob)]
    pub guid: Uuid,
    pub name: String,
    pub version: i64,
    #[column(blob)]
    pub digest: Digest,
}

/// One logical scheduler job and its current state.
///
/// SQLite treats NULLs as distinct in a UNIQUE index, so Plan and Build jobs
/// use separate Drizzle-owned partial unique indices below. The CHECK makes the
/// work/builder relationship unrepresentable in durable state.
#[SQLiteTable(
    STRICT,
    CHECK(
        expr = "((kind = 0 AND builder IS NULL) OR (kind = 1 AND builder IS NOT NULL)) AND status IN (0, 1, 2, 3)"
    )
)]
pub struct Jobs {
    #[column(primary)]
    pub job_id: i64,
    #[column(references = Workspaces::workspace_id, on_delete = CASCADE)]
    pub workspace_pk: i64,
    #[column(references = Assets::asset_id, on_delete = CASCADE)]
    pub asset_pk: i64,
    #[column(integer, ENUM)]
    pub kind: Work,
    #[column(blob)]
    pub builder: Option<Uuid>,
    pub key: String,
    pub platform: String,
    #[column(integer, ENUM)]
    pub status: Status,
    #[column(default = false)]
    pub ready: bool,
    #[column(default = 0)]
    pub attempts: i64,
}

/// Bounded execution history for one Job.
#[SQLiteTable(STRICT, CHECK(expr = "status IN (1, 2, 3, 4)"))]
pub struct Attempts {
    #[column(primary)]
    pub attempt_id: i64,
    #[column(references = Jobs::job_id, on_delete = CASCADE)]
    pub job_pk: i64,
    pub ordinal: i64,
    #[column(integer, ENUM)]
    pub status: Status,
    pub owner: Option<String>,
    /// Diagnostic wall-clock projection. The dispatcher owns the authoritative
    /// monotonic deadline and renewal never writes this field.
    pub expires: Option<i64>,
    pub staging: Option<String>,
    pub finished: Option<i64>,
    #[column(default = 0)]
    pub errors: i64,
    #[column(default = 0)]
    pub warnings: i64,
}

/// Current product set. Job is provenance; workspace/platform/asset/sub-id is
/// product identity.
#[SQLiteTable(STRICT)]
pub struct Products {
    #[column(primary)]
    pub product_id: i64,
    #[column(references = Workspaces::workspace_id, on_delete = CASCADE)]
    pub workspace_pk: i64,
    #[column(references = Assets::asset_id, on_delete = CASCADE)]
    pub asset_pk: i64,
    pub platform: String,
    pub sub_id: i64,
    #[column(references = Jobs::job_id, on_delete = CASCADE)]
    pub job_pk: i64,
    pub path: String,
    #[column(blob)]
    pub kind: Uuid,
    pub format: String,
    pub version: i64,
    #[column(JSON)]
    pub aliases: Aliases,
    #[column(integer, ENUM)]
    pub registration: Registration,
    #[column(blob)]
    pub digest: Digest,
    pub bytes: i64,
}

/// Runtime asset references emitted by one Product.
#[SQLiteTable(STRICT)]
pub struct ProductEdges {
    #[column(primary)]
    pub product_edge_id: i64,
    #[column(references = Products::product_id, on_delete = CASCADE)]
    pub product_pk: i64,
    #[column(blob)]
    pub guid: Uuid,
    pub sub_id: i64,
    pub flags: i64,
}

/// Job-order dependency authored by a builder and optionally bound to an Asset.
#[SQLiteTable(STRICT)]
pub struct JobEdges {
    #[column(primary)]
    pub job_edge_id: i64,
    #[column(references = Jobs::job_id, on_delete = CASCADE)]
    pub job_pk: i64,
    #[column(references = Assets::asset_id, on_delete = RESTRICT)]
    pub asset_pk: Option<i64>,
    #[column(blob)]
    pub target: Target,
    pub key: String,
    pub platform: String,
    #[column(integer, ENUM)]
    pub coupling: Coupling,
}

/// Source-analysis dependency authored by a builder and optionally bound to an
/// Asset. Moves do not rewrite either side: both references are identities.
#[SQLiteTable(STRICT)]
pub struct SourceEdges {
    #[column(primary)]
    pub source_edge_id: i64,
    #[column(references = Workspaces::workspace_id, on_delete = CASCADE)]
    pub workspace_pk: i64,
    #[column(blob)]
    pub builder: Uuid,
    #[column(references = Assets::asset_id, on_delete = CASCADE)]
    pub asset_pk: i64,
    #[column(references = Assets::asset_id, on_delete = RESTRICT)]
    pub depends_pk: Option<i64>,
    #[column(blob)]
    pub target: Target,
    #[column(integer, ENUM)]
    pub relation: Relation,
}

/// Runtime catalog projection. Products remain visible while their current Job
/// is queued, leased, or succeeded; a terminal failed rebuild hides
/// the previous product set rather than shipping stale bytes.
#[SQLiteView(
    NAME = "catalog",
    DEFINITION = "SELECT p.product_id AS product_pk, p.workspace_pk, p.platform, a.guid, p.sub_id, p.path, p.kind, p.format, p.version, p.aliases, p.registration, p.digest, p.bytes, e.path AS source, e.schema, p.job_pk, j.builder, j.key AS job_key FROM products AS p INNER JOIN assets AS a ON p.asset_pk = a.asset_id INNER JOIN jobs AS j ON p.job_pk = j.job_id INNER JOIN entries AS e ON e.workspace_pk = p.workspace_pk AND e.asset_pk = p.asset_pk WHERE e.diff IN (0, 1, 2) AND j.status IN (0, 1, 2) ORDER BY p.path, a.guid, p.sub_id"
)]
pub struct Catalog {
    pub product_pk: i64,
    pub workspace_pk: i64,
    pub platform: String,
    #[column(blob)]
    pub guid: Uuid,
    pub sub_id: i64,
    pub path: String,
    #[column(blob)]
    pub kind: Uuid,
    pub format: String,
    pub version: i64,
    #[column(JSON)]
    pub aliases: Aliases,
    #[column(integer, ENUM)]
    pub registration: Registration,
    #[column(blob)]
    pub digest: Digest,
    pub bytes: i64,
    pub source: String,
    pub schema: Option<String>,
    pub job_pk: i64,
    #[column(blob)]
    pub builder: Option<Uuid>,
    pub job_key: String,
}

#[SQLiteIndex(unique)]
pub struct IdxWorkspacesKey(Workspaces::project, Workspaces::root, Workspaces::branch);

#[SQLiteIndex(unique)]
pub struct IdxWorkspaceRootsWorkspaceRoot(WorkspaceRoots::workspace_pk, WorkspaceRoots::root_pk);

#[SQLiteIndex]
pub struct IdxWorkspaceRootsRoot(WorkspaceRoots::root_pk);

#[SQLiteIndex(unique, where = "\"to\" IS NULL")]
pub struct IdxPathsAssetOpen(Paths::workspace_pk, Paths::asset_pk);

#[SQLiteIndex(unique, where = "\"to\" IS NULL")]
pub struct IdxPathsLocatorOpen(Paths::workspace_pk, Paths::root_pk, Paths::path);

#[SQLiteIndex(unique)]
pub struct IdxEntriesPath(Entries::workspace_pk, Entries::root_pk, Entries::path);

#[SQLiteIndex(unique)]
pub struct IdxEntriesAsset(Entries::workspace_pk, Entries::asset_pk);

#[SQLiteIndex(unique)]
pub struct IdxPayloadsDocument(Payloads::workspace_pk, Payloads::document);

#[SQLiteIndex(unique)]
pub struct IdxPayloadsPath(Payloads::workspace_pk, Payloads::root_pk, Payloads::path);

#[SQLiteIndex(unique)]
pub struct IdxBuildersWorkspaceGuid(Builders::workspace_pk, Builders::guid);

// SQLite considers NULL distinct. These two partial indices and Jobs' CHECK
// enforce exactly one logical Plan or Build job without a sentinel value.
#[SQLiteIndex(unique, where = "kind = 0 AND builder IS NULL")]
pub struct IdxJobsPlan(
    Jobs::workspace_pk,
    Jobs::asset_pk,
    Jobs::kind,
    Jobs::key,
    Jobs::platform,
);

#[SQLiteIndex(unique, where = "kind = 1 AND builder IS NOT NULL")]
pub struct IdxJobsBuild(
    Jobs::workspace_pk,
    Jobs::asset_pk,
    Jobs::kind,
    Jobs::builder,
    Jobs::key,
    Jobs::platform,
);

#[SQLiteIndex]
pub struct IdxJobsReady(
    Jobs::workspace_pk,
    Jobs::kind,
    Jobs::status,
    Jobs::ready,
    Jobs::job_id,
);

#[SQLiteIndex]
pub struct IdxJobsTarget(
    Jobs::workspace_pk,
    Jobs::asset_pk,
    Jobs::key,
    Jobs::platform,
    Jobs::status,
);

#[SQLiteIndex]
pub struct IdxJobsProcessing(Jobs::workspace_pk, Jobs::platform, Jobs::status);

#[SQLiteIndex(unique)]
pub struct IdxAttemptsJobOrdinal(Attempts::job_pk, Attempts::ordinal);

#[SQLiteIndex]
pub struct IdxAttemptsLease(Attempts::status, Attempts::expires);

#[SQLiteIndex(unique)]
pub struct IdxProductsIdentity(
    Products::workspace_pk,
    Products::platform,
    Products::asset_pk,
    Products::sub_id,
);

#[SQLiteIndex]
pub struct IdxProductsJob(Products::job_pk);

#[SQLiteIndex]
pub struct IdxProductEdgesProduct(ProductEdges::product_pk);

#[SQLiteIndex]
pub struct IdxJobEdgesJob(JobEdges::job_pk);

#[SQLiteIndex]
pub struct IdxJobEdgesTarget(JobEdges::asset_pk, JobEdges::key, JobEdges::platform);

/// Bind newly appearing assets to authored dependency targets without scanning
/// the complete edge table during a clean cook.
#[SQLiteIndex]
pub struct IdxJobEdgesAuthoredTarget(JobEdges::target);

#[SQLiteIndex(unique)]
pub struct IdxSourceEdgesBuilderAssetTarget(
    SourceEdges::workspace_pk,
    SourceEdges::builder,
    SourceEdges::asset_pk,
    SourceEdges::target,
    SourceEdges::relation,
);

#[SQLiteIndex]
pub struct IdxSourceEdgesTarget(SourceEdges::workspace_pk, SourceEdges::depends_pk);

/// Bind newly appearing assets to authored source targets.
#[SQLiteIndex]
pub struct IdxSourceEdgesAuthoredTarget(SourceEdges::workspace_pk, SourceEdges::target);

// `SQLiteSchema` emits an explicit `Clone` impl alongside `Copy`; the impl is
// macro-generated and cannot be replaced with a derive from here.
#[allow(clippy::expl_impl_clone_on_copy)]
#[derive(SQLiteSchema)]
pub struct AssetSchema {
    pub roots: Roots,
    pub workspaces: Workspaces,
    pub workspace_roots: WorkspaceRoots,
    pub assets: Assets,
    pub paths: Paths,
    pub entries: Entries,
    pub payloads: Payloads,
    pub builders: Builders,
    pub jobs: Jobs,
    pub attempts: Attempts,
    pub products: Products,
    pub product_edges: ProductEdges,
    pub job_edges: JobEdges,
    pub source_edges: SourceEdges,
    pub catalog: Catalog,

    pub idx_workspaces_key: IdxWorkspacesKey,
    pub idx_workspace_roots_workspace_root: IdxWorkspaceRootsWorkspaceRoot,
    pub idx_workspace_roots_root: IdxWorkspaceRootsRoot,
    pub idx_paths_asset_open: IdxPathsAssetOpen,
    pub idx_paths_locator_open: IdxPathsLocatorOpen,
    pub idx_entries_path: IdxEntriesPath,
    pub idx_entries_asset: IdxEntriesAsset,
    pub idx_payloads_document: IdxPayloadsDocument,
    pub idx_payloads_path: IdxPayloadsPath,
    pub idx_builders_workspace_guid: IdxBuildersWorkspaceGuid,
    pub idx_jobs_plan: IdxJobsPlan,
    pub idx_jobs_build: IdxJobsBuild,
    pub idx_jobs_ready: IdxJobsReady,
    pub idx_jobs_target: IdxJobsTarget,
    pub idx_jobs_processing: IdxJobsProcessing,
    pub idx_attempts_job_ordinal: IdxAttemptsJobOrdinal,
    pub idx_attempts_lease: IdxAttemptsLease,
    pub idx_products_identity: IdxProductsIdentity,
    pub idx_products_job: IdxProductsJob,
    pub idx_product_edges_product: IdxProductEdgesProduct,
    pub idx_job_edges_job: IdxJobEdgesJob,
    pub idx_job_edges_target: IdxJobEdgesTarget,
    pub idx_job_edges_authored_target: IdxJobEdgesAuthoredTarget,
    pub idx_source_edges_builder_asset_target: IdxSourceEdgesBuilderAssetTarget,
    pub idx_source_edges_target: IdxSourceEdgesTarget,
    pub idx_source_edges_authored_target: IdxSourceEdgesAuthoredTarget,
}

// `SQLiteTable` derives `PartialEq` on each table's generated row type but not `Eq`.
// Every column above is an `Eq` type, so the missing impls are supplied here rather
// than suppressed; the derive itself lives in drizzle-rs and cannot be changed.
impl Eq for SelectRoots {}
impl Eq for SelectWorkspaceRoots {}
impl Eq for SelectAssets {}
impl Eq for SelectJobs {}
impl Eq for SelectAttempts {}
impl Eq for SelectProductEdges {}
impl Eq for PartialSelectRoots {}
impl Eq for PartialSelectWorkspaceRoots {}
impl Eq for PartialSelectAssets {}
impl Eq for PartialSelectJobs {}
impl Eq for PartialSelectAttempts {}
impl Eq for PartialSelectProductEdges {}
