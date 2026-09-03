//! Offline maintenance for the engine-owned asset registry.
//!
//! The corresponding asset-processor service must be stopped. The reset path
//! exports and verifies unsaved payloads, resets the exact Turso file set under
//! the database deed, applies the checked-in Drizzle baseline through normal
//! open, and restores by natural identity.

use std::fmt;
use std::io::{self, IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use az_assetdb::{
    AssetDb, ExpectedPayload, ImportRecoveredPayloadResult, ImportUnsavedPayload, RetentionPolicy,
    SelectWorkspaces, UnsavedPayload, export_unsaved_payloads_for_reset,
    validate_recovery_payloads,
};
use az_source_control::{LoreCli, SourceControlProvider as _};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

const MILLIS_PER_DAY: i64 = 86_400_000;
const RESET_CONFIRMATION: &str = "assetdb-wave-5";
const RECOVERY_FORMAT: &str = "azoth-assetdb-wave5-recovery-v1";
const RECOVERY_ARTIFACT: &str = "payloads.json";
const RECOVERY_MANIFEST: &str = "manifest.json";
const TARGET_TABLES: [&str; 14] = [
    "assets",
    "attempts",
    "builders",
    "entries",
    "job_edges",
    "jobs",
    "paths",
    "payloads",
    "product_edges",
    "products",
    "roots",
    "source_edges",
    "workspace_roots",
    "workspaces",
];

#[derive(Debug, Parser)]
#[command(
    name = "assetdb-maintenance",
    version,
    about = "Maintain an offline Azoth asset registry",
    long_about = "Maintain an offline Azoth asset registry.\n\nThe corresponding asset-processor service must be stopped before this command runs.",
    after_help = "Environment:\n  RUST_LOG  Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR  Disable automatic color output."
)]
struct Cli {
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,
    #[arg(short, long, conflicts_with = "verbose")]
    quiet: bool,
    #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
    color: ColorArg,
    #[command(subcommand)]
    command: MaintenanceCommand,
}

#[derive(Debug, Subcommand)]
enum MaintenanceCommand {
    /// Apply the checked-in asset-registry schema migrations.
    Migrate(MigrateArgs),
    /// Compact bounded operational history and refresh planner statistics.
    Compact(CompactArgs),
    /// Report or reclaim stale workspace projections.
    GcViews(GcViewsArgs),
    /// Replace the database with the checked-in Wave 5 baseline and restore unsaved payloads.
    Reset(ResetArgs),
}

#[derive(Debug, Args)]
struct MigrateArgs {
    #[arg(long, value_name = "FILE")]
    asset_db: PathBuf,
}

#[derive(Debug, Args)]
struct CompactArgs {
    #[arg(long, value_name = "FILE")]
    asset_db: PathBuf,
    #[arg(long, value_name = "DAYS", default_value_t = 30)]
    finished_attempt_retention_days: i64,
    #[arg(long, value_name = "DAYS", default_value_t = 180)]
    closed_path_retention_days: i64,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    vacuum: bool,
    #[arg(long)]
    reindex: bool,
}

#[derive(Debug, Args)]
struct GcViewsArgs {
    #[arg(long, value_name = "FILE")]
    asset_db: PathBuf,
    #[arg(long)]
    project_id: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
}

#[derive(Debug, Args)]
struct ResetArgs {
    #[arg(long, value_name = "FILE")]
    asset_db: PathBuf,
    /// New directory outside the database that receives the recovery artifact.
    #[arg(long, value_name = "DIR")]
    recovery_dir: PathBuf,
    /// Reuse and re-verify an existing recovery artifact after an interrupted reset.
    #[arg(long)]
    resume: bool,
    /// Required destructive-operation acknowledgement (`assetdb-wave-5`).
    #[arg(long)]
    confirm_reset: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum ColorArg {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorArg {
    fn stderr_ansi(self) -> bool {
        match self {
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && io::stderr().is_terminal(),
            Self::Always => true,
            Self::Never => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StaleWorkspaceReason {
    MissingRoot,
    RootIsNotDirectory,
    MissingBranch,
    BranchChanged { current: String },
}

impl fmt::Display for StaleWorkspaceReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRoot => formatter.write_str("workspace root does not exist"),
            Self::RootIsNotDirectory => formatter.write_str("workspace root is not a directory"),
            Self::MissingBranch => formatter.write_str("workspace has no current Lore branch"),
            Self::BranchChanged { current } => {
                write!(formatter, "current Lore branch is `{current}`")
            }
        }
    }
}

struct StaleWorkspace {
    workspace: SelectWorkspaces,
    reason: StaleWorkspaceReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RecoveryArtifact {
    format: String,
    payloads: Vec<UnsavedPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RecoveryManifest {
    format: String,
    payload_count: usize,
    artifact_digest: String,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    install_observability(cli.quiet, cli.verbose, cli.color)?;
    match cli.command {
        MaintenanceCommand::Migrate(args) => migrate(&args),
        MaintenanceCommand::Compact(args) => compact(&args),
        MaintenanceCommand::GcViews(args) => gc_views(&args),
        MaintenanceCommand::Reset(args) => reset(&args),
    }
}

fn default_migration_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("asset-processor-host has an az crate parent")
        .join("assetdb/drizzle")
}

// The offline maintenance handle is used through the end of the command; the printed
// summary reads state that depends on it, so it cannot be released earlier.
#[allow(clippy::significant_drop_tightening)]
fn migrate(args: &MigrateArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!(asset_db = %args.asset_db.display(), "opening asset database for schema migration");
    let db = AssetDb::open_for_offline_maintenance(&args.asset_db, false)?;
    db.optimize()?;
    db.checkpoint()?;
    println!(
        "Asset registry schema migration complete: {}",
        args.asset_db.display()
    );
    Ok(())
}

fn install_observability(
    quiet: bool,
    verbose: u8,
    color: ColorArg,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let level = match (quiet, verbose) {
        (true, _) => tracing_subscriber::filter::LevelFilter::ERROR,
        (false, 0) => tracing_subscriber::filter::LevelFilter::WARN,
        (false, 1) => tracing_subscriber::filter::LevelFilter::INFO,
        (false, 2) => tracing_subscriber::filter::LevelFilter::DEBUG,
        (false, _) => tracing_subscriber::filter::LevelFilter::TRACE,
    };
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(level.into())
        .from_env_lossy();
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(color.stderr_ansi())
        .try_init()?;
    Ok(())
}

// The offline maintenance handle is used through the end of the command; the printed
// summary reads state that depends on it, so it cannot be released earlier.
#[allow(clippy::significant_drop_tightening)]
fn compact(args: &CompactArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let now_unix_ms = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let finished_attempt_cutoff_unix_ms = retention_cutoff(
        now_unix_ms,
        args.finished_attempt_retention_days,
        "finished attempt",
    )?;
    let closed_path_cutoff_unix_ms =
        retention_cutoff(now_unix_ms, args.closed_path_retention_days, "closed path")?;
    if args.dry_run {
        println!("dry-run: asset registry will not be changed");
        println!("database: {}", args.asset_db.display());
        println!("finished-attempt cutoff (Unix ms): {finished_attempt_cutoff_unix_ms}");
        println!("closed-path cutoff (Unix ms): {closed_path_cutoff_unix_ms}");
        return Ok(());
    }

    let db = AssetDb::open_for_offline_maintenance(&args.asset_db, args.vacuum)?;
    let result = db.compact_operational_history(RetentionPolicy {
        finished_attempt_cutoff_unix_ms,
        closed_path_cutoff_unix_ms,
    })?;
    if args.reindex {
        db.reindex()?;
    }
    db.optimize()?;
    db.checkpoint()?;
    if args.vacuum {
        db.vacuum()?;
        db.checkpoint()?;
    }
    println!(
        "Asset registry maintenance complete: {} finished attempts and {} closed path-history rows removed",
        result.deleted_attempts, result.deleted_path_history_rows
    );
    Ok(())
}

// The offline maintenance handle is used through the end of the command; the printed
// summary reads state that depends on it, so it cannot be released earlier.
#[allow(clippy::significant_drop_tightening)]
fn gc_views(args: &GcViewsArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if args.project_id.trim().is_empty() {
        return Err("project id cannot be empty".into());
    }
    let db = AssetDb::open_for_offline_maintenance(&args.asset_db, false)?;
    let stale = stale_workspaces(&db, &args.project_id)?;
    if stale.is_empty() {
        println!(
            "No stale workspaces found for project `{}`",
            args.project_id
        );
        return Ok(());
    }
    println!("Stale workspaces for project `{}`:", args.project_id);
    for stale_workspace in &stale {
        println!(
            "  workspace {}: {} [{}] — {}",
            stale_workspace.workspace.workspace_id,
            stale_workspace.workspace.root,
            stale_workspace.workspace.branch,
            stale_workspace.reason
        );
    }
    if !args.apply {
        if !args.dry_run {
            println!("Dry-run is the default; pass --apply to reclaim these registry rows");
        }
        println!("No workspace rows were changed");
        return Ok(());
    }
    let mut deleted = 0_u64;
    for stale_workspace in stale {
        if db.delete_workspace_for_maintenance(stale_workspace.workspace.workspace_id)? {
            deleted += 1;
        }
    }
    db.optimize()?;
    db.checkpoint()?;
    println!(
        "Workspace GC complete: {deleted} workspaces removed; processed product files were not touched"
    );
    Ok(())
}

// The offline maintenance handle is used through the end of the command; the printed
// summary reads state that depends on it, so it cannot be released earlier.
#[allow(clippy::significant_drop_tightening)]
fn reset(args: &ResetArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if args.confirm_reset != RESET_CONFIRMATION {
        return Err(format!("--confirm-reset must equal `{RESET_CONFIRMATION}`").into());
    }
    let asset_db = std::path::absolute(&args.asset_db)?;
    let recovery_dir = std::path::absolute(&args.recovery_dir)?;
    for artifact in [RECOVERY_ARTIFACT, RECOVERY_MANIFEST] {
        let artifact_path = recovery_dir.join(artifact);
        if asset_db == artifact_path || asset_db == artifact_path.with_extension("tmp") {
            return Err(format!(
                "database path collides with recovery artifact `{artifact}`: {}",
                asset_db.display()
            )
            .into());
        }
    }
    let payloads = if args.resume {
        read_recovery_artifact(&recovery_dir)?
    } else {
        if recovery_dir.exists() {
            return Err(format!(
                "recovery directory already exists; choose a new path or pass --resume: {}",
                recovery_dir.display()
            )
            .into());
        }
        let payloads = export_unsaved_payloads_for_reset(&asset_db)?;
        write_recovery_artifact(&recovery_dir, payloads)?
    };

    // The bytes on disk, not the in-memory export, are the recovery authority
    // before the database is touched.
    let verified = read_recovery_artifact(&recovery_dir)?;
    if verified != payloads {
        return Err("recovery artifact changed between write and reset".into());
    }
    // Normal AssetDB open applies the migration chain compiled into this
    // binary. Validate that exact checked-in chain; accepting a second path
    // here would let the pre-delete proof diverge from the DDL actually used.
    validate_generated_baseline(&default_migration_dir())?;
    println!("reset database: {}", asset_db.display());
    println!("verified recovery artifact: {}", recovery_dir.display());
    let removed = az_turso::reset_local_database(&asset_db)?;
    tracing::info!(files = removed.len(), "removed offline AssetDB file set");

    let db = AssetDb::open_for_offline_maintenance(&asset_db, false)?;
    let writer = db.writer()?;
    for payload in verified.iter().cloned() {
        match writer
            .import_unsaved_payload(ImportUnsavedPayload {
                payload,
                expected: ExpectedPayload::Absent,
            })
            .wait_blocking()?
        {
            ImportRecoveredPayloadResult::Imported(_)
            | ImportRecoveredPayloadResult::AlreadyPresent(_) => {}
            ImportRecoveredPayloadResult::BaselineConflict => {
                return Err("fresh baseline rejected a recovered payload".into());
            }
        }
    }
    let restored = db.export_unsaved_payloads()?;
    if restored != verified {
        return Err("restored payload set does not match the verified recovery artifact".into());
    }
    db.optimize()?;
    db.checkpoint()?;
    drop(writer);
    db.close()?;
    println!(
        "AssetDB Wave 5 reset complete: {} payloads restored from {}",
        verified.len(),
        recovery_dir.display()
    );
    Ok(())
}

fn validate_generated_baseline(
    migration_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut migrations = Vec::new();
    for entry in std::fs::read_dir(migration_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            migrations.push(entry.path());
        }
    }
    if migrations.len() != 1 {
        return Err(format!(
            "Wave 5 reset requires exactly one generated baseline migration, found {}",
            migrations.len()
        )
        .into());
    }
    let sql = std::fs::read_to_string(migrations[0].join("migration.sql"))?;
    let uppercase = sql.to_ascii_uppercase();
    if uppercase.contains("IF NOT EXISTS") || uppercase.contains("IF EXISTS") {
        return Err("generated baseline contains conditional DDL".into());
    }
    if !uppercase.contains("CREATE VIEW") || !uppercase.contains("CATALOG") {
        return Err("generated baseline does not declare the Catalog view".into());
    }
    let snapshot: serde_json::Value =
        serde_json::from_slice(&std::fs::read(migrations[0].join("snapshot.json"))?)?;
    let previous = snapshot
        .get("prevIds")
        .and_then(serde_json::Value::as_array)
        .ok_or("generated baseline snapshot has no prevIds array")?;
    if !previous.is_empty() {
        return Err("generated baseline snapshot still references an older migration".into());
    }
    let mut tables = snapshot
        .get("ddl")
        .and_then(serde_json::Value::as_array)
        .ok_or("generated baseline snapshot has no DDL array")?
        .iter()
        .filter(|entry| {
            entry.get("entityType").and_then(serde_json::Value::as_str) == Some("tables")
        })
        .filter_map(|entry| entry.get("name").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    tables.sort_unstable();
    if tables != TARGET_TABLES {
        return Err(format!("generated baseline tables do not match Wave 5: {tables:?}").into());
    }
    Ok(())
}

fn write_recovery_artifact(
    recovery_dir: &Path,
    payloads: Vec<UnsavedPayload>,
) -> Result<Vec<UnsavedPayload>, Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir(recovery_dir)?;
    let artifact = RecoveryArtifact {
        format: RECOVERY_FORMAT.to_owned(),
        payloads,
    };
    let bytes = serde_json::to_vec_pretty(&artifact)?;
    let manifest = RecoveryManifest {
        format: RECOVERY_FORMAT.to_owned(),
        payload_count: artifact.payloads.len(),
        artifact_digest: blake3::hash(&bytes).to_hex().to_string(),
    };
    write_atomic(&recovery_dir.join(RECOVERY_ARTIFACT), &bytes)?;
    write_atomic(
        &recovery_dir.join(RECOVERY_MANIFEST),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(artifact.payloads)
}

fn read_recovery_artifact(
    recovery_dir: &Path,
) -> Result<Vec<UnsavedPayload>, Box<dyn std::error::Error + Send + Sync>> {
    let bytes = std::fs::read(recovery_dir.join(RECOVERY_ARTIFACT))?;
    let manifest: RecoveryManifest =
        serde_json::from_slice(&std::fs::read(recovery_dir.join(RECOVERY_MANIFEST))?)?;
    if manifest.format != RECOVERY_FORMAT
        || manifest.artifact_digest != blake3::hash(&bytes).to_hex().to_string()
    {
        return Err("recovery manifest does not match the payload artifact".into());
    }
    let artifact: RecoveryArtifact = serde_json::from_slice(&bytes)?;
    if artifact.format != RECOVERY_FORMAT || artifact.payloads.len() != manifest.payload_count {
        return Err("recovery artifact format or payload count does not match its manifest".into());
    }
    validate_recovery_payloads(&artifact.payloads)?;
    Ok(artifact.payloads)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let temporary = path.with_extension("tmp");
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn stale_workspaces(
    db: &AssetDb,
    project_id: &str,
) -> Result<Vec<StaleWorkspace>, Box<dyn std::error::Error + Send + Sync>> {
    let mut stale = Vec::new();
    for workspace in db
        .workspaces_for_maintenance()?
        .into_iter()
        .filter(|workspace| workspace.project == project_id)
    {
        if let Some(reason) = stale_workspace_reason(&workspace)? {
            stale.push(StaleWorkspace { workspace, reason });
        }
    }
    Ok(stale)
}

fn stale_workspace_reason(
    workspace: &SelectWorkspaces,
) -> Result<Option<StaleWorkspaceReason>, Box<dyn std::error::Error + Send + Sync>> {
    let root = Path::new(&workspace.root);
    let metadata = match std::fs::metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Some(StaleWorkspaceReason::MissingRoot));
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() {
        return Ok(Some(StaleWorkspaceReason::RootIsNotDirectory));
    }
    match LoreCli.current_branch(root)? {
        Some(current) if current == workspace.branch => Ok(None),
        Some(current) => Ok(Some(StaleWorkspaceReason::BranchChanged { current })),
        None => Ok(Some(StaleWorkspaceReason::MissingBranch)),
    }
}

fn retention_cutoff(
    now_unix_ms: i64,
    retention_days: i64,
    label: &str,
) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
    if retention_days < 0 {
        return Err(format!("{label} retention days must be non-negative").into());
    }
    let retention_ms = retention_days
        .checked_mul(MILLIS_PER_DAY)
        .ok_or_else(|| format!("{label} retention interval overflow"))?;
    now_unix_ms
        .checked_sub(retention_ms)
        .ok_or_else(|| format!("{label} retention cutoff overflow").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_cutoff_is_checked() {
        assert_eq!(retention_cutoff(1_000, 0, "test").unwrap(), 1_000);
        assert!(retention_cutoff(1_000, -1, "test").is_err());
        assert!(retention_cutoff(i64::MAX, i64::MAX, "test").is_err());
    }

    #[test]
    fn missing_workspace_root_is_stale_without_calling_lore() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = SelectWorkspaces {
            workspace_id: 7,
            project: "local.test".to_string(),
            root: temp.path().join("missing").to_string_lossy().into_owned(),
            branch: "main".to_string(),
            builders: None,
            created: 1,
            updated: 1,
        };
        assert_eq!(
            stale_workspace_reason(&workspace).unwrap(),
            Some(StaleWorkspaceReason::MissingRoot)
        );
    }

    #[test]
    fn recovery_artifact_is_verified_from_disk() {
        let temp = tempfile::tempdir().unwrap();
        let recovery = temp.path().join("recovery");
        let written = write_recovery_artifact(&recovery, Vec::new()).unwrap();
        assert!(written.is_empty());
        assert_eq!(read_recovery_artifact(&recovery).unwrap(), written);
        std::fs::write(recovery.join(RECOVERY_ARTIFACT), b"{}").unwrap();
        assert!(read_recovery_artifact(&recovery).is_err());
    }
}
