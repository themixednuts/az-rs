//! Project-scoped Azoth-domain cleanup.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use az_filesystem::{AzothDataHome, ProjectDataPaths};
use az_service_supervision::ServiceProcessState;
use az_session::SessionManager;
use tracing::{info, instrument};

use crate::error::{CliError, CliResult};

const CLEAN_SESSION_SHUTDOWN_TIMEOUT_MS: u64 = 30_000;

/// One class of regenerable Azoth-domain data that `azoth clean` can remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CleanScope {
    AssetDb,
    Products,
    Derived,
    Endpoints,
    Sessions,
}

impl CleanScope {
    const ALL: [Self; 5] = [
        Self::AssetDb,
        Self::Products,
        Self::Derived,
        Self::Endpoints,
        Self::Sessions,
    ];
}

/// The scopes a single `azoth clean` invocation acts on; empty means "every scope".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CleanScopes(BTreeSet<CleanScope>);

impl CleanScopes {
    fn all() -> Self {
        Self(CleanScope::ALL.into_iter().collect())
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn contains(&self, scope: CleanScope) -> bool {
        self.0.contains(&scope)
    }
}

impl CleanScopes {
    /// Selects the scopes whose CLI flag was passed. `flags` is parallel to
    /// [`CleanScope::ALL`], so entry `i` enables `CleanScope::ALL[i]`.
    #[must_use]
    pub fn from_flags(flags: [bool; 5]) -> Self {
        CleanScope::ALL
            .into_iter()
            .zip(flags)
            .filter_map(|(scope, selected)| selected.then_some(scope))
            .collect()
    }
}

impl FromIterator<CleanScope> for CleanScopes {
    fn from_iter<I: IntoIterator<Item = CleanScope>>(scopes: I) -> Self {
        Self(scopes.into_iter().collect())
    }
}

#[derive(Debug, Default)]
pub struct CleanOptions {
    pub path: Option<PathBuf>,
    pub scopes: CleanScopes,
    pub platform: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CleanSelection {
    scopes: CleanScopes,
    platform: Option<String>,
}

impl CleanSelection {
    fn from_options(options: &CleanOptions) -> CliResult<Self> {
        if options.platform.is_some() && !options.scopes.contains(CleanScope::Products) {
            return Err(CliError::InvalidArgument {
                message: "--platform requires --products".to_string(),
            });
        }
        let scopes = if options.scopes.is_empty() {
            CleanScopes::all()
        } else {
            options.scopes.clone()
        };
        Ok(Self {
            scopes,
            platform: options.platform.clone(),
        })
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CleanReport {
    removed: Vec<PathBuf>,
}

/// # Errors
///
/// Returns [`CliError::InvalidArgument`] when `options.platform` is set without the
/// [`CleanScope::Products`] scope, [`CliError::ProjectManifest`] when the project
/// manifest under `options.path` cannot be loaded, [`CliError::DataHome`] when the
/// Azoth data home cannot be prepared or migrated, any error
/// [`crate::commands::session::stop_services`] returns while stopping active project
/// sessions, and [`CliError::Io`] when a scoped directory cannot be removed.
#[instrument(skip(options))]
pub fn execute(options: &CleanOptions) -> CliResult<()> {
    let project_root = options.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let selection = CleanSelection::from_options(options)?;
    let manifest = az_project::load_project_manifest(&project_root)?;
    let data_home = AzothDataHome::resolve();
    data_home.prepare()?;
    let paths = data_home.project(&manifest.project.name, &project_root);
    let migration = paths.prepare()?;
    if migration.copied_files > 0 {
        info!(
            copied_files = migration.copied_files,
            project = %manifest.project.id,
            "migrated legacy project state before scoped cleanup"
        );
    }

    stop_active_project_sessions(&project_root)?;
    let report = clean_project_data(&paths, &selection)?;

    if report.removed.is_empty() {
        println!(
            "Azoth data already clean for '{}' ({})",
            manifest.project.name,
            paths.root().display()
        );
    } else {
        println!("Cleaned Azoth data for '{}':", manifest.project.name);
        for path in &report.removed {
            println!("  {}", path.display());
        }
    }
    println!("Cargo target directory was not touched");
    Ok(())
}

fn stop_active_project_sessions(project_root: &Path) -> CliResult<()> {
    let manager = SessionManager::new(project_root)?;
    let active = manager
        .list_sessions()?
        .into_iter()
        .filter(|manifest| {
            manifest.processes.iter().any(|process| {
                matches!(
                    process.state,
                    ServiceProcessState::Starting | ServiceProcessState::Running
                )
            })
        })
        .map(|manifest| manifest.slug)
        .collect::<Vec<_>>();

    for session in active {
        info!(%session, "stopping active project session before Azoth cleanup");
        crate::commands::session::stop_services(
            &session,
            Some("azoth clean".to_string()),
            true,
            CLEAN_SESSION_SHUTDOWN_TIMEOUT_MS,
            None,
            None,
            Some(project_root.to_path_buf()),
        )?;
    }
    Ok(())
}

fn clean_project_data(
    paths: &ProjectDataPaths,
    selection: &CleanSelection,
) -> io::Result<CleanReport> {
    let mut report = CleanReport::default();
    if selection.scopes.contains(CleanScope::AssetDb) {
        remove_owned_path(paths.root(), &paths.asset_db_dir(), &mut report)?;
    }
    if selection.scopes.contains(CleanScope::Products) {
        if let Some(platform) = &selection.platform {
            let platform_dir = paths
                .product_cache_dir(platform)
                .map_err(io::Error::other)?;
            remove_owned_path(paths.root(), &platform_dir, &mut report)?;
            remove_if_empty(&paths.product_cache_root())?;
        } else {
            remove_owned_path(paths.root(), &paths.product_cache_root(), &mut report)?;
        }
    }
    if selection.scopes.contains(CleanScope::Derived) {
        remove_owned_path(paths.root(), &paths.derived_dir(), &mut report)?;
    }
    if selection.scopes.contains(CleanScope::Endpoints) {
        remove_owned_path(paths.root(), &paths.endpoints_dir(), &mut report)?;
    }
    if selection.scopes.contains(CleanScope::Sessions) {
        clean_ephemeral_sessions(paths, &mut report)?;
    }
    Ok(report)
}

fn clean_ephemeral_sessions(paths: &ProjectDataPaths, report: &mut CleanReport) -> io::Result<()> {
    remove_owned_path(paths.root(), &paths.sessions_dir(), report)
}

fn remove_owned_path(root: &Path, path: &Path, report: &mut CleanReport) -> io::Result<()> {
    if path == root || !path.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to remove {} outside project data root {}",
                path.display(),
                root.display()
            ),
        ));
    }

    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    report.removed.push(path.to_path_buf());
    Ok(())
}

fn remove_if_empty(path: &Path) -> io::Result<()> {
    let mut entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if entries.next().is_none() {
        fs::remove_dir(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _temp: tempfile::TempDir,
        home: AzothDataHome,
        paths: ProjectDataPaths,
        project_root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let project_root = temp.path().join("project");
            fs::create_dir_all(&project_root).unwrap();
            fs::write(project_root.join("authored.txt"), b"authored").unwrap();
            let home = AzothDataHome::new(temp.path().join("home"));
            let paths = home.project("local.clean_test", &project_root);
            write_fixture_file(&paths.asset_db_path());
            write_fixture_file(&paths.default_product_cache_dir().join("product.bin"));
            write_fixture_file(&paths.product_cache_dir("ios").unwrap().join("product.bin"));
            write_fixture_file(&paths.graphs_dir().join("graph.rs"));
            write_fixture_file(&paths.editor_dir().join("settings.toml"));
            write_fixture_file(&paths.endpoints_dir().join("azd.endpoint.toml"));
            write_fixture_file(&paths.sessions_dir().join("session-a/manifest.toml"));
            write_fixture_file(&home.preferences_dir().join("editor.toml"));
            write_fixture_file(&home.themes_dir().join("custom.toml"));
            Self {
                _temp: temp,
                home,
                paths,
                project_root,
            }
        }

        fn clean(&self, selection: &CleanSelection) -> CleanReport {
            clean_project_data(&self.paths, selection).unwrap()
        }
    }

    fn write_fixture_file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"fixture").unwrap();
    }

    fn only(scope: CleanScope) -> CleanSelection {
        CleanSelection {
            scopes: std::iter::once(scope).collect(),
            platform: None,
        }
    }

    #[test]
    fn assetdb_scope_removes_only_asset_database() {
        let fixture = Fixture::new();

        fixture.clean(&only(CleanScope::AssetDb));

        assert!(!fixture.paths.asset_db_dir().exists());
        assert!(fixture.paths.default_product_cache_dir().exists());
        assert!(fixture.paths.derived_dir().exists());
    }

    #[test]
    fn products_scope_can_remove_one_platform() {
        let fixture = Fixture::new();
        let mut selection = only(CleanScope::Products);
        selection.platform = Some("pc".to_string());

        fixture.clean(&selection);

        assert!(!fixture.paths.default_product_cache_dir().exists());
        assert!(fixture.paths.product_cache_dir("ios").unwrap().exists());
        assert!(fixture.paths.asset_db_path().exists());
    }

    #[test]
    fn derived_scope_removes_graph_editor_and_project_host_state() {
        let fixture = Fixture::new();

        fixture.clean(&only(CleanScope::Derived));

        assert!(!fixture.paths.derived_dir().exists());
        assert!(fixture.paths.asset_db_path().exists());
    }

    #[test]
    fn endpoints_scope_removes_only_endpoint_state() {
        let fixture = Fixture::new();

        fixture.clean(&only(CleanScope::Endpoints));

        assert!(!fixture.paths.endpoints_dir().exists());
        assert!(fixture.paths.sessions_dir().exists());
    }

    #[test]
    fn sessions_scope_removes_only_session_run_state() {
        let fixture = Fixture::new();

        fixture.clean(&only(CleanScope::Sessions));

        assert!(!fixture.paths.sessions_dir().exists());
        assert!(fixture.project_root.join("authored.txt").exists());
    }

    #[test]
    fn default_scope_cleans_all_regenerable_project_data_only() {
        let fixture = Fixture::new();
        let selection = CleanSelection::from_options(&CleanOptions::default()).unwrap();

        fixture.clean(&selection);

        assert!(!fixture.paths.asset_db_dir().exists());
        assert!(!fixture.paths.product_cache_root().exists());
        assert!(!fixture.paths.derived_dir().exists());
        assert!(!fixture.paths.endpoints_dir().exists());
        assert!(!fixture.paths.sessions_dir().join("session-a").exists());
        assert!(fixture.project_root.join("authored.txt").exists());
        assert!(fixture.home.preferences_dir().join("editor.toml").exists());
        assert!(fixture.home.themes_dir().join("custom.toml").exists());
    }

    #[test]
    fn platform_without_products_is_rejected() {
        let options = CleanOptions {
            platform: Some("pc".to_string()),
            ..CleanOptions::default()
        };

        assert!(matches!(
            CleanSelection::from_options(&options),
            Err(CliError::InvalidArgument { .. })
        ));
    }
}
