use crate::error::{CliError, CliResult};
use az_project::{
    LockedPackage, ProjectBuildTargetKind, ProjectBuildTargetRole, ProjectLock, ProjectManifest,
    ProjectPackageCompression, ProjectPackageContainer, ProjectPackageOodleCompressor,
    ProjectPackageOodleEffort, ProjectServiceRole, load_project_manifest,
    load_resolved_project_graph, project_lock_path, project_manifest_path, refresh_project_lock,
};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, value};
use tracing::info;

const SCALAR_KEYS: &[&str] = &[
    "project.id",
    "project.name",
    "project.version",
    "project.engine_version",
    "source_control.lore.remote_url",
    "paths.assets",
    "paths.scripts",
];

/// # Errors
///
/// Returns [`CliError::ProjectManifest`] when the manifest under `path` cannot be
/// loaded, and [`CliError::UnsupportedConfigKey`] when `key` is not one of the
/// readable manifest keys.
pub fn get(key: &str, path: Option<PathBuf>) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!("Getting config: {} from {}", key, project_path.display());

    let manifest = load_project_manifest(&project_path)?;
    let value = config_value(&manifest, key)?;
    println!("{key} = {value}");

    Ok(())
}

/// # Errors
///
/// Returns any error [`set_config_value`] returns: [`CliError::UnsupportedConfigKey`]
/// for a key outside the writable set, [`CliError::ConfigParse`] when the manifest is
/// not valid TOML, [`CliError::ProjectManifest`] when the rewritten manifest fails
/// validation, and [`CliError::Io`] when the manifest cannot be read or written.
pub fn set(key: &str, new_value: &str, path: Option<PathBuf>) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!(
        "Setting config: {} = {} in {}",
        key,
        new_value,
        project_path.display()
    );

    set_config_value(&project_path, key, new_value)?;
    println!("{key} = {}", display_value(new_value));

    Ok(())
}

pub fn list(path: Option<PathBuf>) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!("Listing config for {}", project_path.display());

    let manifest = load_project_manifest(&project_path)?;
    for line in manifest_config_lines(&manifest)? {
        println!("{line}");
    }

    Ok(())
}

fn manifest_config_lines(manifest: &ProjectManifest) -> CliResult<Vec<String>> {
    let mut lines = Vec::new();
    for key in SCALAR_KEYS {
        lines.push(format!("{key} = {}", config_value(manifest, key)?));
    }
    push_packaging_profile_lines(manifest, &mut lines);
    push_gem_lines(manifest, &mut lines);
    push_build_target_lines(manifest, &mut lines);
    push_service_target_lines(manifest, &mut lines);
    Ok(lines)
}

fn push_packaging_profile_lines(manifest: &ProjectManifest, lines: &mut Vec<String>) {
    if manifest.packaging.profiles.is_empty() {
        lines.push("packaging.profiles = none".to_string());
        return;
    }
    for (index, profile) in manifest.packaging.profiles.iter().enumerate() {
        lines.push(format!(
            "packaging.profiles[{index}].name = {}",
            profile.name
        ));
        lines.push(format!(
            "packaging.profiles[{index}].asset_platform = {}",
            profile.asset_platform
        ));
        lines.push(format!(
            "packaging.profiles[{index}].cargo_profile = {}",
            profile.cargo_profile
        ));
        lines.push(format!(
            "packaging.profiles[{index}].container = {}",
            package_container_label(profile.container)
        ));
        lines.push(format!(
            "packaging.profiles[{index}].compression = {}",
            package_compression_label(profile.compression)
        ));
        if let Some(oodle) = &profile.oodle {
            lines.push(format!(
                "packaging.profiles[{index}].oodle.compressor = {}",
                package_oodle_compressor_label(oodle.compressor)
            ));
            lines.push(format!(
                "packaging.profiles[{index}].oodle.effort = {}",
                package_oodle_effort_label(oodle.effort)
            ));
        }
    }
}

fn push_gem_lines(manifest: &ProjectManifest, lines: &mut Vec<String>) {
    if manifest.gems.is_empty() {
        lines.push("gems = none".to_string());
        return;
    }
    for (index, gem) in manifest.gems.iter().enumerate() {
        lines.push(format!("gems[{index}].id = {}", gem.id));
        lines.push(format!("gems[{index}].enabled = {}", gem.enabled));
        if let Some(path) = &gem.path {
            lines.push(format!("gems[{index}].path = {}", path.display()));
        }
    }
}

fn push_build_target_lines(manifest: &ProjectManifest, lines: &mut Vec<String>) {
    if manifest.tools.build_targets.is_empty() {
        lines.push("tools.build_targets = none".to_string());
        return;
    }
    for (index, target) in manifest.tools.build_targets.iter().enumerate() {
        lines.push(format!(
            "tools.build_targets[{index}].name = {}",
            target.name
        ));
        if let Some(package) = &target.package {
            lines.push(format!("tools.build_targets[{index}].package = {package}"));
        }
        lines.push(format!(
            "tools.build_targets[{index}].kind = {}",
            build_target_kind_label(target.kind)
        ));
        lines.push(format!(
            "tools.build_targets[{index}].role = {}",
            build_target_role_label(target.role)
        ));
        if let Some(settings) = &target.settings {
            lines.push(format!(
                "tools.build_targets[{index}].settings = {settings}"
            ));
        }
        lines.push(format!(
            "tools.build_targets[{index}].default = {}",
            target.default
        ));
        if !target.features.is_empty() {
            lines.push(format!(
                "tools.build_targets[{index}].features = {}",
                target.features.join(",")
            ));
        }
    }
}

fn push_service_target_lines(manifest: &ProjectManifest, lines: &mut Vec<String>) {
    if manifest.tools.service_targets.is_empty() {
        lines.push("tools.service_targets = none".to_string());
        return;
    }
    for (index, target) in manifest.tools.service_targets.iter().enumerate() {
        lines.push(format!(
            "tools.service_targets[{index}].name = {}",
            target.name
        ));
        lines.push(format!(
            "tools.service_targets[{index}].role = {}",
            service_role_label(target.role)
        ));
        if let Some(settings) = &target.settings {
            lines.push(format!(
                "tools.service_targets[{index}].settings = {settings}"
            ));
        }
        lines.push(format!(
            "tools.service_targets[{index}].package = {}",
            target.package
        ));
        lines.push(format!(
            "tools.service_targets[{index}].bin = {}",
            target.bin
        ));
        lines.push(format!(
            "tools.service_targets[{index}].default = {}",
            target.default
        ));
        if !target.args.is_empty() {
            lines.push(format!(
                "tools.service_targets[{index}].args = {}",
                target.args.join(",")
            ));
        }
    }
}

pub fn lock(path: Option<PathBuf>, check: bool) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let lock_path = project_lock_path(&project_path);

    if check {
        let graph = load_resolved_project_graph(&project_path)?;
        println!("Lock is current: {}", lock_path.display());
        println!("Project: {}", graph.lock.project.id);
        println!("Packages: {}", graph.lock.packages.len());
        println!("Source roots: {}", graph.lock.source_roots.len());
        println!("Build targets: {}", graph.lock.tools.build_targets.len());
        println!(
            "Service targets: {}",
            graph.lock.tools.service_targets.len()
        );
        print_gem_deprecation_warnings(&graph.lock);
    } else {
        az_project_scaffold::gem::sync_enabled_gem_dependencies(Some(project_path.clone()))?;
        let lock = refresh_project_lock(&project_path)?;
        println!("Lock refreshed: {}", lock_path.display());
        println!("Project: {}", lock.project.id);
        println!("Packages: {}", lock.packages.len());
        println!("Source roots: {}", lock.source_roots.len());
        println!("Build targets: {}", lock.tools.build_targets.len());
        println!("Service targets: {}", lock.tools.service_targets.len());
        print_gem_deprecation_warnings(&lock);
    }

    Ok(())
}

fn print_gem_deprecation_warnings(lock: &ProjectLock) {
    for warning in lock.packages.iter().filter_map(gem_deprecation_warning) {
        eprintln!("{warning}");
    }
}

fn gem_deprecation_warning(package: &LockedPackage) -> Option<String> {
    let deprecation = package.deprecation.as_ref()?;
    let since = deprecation
        .since
        .as_deref()
        .map(|version| format!(" since v{version}"))
        .unwrap_or_default();
    let replacement = deprecation
        .replacement
        .as_ref()
        .map(|replacement| {
            let version = replacement
                .version
                .as_deref()
                .map(|requirement| format!(" {requirement}"))
                .unwrap_or_default();
            format!("; use `{}`{version}", replacement.id)
        })
        .unwrap_or_default();
    Some(format!(
        "warning: gem `{}` ({}) is deprecated{since}: {}{replacement}",
        package.id, package.name, deprecation.message
    ))
}

fn config_value(manifest: &ProjectManifest, key: &str) -> CliResult<String> {
    match key {
        "project.id" => Ok(manifest.project.id.clone()),
        "project.name" => Ok(manifest.project.name.clone()),
        "project.version" => Ok(manifest.project.version.clone()),
        "project.engine_version" => Ok(manifest.project.engine_version.clone()),
        "source_control.lore.remote_url" => Ok(manifest
            .source_control
            .lore
            .as_ref()
            .map_or_else(|| "<unset>".to_string(), |lore| lore.remote_url.clone())),
        "paths.assets" => Ok(manifest.paths.assets.display().to_string()),
        "paths.scripts" => Ok(manifest.paths.scripts.display().to_string()),
        _ => Err(CliError::UnsupportedConfigKey(key.to_string())),
    }
}

fn set_config_value(project_path: &Path, key: &str, new_value: &str) -> CliResult<()> {
    if !SCALAR_KEYS.contains(&key) {
        return Err(CliError::UnsupportedConfigKey(key.to_string()));
    }

    let path = project_manifest_path(project_path);
    let manifest_str = std::fs::read_to_string(&path)?;
    let mut document =
        manifest_str
            .parse::<DocumentMut>()
            .map_err(|source| CliError::ConfigParse {
                path: path.clone(),
                message: source.to_string(),
            })?;

    match key {
        "project.id" => set_required_string(&mut document, "project", "id", new_value),
        "project.name" => set_required_string(&mut document, "project", "name", new_value),
        "project.version" => set_required_string(&mut document, "project", "version", new_value),
        "project.engine_version" => {
            set_required_string(&mut document, "project", "engine_version", new_value);
        }
        "source_control.lore.remote_url" => set_lore_remote_url(&mut document, new_value),
        "paths.assets" => set_required_string(&mut document, "paths", "assets", new_value),
        "paths.scripts" => set_required_string(&mut document, "paths", "scripts", new_value),
        _ => return Err(CliError::UnsupportedConfigKey(key.to_string())),
    }

    let serialized = document.to_string();
    let parsed: ProjectManifest =
        toml::from_str(&serialized).map_err(|source| CliError::ConfigParse {
            path: path.clone(),
            message: source.to_string(),
        })?;
    parsed.validate()?;

    std::fs::write(path, serialized)?;
    refresh_project_lock(project_path)?;
    if key == "source_control.lore.remote_url" && !is_unset_value(new_value) {
        az_source_control::reconcile_lore_remote_url(project_path, new_value)?;
    }
    Ok(())
}

fn set_required_string(document: &mut DocumentMut, table: &str, field: &str, new_value: &str) {
    document[table][field] = value(new_value);
}

fn set_lore_remote_url(document: &mut DocumentMut, new_value: &str) {
    if is_unset_value(new_value) {
        if let Some(source_control) = document["source_control"].as_table_like_mut() {
            source_control.remove("lore");
            if source_control.is_empty() {
                document.as_table_mut().remove("source_control");
            }
        }
    } else {
        document["source_control"]["lore"]["remote_url"] = value(new_value);
    }
}

fn is_unset_value(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("null")
}

fn display_value(value: &str) -> String {
    if is_unset_value(value) {
        "<unset>".to_string()
    } else {
        value.to_string()
    }
}

const fn build_target_kind_label(kind: ProjectBuildTargetKind) -> &'static str {
    match kind {
        ProjectBuildTargetKind::Package => "package",
        ProjectBuildTargetKind::Bin => "bin",
        ProjectBuildTargetKind::Example => "example",
    }
}

const fn build_target_role_label(role: ProjectBuildTargetRole) -> &'static str {
    match role {
        ProjectBuildTargetRole::Generic => "generic",
        ProjectBuildTargetRole::ProjectServices => "project-services",
        ProjectBuildTargetRole::Client => "client",
        ProjectBuildTargetRole::Server => "server",
        ProjectBuildTargetRole::SupportService => "support-service",
        ProjectBuildTargetRole::Tool => "tool",
    }
}

const fn service_role_label(role: ProjectServiceRole) -> &'static str {
    match role {
        ProjectServiceRole::ProjectHost => "project-host",
        ProjectServiceRole::AssetProcessor => "asset-processor",
        ProjectServiceRole::AssetWorker => "asset-worker",
        ProjectServiceRole::RuntimeHost => "runtime-host",
    }
}

const fn package_container_label(container: ProjectPackageContainer) -> &'static str {
    match container {
        ProjectPackageContainer::Loose => "loose",
        ProjectPackageContainer::AzPack => "azpack",
        ProjectPackageContainer::Pak => "pak",
    }
}

const fn package_compression_label(compression: ProjectPackageCompression) -> &'static str {
    match compression {
        ProjectPackageCompression::None => "none",
        ProjectPackageCompression::Oodle => "oodle",
    }
}

const fn package_oodle_compressor_label(compressor: ProjectPackageOodleCompressor) -> &'static str {
    match compressor {
        ProjectPackageOodleCompressor::Kraken => "kraken",
        ProjectPackageOodleCompressor::Mermaid => "mermaid",
        ProjectPackageOodleCompressor::Selkie => "selkie",
        ProjectPackageOodleCompressor::Leviathan => "leviathan",
        ProjectPackageOodleCompressor::Hydra => "hydra",
    }
}

const fn package_oodle_effort_label(effort: ProjectPackageOodleEffort) -> &'static str {
    match effort {
        ProjectPackageOodleEffort::SuperFast => "super-fast",
        ProjectPackageOodleEffort::VeryFast => "very-fast",
        ProjectPackageOodleEffort::Fast => "fast",
        ProjectPackageOodleEffort::Normal => "normal",
        ProjectPackageOodleEffort::Optimal1 => "optimal1",
        ProjectPackageOodleEffort::Optimal2 => "optimal2",
        ProjectPackageOodleEffort::Optimal3 => "optimal3",
        ProjectPackageOodleEffort::Optimal4 => "optimal4",
        ProjectPackageOodleEffort::Optimal5 => "optimal5",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_project::{
        GemDeprecation, GemReplacement, LockedPackage, LockedPackageKind, ProjectBuildTarget,
        ProjectManifest, load_project_lock, write_project_manifest,
    };

    #[test]
    fn config_value_reads_project_manifest_scalars() {
        let manifest = ProjectManifest::new("local.example", "Example", "0.1.0");

        assert_eq!(
            config_value(&manifest, "project.id").unwrap(),
            "local.example"
        );
        assert_eq!(config_value(&manifest, "project.name").unwrap(), "Example");
        assert!(matches!(
            config_value(&manifest, "paths.cache"),
            Err(CliError::UnsupportedConfigKey(_))
        ));
        assert!(matches!(
            config_value(&manifest, "tools.build_targets[0].name"),
            Err(CliError::UnsupportedConfigKey(_))
        ));
    }

    #[test]
    fn config_list_includes_package_profiles() {
        let manifest = ProjectManifest::new("local.example", "Example", "0.1.0");

        let lines = manifest_config_lines(&manifest).unwrap();

        assert!(
            lines
                .iter()
                .any(|line| line == "packaging.profiles[0].name = pc-dev")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "packaging.profiles[0].container = loose")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "packaging.profiles[0].compression = none")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "packaging.profiles[1].name = pc-release")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "packaging.profiles[1].container = azpack")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "packaging.profiles[1].compression = oodle")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "packaging.profiles[1].oodle.compressor = kraken")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "packaging.profiles[1].oodle.effort = normal")
        );
    }

    #[test]
    fn set_config_value_preserves_manifest_comments() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path();
        let mut manifest = ProjectManifest::new("local.example", "Example", "0.1.0");
        manifest
            .tools
            .build_targets
            .push(ProjectBuildTarget::package("game", "example"));
        write_project_manifest(path, &manifest).unwrap();
        refresh_project_lock(path).unwrap();

        let manifest_path = project_manifest_path(path);
        let original = std::fs::read_to_string(&manifest_path).unwrap();
        std::fs::write(
            &manifest_path,
            format!("# hand-authored comment\n{original}"),
        )
        .unwrap();

        set_config_value(path, "project.name", "Renamed").unwrap();

        let written = std::fs::read_to_string(&manifest_path).unwrap();
        assert!(written.contains("# hand-authored comment"));

        let loaded = load_project_manifest(path).unwrap();
        assert_eq!(loaded.project.name, "Renamed");
        let lock = load_project_lock(path).unwrap();
        assert_eq!(lock.project.name, "Renamed");
    }

    #[test]
    fn set_lore_remote_updates_portable_and_instance_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path();
        write_project_manifest(
            path,
            &ProjectManifest::new("local.example", "Example", "0.1.0"),
        )
        .unwrap();
        refresh_project_lock(path).unwrap();
        std::fs::create_dir(path.join(".lore")).unwrap();
        std::fs::write(
            path.join(".lore/config.toml"),
            "remote_url = \"lore://127.0.0.1:41337\"\nidentity = \"developer@example.com\"\n",
        )
        .unwrap();

        set_config_value(
            path,
            "source_control.lore.remote_url",
            "lore://192.0.2.10:41337",
        )
        .unwrap();

        let manifest = load_project_manifest(path).unwrap();
        assert_eq!(
            manifest
                .source_control
                .lore
                .as_ref()
                .map(|lore| lore.remote_url.as_str()),
            Some("lore://192.0.2.10:41337")
        );
        let lore_config = std::fs::read_to_string(path.join(".lore/config.toml")).unwrap();
        assert!(lore_config.contains("remote_url = \"lore://192.0.2.10:41337\""));
        assert!(lore_config.contains("identity = \"developer@example.com\""));
    }

    #[test]
    fn set_config_value_rejects_non_portable_paths_without_mutating_project_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path();
        let manifest = ProjectManifest::new("local.example", "Example", "0.1.0");
        write_project_manifest(path, &manifest).unwrap();
        refresh_project_lock(path).unwrap();

        let manifest_path = project_manifest_path(path);
        let lock_path = project_lock_path(path);
        let original_manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let original_lock = std::fs::read_to_string(&lock_path).unwrap();

        let error = set_config_value(path, "paths.assets", "../shared-assets")
            .expect_err("escaping manifest path must be rejected");

        assert!(matches!(
            error,
            CliError::ProjectManifest(manifest_error)
                if matches!(
                    *manifest_error,
                    az_project::ProjectManifestError::InvalidManifestPath {
                        field: "project assets",
                        ..
                    }
                )
        ));
        assert_eq!(
            std::fs::read_to_string(&manifest_path).unwrap(),
            original_manifest
        );
        assert_eq!(std::fs::read_to_string(&lock_path).unwrap(), original_lock);
    }

    #[test]
    fn lock_command_refreshes_and_checks_project_lock() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path();
        az_project_scaffold::new::execute("lock-cli".to_string(), Some(path.to_path_buf()), None)
            .unwrap();

        lock(Some(path.to_path_buf()), false).unwrap();
        lock(Some(path.to_path_buf()), true).unwrap();

        let loaded = load_project_lock(path).unwrap();
        assert_eq!(loaded.project.id, "lock_cli");
    }

    #[test]
    fn deprecated_gem_warning_names_replacement() {
        let package = LockedPackage {
            id: "azoth.old_render".to_string(),
            kind: LockedPackageKind::EngineGem,
            name: "Old Render".to_string(),
            version: "0.4.0".to_string(),
            capabilities: Vec::new(),
            deprecation: Some(GemDeprecation {
                message: "The renderer has moved to the frame-graph implementation.".to_string(),
                since: Some("0.4.0".to_string()),
                replacement: Some(GemReplacement {
                    id: "azoth.frame_graph".to_string(),
                    version: Some("^0.5.0".to_string()),
                }),
            }),
            provenance: None,
            linkage: None,
            dependencies: Vec::new(),
            contribution_fingerprints: std::collections::BTreeMap::default(),
            root: PathBuf::from("gems/old-render"),
            manifest_path: PathBuf::from("gems/old-render/gem.toml"),
        };

        let warning = gem_deprecation_warning(&package).unwrap();

        assert!(warning.contains("`azoth.old_render` (Old Render) is deprecated since v0.4.0"));
        assert!(warning.contains("use `azoth.frame_graph` ^0.5.0"));
    }
}
