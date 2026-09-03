use crate::error::{ScaffoldError, ScaffoldResult};
use crate::new;
use az_project::{ProjectManifest, project_lock_path, project_manifest_path};
use std::path::{Path, PathBuf};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value, table, value};
use tracing::info;

/// Initialize an Azoth project in an existing directory.
///
/// `init` and `new` produce the same thing — a primary-gem project whose
/// runtime, authoring, and builder role packages live under `gems/<slug>` —
/// and differ only in what they accept as a starting point: `new` demands an
/// empty directory, `init` adopts one that already holds assets, scripts, or
/// source control. Re-running `init` on a project it already owns refreshes
/// engine-owned state (packaging profiles, workspace contract, generated
/// targets, repository metadata) without touching authored source.
///
/// A pre-ADR-0025 `crates/game` project is refused, not migrated: see
/// [`ScaffoldError::LegacyProjectLayout`].
///
/// # Errors
///
/// Returns [`ScaffoldError::Io`] if the project directory cannot be created,
/// [`ScaffoldError::LegacyProjectLayout`] if `path` holds a retired
/// `crates/game` layout or a Cargo workspace with no `azoth.toml`,
/// [`ScaffoldError::InvalidProjectName`] if the derived name is not a legal
/// project name, and any [`ScaffoldError::ProjectManifest`] or
/// [`ScaffoldError::SourceControl`] raised while writing the manifest,
/// synchronizing the project contract, or checkpointing source control.
///
/// # Panics
///
/// Panics if the manifest that was just written or refreshed carries no
/// `[project].primary_gem`; both paths guarantee one, so reaching this means
/// the manifest was mutated concurrently.
pub fn execute(
    path: Option<PathBuf>,
    name: Option<String>,
    lore_url: Option<String>,
) -> ScaffoldResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&project_path)?;

    let manifest = if project_manifest_path(&project_path).exists() {
        refresh_existing_project(&project_path, lore_url.as_deref())?
    } else {
        create_project(&project_path, name, lore_url.as_deref())?
    };

    let project_name = manifest.project.name.clone();
    // Both paths guarantee one: `create_project` scaffolds it, and
    // `refresh_existing_project` refuses a manifest without it.
    let primary_gem = manifest
        .project
        .primary_gem
        .expect("an initialized Azoth project always declares a primary gem");
    let setup = lore_url
        .map(new::LoreRepositorySetup::new)
        .map(|setup| setup.description(format!("Azoth project {project_name}")));
    let source_control_state = new::ensure_project_lore_checkpoint(
        &project_path,
        new::INITIAL_PROJECT_COMMIT_MESSAGE,
        setup.as_ref(),
    )?;

    println!("Azoth project initialized: {}", project_path.display());
    println!(
        "Manifest: {}",
        project_manifest_path(&project_path).display()
    );
    println!("Lock: {}", project_lock_path(&project_path).display());
    println!("Primary gem: {primary_gem}");
    println!("Next steps:");
    new::print_project_workflow_next_steps(
        source_control_state,
        new::INITIAL_PROJECT_COMMIT_MESSAGE,
        Some(&project_path),
    );

    Ok(())
}

/// Scaffold a primary-gem project into a directory that holds no `azoth.toml`.
fn create_project(
    project_path: &Path,
    name: Option<String>,
    lore_url: Option<&str>,
) -> ScaffoldResult<ProjectManifest> {
    // The layout is written whole, root `Cargo.toml` included. A directory
    // that already carries a Cargo workspace is somebody else's project: init
    // cannot infer which of its crates are the runtime, authoring, and builder
    // roles, and it will not overwrite the manifest to guess.
    if project_path.join("Cargo.toml").exists() {
        return Err(ScaffoldError::LegacyProjectLayout {
            path: project_path.to_path_buf(),
            reason: "the directory already holds a Cargo workspace but no `azoth.toml`".to_string(),
        });
    }
    let project_name = name.unwrap_or_else(|| {
        project_path
            .file_name()
            .and_then(|name| name.to_str())
            .map_or_else(|| "project".to_string(), str::to_string)
    });
    if !new::is_valid_project_name(&project_name) {
        return Err(ScaffoldError::InvalidProjectName(project_name));
    }

    info!(
        root = %project_path.display(),
        project_name,
        "initializing Azoth project"
    );

    new::scaffold_primary_project(
        project_path,
        &project_name,
        &new::ProjectCreateOptions {
            lore_url: lore_url.map(str::to_string),
            ..new::ProjectCreateOptions::default()
        },
    )
}

/// Refresh engine-owned state in a directory that already carries an
/// `azoth.toml`.
fn refresh_existing_project(
    project_path: &Path,
    lore_url: Option<&str>,
) -> ScaffoldResult<ProjectManifest> {
    let prepared = prepare_existing_project_manifest(&project_manifest_path(project_path))?;

    info!(
        root = %project_path.display(),
        project_name = prepared.manifest.project.name,
        "refreshing Azoth project workflow"
    );

    ensure_project_directories(project_path, &prepared.manifest)?;
    for (path, contents) in new::repository_metadata_files(project_path) {
        ensure_ignore_file(&path, contents)?;
    }
    commit_project_manifest(project_path, &prepared)?;
    if let Some(remote_url) = lore_url {
        ensure_project_manifest_lore_remote(project_path, remote_url)?;
    }
    crate::project_contract::sync_project_contract(project_path)?;
    Ok(prepared.manifest)
}

fn ensure_project_manifest_lore_remote(
    project_path: &Path,
    remote_url: &str,
) -> ScaffoldResult<()> {
    let manifest_path = project_manifest_path(project_path);
    let text = std::fs::read_to_string(&manifest_path)?;
    let mut document =
        text.parse::<DocumentMut>()
            .map_err(|source| ScaffoldError::ConfigParse {
                path: manifest_path.clone(),
                message: source.to_string(),
            })?;
    document["source_control"]["lore"]["remote_url"] = value(remote_url);
    let updated = document.to_string();
    project_manifest_from_toml(&manifest_path, &updated)?;
    if updated != text {
        std::fs::write(manifest_path, updated)?;
    }
    Ok(())
}

/// A parsed `azoth.toml` plus the rewritten text, when repair changed it.
struct PreparedProjectManifest {
    manifest: ProjectManifest,
    updated_text: Option<String>,
}

fn prepare_existing_project_manifest(
    manifest_path: &Path,
) -> ScaffoldResult<PreparedProjectManifest> {
    let text = std::fs::read_to_string(manifest_path)?;
    let mut document =
        text.parse::<DocumentMut>()
            .map_err(|source| ScaffoldError::ConfigParse {
                path: manifest_path.to_path_buf(),
                message: source.to_string(),
            })?;

    if !document_has_primary_gem(&document) {
        return Err(ScaffoldError::LegacyProjectLayout {
            path: manifest_path.to_path_buf(),
            reason: "`azoth.toml` declares no `[project].primary_gem`".to_string(),
        });
    }
    ensure_project_manifest_packaging(&mut document, manifest_path)?;
    let updated = document.to_string();
    let manifest = project_manifest_from_toml(manifest_path, &updated)?;
    let updated_text = (updated != text).then_some(updated);

    Ok(PreparedProjectManifest {
        manifest,
        updated_text,
    })
}

fn commit_project_manifest(
    project_path: &Path,
    prepared: &PreparedProjectManifest,
) -> ScaffoldResult<()> {
    if let Some(text) = &prepared.updated_text {
        std::fs::write(project_manifest_path(project_path), text)?;
    }
    Ok(())
}

fn project_manifest_from_toml(manifest_path: &Path, text: &str) -> ScaffoldResult<ProjectManifest> {
    let manifest =
        toml::from_str::<ProjectManifest>(text).map_err(|source| ScaffoldError::ConfigParse {
            path: manifest_path.to_path_buf(),
            message: source.to_string(),
        })?;
    manifest.validate()?;
    Ok(manifest)
}

fn document_has_primary_gem(document: &DocumentMut) -> bool {
    document
        .get("project")
        .and_then(Item::as_table)
        .and_then(|project| project.get("primary_gem"))
        .and_then(Item::as_str)
        .is_some_and(|primary_gem| !primary_gem.trim().is_empty())
}

fn ensure_project_manifest_packaging(
    document: &mut DocumentMut,
    manifest_path: &Path,
) -> ScaffoldResult<()> {
    if !document.as_table().contains_key("packaging") {
        document.as_table_mut().insert("packaging", table());
    }
    let packaging = document
        .as_table_mut()
        .get_mut("packaging")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| ScaffoldError::ConfigParse {
            path: manifest_path.to_path_buf(),
            message: "`packaging` must be a TOML table".to_string(),
        })?;
    let profiles = ensure_array_of_tables(packaging, "packaging", "profiles", manifest_path)?;
    if !array_of_tables_has_name(profiles, "pc-dev") {
        profiles.push(package_profile_table(
            "pc-dev", "pc", "debug", "loose", "none", None,
        ));
    }
    if !array_of_tables_has_name(profiles, "pc-release") {
        let mut oodle = InlineTable::new();
        oodle.insert("compressor", Value::from("kraken"));
        oodle.insert("effort", Value::from("normal"));
        oodle.fmt();
        profiles.push(package_profile_table(
            "pc-release",
            "pc",
            "release",
            "azpack",
            "oodle",
            Some(oodle),
        ));
    }
    Ok(())
}

fn ensure_array_of_tables<'a>(
    table: &'a mut Table,
    parent_label: &str,
    key: &str,
    manifest_path: &Path,
) -> ScaffoldResult<&'a mut ArrayOfTables> {
    if !table.contains_key(key) {
        table.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    table
        .get_mut(key)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| ScaffoldError::ConfigParse {
            path: manifest_path.to_path_buf(),
            message: format!("`{parent_label}.{key}` must be a TOML array of tables"),
        })
}

fn array_of_tables_has_name(array: &ArrayOfTables, name: &str) -> bool {
    array
        .iter()
        .any(|table| table.get("name").and_then(Item::as_str) == Some(name))
}

fn package_profile_table(
    name: &str,
    asset_platform: &str,
    cargo_profile: &str,
    container: &str,
    compression: &str,
    oodle: Option<InlineTable>,
) -> Table {
    let mut profile = Table::new();
    profile.insert("name", value(name));
    profile.insert("asset_platform", value(asset_platform));
    profile.insert("cargo_profile", value(cargo_profile));
    profile.insert("container", value(container));
    profile.insert("compression", value(compression));
    if let Some(oodle) = oodle {
        profile.insert("oodle", Item::Value(Value::InlineTable(oodle)));
    }
    profile
}

fn ensure_project_directories(
    project_path: &Path,
    manifest: &ProjectManifest,
) -> ScaffoldResult<()> {
    std::fs::create_dir_all(project_path.join(&manifest.paths.assets))?;
    std::fs::create_dir_all(project_path.join(&manifest.paths.scripts))?;
    std::fs::create_dir_all(project_path.join("gems"))?;
    Ok(())
}

pub(crate) fn ensure_workspace_contract(
    project_path: &Path,
    required_members: &[String],
    default_members: &[String],
    retired_members: &[&str],
) -> ScaffoldResult<()> {
    let cargo_path = project_path.join("Cargo.toml");
    let text = std::fs::read_to_string(&cargo_path)?;
    let mut document =
        text.parse::<DocumentMut>()
            .map_err(|source| ScaffoldError::ConfigParse {
                path: cargo_path.clone(),
                message: source.to_string(),
            })?;

    if cargo_package_name(&document).is_some() {
        return Err(ScaffoldError::ConfigParse {
            path: cargo_path,
            message: "Azoth projects use a virtual workspace root; move the root package into a \
                      gem role package under `gems/` or start from `azoth project new`"
                .to_string(),
        });
    }

    if !document.as_table().contains_key("workspace") {
        document.as_table_mut().insert("workspace", table());
    }
    let workspace = document
        .as_table_mut()
        .get_mut("workspace")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| ScaffoldError::ConfigParse {
            path: cargo_path.clone(),
            message: "`workspace` must be a TOML table".to_string(),
        })?;
    ensure_workspace_entries(workspace, "members", required_members);
    ensure_workspace_entries(workspace, "default-members", default_members);
    remove_workspace_entries(workspace, "members", retired_members);
    remove_workspace_entries(workspace, "default-members", retired_members);
    if !workspace.contains_key("resolver") {
        workspace.insert("resolver", value("3"));
    }
    ensure_project_workspace_dev_profile(&mut document, &cargo_path)?;

    let updated = document.to_string();
    if updated != text {
        std::fs::write(cargo_path, updated)?;
    }
    Ok(())
}

fn remove_workspace_entries(workspace: &mut Table, key: &str, retired: &[&str]) {
    let Some(entries) = workspace.get_mut(key).and_then(Item::as_array_mut) else {
        return;
    };
    entries.retain(|entry| entry.as_str().is_none_or(|entry| !retired.contains(&entry)));
}

fn ensure_workspace_entries(workspace: &mut Table, key: &str, required: &[String]) {
    let item = workspace
        .entry(key)
        .or_insert_with(|| Item::Value(Value::Array(Array::new())));
    let Some(entries) = item.as_array_mut() else {
        *item = Item::Value(Value::Array(Array::new()));
        let Some(entries) = item.as_array_mut() else {
            return;
        };
        for entry in required {
            entries.push(entry.as_str());
        }
        return;
    };
    for entry in required {
        if !entries
            .iter()
            .any(|existing| existing.as_str() == Some(entry.as_str()))
        {
            entries.push(entry.as_str());
        }
    }
}

fn ensure_project_workspace_dev_profile(
    document: &mut DocumentMut,
    cargo_path: &Path,
) -> ScaffoldResult<()> {
    let profile = ensure_child_table(document.as_table_mut(), "profile", cargo_path)?;
    let dev = ensure_child_table(profile, "dev", cargo_path)?;
    dev.insert("opt-level", value(1));
    let packages = ensure_child_table(dev, "package", cargo_path)?;
    let dependencies = ensure_child_table(packages, "*", cargo_path)?;
    dependencies.insert("opt-level", value(3));
    Ok(())
}

fn ensure_child_table<'a>(
    parent: &'a mut Table,
    key: &str,
    cargo_path: &Path,
) -> ScaffoldResult<&'a mut Table> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| ScaffoldError::ConfigParse {
            path: cargo_path.to_path_buf(),
            message: format!("workspace policy `{key}` must be a TOML table"),
        })
}

fn ensure_ignore_file(path: &Path, lines: &str) -> ScaffoldResult<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };

    let mut updated = existing.clone();
    for line in lines.lines().filter(|line| !line.trim().is_empty()) {
        if !existing
            .lines()
            .any(|existing| existing.trim() == line.trim())
        {
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(line);
            updated.push('\n');
        }
    }

    if updated != existing {
        std::fs::write(path, updated)?;
    }
    Ok(())
}

fn cargo_package_name(document: &DocumentMut) -> Option<&str> {
    document
        .as_table()
        .get("package")?
        .as_table()?
        .get("name")?
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_project::load_project_manifest;

    /// A pre-ADR-0025 project: a root Cargo workspace whose members are the
    /// authored `crates/game` cluster. Nothing produces this layout any more;
    /// it exists here only as the thing `init` must refuse.
    fn write_legacy_game_cluster(root: &Path) {
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/game\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("crates/game/src")).unwrap();
        std::fs::write(
            root.join("crates/game/Cargo.toml"),
            "[package]\nname = \"game\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(root.join("crates/game/src/lib.rs"), "// hand-authored\n").unwrap();
    }

    fn assert_no_legacy_cluster(root: &Path) {
        assert!(!root.join("crates/game").exists());
        assert!(!root.join("crates/asset-processor").exists());
        assert!(!root.join("crates/asset-worker").exists());
    }

    #[test]
    fn init_and_new_produce_the_same_primary_gem_layout() {
        let temp = tempfile::tempdir().unwrap();
        let new_root = temp.path().join("created-by-new");
        let init_root = temp.path().join("created-by-init");

        new::execute("sample-game".to_string(), Some(new_root.clone()), None).unwrap();
        execute(
            Some(init_root.clone()),
            Some("sample-game".to_string()),
            None,
        )
        .unwrap();

        for root in [&new_root, &init_root] {
            let manifest = load_project_manifest(root).unwrap();
            assert_eq!(
                manifest.project.primary_gem.as_deref(),
                Some("sample_game.game")
            );
            assert!(root.join("gems/sample-game/runtime").is_dir());
            assert!(root.join("gems/sample-game/authoring").is_dir());
            assert!(root.join("gems/sample-game/builders").is_dir());
            assert_no_legacy_cluster(root);
        }
    }

    #[test]
    fn init_creates_the_primary_gem_project_in_an_existing_directory() {
        let temp = tempfile::tempdir().unwrap();
        // A directory that already holds authored content but no project.
        std::fs::create_dir_all(temp.path().join("art")).unwrap();
        std::fs::write(temp.path().join("art/notes.md"), "wip\n").unwrap();

        execute(
            Some(temp.path().to_path_buf()),
            Some("sample_game".to_string()),
            None,
        )
        .unwrap();

        let manifest = load_project_manifest(temp.path()).unwrap();
        assert_eq!(manifest.project.name, "sample_game");
        assert_eq!(
            manifest.project.primary_gem.as_deref(),
            Some("sample_game.game")
        );
        assert_eq!(manifest.packaging.profiles.len(), 2);
        // Services are prebuilt engine binaries (ADR 0040): a scaffolded
        // project routes none of its own.
        assert!(manifest.tools.service_targets.is_empty());
        assert!(manifest.tools.build_targets.is_empty());

        let lock = az_project::load_project_lock(temp.path()).unwrap();
        assert_eq!(lock.project.id, "sample_game");
        assert_eq!(lock.packages.len(), 2);
        assert_eq!(lock.packaging.profiles.len(), 2);
        assert_eq!(
            lock.packaging.profiles[1].container,
            az_project::ProjectPackageContainer::AzPack
        );
        assert_eq!(
            lock.packaging.profiles[1].compression,
            az_project::ProjectPackageCompression::Oodle
        );
        assert!(project_lock_path(temp.path()).exists());

        assert!(temp.path().join("gems/sample-game/gem.toml").is_file());
        assert!(
            temp.path()
                .join("gems/sample-game/runtime/src/graphs.rs")
                .is_file()
        );
        assert!(
            temp.path()
                .join("gems/sample-game/authoring/src/lib.rs")
                .is_file()
        );
        assert!(
            temp.path()
                .join("gems/sample-game/builders/src/assets.rs")
                .is_file()
        );
        assert_no_legacy_cluster(temp.path());
        // Pre-existing authored content is left alone.
        assert_eq!(
            std::fs::read_to_string(temp.path().join("art/notes.md")).unwrap(),
            "wip\n"
        );

        // Engine-owned generated state lives under `.azoth`; a vendored
        // engine copy does not.
        assert!(temp.path().join(".azoth/targets/generation.json").is_file());
        assert!(!temp.path().join(".azoth/engine").exists());
        assert!(!temp.path().join(".cache").exists());
        assert!(!temp.path().join("Cache").exists());
        let loreignore = std::fs::read_to_string(temp.path().join(".loreignore")).unwrap();
        assert!(loreignore.contains("/Cache/"));
    }

    #[test]
    fn init_without_lore_url_reports_missing_repository_for_sessions() {
        let temp = tempfile::tempdir().unwrap();

        execute(
            Some(temp.path().to_path_buf()),
            Some("sample_game".to_string()),
            None,
        )
        .unwrap();

        assert_eq!(
            new::project_workflow_source_control_state(temp.path()).unwrap(),
            new::ProjectWorkflowSourceControlState {
                has_lore_repository: false,
                has_committed_revision: false,
                has_local_changes: false,
            }
        );
    }

    #[test]
    fn init_writes_repository_metadata_from_the_shared_catalog() {
        let temp = tempfile::tempdir().unwrap();

        execute(
            Some(temp.path().to_path_buf()),
            Some("sample_game".to_string()),
            None,
        )
        .unwrap();

        for (path, contents) in new::repository_metadata_files(temp.path()) {
            let written = std::fs::read_to_string(&path).unwrap();
            for line in contents.lines().filter(|line| !line.trim().is_empty()) {
                assert!(
                    written
                        .lines()
                        .any(|existing| existing.trim() == line.trim()),
                    "metadata file `{}` is missing catalog line `{line}`",
                    path.display()
                );
            }
        }
    }

    /// Successor to the retired `init_updates_existing_game_crate_without_
    /// overwriting_sources`: the crate whose sources init must not clobber is
    /// now a gem role package, not `crates/game`.
    #[test]
    fn init_refreshes_an_existing_project_without_overwriting_authored_sources() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        let runtime_lib = project_root.join("gems/sample-game/runtime/src/lib.rs");
        let authored = "//! hand-authored runtime\npub fn tick() {}\n";
        std::fs::write(&runtime_lib, authored).unwrap();
        let gem_manifest = project_root.join("gems/sample-game/gem.toml");
        let gem_manifest_before = std::fs::read_to_string(&gem_manifest).unwrap();
        std::fs::remove_file(project_root.join(".loreignore")).unwrap();

        execute(Some(project_root.clone()), None, None).unwrap();

        assert_eq!(std::fs::read_to_string(&runtime_lib).unwrap(), authored);
        assert_eq!(
            std::fs::read_to_string(&gem_manifest).unwrap(),
            gem_manifest_before
        );
        // Engine-owned repository metadata is restored.
        assert!(project_root.join(".loreignore").is_file());
        let manifest = load_project_manifest(&project_root).unwrap();
        assert!(manifest.project.primary_gem.is_some());
        assert!(manifest.tools.service_targets.is_empty());
        let root_cargo = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap();
        assert!(root_cargo.contains("\"gems/sample-game/runtime\""));
        assert_no_legacy_cluster(&project_root);
    }

    #[test]
    fn init_seeds_missing_packaging_profiles_into_an_existing_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        new::execute("sample-game".to_string(), Some(project_root.clone()), None).unwrap();

        let manifest_path = project_manifest_path(&project_root);
        let text = std::fs::read_to_string(&manifest_path).unwrap();
        let stripped = text
            .split("[[packaging.profiles]]")
            .next()
            .unwrap()
            .to_string();
        std::fs::write(
            &manifest_path,
            format!("# keep this hand-authored comment\n{stripped}"),
        )
        .unwrap();

        execute(Some(project_root.clone()), None, None).unwrap();

        let refreshed = std::fs::read_to_string(&manifest_path).unwrap();
        assert!(refreshed.starts_with("# keep this hand-authored comment"));
        let manifest = load_project_manifest(&project_root).unwrap();
        assert_eq!(
            manifest
                .packaging
                .profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>(),
            ["pc-dev", "pc-release"]
        );
    }

    #[test]
    fn init_refuses_a_manifest_without_a_primary_gem() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("azoth.toml"),
            r#"[manifest]
kind = "project"
schema = "azoth.project/v1"

[project]
id = "sample_project"
name = "sample-project"
version = "0.1.0"
engine_version = "0.1.0"
"#,
        )
        .unwrap();
        let before = std::fs::read_to_string(temp.path().join("azoth.toml")).unwrap();

        let error = execute(Some(temp.path().to_path_buf()), None, None)
            .expect_err("a project without a primary gem is not initializable");

        let ScaffoldError::LegacyProjectLayout { reason, .. } = &error else {
            panic!("expected a legacy-layout refusal, got {error:?}");
        };
        assert!(reason.contains("primary_gem"), "{reason}");
        // A refusal writes nothing.
        assert_eq!(
            std::fs::read_to_string(temp.path().join("azoth.toml")).unwrap(),
            before
        );
        assert!(!temp.path().join("gems").exists());
        assert!(!temp.path().join("Cargo.toml").exists());
    }

    #[test]
    fn init_refuses_a_legacy_crates_game_cluster() {
        let temp = tempfile::tempdir().unwrap();
        write_legacy_game_cluster(temp.path());
        let root_cargo_before = std::fs::read_to_string(temp.path().join("Cargo.toml")).unwrap();

        let error = execute(Some(temp.path().to_path_buf()), None, None)
            .expect_err("the retired crates/game layout is not initializable");

        let ScaffoldError::LegacyProjectLayout { reason, .. } = &error else {
            panic!("expected a legacy-layout refusal, got {error:?}");
        };
        assert!(reason.contains("Cargo workspace"), "{reason}");
        assert!(error.to_string().contains("azoth project new"));
        // The authored workspace is not rewritten, and no project is created.
        assert_eq!(
            std::fs::read_to_string(temp.path().join("Cargo.toml")).unwrap(),
            root_cargo_before
        );
        assert!(!temp.path().join("azoth.toml").exists());
        assert!(!temp.path().join("gems").exists());
    }

    #[test]
    fn workspace_contract_repairs_defaults_and_is_byte_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            r#"[workspace]
members = ["tools/helper"]
default-members = ["tools/helper", ".azoth/targets/*"]
resolver = "3"

[workspace.dependencies]
custom-domain = { path = "crates/custom-domain" }

[profile.dev]
opt-level = 0
debug = 0

[profile.dev.package."*"]
opt-level = 0
"#,
        )
        .unwrap();

        let required = vec!["gems/sample/server".to_string()];
        let retired = [".azoth/targets/*"];
        ensure_workspace_contract(temp.path(), &required, &required, &retired).unwrap();
        let first = std::fs::read_to_string(temp.path().join("Cargo.toml")).unwrap();
        ensure_workspace_contract(temp.path(), &required, &required, &retired).unwrap();
        let second = std::fs::read_to_string(temp.path().join("Cargo.toml")).unwrap();

        assert_eq!(first, second);
        assert!(first.contains("\"tools/helper\""));
        assert!(first.contains("\"gems/sample/server\""));
        assert!(!first.contains(".azoth/targets/*"));
        assert!(first.contains("custom-domain = { path = \"crates/custom-domain\" }"));
        assert!(!first.contains("az-core"));
        assert!(first.contains("[profile.dev]\nopt-level = 1\ndebug = 0"));
        assert!(first.contains("[profile.dev.package.\"*\"]\nopt-level = 3"));
    }

    #[test]
    fn workspace_contract_rejects_a_package_root() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"root_package\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();

        let error = ensure_workspace_contract(temp.path(), &[], &[], &[])
            .expect_err("an Azoth project root is a virtual workspace");

        assert!(matches!(error, ScaffoldError::ConfigParse { .. }));
        assert!(error.to_string().contains("virtual workspace root"));
    }
}
