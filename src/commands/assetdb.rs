//! Project-scoped asset-registry maintenance commands.

use std::path::PathBuf;
use std::process::Command;

use az_filesystem::AzothDataHome;
use tracing::instrument;

use crate::error::{CliError, CliResult, CommandFailedDetails};

#[derive(Debug)]
pub enum AssetDbMaintenanceAction {
    Compact {
        finished_attempt_retention_days: i64,
        closed_path_retention_days: i64,
        dry_run: bool,
        vacuum: bool,
        reindex: bool,
    },
    GcViews {
        dry_run: bool,
        apply: bool,
        batch_size: u32,
    },
}

#[derive(Debug)]
struct ProjectAssetDb {
    project_id: String,
    project_root: PathBuf,
    database: PathBuf,
}

#[instrument(skip(path))]
pub fn migrate(path: Option<PathBuf>) -> CliResult<()> {
    let project = resolve_project_asset_db(path)?;
    run_maintenance_tool(
        &project,
        vec![
            "migrate".to_string(),
            "--asset-db".to_string(),
            project.database.to_string_lossy().into_owned(),
        ],
    )
}

#[instrument(skip(path, action), fields(action = ?action))]
pub fn maintain(path: Option<PathBuf>, action: &AssetDbMaintenanceAction) -> CliResult<()> {
    let project = resolve_project_asset_db(path)?;
    let database = project.database.to_string_lossy().into_owned();
    let args = match *action {
        AssetDbMaintenanceAction::Compact {
            finished_attempt_retention_days,
            closed_path_retention_days,
            dry_run,
            vacuum,
            reindex,
        } => {
            let mut args = vec!["compact".to_string(), "--asset-db".to_string(), database];
            args.extend([
                "--finished-attempt-retention-days".to_string(),
                finished_attempt_retention_days.to_string(),
                "--closed-path-retention-days".to_string(),
                closed_path_retention_days.to_string(),
            ]);
            if dry_run {
                args.push("--dry-run".to_string());
            }
            if vacuum {
                args.push("--vacuum".to_string());
            }
            if reindex {
                args.push("--reindex".to_string());
            }
            args
        }
        AssetDbMaintenanceAction::GcViews {
            dry_run,
            apply,
            batch_size,
        } => {
            let mut args = vec!["gc-views".to_string(), "--asset-db".to_string(), database];
            args.extend([
                "--project-id".to_string(),
                project.project_id.clone(),
                "--batch-size".to_string(),
                batch_size.to_string(),
            ]);
            if dry_run {
                args.push("--dry-run".to_string());
            }
            if apply {
                args.push("--apply".to_string());
            }
            args
        }
    };

    run_maintenance_tool(&project, args)
}

fn run_maintenance_tool(project: &ProjectAssetDb, args: Vec<String>) -> CliResult<()> {
    let tool = crate::commands::host_tools::ensure_assetdb_maintenance()?;
    let mut command = Command::new(&tool);
    command.args(&args).current_dir(&project.project_root);
    let owner = az_work::OwnedSynchronousCommandTree::new()?;
    let mut child = owner.spawn(&mut command)?;
    let status = child.wait()?;
    if !status.success() {
        return Err(CliError::CommandFailed(Box::new(CommandFailedDetails {
            program: tool.to_string_lossy().into_owned(),
            args,
            cwd: project.project_root.clone(),
            status: status.code(),
        })));
    }
    Ok(())
}

fn resolve_project_asset_db(path: Option<PathBuf>) -> CliResult<ProjectAssetDb> {
    let requested_root = path.unwrap_or_else(|| PathBuf::from("."));
    let project_root = requested_root.canonicalize()?;
    let manifest = az_project::load_project_manifest(&project_root)?;
    let data_home = AzothDataHome::resolve();
    data_home.prepare()?;
    let paths = data_home.project(&manifest.project.name, &project_root);
    paths.prepare()?;
    Ok(ProjectAssetDb {
        project_id: manifest.project.id,
        project_root,
        database: paths.asset_db_path(),
    })
}
