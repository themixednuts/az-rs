#![allow(
    dead_code,
    reason = "this support module is compiled independently by the commit and wal bench targets"
)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use az_assetdb::{
    Aliases, ApplySweepDelta, AssetDb, AssetDbWriter, BuilderDescriptor, ClaimReadyJob,
    ClaimReadyJobResult, CompleteAttempt, CompleteAttemptResult, Diff, Digest, Exclusions,
    PlanDelta, PlannedJob, ProductInput, RegisterWorkspace, RegisterWorkspaceRoot, Registration,
    ReplaceBuilderCatalog, SelectWorkspaceRoots, SelectWorkspaces, Status, SweepEntry,
    SweepPlannerJob, SweepRecord, WorkspaceKey,
};
use tempfile::TempDir;
use uuid::Uuid;

const SMALL_PAYLOAD_BYTES: &[usize] = &[128, 4 * 1024, 64 * 1024];
const FULL_PAYLOAD_BYTES: &[usize] = &[128, 4 * 1024, 64 * 1024, 1024 * 1024];

pub fn payload_bytes() -> &'static [usize] {
    match std::env::var("AZ_ASSETDB_BENCH_SCALE").as_deref() {
        Ok("full") => FULL_PAYLOAD_BYTES,
        Ok("small") | Err(_) => SMALL_PAYLOAD_BYTES,
        Ok(other) => {
            panic!("AZ_ASSETDB_BENCH_SCALE must be `small` or `full`, got `{other}`")
        }
    }
}

// The `_bytes` suffix is load-bearing here: these are three byte counts and the
// Display impl below prints them side by side, so dropping it would make the
// fields read as unrelated quantities.
#[allow(clippy::struct_field_names)]
#[derive(Debug)]
pub struct JournalSample {
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub database_bytes: u64,
}

impl std::fmt::Display for JournalSample {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "journal_before={} journal_after={} journal_growth={} database_bytes={}",
            self.before_bytes,
            self.after_bytes,
            self.after_bytes.saturating_sub(self.before_bytes),
            self.database_bytes,
        )
    }
}

pub struct PreparedCompletion {
    // Drop order is significant: the writer joins before the DB owner and
    // deed, and both release before the temporary fixture directory.
    group: AssetDbWriter,
    _db: AssetDb,
    _temp: TempDir,
    db_path: PathBuf,
    attempt_id: i64,
    asset_pk: i64,
    payload_bytes: usize,
}

impl PreparedCompletion {
    /// Registers the workspace, source root and seeded source Entry this bench
    /// fixture measures against.
    fn seed_workspace(
        _db: &AssetDb,
        group: &AssetDbWriter,
    ) -> (SelectWorkspaces, SelectWorkspaceRoots) {
        let workspace = group
            .register_workspace(RegisterWorkspace {
                key: WorkspaceKey {
                    project: "bench.project".to_string(),
                    root: "/bench".to_string(),
                    branch: "az/bench".to_string(),
                },
                now: 1,
            })
            .wait_blocking()
            .expect("seed workspace");
        let (_, root) = group
            .register_workspace_root(RegisterWorkspaceRoot {
                workspace_pk: workspace.workspace_id,
                key: "bench:assets".to_string(),
                owner: "bench.project".to_string(),
                path: "/bench/assets".to_string(),
                exclusions: Exclusions::default(),
            })
            .wait_blocking()
            .expect("seed source root");
        group
            .apply_sweep_delta(ApplySweepDelta {
                workspace_pk: workspace.workspace_id,
                workspace_root_pk: root.workspace_root_id,
                records: vec![SweepRecord {
                    source: SweepEntry {
                        path: "generated.fixture".to_string(),
                        guid: Uuid::from_bytes([0x01; 16]),
                        schema: Some("az.bench.Generated".to_string()),
                        digest: Digest::from(blake3::hash(b"bench-hash")),
                        diff: Diff::Added,
                        diagnostics: 0,
                        updated: 2,
                        src_bytes: 0,
                        src_mtime: 0,
                        meta_bytes: 0,
                        meta_mtime: 0,
                        observed: 2,
                        session: None,
                    },
                    planner: SweepPlannerJob {
                        key: "create-jobs".to_string(),
                        platform: "pc".to_string(),
                    },
                }],
                removals: Vec::new(),
            })
            .wait_blocking()
            .expect("seed source Entry");
        (workspace, root)
    }

    pub fn new(payload_bytes: usize) -> Self {
        let temp = TempDir::new().expect("create generated AssetDb bench fixture");
        let db_path = temp.path().join("assetdb.sqlite");
        let db = AssetDb::open(&db_path).expect("open shipping AssetDb");
        let group = db.writer().expect("start shipping AssetDbWriter");
        let (workspace, root) = Self::seed_workspace(&db, &group);
        let asset_pk = db
            .source_asset(workspace.workspace_id, root.root_pk, "generated.fixture")
            .expect("read seeded source")
            .expect("seeded source exists")
            .0
            .asset_id;
        let builder_guid = Uuid::from_bytes([0x02; 16]);
        let catalog_digest = Digest::from(blake3::hash(b"bench-catalog"));
        group
            .replace_builder_catalog(ReplaceBuilderCatalog {
                workspace_pk: workspace.workspace_id,
                expected: None,
                replacement: catalog_digest,
                builders: vec![BuilderDescriptor {
                    guid: builder_guid,
                    name: "bench.generated".to_string(),
                    version: 1,
                    digest: catalog_digest,
                }],
                plan_delta: PlanDelta {
                    replacements: vec![PlannedJob::build(
                        asset_pk,
                        builder_guid,
                        "generated",
                        "pc",
                        Vec::new(),
                    )],
                    ..PlanDelta::default()
                },
                updated: 3,
            })
            .wait_blocking()
            .expect("register builder and Build Job");
        let job = db
            .jobs_for_asset(workspace.workspace_id, asset_pk)
            .expect("read seeded Jobs")
            .into_iter()
            .find(|job| job.builder == Some(builder_guid))
            .expect("seeded Build Job");
        let claimed = group
            .claim_ready_job(ClaimReadyJob {
                job_id: job.job_id,
                expected_attempts: 0,
                owner: "bench-worker".to_string(),
                lease_duration_ms: i64::MAX as u64,
                staging: "generated.staged".to_string(),
            })
            .wait_blocking()
            .expect("claim shipping Job");
        let ClaimReadyJobResult::Claimed { context } = claimed else {
            panic!("seeded Build Job was no longer claimable")
        };

        Self {
            group,
            _db: db,
            _temp: temp,
            db_path,
            attempt_id: context.attempt.attempt_id,
            asset_pk,
            payload_bytes,
        }
    }

    pub fn commit(&self) {
        assert!(
            self.group
                .complete_attempt(CompleteAttempt {
                    attempt_id: self.attempt_id,
                    owner: "bench-worker".to_string(),
                    status: Status::Succeeded,
                    finished: 4,
                    errors: 0,
                    warnings: 0,
                    products: vec![ProductInput {
                        asset_pk: self.asset_pk,
                        platform: "pc".to_string(),
                        sub_id: 0,
                        path: "generated.product".to_string(),
                        kind: Uuid::from_bytes([0x03; 16]),
                        format: "generated".to_string(),
                        version: 1,
                        aliases: Aliases::new(vec!["x".repeat(self.payload_bytes)]),
                        registration: Registration::AssetIdOnly,
                        digest: Digest::from(blake3::hash(b"bench-hash")),
                        bytes: i64::try_from(self.payload_bytes)
                            .expect("benchmark payload fits i64"),
                        edges: Vec::new(),
                    }],
                    job_edges: None,
                    plan_delta: None,
                })
                .wait_blocking()
                .is_ok_and(|result| matches!(result, CompleteAttemptResult::Completed { .. }))
        );
    }

    pub fn commit_and_sample_journal(&self) -> JournalSample {
        let before_bytes = journal_bytes(&self.db_path);
        self.commit();
        JournalSample {
            before_bytes,
            after_bytes: journal_bytes(&self.db_path),
            database_bytes: fs::metadata(&self.db_path).map_or(0, |metadata| metadata.len()),
        }
    }
}

fn journal_bytes(db_path: &Path) -> u64 {
    let mvcc_log = db_path.with_extension("db-log");
    let mut wal = db_path.as_os_str().to_os_string();
    wal.push("-wal");
    [mvcc_log, PathBuf::from(wal)]
        .into_iter()
        .map(|path| fs::metadata(path).map_or(0, |metadata| metadata.len()))
        .sum()
}
