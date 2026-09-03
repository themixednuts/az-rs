use std::path::PathBuf;

use az_project::{
    GeneratedTargetsFreshness, GeneratedTargetsSyncStatus, project_generated_targets_status,
    regenerate_project_generated_targets,
};

use crate::error::CliResult;

pub fn list(path: Option<PathBuf>) -> CliResult<()> {
    let project_root = path.unwrap_or_else(|| PathBuf::from("."));
    let report = project_generated_targets_status(&project_root)?;
    if report.freshness == GeneratedTargetsFreshness::LegacyLayout {
        println!("Legacy crates/game layout: generated targets are inactive until migration.");
        return Ok(());
    }

    println!(
        "Generated targets: {}",
        report.workspace_root.as_ref().map_or_else(
            || "<missing>".to_string(),
            |root| root.display().to_string()
        )
    );
    println!("Freshness: {}", freshness_name(report.freshness));
    println!(
        "Fingerprint: {} -> {}",
        report.stored_fingerprint.as_deref().unwrap_or("<missing>"),
        report.expected_fingerprint.as_deref().unwrap_or("<none>")
    );
    print_targets(&report.targets);
    Ok(())
}

pub fn regenerate(path: Option<PathBuf>) -> CliResult<()> {
    let project_root = path.unwrap_or_else(|| PathBuf::from("."));
    let report = regenerate_project_generated_targets(&project_root)?;
    if report.status == GeneratedTargetsSyncStatus::LegacyLayout {
        println!("Legacy crates/game layout: no generated targets were written.");
        return Ok(());
    }

    println!(
        "Generated target workspace regenerated: {}",
        report
            .workspace_root
            .as_ref()
            .expect("non-legacy generated target report has a workspace root")
            .display()
    );
    println!(
        "Fingerprint: {} -> {}",
        report.old_fingerprint.as_deref().unwrap_or("<missing>"),
        report.fingerprint.as_deref().unwrap_or("<missing>")
    );
    print_targets(&report.targets);
    Ok(())
}

fn print_targets(targets: &[az_project::GeneratedTargetPackage]) {
    for target in targets {
        println!(
            "  {} [{}] -> {}",
            target.name,
            target
                .roles
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("+"),
            if target.linked_packages.is_empty() {
                "<no gem packages>".to_string()
            } else {
                target.linked_packages.join(", ")
            }
        );
    }
}

const fn freshness_name(freshness: GeneratedTargetsFreshness) -> &'static str {
    match freshness {
        GeneratedTargetsFreshness::LegacyLayout => "legacy-layout",
        GeneratedTargetsFreshness::Missing => "missing",
        GeneratedTargetsFreshness::Stale => "stale",
        GeneratedTargetsFreshness::Fresh => "fresh",
    }
}
