use std::path::PathBuf;

use az_project::{GeneratedTargetsSyncStatus, ProjectEnginePatchSyncStatus};

use crate::error::CliResult;

pub fn sync(path: Option<PathBuf>) -> CliResult<()> {
    let project_root = path.unwrap_or_else(|| PathBuf::from("."));
    crate::commands::host_tools::ensure_project_host_tools()?;
    let contract =
        az_project_scaffold::project_contract::sync_project_contract_force_engine(&project_root)?;
    let target_status = match contract.generated_targets.status {
        GeneratedTargetsSyncStatus::LegacyLayout => "legacy authored layout",
        GeneratedTargetsSyncStatus::Unchanged => "unchanged",
        GeneratedTargetsSyncStatus::Regenerated => "regenerated",
    };
    println!(
        "Project lock synchronized: {}",
        contract.project_lock_path.display()
    );
    println!("Generated target workspace: {target_status}");
    let report = contract.engine_patch;
    let status = match report.status {
        ProjectEnginePatchSyncStatus::Unchanged => "unchanged after forced recomputation",
        ProjectEnginePatchSyncStatus::Regenerated => "regenerated",
    };

    println!(
        "Engine patch table {status}: {}",
        report.config_path.display()
    );
    println!(
        "Fingerprint: {} -> {}",
        report
            .old_fingerprint
            .as_ref()
            .map_or_else(|| "<missing-or-unstamped>".to_string(), ToString::to_string),
        report.fingerprint
    );
    print_package_changes("Added", &report.added_packages);
    print_package_changes("Removed", &report.removed_packages);
    Ok(())
}

fn print_package_changes(label: &str, packages: &[String]) {
    if packages.is_empty() {
        println!("{label} patches: none");
    } else {
        println!(
            "{label} patches ({}): {}",
            packages.len(),
            packages.join(", ")
        );
    }
}
