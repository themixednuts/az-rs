use crate::error::CliResult;
use az_project::{
    ProjectServiceRole, gem_validation_warnings, load_project_manifest, project_target_roles,
    resolve_project_gems,
};
use std::path::PathBuf;

pub fn execute(path: Option<PathBuf>) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let manifest = load_project_manifest(&project_path)?;
    let resolved_gems = resolve_project_gems(&project_path, &manifest)?;

    println!("Project: {}", manifest.project.name);
    println!("ID: {}", manifest.project.id);
    println!("Version: {}", manifest.project.version);
    println!("Engine version: {}", manifest.project.engine_version);
    println!("Root: {}", project_path.display());
    println!(
        "Assets: {}",
        project_path.join(&manifest.paths.assets).display()
    );
    println!(
        "Scripts: {}",
        project_path.join(&manifest.paths.scripts).display()
    );

    if manifest.gems.is_empty() {
        println!("Gems: none");
    } else {
        println!("Gems:");
        for gem in &manifest.gems {
            let state = if gem.enabled { "enabled" } else { "disabled" };
            if let Some(path) = &gem.path {
                println!("  {} ({state}, {})", gem.id, path.display());
            } else {
                println!("  {} ({state})", gem.id);
            }
        }
    }

    if resolved_gems.is_empty() {
        println!("Resolved gem closure: none");
    } else {
        println!("Resolved gem closure:");
        for gem in &resolved_gems {
            println!(
                "  {} ({}; catalog {} at {})",
                gem.manifest.gem.id,
                gem.provenance.home,
                gem.provenance.catalog_id,
                gem.provenance.catalog_path.display()
            );
            println!("    manifest: {}", gem.provenance.manifest_path.display());
            println!("    checksum: {}", gem.provenance.manifest_checksum);
            if let Some(origin) = &gem.manifest.gem.origin {
                println!(
                    "    origin: publisher {}, repository {}, license {}, upstream {}",
                    origin.publisher.as_deref().unwrap_or("unspecified"),
                    origin.repository.as_deref().unwrap_or("unspecified"),
                    origin.license.as_deref().unwrap_or("unspecified"),
                    origin.upstream.as_deref().unwrap_or("unspecified")
                );
            }
            for contribution in &gem.manifest.contributions {
                println!(
                    "    contribution {} ({}) -> {}",
                    contribution.id,
                    contribution.kind,
                    contribution
                        .roles
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }
    for warning in gem_validation_warnings(&resolved_gems, project_target_roles(&manifest))? {
        println!("Warning: {warning}");
    }

    if manifest.tools.service_targets.is_empty() {
        println!("Services: none");
    } else {
        println!("Services:");
        for service in &manifest.tools.service_targets {
            let settings = service
                .settings
                .as_deref()
                .map(|settings| format!(", settings {settings}"))
                .unwrap_or_default();
            println!(
                "  {} ({}, package {}, bin {}{})",
                service.name,
                service_role_label(service.role),
                service.package,
                service.bin,
                settings
            );
        }
    }

    Ok(())
}

const fn service_role_label(role: ProjectServiceRole) -> &'static str {
    match role {
        ProjectServiceRole::ProjectHost => "project-host",
        ProjectServiceRole::AssetProcessor => "asset-processor",
        ProjectServiceRole::AssetWorker => "asset-worker",
        ProjectServiceRole::RuntimeHost => "runtime-host",
    }
}
