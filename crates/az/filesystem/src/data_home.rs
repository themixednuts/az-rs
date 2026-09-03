//! Machine-local Azoth data-home layout and migration policy.
//!
//! Cargo owns project `target/` directories. Azoth owns this layout, rooted at
//! `AZOTH_HOME` or `~/.azoth`, so asset products and editor/session state do
//! not disappear during `cargo clean`.
//!
//! Per-instance project state is keyed by the human project name plus the Lore
//! repository instance id at that working tree:
//! `projects/<project-name>/<lore-instance-id>/`.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::normalize;

/// Versioned project-key algorithm required by ADR 0030.
pub const PROJECT_MACHINE_KEY_VERSION: &str = "ProjectMachineKey/v2";

/// Lore stores the repository instance id as 16 raw bytes in `.lore/instance`.
const LORE_INSTANCE_FILE: &str = "instance";
const LORE_DIR_NAME: &str = ".lore";
const UNLINKED_INSTANCE_PREFIX: &str = "unlinked-";

/// Environment override for the machine-local Azoth data home.
pub const AZOTH_HOME_ENV: &str = "AZOTH_HOME";

/// Default Azoth data-home directory name beneath the user's home directory.
pub const AZOTH_HOME_DIR_NAME: &str = ".azoth";

/// Per-project machine-state directory beneath the Azoth data home.
pub const PROJECTS_STATE_DIR_NAME: &str = "projects";

/// O3DE-compatible directory name for cooked products.
pub const PRODUCT_CACHE_DIR_NAME: &str = "Cache";

/// Legacy project-local Azoth state directory name.
pub const AZOTH_STATE_DIR_NAME: &str = ".azoth";

/// Asset registry directory beneath one project machine-state entry.
pub const ASSET_DB_DIR_NAME: &str = "assetdb";

/// Durable asset registry database file name.
pub const ASSET_DB_FILE_NAME: &str = "assetdb.sqlite";

/// Default desktop asset platform until launch has an explicit platform.
pub const DEFAULT_ASSET_PLATFORM: &str = "pc";

const PREFERENCES_DIR_NAME: &str = "preferences";
const THEMES_DIR_NAME: &str = "themes";
const RUNTIME_DIR_NAME: &str = "runtime";
const DERIVED_DIR_NAME: &str = "derived";
const ENDPOINTS_DIR_NAME: &str = "endpoints";
const SESSIONS_DIR_NAME: &str = "sessions";
const SERVICES_DIR_NAME: &str = "services";
const STAGING_DIR_NAME: &str = "staging";
const SECRETS_DIR_NAME: &str = "secrets";
const PROJECT_MIGRATION_MARKER: &str = "project-root-v1.complete";
const GLOBAL_MIGRATION_MARKER: &str = "data-home-v1.complete";

/// Failure to validate or migrate the machine-local data-home layout.
#[derive(Debug, Error)]
pub enum DataHomeError {
    #[error("failed to inspect data-home path {path}: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to create data-home directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to read legacy data-home file {path}: {source}")]
    ReadLegacy {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to write migrated data-home file {path}: {source}")]
    WriteMigrated {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("legacy data-home entry {path} is a symbolic link or unsupported file type")]
    UnsupportedLegacyEntry { path: PathBuf },

    #[error(
        "legacy data-home entry {source_path} conflicts with existing destination {destination}"
    )]
    MigrationConflict {
        source_path: PathBuf,
        destination: PathBuf,
    },

    #[error("data-home component `{value}` is not one safe path component")]
    InvalidComponent { value: String },

    #[error(
        "Azoth project data {project_data} must not overlap Cargo target directory {target_dir}"
    )]
    CargoDomainOverlap {
        project_data: PathBuf,
        target_dir: PathBuf,
    },
}

/// Stable readable key for one Lore repository instance of a project.
///
/// `ProjectMachineKey/v2` layout under `~/.azoth/projects` is:
/// `<sanitized-project-name>/<lore-instance-id>/`.
///
/// The project-name segment is display-oriented (spaces → hyphens). The
/// instance segment is the `UUIDv7` from `.lore/instance` when present. Trees
/// without Lore yet (scaffold / tests) fall back to `unlinked-<path-fnv1a64>`
/// so two unlinked checkouts stay isolated until a real Lore instance id
/// exists.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectMachineKey {
    project_name: String,
    instance_id: String,
    /// Compact instance discriminator for IPC names (no path separators).
    instance_key: String,
}

impl ProjectMachineKey {
    #[must_use]
    pub fn derive(project_name: &str, project_root: &Path) -> Self {
        let project_name = sanitize_project_name_component(project_name);
        let instance_id = read_lore_instance_id(project_root)
            .unwrap_or_else(|| unlinked_instance_id(project_root));
        let instance_key = compact_instance_key(&instance_id);
        Self {
            project_name,
            instance_id,
            instance_key,
        }
    }

    /// Relative path beneath `projects/`: `<name>/<instance-id>`.
    #[must_use]
    pub fn relative_path(&self) -> PathBuf {
        PathBuf::from(&self.project_name).join(&self.instance_id)
    }

    #[must_use]
    pub fn project_name(&self) -> &str {
        &self.project_name
    }

    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Slash-joined `<name>/<instance-id>` for logs.
    #[must_use]
    pub fn as_str(&self) -> String {
        format!("{}/{}", self.project_name, self.instance_id)
    }

    /// Short IPC-safe discriminator (Lore UUID without dashes, or unlinked hash).
    #[must_use]
    pub fn hash_suffix(&self) -> &str {
        &self.instance_key
    }
}

impl fmt::Display for ProjectMachineKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.project_name, self.instance_id)
    }
}

/// Typed root for all machine-local Azoth data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzothDataHome {
    root: PathBuf,
    migrate_machine_legacy_locations: bool,
}

impl AzothDataHome {
    /// Resolve `AZOTH_HOME`, then `~/.azoth`, with a service-safe temp fallback.
    #[must_use]
    pub fn resolve() -> Self {
        Self {
            root: normalize(&resolve_azoth_home_dir()),
            migrate_machine_legacy_locations: true,
        }
    }

    /// Construct an explicit data home, primarily for embedding and tests.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            root: normalize(&root),
            migrate_machine_legacy_locations: false,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn projects_dir(&self) -> PathBuf {
        self.root.join(PROJECTS_STATE_DIR_NAME)
    }

    #[must_use]
    pub fn preferences_dir(&self) -> PathBuf {
        self.root.join(PREFERENCES_DIR_NAME)
    }

    #[must_use]
    pub fn themes_dir(&self) -> PathBuf {
        self.root.join(THEMES_DIR_NAME)
    }

    #[must_use]
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join(RUNTIME_DIR_NAME)
    }

    #[must_use]
    pub fn editor_preferences_path(&self) -> PathBuf {
        self.preferences_dir().join("editor.toml")
    }

    #[must_use]
    pub fn recent_projects_path(&self) -> PathBuf {
        self.preferences_dir().join("recent-projects.json")
    }

    #[must_use]
    pub fn project_manager_preferences_path(&self) -> PathBuf {
        self.preferences_dir().join("project-manager.sqlite3")
    }

    #[must_use]
    pub fn daemon_endpoint_record_path(&self) -> PathBuf {
        self.runtime_dir().join("azd.endpoint.toml")
    }

    #[must_use]
    pub fn daemon_project_registry_path(&self) -> PathBuf {
        self.runtime_dir().join("azd.projects.toml")
    }

    #[must_use]
    pub fn engine_installations_dir(&self) -> PathBuf {
        self.runtime_dir().join("engines")
    }

    /// Resolve machine-local paths for one project working tree.
    ///
    /// `project_name` should be the human `[project].name` when available. A
    /// package id is accepted too and sanitized the same way. Isolation across
    /// checkouts comes from the Lore instance id at `project_root`.
    #[must_use]
    pub fn project(&self, project_name: &str, project_root: &Path) -> ProjectDataPaths {
        let key = ProjectMachineKey::derive(project_name, project_root);
        let root = self.projects_dir().join(key.relative_path());
        ProjectDataPaths {
            previous_root: None,
            root,
            project_root: normalize(project_root),
            key,
        }
    }

    /// Lazily import known pre-ADR-0030 user-wide state without deleting it.
    ///
    /// # Errors
    ///
    /// Returns [`DataHomeError`] when the one-time global state migration fails.
    pub fn prepare(&self) -> Result<MigrationReport, DataHomeError> {
        let marker = self
            .runtime_dir()
            .join("migrations")
            .join(GLOBAL_MIGRATION_MARKER);
        migrate_once(&marker, &self.global_legacy_mappings())
    }

    fn global_legacy_mappings(&self) -> Vec<(PathBuf, PathBuf)> {
        let legacy_editor = self.root.join("editor");
        let mut mappings = vec![
            (
                legacy_editor.join("settings.toml"),
                self.editor_preferences_path(),
            ),
            (
                legacy_editor.join("recent-projects.json"),
                self.recent_projects_path(),
            ),
            (
                legacy_editor.join("project-manager.sqlite3"),
                self.project_manager_preferences_path(),
            ),
            (
                legacy_editor.join("project-manager.sqlite3-wal"),
                self.preferences_dir().join("project-manager.sqlite3-wal"),
            ),
            (
                legacy_editor.join("project-manager.sqlite3-shm"),
                self.preferences_dir().join("project-manager.sqlite3-shm"),
            ),
            (self.root.join("engines"), self.engine_installations_dir()),
        ];

        if self.migrate_machine_legacy_locations {
            if let Some(source) = legacy_daemon_registry_candidates()
                .into_iter()
                .find(|path| path.exists())
            {
                mappings.push((source, self.daemon_project_registry_path()));
            }
            mappings.push((
                std::env::temp_dir().join("azoth").join("azd.endpoint.toml"),
                self.daemon_endpoint_record_path(),
            ));
        }
        mappings
    }
}

impl Default for AzothDataHome {
    fn default() -> Self {
        Self::resolve()
    }
}

/// Typed machine-local layout for one project key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDataPaths {
    root: PathBuf,
    previous_root: Option<PathBuf>,
    project_root: PathBuf,
    key: ProjectMachineKey,
}

impl ProjectDataPaths {
    #[must_use]
    pub const fn key(&self) -> &ProjectMachineKey {
        &self.key
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    #[must_use]
    pub fn asset_db_dir(&self) -> PathBuf {
        self.root.join(ASSET_DB_DIR_NAME)
    }

    #[must_use]
    pub fn asset_db_path(&self) -> PathBuf {
        self.asset_db_dir().join(ASSET_DB_FILE_NAME)
    }

    #[must_use]
    pub fn product_cache_root(&self) -> PathBuf {
        self.root.join(PRODUCT_CACHE_DIR_NAME)
    }

    /// Resolve one platform product cache while rejecting path traversal.
    ///
    /// # Errors
    ///
    /// Returns [`DataHomeError`] when `platform` is not a safe path component.
    pub fn product_cache_dir(&self, platform: &str) -> Result<PathBuf, DataHomeError> {
        validate_component(platform)?;
        Ok(self.product_cache_root().join(platform))
    }

    #[must_use]
    pub fn default_product_cache_dir(&self) -> PathBuf {
        self.product_cache_root().join(DEFAULT_ASSET_PLATFORM)
    }

    #[must_use]
    pub fn derived_dir(&self) -> PathBuf {
        self.root.join(DERIVED_DIR_NAME)
    }

    #[must_use]
    pub fn graphs_dir(&self) -> PathBuf {
        self.derived_dir().join("graphs")
    }

    #[must_use]
    pub fn editor_dir(&self) -> PathBuf {
        self.derived_dir().join("editor")
    }

    #[must_use]
    pub fn editor_settings_path(&self) -> PathBuf {
        self.editor_dir().join("settings.toml")
    }

    #[must_use]
    pub fn project_host_dir(&self) -> PathBuf {
        self.derived_dir().join("project-host")
    }

    #[must_use]
    pub fn project_host_transactions_dir(&self) -> PathBuf {
        self.project_host_dir().join("transactions")
    }

    #[must_use]
    pub fn endpoints_dir(&self) -> PathBuf {
        self.root.join(ENDPOINTS_DIR_NAME)
    }

    #[must_use]
    pub fn daemon_endpoint_record_path(&self) -> PathBuf {
        self.endpoints_dir().join("azd.endpoint.toml")
    }

    #[must_use]
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join(SESSIONS_DIR_NAME)
    }

    /// Durable supervision state for services owned by this project instance.
    #[must_use]
    pub fn services_dir(&self) -> PathBuf {
        self.root.join(SERVICES_DIR_NAME)
    }

    /// Ephemeral asset-worker staging owned by the project instance.
    ///
    /// This deliberately lives outside every session run directory so retiring
    /// an editor/runtime session cannot invalidate an active worker lease.
    #[must_use]
    pub fn asset_processing_staging_dir(&self) -> PathBuf {
        self.root.join(STAGING_DIR_NAME).join("asset-processor")
    }

    /// Machine-local secret values owned by this Lore project instance.
    ///
    /// The directory is deliberately separate from regenerable derived state
    /// and from shareable project configuration. Values placed here survive
    /// project synchronization and never enter source control or generated
    /// target sources.
    #[must_use]
    pub fn secrets_dir(&self) -> PathBuf {
        self.root.join(SECRETS_DIR_NAME)
    }

    /// Reject an `AZOTH_HOME` override that crosses into Cargo's domain.
    ///
    /// # Errors
    ///
    /// Returns [`DataHomeError`] when the data root and `target_dir` overlap.
    pub fn ensure_outside_target_dir(&self, target_dir: &Path) -> Result<(), DataHomeError> {
        let project_data = comparable_path(&self.root);
        let target_dir = comparable_path(target_dir);
        if project_data.starts_with(&target_dir) || target_dir.starts_with(&project_data) {
            return Err(DataHomeError::CargoDomainOverlap {
                project_data: self.root.clone(),
                target_dir,
            });
        }
        Ok(())
    }

    /// Lazily copy known project-root state into this machine-home key.
    ///
    /// # Errors
    ///
    /// Returns [`DataHomeError`] when the one-time state migration fails.
    pub fn prepare(&self) -> Result<MigrationReport, DataHomeError> {
        let marker = self.root.join(PROJECT_MIGRATION_MARKER);
        migrate_once(&marker, &self.legacy_mappings())
    }

    fn legacy_mappings(&self) -> Vec<(PathBuf, PathBuf)> {
        let legacy = self.project_root.join(AZOTH_STATE_DIR_NAME);
        let mut mappings = Vec::new();
        if let Some(previous_root) = &self.previous_root {
            mappings.push((previous_root.clone(), self.root.clone()));
        }
        mappings.extend([
            (legacy.join(ASSET_DB_DIR_NAME), self.asset_db_dir()),
            (
                self.project_root.join(PRODUCT_CACHE_DIR_NAME),
                self.product_cache_root(),
            ),
            (legacy.join("generated").join("graphs"), self.graphs_dir()),
            (legacy.join("derived").join("graphs"), self.graphs_dir()),
            (legacy.join("editor"), self.editor_dir()),
            (legacy.join("derived").join("editor"), self.editor_dir()),
            (legacy.join("project-host"), self.project_host_dir()),
            (
                legacy.join("derived").join("project-host"),
                self.project_host_dir(),
            ),
            (
                legacy.join("transactions"),
                self.project_host_transactions_dir(),
            ),
            (
                legacy.join("journal"),
                self.project_host_dir().join("journal"),
            ),
            (
                legacy.join("journals"),
                self.project_host_dir().join("journals"),
            ),
            (legacy.join("endpoints"), self.endpoints_dir()),
            (
                legacy.join("azd.endpoint.toml"),
                self.daemon_endpoint_record_path(),
            ),
            (legacy.join("sessions"), self.sessions_dir()),
            (legacy.join("runs"), self.sessions_dir()),
        ]);
        mappings
    }
}

/// Result of one bounded lazy migration pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    pub copied_files: usize,
    pub already_completed: bool,
}

/// Resolve the active machine-local data-home root.
#[must_use]
pub fn azoth_home_dir() -> PathBuf {
    AzothDataHome::resolve().root
}

/// Resolve one project's machine-local state root.
#[must_use]
pub fn project_state_dir(project_name: &str, project_root: &Path) -> PathBuf {
    AzothDataHome::resolve()
        .project(project_name, project_root)
        .root
}

/// Resolve one project's durable asset registry path.
#[must_use]
pub fn project_asset_db_path(project_name: &str, project_root: &Path) -> PathBuf {
    AzothDataHome::resolve()
        .project(project_name, project_root)
        .asset_db_path()
}

/// Resolve one project's processed-product cache for a validated platform id.
#[must_use]
pub fn project_platform_product_cache_dir(
    project_name: &str,
    project_root: &Path,
    platform: &str,
) -> PathBuf {
    AzothDataHome::resolve()
        .project(project_name, project_root)
        .product_cache_root()
        .join(platform)
}

/// Resolve one project's default desktop processed-product cache.
#[must_use]
pub fn project_default_platform_product_cache_dir(
    project_name: &str,
    project_root: &Path,
) -> PathBuf {
    AzothDataHome::resolve()
        .project(project_name, project_root)
        .default_product_cache_dir()
}

/// Resolve project-instance staging owned by the asset-processor worker pool.
#[must_use]
pub fn project_asset_processing_staging_dir(project_name: &str, project_root: &Path) -> PathBuf {
    AzothDataHome::resolve()
        .project(project_name, project_root)
        .asset_processing_staging_dir()
}

/// Legacy project-relative asset DB helper retained for fixture adapters.
#[must_use]
pub fn asset_db_path(project_path: &Path) -> PathBuf {
    project_path
        .join(AZOTH_STATE_DIR_NAME)
        .join(ASSET_DB_DIR_NAME)
        .join(ASSET_DB_FILE_NAME)
}

/// Legacy project-relative product-cache helper retained for fixture adapters.
#[must_use]
pub fn platform_product_cache_dir(project_path: &Path, platform: &str) -> PathBuf {
    project_path.join(PRODUCT_CACHE_DIR_NAME).join(platform)
}

/// Legacy project-relative default product-cache helper retained for fixtures.
#[must_use]
pub fn default_platform_product_cache_dir(project_path: &Path) -> PathBuf {
    platform_product_cache_dir(project_path, DEFAULT_ASSET_PLATFORM)
}

/// Read the Lore repository instance id from `.lore/instance` when present.
#[must_use]
pub fn read_lore_instance_id(project_root: &Path) -> Option<String> {
    let path = project_root.join(LORE_DIR_NAME).join(LORE_INSTANCE_FILE);
    let bytes = fs::read(&path).ok()?;
    let bytes: [u8; 16] = bytes.as_slice().try_into().ok()?;
    Some(format_uuid_bytes(&bytes))
}

// TODO(lore-auto-create): `unlinked-*` is a temporary machine-key fallback for
// trees without `.lore/instance`. Replace it with a design that auto-creates the
// Lore repository (shared store, remote URL, first commit) for editor create,
// CLI `azoth new`/`init`, and open-existing flows so every real project lands
// on a stable Lore instance id instead of this path hash. See
// `az_project_scaffold::new::ensure_project_lore_checkpoint`.
fn unlinked_instance_id(project_root: &Path) -> String {
    let normalized = normalized_project_root_for_key(project_root);
    format!(
        "{UNLINKED_INSTANCE_PREFIX}{:016x}",
        fnv1a64(normalized.as_bytes())
    )
}

fn compact_instance_key(instance_id: &str) -> String {
    instance_id
        .chars()
        .filter(|ch| *ch != '-')
        .collect::<String>()
}

fn format_uuid_bytes(bytes: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

fn resolve_azoth_home_dir() -> PathBuf {
    if let Some(value) = std::env::var_os(AZOTH_HOME_ENV)
        && !value.is_empty()
    {
        return PathBuf::from(value);
    }

    directories::BaseDirs::new().map_or_else(
        || std::env::temp_dir().join("azoth"),
        |dirs| dirs.home_dir().join(AZOTH_HOME_DIR_NAME),
    )
}

fn normalized_project_root_for_key(project_root: &Path) -> String {
    let normalized = normalize(project_root).to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn comparable_path(path: &Path) -> PathBuf {
    let normalized = normalize(path);
    if cfg!(windows) {
        // `OsStr::to_ascii_lowercase` folds without a UTF-8 round trip. Going
        // through `to_string_lossy` would replace a name Windows accepts but
        // Rust cannot decode with U+FFFD, and two unrelated roots that differ
        // only in such a name would then compare equal.
        PathBuf::from(normalized.as_os_str().to_ascii_lowercase())
    } else {
        normalized
    }
}

/// Sanitize a project display name (or package id) into one path component.
///
/// Separators become hyphens.
fn sanitize_project_name_component(value: &str) -> String {
    let value = value.trim();
    let mut out = String::with_capacity(value.len().max(7));
    let mut last_was_separator = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            out.push('-');
            last_was_separator = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    while out.starts_with('-') {
        out.remove(0);
    }
    if out.is_empty() {
        out.push_str("project");
    }
    out
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn validate_component(value: &str) -> Result<(), DataHomeError> {
    let path = Path::new(value);
    let mut components = path.components();
    let valid = !value.trim().is_empty()
        && components.next().is_some()
        && components.next().is_none()
        && path.file_name().is_some_and(|name| name == value)
        && value != "."
        && value != "..";
    if valid {
        Ok(())
    } else {
        Err(DataHomeError::InvalidComponent {
            value: value.to_string(),
        })
    }
}

fn migrate_once(
    marker: &Path,
    mappings: &[(PathBuf, PathBuf)],
) -> Result<MigrationReport, DataHomeError> {
    if marker.exists() {
        return Ok(MigrationReport {
            already_completed: true,
            ..MigrationReport::default()
        });
    }

    let mut report = MigrationReport::default();
    for (source, destination) in mappings {
        if source == destination || !source.exists() {
            continue;
        }
        copy_legacy_entry(source, destination, &mut report)?;
    }

    create_parent(marker)?;
    fs::write(marker, format!("{PROJECT_MACHINE_KEY_VERSION}\n")).map_err(|source| {
        DataHomeError::WriteMigrated {
            path: marker.to_path_buf(),
            source,
        }
    })?;
    Ok(report)
}

fn copy_legacy_entry(
    source: &Path,
    destination: &Path,
    report: &mut MigrationReport,
) -> Result<(), DataHomeError> {
    let metadata = fs::symlink_metadata(source).map_err(|source_error| DataHomeError::Inspect {
        path: source.to_path_buf(),
        source: source_error,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(DataHomeError::UnsupportedLegacyEntry {
            path: source.to_path_buf(),
        });
    }
    if metadata.is_file() {
        return copy_legacy_file(source, destination, report);
    }
    if !metadata.is_dir() {
        return Err(DataHomeError::UnsupportedLegacyEntry {
            path: source.to_path_buf(),
        });
    }

    fs::create_dir_all(destination).map_err(|source| DataHomeError::CreateDirectory {
        path: destination.to_path_buf(),
        source,
    })?;
    for entry in fs::read_dir(source).map_err(|source_error| DataHomeError::ReadLegacy {
        path: source.to_path_buf(),
        source: source_error,
    })? {
        let entry = entry.map_err(|source_error| DataHomeError::ReadLegacy {
            path: source.to_path_buf(),
            source: source_error,
        })?;
        copy_legacy_entry(&entry.path(), &destination.join(entry.file_name()), report)?;
    }
    Ok(())
}

fn copy_legacy_file(
    source: &Path,
    destination: &Path,
    report: &mut MigrationReport,
) -> Result<(), DataHomeError> {
    if destination.exists() {
        return if files_equal(source, destination)? {
            Ok(())
        } else {
            Err(DataHomeError::MigrationConflict {
                source_path: source.to_path_buf(),
                destination: destination.to_path_buf(),
            })
        };
    }

    create_parent(destination)?;
    let mut input = File::open(source).map_err(|source_error| DataHomeError::ReadLegacy {
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let mut output = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
    {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return if files_equal(source, destination)? {
                Ok(())
            } else {
                Err(DataHomeError::MigrationConflict {
                    source_path: source.to_path_buf(),
                    destination: destination.to_path_buf(),
                })
            };
        }
        Err(source_error) => {
            return Err(DataHomeError::WriteMigrated {
                path: destination.to_path_buf(),
                source: source_error,
            });
        }
    };
    io::copy(&mut input, &mut output).map_err(|source_error| DataHomeError::WriteMigrated {
        path: destination.to_path_buf(),
        source: source_error,
    })?;
    output
        .flush()
        .map_err(|source_error| DataHomeError::WriteMigrated {
            path: destination.to_path_buf(),
            source: source_error,
        })?;
    report.copied_files += 1;
    Ok(())
}

fn files_equal(left: &Path, right: &Path) -> Result<bool, DataHomeError> {
    let left_metadata = fs::metadata(left).map_err(|source| DataHomeError::Inspect {
        path: left.to_path_buf(),
        source,
    })?;
    let right_metadata = fs::metadata(right).map_err(|source| DataHomeError::Inspect {
        path: right.to_path_buf(),
        source,
    })?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }

    let left_path = left.to_path_buf();
    let right_path = right.to_path_buf();
    let mut left_reader =
        BufReader::new(
            File::open(left).map_err(|source| DataHomeError::ReadLegacy {
                path: left_path.clone(),
                source,
            })?,
        );
    let mut right_reader =
        BufReader::new(
            File::open(right).map_err(|source| DataHomeError::ReadLegacy {
                path: right_path.clone(),
                source,
            })?,
        );
    let mut left_buffer = [0_u8; 8 * 1024];
    let mut right_buffer = [0_u8; 8 * 1024];
    loop {
        let left_count =
            left_reader
                .read(&mut left_buffer)
                .map_err(|source| DataHomeError::ReadLegacy {
                    path: left_path.clone(),
                    source,
                })?;
        let right_count =
            right_reader
                .read(&mut right_buffer)
                .map_err(|source| DataHomeError::ReadLegacy {
                    path: right_path.clone(),
                    source,
                })?;
        if left_count != right_count {
            return Ok(false);
        }
        if left_buffer.get(..left_count) != right_buffer.get(..left_count) {
            return Ok(false);
        }
        if left_count == 0 {
            return Ok(true);
        }
    }
}

fn create_parent(path: &Path) -> Result<(), DataHomeError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|source| DataHomeError::CreateDirectory {
        path: parent.to_path_buf(),
        source,
    })
}

fn legacy_daemon_registry_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("AZOTH_DAEMON_PROJECT_REGISTRY")
        && !path.is_empty()
    {
        candidates.push(PathBuf::from(path));
    }
    #[cfg(windows)]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local_app_data)
                .join("Azoth")
                .join("azd.projects.toml"),
        );
    }
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
        candidates.push(
            PathBuf::from(state_home)
                .join("azoth")
                .join("azd.projects.toml"),
        );
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("azoth")
                .join("azd.projects.toml"),
        );
    }
    candidates.push(std::env::temp_dir().join("azoth").join("azd.projects.toml"));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_lore_instance(project_root: &Path, bytes: [u8; 16]) {
        let lore = project_root.join(LORE_DIR_NAME);
        fs::create_dir_all(&lore).unwrap();
        fs::write(lore.join(LORE_INSTANCE_FILE), bytes).unwrap();
    }

    #[test]
    fn layout_accessors_match_adr_0030() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let instance = [
            0x01, 0x9e, 0xe1, 0x08, 0x92, 0x9e, 0x70, 0x93, 0xbd, 0xf2, 0x47, 0x1e, 0xb9, 0x9a,
            0x9b, 0xd2,
        ];
        write_lore_instance(&project_root, instance);
        let home = AzothDataHome::new(temp.path().join("home"));
        let project = home.project("Sample Game", &project_root);

        assert_eq!(home.preferences_dir(), home.root().join("preferences"));
        assert_eq!(home.themes_dir(), home.root().join("themes"));
        assert_eq!(home.runtime_dir(), home.root().join("runtime"));
        assert_eq!(
            project.root(),
            home.projects_dir()
                .join("sample-game")
                .join("019ee108-929e-7093-bdf2-471eb99a9bd2")
        );
        assert_eq!(
            project.asset_db_path(),
            project.root().join("assetdb").join("assetdb.sqlite")
        );
        assert_eq!(
            project.default_product_cache_dir(),
            project.root().join("Cache").join("pc")
        );
        assert_eq!(project.graphs_dir(), project.root().join("derived/graphs"));
        assert_eq!(project.editor_dir(), project.root().join("derived/editor"));
        assert_eq!(
            project.project_host_dir(),
            project.root().join("derived/project-host")
        );
        assert_eq!(project.endpoints_dir(), project.root().join("endpoints"));
        assert_eq!(project.sessions_dir(), project.root().join("sessions"));
        assert_eq!(project.services_dir(), project.root().join("services"));
        assert_eq!(
            project.asset_processing_staging_dir(),
            project.root().join("staging/asset-processor")
        );
        assert_eq!(project.key().project_name(), "sample-game");
        assert_eq!(
            project.key().instance_id(),
            "019ee108-929e-7093-bdf2-471eb99a9bd2"
        );
    }

    #[test]
    fn project_machine_key_uses_lore_instance_not_path_hash() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let shared_instance = [
            0x01, 0x9e, 0xe1, 0x08, 0x92, 0x9e, 0x70, 0x93, 0xbd, 0xf2, 0x47, 0x1e, 0xb9, 0x9a,
            0x9b, 0xd2,
        ];
        write_lore_instance(&first, shared_instance);
        write_lore_instance(&second, shared_instance);

        let first_key = ProjectMachineKey::derive("Sample Game", &first);
        let second_key = ProjectMachineKey::derive("sample-game", &second);
        assert_eq!(first_key.instance_id(), second_key.instance_id());
        assert_eq!(first_key.project_name(), "sample-game");
        assert_eq!(second_key.project_name(), "sample-game");
        assert_eq!(first_key.hash_suffix().len(), 32);
        assert!(
            first_key
                .hash_suffix()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
    }

    #[test]
    fn unlinked_trees_are_isolated_by_path_until_lore_exists() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();

        let first_key = ProjectMachineKey::derive("game", &first);
        let second_key = ProjectMachineKey::derive("game", &second);
        assert_ne!(first_key.instance_id(), second_key.instance_id());
        assert!(
            first_key
                .instance_id()
                .starts_with(UNLINKED_INSTANCE_PREFIX)
        );
    }

    #[test]
    fn lazy_project_migration_is_copy_only_and_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let legacy = project_root.join(".azoth");
        fs::create_dir_all(legacy.join("assetdb")).unwrap();
        fs::create_dir_all(legacy.join("generated/graphs")).unwrap();
        fs::create_dir_all(project_root.join("Cache/pc")).unwrap();
        fs::write(legacy.join("assetdb/assetdb.sqlite"), b"db").unwrap();
        fs::write(legacy.join("generated/graphs/main.rs"), b"graph").unwrap();
        fs::write(legacy.join("azd.endpoint.toml"), b"endpoint").unwrap();
        fs::write(project_root.join("Cache/pc/product.bin"), b"product").unwrap();

        let paths = AzothDataHome::new(temp.path().join("home")).project("game", &project_root);
        let first = paths.prepare().unwrap();
        let second = paths.prepare().unwrap();

        assert_eq!(first.copied_files, 4);
        assert!(!first.already_completed);
        assert!(second.already_completed);
        assert_eq!(fs::read(paths.asset_db_path()).unwrap(), b"db");
        assert_eq!(
            fs::read(paths.default_product_cache_dir().join("product.bin")).unwrap(),
            b"product"
        );
        assert_eq!(
            fs::read(paths.graphs_dir().join("main.rs")).unwrap(),
            b"graph"
        );
        assert_eq!(
            fs::read(paths.daemon_endpoint_record_path()).unwrap(),
            b"endpoint"
        );
        assert!(legacy.join("assetdb/assetdb.sqlite").exists());
        assert!(project_root.join("Cache/pc/product.bin").exists());
    }

    #[test]
    fn lazy_global_migration_preserves_old_user_state() {
        let temp = tempfile::tempdir().unwrap();
        let home = AzothDataHome::new(temp.path().join(".azoth"));
        fs::create_dir_all(home.root().join("editor")).unwrap();
        fs::write(home.root().join("editor/settings.toml"), b"theme = 'old'").unwrap();

        let first = home.prepare().unwrap();
        let second = home.prepare().unwrap();

        assert_eq!(first.copied_files, 1);
        assert!(second.already_completed);
        assert_eq!(
            fs::read(home.editor_preferences_path()).unwrap(),
            b"theme = 'old'"
        );
        assert!(home.root().join("editor/settings.toml").exists());
    }

    #[test]
    fn cargo_domain_guard_rejects_data_home_beneath_target() {
        let temp = tempfile::tempdir().unwrap();
        // `comparable_path` normalizes both sides, but only the existing one
        // resolves an 8.3 short name. Starting from the canonical temp root
        // keeps the nested data home and the on-disk target dir spelled the
        // same way, so the guard is judged on containment, not on spelling.
        let project_root = normalize(temp.path()).join("project");
        let target = project_root.join("target");
        fs::create_dir_all(&target).unwrap();
        let paths = AzothDataHome::new(target.join("azoth-state")).project("game", &project_root);

        assert!(matches!(
            paths.ensure_outside_target_dir(&target),
            Err(DataHomeError::CargoDomainOverlap { .. })
        ));
    }

    #[test]
    fn cargo_domain_guard_rejects_target_beneath_project_data() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let paths =
            AzothDataHome::new(temp.path().join("machine-home")).project("game", &project_root);
        let target = paths.root().join("cargo-target");

        assert!(matches!(
            paths.ensure_outside_target_dir(&target),
            Err(DataHomeError::CargoDomainOverlap { .. })
        ));
    }

    #[test]
    fn stored_project_root_matches_a_separately_canonicalized_root() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let paths = AzothDataHome::new(temp.path().join("home")).project("game", &project_root);

        // Services that record a workspace root of their own resolve it with
        // `canonical`. A data home that kept the raw `fs::canonicalize` output
        // would hold a Windows verbatim `\\?\` root here, which compares
        // unequal to the same directory named any other way and takes
        // product-cache and worker paths down with it.
        assert_eq!(
            paths.project_root(),
            crate::canonical(&project_root).unwrap()
        );
        assert!(
            !paths.project_root().to_string_lossy().contains(r"\\?\"),
            "stored project root {} kept a verbatim prefix a plain temp dir does not need",
            paths.project_root().display()
        );
    }

    #[test]
    fn product_platform_rejects_path_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AzothDataHome::new(temp.path().join("home"))
            .project("game", &temp.path().join("project"));

        assert!(matches!(
            paths.product_cache_dir("../other"),
            Err(DataHomeError::InvalidComponent { .. })
        ));
    }
}
