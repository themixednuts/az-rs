//! Shared Azoth project scaffolding workflows.
//!
//! This crate owns project/gem generation and initialization logic so tools
//! like `azoth` and `az-editor` can use the same implementation.

#[cfg(test)]
use std::path::Component;
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;

pub mod error;
pub mod gem;
pub mod init;
pub mod new;
pub mod project_contract;
pub mod topology;

#[cfg(test)]
pub(crate) mod manifest_test_support;

pub use error::{ScaffoldError, ScaffoldResult};

// Bootstrap surface re-exported so universal tooling (the editor shell) can
// prepare and read project/gem manifests through the shared scaffold crate
// without taking a direct `az-project` dependency.
pub use az_project::{
    GemManifest, ProjectTopologyKind, ensure_project_generated_targets, load_gem_manifest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub engine_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineGemSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
}

impl ProjectSummary {
    fn from_manifest(manifest: &az_project::ProjectManifest) -> Self {
        Self {
            id: manifest.project.id.clone(),
            name: manifest.project.name.clone(),
            engine_version: manifest.project.engine_version.clone(),
        }
    }
}

/// Read one project's manifest and reduce it to its summary fields.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if `project_root` holds no
/// readable, schema-valid `azoth.toml`.
pub fn load_project_summary(project_root: impl AsRef<Path>) -> ScaffoldResult<ProjectSummary> {
    let manifest = az_project::load_project_manifest(project_root)?;
    Ok(ProjectSummary::from_manifest(&manifest))
}

/// Summarize every gem the resolved engine registers.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if the engine root cannot be
/// resolved, plus any error [`load_engine_gem_summaries_from_root`] returns.
pub fn load_engine_gem_summaries() -> ScaffoldResult<Vec<EngineGemSummary>> {
    let engine_root = az_project::resolve_engine_root()?;
    load_engine_gem_summaries_from_root(engine_root)
}

/// Summarize every gem registered by the engine at `engine_root`.
///
/// # Errors
///
/// Returns [`ScaffoldError::ProjectManifest`] if `engine_root` holds no
/// readable `engine.toml`, or if one of the gem manifests it registers cannot
/// be loaded or validated.
pub fn load_engine_gem_summaries_from_root(
    engine_root: impl AsRef<Path>,
) -> ScaffoldResult<Vec<EngineGemSummary>> {
    let resolved = az_project::resolve_engine_gems(engine_root)?;
    Ok(resolved
        .into_iter()
        .map(|gem| EngineGemSummary {
            id: gem.manifest.gem.id,
            name: gem.manifest.gem.name,
            version: gem.manifest.gem.version,
            root: gem.root,
        })
        .collect())
}

pub(crate) fn azoth_workspace_crate_version(package_name: &str) -> ScaffoldResult<String> {
    let root = az_project::resolve_engine_root()?;
    let package_path = engine_workspace_package_path(&root, package_name)?;
    engine_workspace_package_version(&package_path, package_name)
}

pub(crate) fn engine_workspace_package_path(
    engine_root: &Path,
    package_name: &str,
) -> ScaffoldResult<PathBuf> {
    let manifest_path = engine_root.join("Cargo.toml");
    let manifest = read_toml_value(&manifest_path)?;
    if let Some(path) = workspace_dependency_package_path(engine_root, &manifest, package_name) {
        return Ok(path);
    }
    if let Some(path) = workspace_member_package_path(engine_root, &manifest, package_name)? {
        return Ok(path);
    }
    Err(ScaffoldError::UnsupportedEngineWorkspaceCrate(
        package_name.to_string(),
    ))
}

fn engine_workspace_package_version(
    package_path: &Path,
    package_name: &str,
) -> ScaffoldResult<String> {
    let manifest_path = package_path.join("Cargo.toml");
    let manifest = read_toml_value(&manifest_path)?;
    manifest
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(TomlValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| ScaffoldError::ConfigParse {
            path: manifest_path,
            message: format!("engine package `{package_name}` has no [package].version"),
        })
}

fn read_toml_value(path: &Path) -> ScaffoldResult<TomlValue> {
    let text = std::fs::read_to_string(path)?;
    toml::from_str(&text).map_err(|source| ScaffoldError::ConfigParse {
        path: path.to_path_buf(),
        message: source.to_string(),
    })
}

fn workspace_dependency_package_path(
    engine_root: &Path,
    manifest: &TomlValue,
    package_name: &str,
) -> Option<PathBuf> {
    let dependencies = manifest.get("workspace")?.get("dependencies")?.as_table()?;
    if let Some(path) = dependency_path(engine_root, dependencies.get(package_name)) {
        return Some(path);
    }
    dependencies
        .values()
        .find_map(|dependency| match dependency.as_table() {
            Some(table)
                if table.get("package").and_then(TomlValue::as_str) == Some(package_name) =>
            {
                dependency_path(engine_root, Some(dependency))
            }
            _ => None,
        })
}

fn dependency_path(engine_root: &Path, dependency: Option<&TomlValue>) -> Option<PathBuf> {
    dependency?
        .as_table()?
        .get("path")
        .and_then(TomlValue::as_str)
        .map(|path| engine_root.join(path))
}

fn workspace_member_package_path(
    engine_root: &Path,
    manifest: &TomlValue,
    package_name: &str,
) -> ScaffoldResult<Option<PathBuf>> {
    let Some(members) = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(TomlValue::as_array)
    else {
        return Ok(None);
    };

    for member in members.iter().filter_map(TomlValue::as_str) {
        if member.contains('*') {
            continue;
        }
        let package_path = engine_root.join(member);
        let manifest_path = package_path.join("Cargo.toml");
        if !manifest_path.exists() {
            continue;
        }
        let member_manifest = read_toml_value(&manifest_path)?;
        if member_manifest
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(TomlValue::as_str)
            == Some(package_name)
        {
            return Ok(Some(package_path));
        }
    }

    Ok(None)
}

#[cfg(test)]
fn path_to_toml(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    path.strip_prefix("//?/UNC/").map_or_else(
        || {
            path.strip_prefix("//?/")
                .map_or_else(|| path.clone(), str::to_owned)
        },
        |stripped| format!("//{stripped}"),
    )
}

#[cfg(test)]
fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    let base_components = base.components().collect::<Vec<_>>();
    let target_components = target.components().collect::<Vec<_>>();
    if base_components.is_empty() || target_components.is_empty() {
        return None;
    }
    if !same_root(&base_components, &target_components) {
        return None;
    }

    let mut common = 0;
    while common < base_components.len()
        && common < target_components.len()
        && component_eq(base_components[common], target_components[common])
    {
        common += 1;
    }

    let mut relative = PathBuf::new();
    for component in base_components[common..].iter().copied() {
        if matches!(component, Component::Normal(_)) {
            relative.push("..");
        }
    }
    for component in target_components[common..].iter().copied() {
        match component {
            Component::Normal(segment) => relative.push(segment),
            Component::CurDir => {}
            Component::ParentDir => relative.push(".."),
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }

    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Some(relative)
}

#[cfg(test)]
fn same_root(left: &[Component<'_>], right: &[Component<'_>]) -> bool {
    let mut left_roots = left
        .iter()
        .copied()
        .take_while(|component| !matches!(component, Component::Normal(_)));
    let mut right_roots = right
        .iter()
        .copied()
        .take_while(|component| !matches!(component, Component::Normal(_)));
    loop {
        match (left_roots.next(), right_roots.next()) {
            (Some(left), Some(right)) if component_eq(left, right) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

#[cfg(test)]
fn component_eq(left: Component<'_>, right: Component<'_>) -> bool {
    #[cfg(windows)]
    {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_prefers_project_relative_engine_root_on_same_unix_root() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        let engine_root = temp.path().join("az-rs");
        assert_eq!(
            relative_path(&project_root, &engine_root),
            Some(PathBuf::from("../az-rs"))
        );
        assert_eq!(
            path_to_toml(&PathBuf::from("../az-rs/crates/az/schema")),
            "../az-rs/crates/az/schema"
        );
    }

    #[cfg(windows)]
    #[test]
    fn relative_path_prefers_project_relative_engine_root_on_same_windows_drive() {
        let project_drive = "E:";
        let other_drive = "D:";
        let project_root = format!("{project_drive}/Projects/sample-game");
        let engine_root = format!("{project_drive}/Projects/az-rs");
        let other_project_root = format!("{other_drive}/Projects/sample-game");
        assert_eq!(
            relative_path(Path::new(&project_root), Path::new(&engine_root)),
            Some(PathBuf::from("../az-rs"))
        );
        assert!(relative_path(Path::new(&other_project_root), Path::new(&engine_root)).is_none());
    }

    #[test]
    fn engine_workspace_package_path_prefers_workspace_dependency_paths() {
        let temp = tempfile::tempdir().unwrap();
        let member = temp.path().join("crates/az/schema");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            r#"[workspace]
members = ["moved/schema"]

[workspace.dependencies]
az-schema = { path = "crates/az/schema" }
"#,
        )
        .unwrap();

        assert_eq!(
            engine_workspace_package_path(temp.path(), "az-schema").unwrap(),
            temp.path().join("crates/az/schema")
        );
    }

    #[test]
    fn engine_workspace_package_path_falls_back_to_workspace_members() {
        let temp = tempfile::tempdir().unwrap();
        let member = temp.path().join("crates/editor/ui");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            r#"[workspace]
members = ["crates/editor/ui"]
"#,
        )
        .unwrap();
        std::fs::write(
            member.join("Cargo.toml"),
            r#"[package]
name = "az-editor-ui"
version = "0.1.0"
edition = "2024"
"#,
        )
        .unwrap();

        assert_eq!(
            engine_workspace_package_path(temp.path(), "az-editor-ui").unwrap(),
            member
        );
    }

    #[test]
    fn engine_workspace_package_path_rejects_unknown_packages() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            r"[workspace]
members = []
",
        )
        .unwrap();

        assert!(matches!(
            engine_workspace_package_path(temp.path(), "missing-package").unwrap_err(),
            ScaffoldError::UnsupportedEngineWorkspaceCrate(package)
                if package == "missing-package"
        ));
    }
}
