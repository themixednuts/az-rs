//! Fingerprinted project-local Cargo patch-table projection.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{info, instrument};

use crate::atomic_write::atomic_write;
use crate::manifest::{
    AZOTH_ENGINE_ROOT_ENV, EngineCargoPatch, EngineManifest, GENERATED_CARGO_CONFIG_HEADER,
    PROJECT_LOCK_FILE, ProjectManifestError, StaleEnginePatchTable, load_engine_manifest,
    project_local_cargo_config_path, read_toml_manifest, resolve_engine_graph_at,
    resolve_engine_root, resolve_engine_root_at,
};

/// Version of the generated `.cargo/config.toml` header and body contract.
pub const ENGINE_PATCH_CONFIG_FORMAT_VERSION: u32 = 8;

const CONFIG_FORMAT_VERSION_KEY: &str = "config-format-version";
const SELECTED_ENGINE_ID_KEY: &str = "selected-engine-id";
const SELECTED_ENGINE_VERSION_KEY: &str = "selected-engine-version";
const SELECTED_ENGINE_ROOT_KEY: &str = "selected-engine-root";
const ENGINE_LOCK_REVISION_KEY: &str = "engine-lock-revision";
const ENGINE_CRATE_CATALOG_HASH_KEY: &str = "engine-crate-catalog-sha256";
const PROJECT_CARGO_GRAPH_HASH_KEY: &str = "project-cargo-graph-sha256";
const JOINED_ENGINE_PACKAGES_HASH_KEY: &str = "joined-engine-packages-sha256";
const PATCH_TABLE_HASH_KEY: &str = "patch-table-sha256";
const MISSING_TABLE_REVISION: &str = "missing-or-unstamped";
const EMPTY_GRAPH_MARKER: &[u8] = b"azoth-project-without-cargo-manifest";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Complete freshness stamp stored in the generated Cargo config header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnginePatchFingerprint {
    pub config_format_version: u32,
    pub selected_engine_id: String,
    pub selected_engine_version: String,
    pub selected_engine_root: String,
    pub engine_lock_revision: String,
    pub engine_crate_catalog_sha256: String,
    pub project_cargo_graph_sha256: String,
    pub joined_engine_packages_sha256: String,
    pub patch_table_sha256: String,
}

impl std::fmt::Display for EnginePatchFingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "v{}:engine={}@{}:rev={}:catalog={}:graph={}:joined={}:patches={}",
            self.config_format_version,
            self.selected_engine_id,
            self.selected_engine_version,
            self.engine_lock_revision,
            self.engine_crate_catalog_sha256,
            self.project_cargo_graph_sha256,
            self.joined_engine_packages_sha256,
            self.patch_table_sha256
        )
    }
}

/// Whether a patch-table sync changed the generated file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectEnginePatchSyncStatus {
    Unchanged,
    Regenerated,
}

/// Patch-set and fingerprint explanation returned by ensure/sync operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEnginePatchSyncReport {
    pub status: ProjectEnginePatchSyncStatus,
    pub config_path: PathBuf,
    pub old_fingerprint: Option<EnginePatchFingerprint>,
    pub fingerprint: EnginePatchFingerprint,
    pub added_packages: Vec<String>,
    pub removed_packages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EngineCrateCatalogEntry {
    package: String,
    version: String,
    registration_owner: String,
    contribution_identity: String,
    manifest_relative_source: String,
    manifest_checksum: String,
    manifest_path: PathBuf,
    root: PathBuf,
}

#[derive(Debug)]
struct EngineCrateCatalog {
    entries: BTreeMap<String, EngineCrateCatalogEntry>,
    sha256: String,
}

pub type CargoPatchSources = BTreeMap<String, BTreeMap<String, toml::Value>>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CargoPatchIdentity {
    source: String,
    package: String,
}

#[derive(Debug, Clone)]
struct FingerprintBase {
    config_format_version: u32,
    selected_engine_id: String,
    selected_engine_version: String,
    selected_engine_root: String,
    engine_lock_revision: String,
    engine_crate_catalog_sha256: String,
    project_cargo_graph_sha256: String,
}

impl FingerprintBase {
    fn finish(
        &self,
        joined_engine_packages_sha256: String,
        patch_table_sha256: String,
    ) -> EnginePatchFingerprint {
        EnginePatchFingerprint {
            config_format_version: self.config_format_version,
            selected_engine_id: self.selected_engine_id.clone(),
            selected_engine_version: self.selected_engine_version.clone(),
            selected_engine_root: self.selected_engine_root.clone(),
            engine_lock_revision: self.engine_lock_revision.clone(),
            engine_crate_catalog_sha256: self.engine_crate_catalog_sha256.clone(),
            project_cargo_graph_sha256: self.project_cargo_graph_sha256.clone(),
            joined_engine_packages_sha256,
            patch_table_sha256,
        }
    }

    fn matches(&self, fingerprint: &EnginePatchFingerprint) -> bool {
        self.config_format_version == fingerprint.config_format_version
            && self.selected_engine_id == fingerprint.selected_engine_id
            && self.selected_engine_version == fingerprint.selected_engine_version
            && self.selected_engine_root == fingerprint.selected_engine_root
            && self.engine_lock_revision == fingerprint.engine_lock_revision
            && self.engine_crate_catalog_sha256 == fingerprint.engine_crate_catalog_sha256
            && self.project_cargo_graph_sha256 == fingerprint.project_cargo_graph_sha256
    }
}

#[derive(Debug, Default)]
struct ExistingGeneratedConfig {
    contents: Option<String>,
    fingerprint: Option<EnginePatchFingerprint>,
    patches: CargoPatchSources,
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoMetadataPackage>,
    workspace_members: Vec<String>,
    workspace_root: PathBuf,
    resolve: Option<CargoMetadataResolve>,
    #[serde(skip)]
    used_patches: BTreeSet<CargoPatchIdentity>,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataPackage {
    id: String,
    name: String,
    version: String,
    source: Option<String>,
    dependencies: Vec<CargoMetadataDependency>,
    manifest_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataDependency {
    name: String,
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataResolve {
    nodes: Vec<CargoMetadataNode>,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataNode {
    id: String,
    dependencies: Vec<String>,
}

/// Idempotently ensures the selected engine's real project-graph patch table.
///
/// # Errors
///
/// Returns any error [`resolve_engine_root`] returns when `AZOTH_ENGINE_ROOT`
/// (or the default engine location) does not name a valid engine checkout, then
/// any error [`ensure_project_engine_patch_table_to`] returns for that root.
#[instrument(skip_all, fields(project_root = %project_root.as_ref().display()))]
pub fn ensure_project_engine_patch_table(
    project_root: impl AsRef<Path>,
) -> Result<ProjectEnginePatchSyncReport, ProjectManifestError> {
    let engine_root = resolve_engine_root()?;
    ensure_project_engine_patch_table_to(project_root, engine_root)
}

/// Idempotently ensures a project patch table for an explicit engine root.
///
/// A first pass that fails with [`ProjectManifestError::StaleEnginePatchTable`]
/// is retried once with recomputation forced, so a stale stamp repairs itself
/// rather than surfacing.
///
/// # Errors
///
/// Returns [`ProjectManifestError::LocalCargoConfigNotGenerated`] if the
/// project's `.cargo/config.toml` exists but was not written by this generator,
/// or [`ProjectManifestError::StaleEnginePatchTable`] if the forced repair pass
/// also fails — every other failure inside the sync (unreadable engine
/// manifest, `cargo metadata` failure, unwritable config, lock desynchronization)
/// is folded into that variant's `reason`.
#[instrument(skip_all, fields(project_root = %project_root.as_ref().display(), engine_root = %engine_root.as_ref().display()))]
pub fn ensure_project_engine_patch_table_to(
    project_root: impl AsRef<Path>,
    engine_root: impl AsRef<Path>,
) -> Result<ProjectEnginePatchSyncReport, ProjectManifestError> {
    let project_root = project_root.as_ref();
    let engine_root = engine_root.as_ref();
    match sync_project_engine_patch_table_to(project_root, engine_root, false) {
        Ok(report) => Ok(report),
        Err(error @ ProjectManifestError::StaleEnginePatchTable(..)) => {
            info!(
                project_root = %project_root.display(),
                reason = %error,
                "repairing stale engine patch table and validating Cargo lock"
            );
            sync_project_engine_patch_table_to(project_root, engine_root, true)
        }
        Err(error) => Err(error),
    }
}

/// Forces graph recomputation and reports the old/new patch projection.
///
/// # Errors
///
/// Returns any error [`resolve_engine_root`] returns for the ambient engine
/// selection, [`ProjectManifestError::LocalCargoConfigNotGenerated`] if the
/// project's `.cargo/config.toml` was not written by this generator, or
/// [`ProjectManifestError::StaleEnginePatchTable`], which carries the reason for
/// every other failure inside the forced recomputation.
#[instrument(skip_all, fields(project_root = %project_root.as_ref().display()))]
pub fn sync_project_engine_patch_table(
    project_root: impl AsRef<Path>,
) -> Result<ProjectEnginePatchSyncReport, ProjectManifestError> {
    let engine_root = resolve_engine_root()?;
    sync_project_engine_patch_table_to(project_root.as_ref(), &engine_root, true)
}

/// Forces an authored-workspace-only bootstrap projection.
///
/// Generated target packages are disposable and may describe capabilities or
/// Cargo features from the previous engine graph. Target regeneration uses
/// this projection only when that stale generated graph cannot be resolved;
/// it then materializes the current targets and performs the normal full
/// authored/generated sync.
///
/// # Errors
///
/// Returns any error [`resolve_engine_root`] returns for the ambient engine
/// selection, [`ProjectManifestError::LocalCargoConfigNotGenerated`] if the
/// project's `.cargo/config.toml` was not written by this generator, or
/// [`ProjectManifestError::StaleEnginePatchTable`], which carries the reason for
/// every other failure inside the forced recomputation.
#[instrument(skip_all, fields(project_root = %project_root.as_ref().display()))]
pub fn sync_project_authored_engine_patch_table(
    project_root: impl AsRef<Path>,
) -> Result<ProjectEnginePatchSyncReport, ProjectManifestError> {
    let engine_root = resolve_engine_root()?;
    sync_project_engine_patch_table_to_scope(project_root.as_ref(), &engine_root, true, false)
}

/// Parses a complete generated-header fingerprint.
#[must_use]
pub fn parse_project_engine_patch_fingerprint(config: &str) -> Option<EnginePatchFingerprint> {
    if !config.starts_with(GENERATED_CARGO_CONFIG_HEADER) {
        return None;
    }

    let fields = generated_header_fields(config);
    Some(EnginePatchFingerprint {
        config_format_version: fields.get(CONFIG_FORMAT_VERSION_KEY)?.parse().ok()?,
        selected_engine_id: fields.get(SELECTED_ENGINE_ID_KEY)?.clone(),
        selected_engine_version: fields.get(SELECTED_ENGINE_VERSION_KEY)?.clone(),
        selected_engine_root: fields.get(SELECTED_ENGINE_ROOT_KEY)?.clone(),
        engine_lock_revision: fields.get(ENGINE_LOCK_REVISION_KEY)?.clone(),
        engine_crate_catalog_sha256: fields.get(ENGINE_CRATE_CATALOG_HASH_KEY)?.clone(),
        project_cargo_graph_sha256: fields.get(PROJECT_CARGO_GRAPH_HASH_KEY)?.clone(),
        joined_engine_packages_sha256: fields.get(JOINED_ENGINE_PACKAGES_HASH_KEY)?.clone(),
        patch_table_sha256: fields.get(PATCH_TABLE_HASH_KEY)?.clone(),
    })
}

/// Every Cargo package the selected engine registers, as path patches.
///
/// # Errors
///
/// Returns [`ProjectManifestError::InvalidEngineRoot`] if `engine_root` is
/// empty, if its workspace names a glob member or a registered gem without a
/// Cargo package, if a catalogued source root is missing or escapes the engine,
/// or if a package declares no name/version or an unparsable semver;
/// [`ProjectManifestError::Read`] if the engine root cannot be canonicalized or
/// a manifest cannot be read; and [`ProjectManifestError::Parse`] if a manifest
/// is not valid TOML.
pub fn engine_cargo_patches(
    engine_root: impl AsRef<Path>,
) -> Result<Vec<EngineCargoPatch>, ProjectManifestError> {
    let engine_root = resolve_engine_root_at(engine_root)?;
    let engine_manifest = load_engine_manifest(&engine_root)?;
    Ok(build_engine_crate_catalog(&engine_root, &engine_manifest)?
        .entries
        .into_values()
        .map(|entry| EngineCargoPatch {
            package: entry.package,
            root: entry.root,
        })
        .collect())
}

/// The `[patch]` sources the engine workspace's own `Cargo.toml` declares.
///
/// # Errors
///
/// Returns any error [`resolve_engine_root_at`] returns if `engine_root` is not
/// a valid engine checkout, [`ProjectManifestError::Read`] if the engine
/// workspace `Cargo.toml` cannot be read, or [`ProjectManifestError::Parse`] if
/// it is not valid TOML.
pub fn engine_dependency_patches(
    engine_root: impl AsRef<Path>,
) -> Result<CargoPatchSources, ProjectManifestError> {
    let engine_root = resolve_engine_root_at(engine_root)?;
    load_engine_dependency_patches(&engine_root)
}

/// The patch table one Cargo manifest actually resolves against the engine.
///
/// Only patches Cargo reported as used survive, so the result is the manifest's
/// real overlay rather than the engine's full catalog.
///
/// # Errors
///
/// Returns any error [`engine_cargo_patches`] returns while cataloguing the
/// engine, plus [`ProjectManifestError::InvalidEngineRoot`] if `cargo metadata`
/// fails or returns JSON without a resolved graph, if the resolve references an
/// unknown package, if a resolved engine package's version disagrees with the
/// registered one, or if an engine package collides with a `crates-io`
/// dependency override the engine workspace already declares.
pub fn resolved_manifest_patch_table(
    engine_root: impl AsRef<Path>,
    manifest_path: &Path,
) -> Result<CargoPatchSources, ProjectManifestError> {
    let engine_root = resolve_engine_root_at(engine_root)?;
    let engine_manifest = load_engine_manifest(&engine_root)?;
    let catalog = build_engine_crate_catalog(&engine_root, &engine_manifest)?;
    let dependency_patches = load_engine_dependency_patches(&engine_root)?;
    let metadata = cargo_metadata_for_manifest(manifest_path, &engine_root, &catalog, true, None)?;
    let mut joined = join_project_metadata_to_engine_catalog(&metadata, &catalog)?;
    joined.retain(|package, _| {
        metadata.used_patches.contains(&CargoPatchIdentity {
            source: "crates-io".to_string(),
            package: package.clone(),
        })
    });
    let dependency_patches = retain_used_patches(&dependency_patches, &metadata.used_patches);
    compose_patch_sources(&joined, &dependency_patches)
}

/// Render the unfingerprinted bootstrap overlay for a candidate engine root.
///
/// # Errors
///
/// Returns any error [`engine_cargo_patches`] returns while cataloguing the
/// engine, plus [`ProjectManifestError::InvalidEngineRoot`] if a catalogued
/// engine package collides with a `crates-io` dependency override the engine
/// workspace already declares.
pub fn render_engine_candidate_cargo_config(
    engine_root: impl AsRef<Path>,
) -> Result<String, ProjectManifestError> {
    let engine_root = resolve_engine_root_at(engine_root)?;
    let engine_manifest = load_engine_manifest(&engine_root)?;
    let catalog = build_engine_crate_catalog(&engine_root, &engine_manifest)?;
    let dependency_patches = load_engine_dependency_patches(&engine_root)?;
    render_bootstrap_overlay(&engine_root, &catalog.entries, &dependency_patches)
}

/// Force a sync against `engine_root` and return the generated config text.
///
/// # Errors
///
/// Returns [`ProjectManifestError::LocalCargoConfigNotGenerated`] if the
/// project's `.cargo/config.toml` was not written by this generator,
/// [`ProjectManifestError::StaleEnginePatchTable`] carrying the reason for any
/// other failure inside the forced sync, or [`ProjectManifestError::Read`] if
/// the freshly written config cannot be read back.
pub fn render_project_engine_patch_table(
    project_root: impl AsRef<Path>,
    engine_root: impl AsRef<Path>,
) -> Result<String, ProjectManifestError> {
    let report =
        sync_project_engine_patch_table_to(project_root.as_ref(), engine_root.as_ref(), true)?;
    std::fs::read_to_string(&report.config_path).map_err(|source| ProjectManifestError::Read {
        path: report.config_path,
        source,
    })
}

fn sync_project_engine_patch_table_to(
    project_root: &Path,
    engine_root: &Path,
    force: bool,
) -> Result<ProjectEnginePatchSyncReport, ProjectManifestError> {
    sync_project_engine_patch_table_to_scope(project_root, engine_root, force, true)
}

fn sync_project_engine_patch_table_to_scope(
    project_root: &Path,
    engine_root: &Path,
    force: bool,
    _include_generated_workspace: bool,
) -> Result<ProjectEnginePatchSyncReport, ProjectManifestError> {
    let engine_root = resolve_engine_root_at(engine_root)?;
    let engine_manifest = load_engine_manifest(&engine_root)?;
    let config_path = project_local_cargo_config_path(project_root);
    let existing = read_existing_generated_config(&config_path)?;
    let table_revision = existing
        .fingerprint
        .as_ref()
        .map_or(MISSING_TABLE_REVISION, |fingerprint| {
            fingerprint.engine_lock_revision.as_str()
        })
        .to_string();
    let engine_revision = selected_engine_lock_revision(project_root, &engine_manifest)?;

    let projection = project_engine_patch_projection(
        &PatchTableScope {
            project_root,
            engine_root: &engine_root,
            config_path: &config_path,
            existing: &existing,
            force,
        },
        &engine_manifest,
        &engine_revision,
    );

    projection.map_err(|error| match error {
        ProjectManifestError::LocalCargoConfigNotGenerated { .. }
        | ProjectManifestError::StaleEnginePatchTable(..) => error,
        error => ProjectManifestError::StaleEnginePatchTable(Box::new(StaleEnginePatchTable {
            selected_engine: format!(
                "{}@{}",
                engine_manifest.engine.id, engine_manifest.engine.version
            ),
            engine_revision,
            table_revision,
            config_path,
            project_root: project_root.to_path_buf(),
            reason: error.to_string(),
        })),
    })
}

/// The unchanging inputs every stage of one patch-table sync shares.
struct PatchTableScope<'a> {
    project_root: &'a Path,
    engine_root: &'a Path,
    config_path: &'a Path,
    existing: &'a ExistingGeneratedConfig,
    force: bool,
}

/// Compute the fingerprint base, then either report the existing config as
/// fresh or regenerate the patch table.
fn project_engine_patch_projection(
    scope: &PatchTableScope<'_>,
    engine_manifest: &EngineManifest,
    engine_revision: &str,
) -> Result<ProjectEnginePatchSyncReport, ProjectManifestError> {
    let catalog = build_engine_crate_catalog(scope.engine_root, engine_manifest)?;
    let dependency_patches = load_engine_dependency_patches(scope.engine_root)?;
    let base = FingerprintBase {
        config_format_version: ENGINE_PATCH_CONFIG_FORMAT_VERSION,
        selected_engine_id: engine_manifest.engine.id.clone(),
        selected_engine_version: engine_manifest.engine.version.clone(),
        selected_engine_root: portable_path_string(scope.engine_root),
        engine_lock_revision: engine_revision.to_string(),
        engine_crate_catalog_sha256: catalog.sha256.clone(),
        project_cargo_graph_sha256: project_cargo_graph_hash(
            scope.project_root,
            scope.engine_root,
            &catalog,
        )?,
    };

    if !scope.force
        && let Some(fingerprint) = scope.existing.fingerprint.as_ref().filter(|fingerprint| {
            generated_config_is_fresh(
                &base,
                fingerprint,
                &scope.existing.patches,
                &catalog,
                &dependency_patches,
            )
        })
    {
        info!(
            config_path = %scope.config_path.display(),
            patch_count = scope.existing.patches.len(),
            "engine patch table is fresh"
        );
        return Ok(ProjectEnginePatchSyncReport {
            status: ProjectEnginePatchSyncStatus::Unchanged,
            config_path: scope.config_path.to_path_buf(),
            old_fingerprint: Some(fingerprint.clone()),
            fingerprint: fingerprint.clone(),
            added_packages: Vec::new(),
            removed_packages: Vec::new(),
        });
    }

    regenerate_engine_patch_table(scope, &catalog, &dependency_patches, &base)
}

/// Re-resolve the project's Cargo graph, render the patch table, and write it
/// out when the rendered bytes differ from what is already on disk.
fn regenerate_engine_patch_table(
    scope: &PatchTableScope<'_>,
    catalog: &EngineCrateCatalog,
    dependency_patches: &CargoPatchSources,
    base: &FingerprintBase,
) -> Result<ProjectEnginePatchSyncReport, ProjectManifestError> {
    let project_root = scope.project_root;
    let existing = scope.existing;
    let mut lock_manifests = Vec::new();
    let mut used_patches = BTreeSet::new();
    let mut joined = if project_root.join("Cargo.toml").is_file() {
        let metadata = project_cargo_metadata_preserving_lock(
            project_root,
            scope.engine_root,
            catalog,
            scope.force,
        )?;
        used_patches.extend(metadata.used_patches.iter().cloned());
        lock_manifests.push(project_root.join("Cargo.toml"));
        // Generated target packages are ordinary members of this metadata
        // graph. A second independent workspace/lock would diverge from
        // native Cargo and duplicate resolution work.
        join_project_metadata_to_engine_catalog(&metadata, catalog)?
    } else {
        BTreeMap::new()
    };
    joined.retain(|package, _| {
        used_patches.contains(&CargoPatchIdentity {
            source: "crates-io".to_string(),
            package: package.clone(),
        })
    });
    let dependency_patches = retain_used_patches(dependency_patches, &used_patches);
    let joined_hash = joined_engine_packages_hash(&joined);
    let patches = compose_patch_sources(&joined, &dependency_patches)?;
    let patch_hash = patch_table_hash(&patches);
    let fingerprint = base.finish(joined_hash, patch_hash);
    let rendered = render_generated_config(scope.engine_root, &fingerprint, &patches);
    let old_packages = patch_identities(&existing.patches);
    let new_packages = patch_identities(&patches);
    let added_packages = new_packages.difference(&old_packages).cloned().collect();
    let removed_packages = old_packages.difference(&new_packages).cloned().collect();

    let status = if existing.contents.as_deref() == Some(rendered.as_str()) {
        ProjectEnginePatchSyncStatus::Unchanged
    } else {
        atomic_write(scope.config_path, rendered.as_bytes())?;
        ProjectEnginePatchSyncStatus::Regenerated
    };
    for manifest_path in lock_manifests {
        synchronize_workspace_lock(project_root, &manifest_path)?;
    }
    info!(
        config_path = %scope.config_path.display(),
        patch_count = joined.len(),
        ?status,
        "engine patch table synchronized"
    );
    Ok(ProjectEnginePatchSyncReport {
        status,
        config_path: scope.config_path.to_path_buf(),
        old_fingerprint: existing.fingerprint.clone(),
        fingerprint,
        added_packages,
        removed_packages,
    })
}

fn read_existing_generated_config(
    config_path: &Path,
) -> Result<ExistingGeneratedConfig, ProjectManifestError> {
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ExistingGeneratedConfig::default());
        }
        Err(source) => {
            return Err(ProjectManifestError::Read {
                path: config_path.to_path_buf(),
                source,
            });
        }
    };
    if !contents.starts_with(GENERATED_CARGO_CONFIG_HEADER) {
        return Err(ProjectManifestError::LocalCargoConfigNotGenerated {
            path: config_path.to_path_buf(),
        });
    }

    let patches = parse_patch_paths(&contents).unwrap_or_default();
    Ok(ExistingGeneratedConfig {
        fingerprint: parse_project_engine_patch_fingerprint(&contents),
        patches,
        contents: Some(contents),
    })
}

fn generated_header_fields(config: &str) -> BTreeMap<String, String> {
    config
        .lines()
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("# "))
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
}

fn parse_patch_paths(config: &str) -> Option<CargoPatchSources> {
    let document = toml::from_str::<toml::Value>(config).ok()?;
    document
        .get("patch")?
        .as_table()?
        .iter()
        .map(|(source, packages)| {
            Some((
                source.clone(),
                packages
                    .as_table()?
                    .iter()
                    .map(|(package, value)| (package.clone(), value.clone()))
                    .collect(),
            ))
        })
        .collect()
}

fn generated_config_is_fresh(
    base: &FingerprintBase,
    fingerprint: &EnginePatchFingerprint,
    patches: &CargoPatchSources,
    catalog: &EngineCrateCatalog,
    dependency_patches: &CargoPatchSources,
) -> bool {
    if !base.matches(fingerprint) {
        return false;
    }

    let joined = patches
        .get("crates-io")
        .into_iter()
        .flat_map(|packages| packages.iter())
        .filter_map(|(package, value)| {
            let entry = catalog.entries.get(package)?;
            patch_path(value)
                .is_some_and(|path| path == portable_path_string(&entry.root))
                .then(|| (package.clone(), entry.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let joined_hash = joined_engine_packages_hash(&joined);
    let Ok(expected_patches) = compose_patch_sources(&joined, dependency_patches) else {
        return false;
    };
    let actual_patch_hash = patch_table_hash(patches);
    joined_hash == fingerprint.joined_engine_packages_sha256
        && actual_patch_hash == fingerprint.patch_table_sha256
        && patch_sources_are_subset_of(patches, &expected_patches)
}

fn patch_sources_are_subset_of(
    patches: &CargoPatchSources,
    candidates: &CargoPatchSources,
) -> bool {
    patches.iter().all(|(source, packages)| {
        candidates.get(source).is_some_and(|candidates| {
            packages.iter().all(|(package, value)| {
                candidates
                    .get(package)
                    .is_some_and(|candidate| candidate == value)
            })
        })
    })
}

fn load_engine_dependency_patches(
    engine_root: &Path,
) -> Result<CargoPatchSources, ProjectManifestError> {
    let manifest_path = engine_root.join("Cargo.toml");
    let manifest = read_toml_manifest(&manifest_path)?;
    let Some(sources) = manifest.get("patch").and_then(toml::Value::as_table) else {
        return Ok(CargoPatchSources::new());
    };

    sources
        .iter()
        .map(|(source, packages)| {
            let packages =
                packages
                    .as_table()
                    .ok_or_else(|| ProjectManifestError::InvalidEngineRoot {
                        path: manifest_path.clone(),
                        reason: format!("[patch.{source}] must be a TOML table"),
                    })?;
            let packages = packages
                .iter()
                .map(|(package, value)| {
                    let mut value = value.clone();
                    absolutize_engine_patch_path(
                        engine_root,
                        &manifest_path,
                        source,
                        package,
                        &mut value,
                    )?;
                    Ok((package.clone(), value))
                })
                .collect::<Result<BTreeMap<_, _>, ProjectManifestError>>()?;
            Ok((source.clone(), packages))
        })
        .collect()
}

fn absolutize_engine_patch_path(
    engine_root: &Path,
    manifest_path: &Path,
    source: &str,
    package: &str,
    value: &mut toml::Value,
) -> Result<(), ProjectManifestError> {
    let Some(table) = value.as_table_mut() else {
        return Ok(());
    };
    let Some(path_value) = table.get_mut("path") else {
        return Ok(());
    };
    let path = path_value
        .as_str()
        .ok_or_else(|| ProjectManifestError::InvalidEngineRoot {
            path: manifest_path.to_path_buf(),
            reason: format!("[patch.{source}].{package}.path must be a string"),
        })?;
    let path = PathBuf::from(path);
    let resolved = if path.is_absolute() {
        path
    } else {
        engine_root.join(path)
    };
    if !resolved.exists() {
        return Err(ProjectManifestError::InvalidEngineRoot {
            path: manifest_path.to_path_buf(),
            reason: format!(
                "[patch.{source}].{package} resolves to missing path `{}`",
                resolved.display()
            ),
        });
    }
    *path_value = toml::Value::String(portable_path_string(&resolved));
    Ok(())
}

fn compose_patch_sources(
    entries: &BTreeMap<String, EngineCrateCatalogEntry>,
    dependency_patches: &CargoPatchSources,
) -> Result<CargoPatchSources, ProjectManifestError> {
    let mut patches = dependency_patches.clone();
    let crates_io = patches.entry("crates-io".to_string()).or_default();
    for entry in entries.values() {
        let value = path_patch_value(&entry.root);
        if let Some(existing) = crates_io.insert(entry.package.clone(), value.clone())
            && existing != value
        {
            return Err(ProjectManifestError::InvalidEngineRoot {
                path: entry.manifest_path.clone(),
                reason: format!(
                    "engine package `{}` conflicts with the engine workspace's crates.io dependency override",
                    entry.package
                ),
            });
        }
    }
    Ok(patches)
}

fn retain_used_patches(
    patches: &CargoPatchSources,
    used_patches: &BTreeSet<CargoPatchIdentity>,
) -> CargoPatchSources {
    patches
        .iter()
        .filter_map(|(source, packages)| {
            let packages = packages
                .iter()
                .filter(|(package, _)| {
                    used_patches.contains(&CargoPatchIdentity {
                        source: source.clone(),
                        package: (*package).clone(),
                    })
                })
                .map(|(package, value)| (package.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>();
            (!packages.is_empty()).then(|| (source.clone(), packages))
        })
        .collect()
}

fn path_patch_value(path: &Path) -> toml::Value {
    toml::Value::Table(toml::map::Map::from_iter([(
        "path".to_string(),
        toml::Value::String(portable_path_string(path)),
    )]))
}

fn patch_path(value: &toml::Value) -> Option<&str> {
    value.get("path").and_then(toml::Value::as_str)
}

fn patch_identities(patches: &CargoPatchSources) -> BTreeSet<String> {
    patches
        .iter()
        .flat_map(|(source, packages)| {
            packages
                .keys()
                .map(move |package| format!("{source}:{package}"))
        })
        .collect()
}

fn build_engine_crate_catalog(
    engine_root: &Path,
    engine_manifest: &EngineManifest,
) -> Result<EngineCrateCatalog, ProjectManifestError> {
    let gem_roots = engine_gem_package_roots(engine_root)?;
    let roots = engine_package_source_roots(engine_root, &gem_roots)?;

    let mut entries = BTreeMap::<String, EngineCrateCatalogEntry>::new();
    for root in roots {
        let entry = engine_crate_catalog_entry(engine_root, engine_manifest, &gem_roots, root)?;
        let package = entry.package.clone();
        let root = entry.root.clone();
        if let Some(existing) = entries.insert(package.clone(), entry) {
            return Err(ProjectManifestError::InvalidEngineRoot {
                path: engine_root.to_path_buf(),
                reason: format!(
                    "ambiguous Cargo package `{package}` is registered at both `{}` and `{}`",
                    existing.root.display(),
                    root.display()
                ),
            });
        }
    }

    let sha256 = engine_crate_catalog_hash(&entries);
    Ok(EngineCrateCatalog { entries, sha256 })
}

/// Every registered engine gem's package root, keyed by canonical directory.
fn engine_gem_package_roots(
    engine_root: &Path,
) -> Result<BTreeMap<PathBuf, String>, ProjectManifestError> {
    let graph = resolve_engine_graph_at(engine_root)?;
    let mut gem_roots = BTreeMap::<PathBuf, String>::new();
    for gem in graph.gems {
        if !gem.root.join("Cargo.toml").is_file() {
            return Err(ProjectManifestError::InvalidEngineRoot {
                path: engine_root.to_path_buf(),
                reason: format!(
                    "registered gem `{}` has no Cargo package at `{}`",
                    gem.declaration.id,
                    gem.root.join("Cargo.toml").display()
                ),
            });
        }
        gem_roots.insert(gem.root, gem.declaration.id);
    }
    Ok(gem_roots)
}

/// Every directory the selected engine registers as a patchable package source:
/// the workspace root itself, its concrete members, its in-engine path
/// dependencies, and the registered gem roots.
fn engine_package_source_roots(
    engine_root: &Path,
    gem_roots: &BTreeMap<PathBuf, String>,
) -> Result<BTreeSet<PathBuf>, ProjectManifestError> {
    let workspace_manifest_path = engine_root.join("Cargo.toml");
    let workspace_manifest = read_toml_manifest(&workspace_manifest_path)?;
    let mut roots = BTreeSet::new();
    if workspace_manifest.get("package").is_some() {
        roots.insert(engine_root.to_path_buf());
    }
    if let Some(members) = workspace_manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
    {
        for member in members.iter().filter_map(toml::Value::as_str) {
            if member.contains(['*', '?', '[']) {
                return Err(ProjectManifestError::InvalidEngineRoot {
                    path: workspace_manifest_path,
                    reason: format!(
                        "workspace member pattern `{member}` is not a concrete registered engine package"
                    ),
                });
            }
            roots.insert(engine_root.join(member));
        }
    }
    if let Some(dependencies) = workspace_manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        for dependency in dependencies.values() {
            let Some(path) = dependency
                .as_table()
                .and_then(|dependency| dependency.get("path"))
                .and_then(toml::Value::as_str)
            else {
                continue;
            };
            let path = engine_root.join(path);
            if canonical_path_if_exists(&path).is_some_and(|path| path.starts_with(engine_root)) {
                roots.insert(path);
            }
        }
    }
    roots.extend(gem_roots.keys().cloned());
    Ok(roots)
}

/// Read one registered source root's Cargo manifest into a catalog entry.
fn engine_crate_catalog_entry(
    engine_root: &Path,
    engine_manifest: &EngineManifest,
    gem_roots: &BTreeMap<PathBuf, String>,
    root: PathBuf,
) -> Result<EngineCrateCatalogEntry, ProjectManifestError> {
    let Some(root) = canonical_path_if_exists(&root) else {
        return Err(ProjectManifestError::InvalidEngineRoot {
            path: root,
            reason: "registered engine package source root does not exist".to_string(),
        });
    };
    if !root.starts_with(engine_root) {
        return Err(ProjectManifestError::InvalidEngineRoot {
            path: root,
            reason: "registered engine package source root is outside the selected engine"
                .to_string(),
        });
    }
    let manifest_path = root.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(ProjectManifestError::InvalidEngineRoot {
            path: manifest_path,
            reason: "registered engine package has no Cargo.toml".to_string(),
        });
    }
    let manifest_text =
        std::fs::read_to_string(&manifest_path).map_err(|source| ProjectManifestError::Read {
            path: manifest_path.clone(),
            source,
        })?;
    let manifest = toml::from_str::<toml::Value>(&manifest_text).map_err(|source| {
        ProjectManifestError::Parse {
            path: manifest_path.clone(),
            source: Box::new(source),
        }
    })?;
    let package = required_package_field(&manifest, &manifest_path, "name")?;
    let version = required_package_field(&manifest, &manifest_path, "version")?;
    semver::Version::parse(&version).map_err(|error| ProjectManifestError::InvalidEngineRoot {
        path: manifest_path.clone(),
        reason: format!("Cargo package `{package}` has invalid version `{version}`: {error}"),
    })?;
    let relative_manifest = manifest_path.strip_prefix(engine_root).map_err(|_| {
        ProjectManifestError::InvalidEngineRoot {
            path: manifest_path.clone(),
            reason: "Cargo package manifest is outside the selected engine".to_string(),
        }
    })?;
    let (registration_owner, contribution_identity) = gem_roots.get(&root).map_or_else(
        || {
            (
                engine_manifest.engine.id.clone(),
                "workspace-package".to_string(),
            )
        },
        |gem_id| (gem_id.clone(), "v1-root-package".to_string()),
    );
    Ok(EngineCrateCatalogEntry {
        package,
        version,
        registration_owner,
        contribution_identity,
        manifest_relative_source: portable_path_string(relative_manifest),
        manifest_checksum: sha256_bytes(manifest_text.as_bytes()),
        manifest_path,
        root,
    })
}

fn required_package_field(
    manifest: &toml::Value,
    manifest_path: &Path,
    field: &'static str,
) -> Result<String, ProjectManifestError> {
    manifest
        .get("package")
        .and_then(|package| package.get(field))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ProjectManifestError::InvalidEngineRoot {
            path: manifest_path.to_path_buf(),
            reason: format!("Cargo manifest has no [package].{field}"),
        })
}

fn engine_crate_catalog_hash(entries: &BTreeMap<String, EngineCrateCatalogEntry>) -> String {
    let mut hasher = Sha256::new();
    for entry in entries.values() {
        for field in [
            entry.package.as_str(),
            entry.version.as_str(),
            entry.registration_owner.as_str(),
            entry.contribution_identity.as_str(),
            entry.manifest_relative_source.as_str(),
            entry.manifest_checksum.as_str(),
        ] {
            hash_field(&mut hasher, field.as_bytes());
        }
    }
    sha256_finish(hasher)
}

fn project_cargo_graph_hash(
    project_root: &Path,
    engine_root: &Path,
    catalog: &EngineCrateCatalog,
) -> Result<String, ProjectManifestError> {
    let root_manifest = project_root.join("Cargo.toml");
    if !root_manifest.is_file() {
        return Ok(sha256_bytes(EMPTY_GRAPH_MARKER));
    }

    let bootstrap = BootstrapOverlay::create(engine_root, catalog)?;
    let inputs = cargo_workspace_manifest_paths(&root_manifest, &bootstrap)?;
    let mut hasher = Sha256::new();
    for path in inputs {
        let relative = path.strip_prefix(project_root).unwrap_or(path.as_path());
        hash_field(&mut hasher, portable_path_string(relative).as_bytes());
        let bytes = std::fs::read(&path).map_err(|source| ProjectManifestError::Read {
            path: path.clone(),
            source,
        })?;
        hash_field(&mut hasher, &bytes);
    }
    Ok(sha256_finish(hasher))
}

fn cargo_workspace_manifest_paths(
    root_manifest: &Path,
    bootstrap: &BootstrapOverlay,
) -> Result<Vec<PathBuf>, ProjectManifestError> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command
        .current_dir(&bootstrap.directory)
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
        .arg("--color")
        .arg("never")
        .arg("--manifest-path")
        .arg(root_manifest)
        .arg("--config")
        .arg(&bootstrap.config_path);
    let output = az_work::owned_command_output(&mut command).map_err(|source| {
        ProjectManifestError::Read {
            path: root_manifest.to_path_buf(),
            source,
        }
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ProjectManifestError::InvalidEngineRoot {
            path: root_manifest.to_path_buf(),
            reason: format!(
                "Cargo could not project workspace-owned manifests: {}",
                if stderr.is_empty() {
                    format!("exit status {}", output.status)
                } else {
                    stderr
                }
            ),
        });
    }

    let metadata = serde_json::from_slice::<CargoMetadata>(&output.stdout).map_err(|error| {
        ProjectManifestError::InvalidEngineRoot {
            path: root_manifest.to_path_buf(),
            reason: format!("Cargo metadata returned invalid JSON: {error}"),
        }
    })?;
    let packages = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    let mut manifests = BTreeSet::from([metadata.workspace_root.join("Cargo.toml")]);
    for member in &metadata.workspace_members {
        let package = packages.get(member.as_str()).ok_or_else(|| {
            ProjectManifestError::InvalidEngineRoot {
                path: root_manifest.to_path_buf(),
                reason: format!(
                    "Cargo metadata workspace member `{member}` has no package manifest"
                ),
            }
        })?;
        manifests.insert(package.manifest_path.clone());
    }
    Ok(manifests.into_iter().collect())
}

fn selected_engine_lock_revision(
    project_root: &Path,
    engine_manifest: &EngineManifest,
) -> Result<String, ProjectManifestError> {
    let lock_path = project_root.join(PROJECT_LOCK_FILE);
    let lock = match std::fs::read_to_string(&lock_path) {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(local_engine_revision(engine_manifest));
        }
        Err(source) => {
            return Err(ProjectManifestError::Read {
                path: lock_path,
                source,
            });
        }
    };
    let value =
        toml::from_str::<toml::Value>(&lock).map_err(|source| ProjectManifestError::Parse {
            path: lock_path,
            source: Box::new(source),
        })?;
    Ok(value
        .get("engine")
        .and_then(|engine| {
            engine
                .get("source")
                .and_then(|source| source.get("revision"))
                .or_else(|| engine.get("revision"))
        })
        .and_then(toml::Value::as_str)
        .filter(|revision| !revision.trim().is_empty())
        .map_or_else(|| local_engine_revision(engine_manifest), str::to_string))
}

fn local_engine_revision(engine_manifest: &EngineManifest) -> String {
    format!(
        "local:{}@{}",
        engine_manifest.engine.id, engine_manifest.engine.version
    )
}

fn project_cargo_metadata(
    project_root: &Path,
    engine_root: &Path,
    catalog: &EngineCrateCatalog,
    allow_lock_update: bool,
) -> Result<CargoMetadata, ProjectManifestError> {
    cargo_metadata_for_manifest(
        &project_root.join("Cargo.toml"),
        engine_root,
        catalog,
        allow_lock_update,
        None,
    )
}

fn project_cargo_metadata_preserving_lock(
    project_root: &Path,
    engine_root: &Path,
    catalog: &EngineCrateCatalog,
    allow_lock_update: bool,
) -> Result<CargoMetadata, ProjectManifestError> {
    if !allow_lock_update {
        return project_cargo_metadata(project_root, engine_root, catalog, false);
    }

    let lock_path = project_root.join("Cargo.lock");
    let original_lock = match std::fs::read(&lock_path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(ProjectManifestError::Read {
                path: lock_path,
                source,
            });
        }
    };
    let metadata = project_cargo_metadata(project_root, engine_root, catalog, true);
    let restore = match original_lock {
        Some(contents) => atomic_write(&lock_path, &contents),
        None => match std::fs::remove_file(&lock_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(ProjectManifestError::Write {
                path: lock_path,
                source,
            }),
        },
    };
    restore?;
    metadata
}

fn cargo_metadata_for_manifest(
    manifest_path: &Path,
    engine_root: &Path,
    catalog: &EngineCrateCatalog,
    allow_lock_update: bool,
    target_directory: Option<&Path>,
) -> Result<CargoMetadata, ProjectManifestError> {
    let bootstrap = BootstrapOverlay::create(engine_root, catalog)?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command
        .current_dir(&bootstrap.directory)
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--all-features")
        .arg("--color")
        .arg("never")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--config")
        .arg(&bootstrap.config_path);
    if let Some(target_directory) = target_directory {
        command.env("CARGO_TARGET_DIR", target_directory);
    }
    if !allow_lock_update
        && manifest_path
            .parent()
            .is_some_and(|root| root.join("Cargo.lock").is_file())
    {
        command.arg("--locked");
    }
    let output = az_work::owned_command_output(&mut command).map_err(|source| {
        ProjectManifestError::Read {
            path: manifest_path.to_path_buf(),
            source,
        }
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ProjectManifestError::InvalidEngineRoot {
            path: engine_root.to_path_buf(),
            reason: format!(
                "Cargo metadata bootstrap overlay failed for `{}`: {}",
                manifest_path.display(),
                if stderr.is_empty() {
                    format!("exit status {}", output.status)
                } else {
                    stderr
                }
            ),
        });
    }
    let mut metadata =
        serde_json::from_slice::<CargoMetadata>(&output.stdout).map_err(|error| {
            ProjectManifestError::InvalidEngineRoot {
                path: manifest_path.to_path_buf(),
                reason: format!("Cargo metadata returned invalid JSON: {error}"),
            }
        })?;
    metadata.used_patches = used_patch_identities(&metadata, &bootstrap.patches);
    Ok(metadata)
}

fn used_patch_identities(
    metadata: &CargoMetadata,
    patches: &CargoPatchSources,
) -> BTreeSet<CargoPatchIdentity> {
    let packages = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    let Some(resolve) = &metadata.resolve else {
        return BTreeSet::new();
    };
    let mut used = BTreeSet::new();
    for node in &resolve.nodes {
        let Some(parent) = packages.get(node.id.as_str()) else {
            continue;
        };
        for dependency_id in &node.dependencies {
            let Some(dependency_package) = packages.get(dependency_id.as_str()) else {
                continue;
            };
            for dependency in parent
                .dependencies
                .iter()
                .filter(|dependency| dependency.name == dependency_package.name)
            {
                let Some(dependency_source) = dependency.source.as_deref() else {
                    continue;
                };
                for (patch_source, packages) in patches {
                    if normalized_patch_source(patch_source)
                        != normalized_dependency_source(dependency_source)
                    {
                        continue;
                    }
                    let Some(value) = packages.get(&dependency_package.name) else {
                        continue;
                    };
                    if patch_value_matches_package(value, dependency_package) {
                        used.insert(CargoPatchIdentity {
                            source: patch_source.clone(),
                            package: dependency_package.name.clone(),
                        });
                    }
                }
            }
        }
    }
    used
}

fn normalized_patch_source(source: &str) -> String {
    if source == "crates-io" {
        source.to_string()
    } else {
        normalized_source_url(source)
    }
}

fn normalized_dependency_source(source: &str) -> String {
    if source == "registry+https://github.com/rust-lang/crates.io-index"
        || source == "sparse+https://index.crates.io/"
    {
        "crates-io".to_string()
    } else if let Some(source) = source.strip_prefix("registry+") {
        normalized_source_url(source)
    } else if let Some(source) = source.strip_prefix("git+") {
        normalized_source_url(source)
    } else {
        normalized_source_url(source)
    }
}

fn normalized_source_url(source: &str) -> String {
    source
        .split(['?', '#'])
        .next()
        .unwrap_or(source)
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_ascii_lowercase()
}

fn patch_value_matches_package(value: &toml::Value, package: &CargoMetadataPackage) -> bool {
    if let Some(path) = patch_path(value) {
        let patch_manifest = Path::new(path).join("Cargo.toml");
        return canonical_path_if_exists(&patch_manifest)
            .zip(canonical_path_if_exists(&package.manifest_path))
            .is_some_and(|(patch_manifest, package_manifest)| {
                same_path(&patch_manifest, &package_manifest)
            });
    }
    if let Some(git) = value.get("git").and_then(toml::Value::as_str) {
        return package.source.as_deref().is_some_and(|source| {
            normalized_dependency_source(source) == normalized_source_url(git)
        });
    }
    true
}

fn synchronize_workspace_lock(
    project_root: &Path,
    manifest_path: &Path,
) -> Result<(), ProjectManifestError> {
    if workspace_lock_is_current(project_root, manifest_path)? {
        return Ok(());
    }
    regenerate_workspace_lock(project_root, manifest_path)
}

fn workspace_lock_is_current(
    project_root: &Path,
    manifest_path: &Path,
) -> Result<bool, ProjectManifestError> {
    if !project_root.join("Cargo.lock").is_file() {
        return Ok(false);
    }

    let mut command = workspace_lock_metadata_command(project_root, manifest_path);
    let output = az_work::owned_command_output(&mut command).map_err(|source| {
        ProjectManifestError::Read {
            path: manifest_path.to_path_buf(),
            source,
        }
    })?;
    Ok(output.status.success())
}

fn workspace_lock_metadata_command(project_root: &Path, manifest_path: &Path) -> Command {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command
        .current_dir(project_root)
        .arg("metadata")
        .arg("--locked")
        .arg("--format-version")
        .arg("1")
        .arg("--color")
        .arg("never")
        .arg("--manifest-path")
        .arg(manifest_path);
    command
}

fn regenerate_workspace_lock(
    project_root: &Path,
    manifest_path: &Path,
) -> Result<(), ProjectManifestError> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command
        .current_dir(project_root)
        .arg("generate-lockfile")
        .arg("--offline")
        .arg("--color")
        .arg("never")
        .arg("--manifest-path")
        .arg(manifest_path);
    let output = az_work::owned_command_output(&mut command).map_err(|source| {
        ProjectManifestError::Read {
            path: manifest_path.to_path_buf(),
            source,
        }
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(ProjectManifestError::InvalidEngineRoot {
        path: manifest_path.to_path_buf(),
        reason: format!(
            "Cargo failed to synchronize the workspace lock from the projected engine graph: {}",
            if stderr.is_empty() {
                format!("exit status {}", output.status)
            } else {
                stderr
            }
        ),
    })
}

struct BootstrapOverlay {
    directory: PathBuf,
    config_path: PathBuf,
    patches: CargoPatchSources,
}

impl BootstrapOverlay {
    fn create(
        engine_root: &Path,
        catalog: &EngineCrateCatalog,
    ) -> Result<Self, ProjectManifestError> {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "azoth-cargo-overlay-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).map_err(|source| ProjectManifestError::Write {
            path: directory.clone(),
            source,
        })?;
        let config_path = directory.join("bootstrap.toml");
        let dependency_patches = load_engine_dependency_patches(engine_root)?;
        let patches = compose_patch_sources(&catalog.entries, &dependency_patches)?;
        let mut contents = String::new();
        render_config_body(&mut contents, engine_root, &patches);
        std::fs::write(&config_path, contents).map_err(|source| ProjectManifestError::Write {
            path: config_path.clone(),
            source,
        })?;
        Ok(Self {
            directory,
            config_path,
            patches,
        })
    }
}

impl Drop for BootstrapOverlay {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.config_path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

fn join_project_metadata_to_engine_catalog(
    metadata: &CargoMetadata,
    catalog: &EngineCrateCatalog,
) -> Result<BTreeMap<String, EngineCrateCatalogEntry>, ProjectManifestError> {
    let packages = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    let Some(resolve) = &metadata.resolve else {
        return Err(ProjectManifestError::InvalidEngineRoot {
            path: PathBuf::from("Cargo.toml"),
            reason: "Cargo metadata did not include a resolved project graph".to_string(),
        });
    };
    let dependencies = resolve
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.dependencies.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let mut queue = VecDeque::from(metadata.workspace_members.clone());
    let mut reachable = BTreeSet::new();
    while let Some(package_id) = queue.pop_front() {
        if !reachable.insert(package_id.clone()) {
            continue;
        }
        if let Some(children) = dependencies.get(package_id.as_str()) {
            queue.extend(children.iter().cloned());
        }
    }

    let mut joined = BTreeMap::new();
    for package_id in reachable {
        let Some(package) = packages.get(package_id.as_str()) else {
            return Err(ProjectManifestError::InvalidEngineRoot {
                path: PathBuf::from("Cargo.toml"),
                reason: format!("Cargo metadata resolve references unknown package `{package_id}`"),
            });
        };
        let Some(candidate) = catalog.entries.get(&package.name) else {
            continue;
        };
        if package.version != candidate.version {
            return Err(ProjectManifestError::InvalidEngineRoot {
                path: package.manifest_path.clone(),
                reason: format!(
                    "engine package `{}` resolved version `{}` but registered engine version is `{}`",
                    package.name, package.version, candidate.version
                ),
            });
        }
        let resolved_manifest =
            canonical_path_if_exists(&package.manifest_path).ok_or_else(|| {
                ProjectManifestError::InvalidEngineRoot {
                    path: package.manifest_path.clone(),
                    reason: format!(
                        "resolved engine package `{}` manifest does not exist",
                        package.name
                    ),
                }
            })?;
        if !same_path(&resolved_manifest, &candidate.manifest_path) {
            return Err(ProjectManifestError::InvalidEngineRoot {
                path: package.manifest_path.clone(),
                reason: format!(
                    "engine package `{}` resolved from `{}` instead of registered source `{}`",
                    package.name,
                    package.manifest_path.display(),
                    candidate.manifest_path.display()
                ),
            });
        }
        joined.insert(package.name.clone(), candidate.clone());
    }
    Ok(joined)
}

fn render_bootstrap_overlay(
    engine_root: &Path,
    entries: &BTreeMap<String, EngineCrateCatalogEntry>,
    dependency_patches: &CargoPatchSources,
) -> Result<String, ProjectManifestError> {
    let patches = compose_patch_sources(entries, dependency_patches)?;
    let mut output = String::new();
    render_config_body(&mut output, engine_root, &patches);
    Ok(output)
}

fn render_generated_config(
    engine_root: &Path,
    fingerprint: &EnginePatchFingerprint,
    patches: &CargoPatchSources,
) -> String {
    let mut output = String::new();
    writeln!(output, "{GENERATED_CARGO_CONFIG_HEADER}").expect("writing to a String cannot fail");
    writeln!(
        output,
        "# {CONFIG_FORMAT_VERSION_KEY}: {}",
        fingerprint.config_format_version
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "# {SELECTED_ENGINE_ID_KEY}: {}",
        fingerprint.selected_engine_id
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "# {SELECTED_ENGINE_VERSION_KEY}: {}",
        fingerprint.selected_engine_version
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "# {SELECTED_ENGINE_ROOT_KEY}: {}",
        fingerprint.selected_engine_root
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "# {ENGINE_LOCK_REVISION_KEY}: {}",
        fingerprint.engine_lock_revision
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "# {ENGINE_CRATE_CATALOG_HASH_KEY}: {}",
        fingerprint.engine_crate_catalog_sha256
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "# {PROJECT_CARGO_GRAPH_HASH_KEY}: {}",
        fingerprint.project_cargo_graph_sha256
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "# {JOINED_ENGINE_PACKAGES_HASH_KEY}: {}",
        fingerprint.joined_engine_packages_sha256
    )
    .expect("writing to a String cannot fail");
    writeln!(
        output,
        "# {PATCH_TABLE_HASH_KEY}: {}",
        fingerprint.patch_table_sha256
    )
    .expect("writing to a String cannot fail");
    output.push_str("# Local engine patch table. Source manifests stay portable.\n\n");
    render_config_body(&mut output, engine_root, patches);
    output
}

fn render_config_body(output: &mut String, engine_root: &Path, patches: &CargoPatchSources) {
    output.push_str("[env]\n");
    writeln!(
        output,
        "{AZOTH_ENGINE_ROOT_ENV} = {{ value = {:?}, force = true }}",
        portable_path_string(engine_root)
    )
    .expect("writing to a String cannot fail");
    for (source, packages) in patches {
        if source == "crates-io" {
            output.push_str("\n[patch.crates-io]\n");
        } else {
            writeln!(output, "\n[patch.{source:?}]").expect("writing to a String cannot fail");
        }
        for (package, value) in packages {
            writeln!(output, "{} = {value}", toml_key(package))
                .expect("writing to a String cannot fail");
        }
    }
}

fn joined_engine_packages_hash(entries: &BTreeMap<String, EngineCrateCatalogEntry>) -> String {
    let mut hasher = Sha256::new();
    for entry in entries.values() {
        hash_field(&mut hasher, entry.package.as_bytes());
        hash_field(&mut hasher, entry.version.as_bytes());
    }
    sha256_finish(hasher)
}

fn patch_table_hash(patches: &CargoPatchSources) -> String {
    let mut hasher = Sha256::new();
    for (source, packages) in patches {
        hash_field(&mut hasher, source.as_bytes());
        for (package, value) in packages {
            hash_field(&mut hasher, package.as_bytes());
            hash_field(&mut hasher, value.to_string().as_bytes());
        }
    }
    sha256_finish(hasher)
}

fn canonical_path_if_exists(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

fn portable_path_string(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if let Some(unc) = path.strip_prefix("//?/UNC/") {
        return format!("//{unc}");
    }
    if let Some(local) = path.strip_prefix("//?/") {
        return local.to_owned();
    }
    path
}

fn toml_key(key: &str) -> String {
    if key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        key.to_string()
    } else {
        format!("{key:?}")
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    sha256_finish(hasher)
}

fn hash_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(bytes.len().to_le_bytes());
    hasher.update(bytes);
}

fn sha256_finish(hasher: Sha256) -> String {
    use std::fmt::Write;

    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{EngineGem, EngineManifest};

    #[test]
    fn workspace_lock_freshness_resolves_the_full_dependency_graph() {
        let command = workspace_lock_metadata_command(
            Path::new("/project"),
            Path::new("/project/Cargo.toml"),
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(args.iter().any(|arg| arg == "--locked"));
        assert!(
            !args.iter().any(|arg| arg == "--no-deps"),
            "lock validation must resolve the graph that package builds consume"
        );
    }

    #[test]
    fn cargo_graph_fingerprint_tracks_only_workspace_owned_manifests() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        std::fs::write(
            project.path().join("Cargo.toml"),
            r#"[workspace]
members = ["crates/member"]
exclude = ["retained/*"]
resolver = "3"
"#,
        )
        .unwrap();
        write_package(&project.path().join("crates/member"), "member");
        write_package(
            &project.path().join("retained/session/unrelated"),
            "unrelated",
        );
        let lock_path = project.path().join("Cargo.lock");
        let lock = b"# preserved lock\nversion = 4\n\n[[package]]\nname = \"member\"\nversion = \"0.1.0\"\n";
        std::fs::write(&lock_path, lock).unwrap();

        let initial = ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        assert_eq!(std::fs::read(&lock_path).unwrap(), lock);
        std::fs::write(
            project.path().join("retained/session/unrelated/Cargo.toml"),
            "[package]\nname = \"unrelated\"\nversion = \"0.2.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        let after_excluded_change =
            ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();

        let member_manifest = project.path().join("crates/member/Cargo.toml");
        let member =
            std::fs::read_to_string(&member_manifest).unwrap() + "\n# fingerprint change\n";
        std::fs::write(member_manifest, member).unwrap();
        let after_member_change =
            ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();

        assert_eq!(
            after_excluded_change.status,
            ProjectEnginePatchSyncStatus::Unchanged
        );
        assert_eq!(after_excluded_change.fingerprint, initial.fingerprint);
        assert_eq!(
            after_member_change.status,
            ProjectEnginePatchSyncStatus::Regenerated
        );
        assert_ne!(
            after_member_change.fingerprint.project_cargo_graph_sha256,
            initial.fingerprint.project_cargo_graph_sha256
        );
        assert_eq!(std::fs::read(lock_path).unwrap(), lock);
    }

    #[test]
    fn unusable_generated_config_does_not_block_its_own_repair() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        write_project_fixture(project.path());
        let initial = ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        let lock_path = project.path().join("Cargo.lock");
        let lock = std::fs::read(&lock_path).unwrap();
        std::fs::write(
            &initial.config_path,
            format!("{GENERATED_CARGO_CONFIG_HEADER}\n\n[patch.crates-io\n"),
        )
        .unwrap();

        let repaired = ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        let config = std::fs::read_to_string(&repaired.config_path).unwrap();

        assert_eq!(repaired.status, ProjectEnginePatchSyncStatus::Regenerated);
        assert!(toml::from_str::<toml::Value>(&config).is_ok());
        assert_eq!(std::fs::read(lock_path).unwrap(), lock);
    }

    fn write_engine_fixture(root: &Path) {
        let mut engine = EngineManifest::new("azoth", "Azoth", "0.1.0");
        engine.gems.push(EngineGem {
            id: "azoth.historical-input".to_string(),
            path: PathBuf::from("gems/historical-input"),
        });
        engine.gems.push(EngineGem {
            id: "azoth.optional-runtime".to_string(),
            path: PathBuf::from("gems/optional-runtime"),
        });
        std::fs::write(
            crate::manifest::engine_manifest_path(root),
            toml::to_string_pretty(&engine).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "azoth"
version = "0.1.0"
edition = "2024"

[workspace]
members = ["crates/az/core", "gems/historical-input", "gems/optional-runtime"]
exclude = ["vendor/upstream-runtime", "vendor/upstream-git"]
resolver = "3"

[workspace.dependencies]
az-core = { path = "crates/az/core" }
az-gem-historical-input = { path = "gems/historical-input" }
az-gem-optional-runtime = { path = "gems/optional-runtime" }

[patch.crates-io]
upstream-runtime = { path = "vendor/upstream-runtime" }

[patch."https://example.invalid/upstream"]
upstream-git = { path = "vendor/upstream-git" }
"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        write_package(&root.join("crates/az/core"), "az-core");
        write_package(
            &root.join("gems/historical-input"),
            "az-gem-historical-input",
        );
        write_package(
            &root.join("gems/optional-runtime"),
            "az-gem-optional-runtime",
        );
        write_package(&root.join("vendor/upstream-runtime"), "upstream-runtime");
        write_package(&root.join("vendor/upstream-git"), "upstream-git");
        std::fs::write(
            root.join("gems/historical-input/gem.toml"),
            r#"[manifest]
kind = "gem"
schema = "azoth.gem/v3"

[gem]
id = "azoth.historical-input"
name = "Historical Input"
version = "0.1.0"
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("gems/optional-runtime/gem.toml"),
            r#"[manifest]
kind = "gem"
schema = "azoth.gem/v3"

[gem]
id = "azoth.optional-runtime"
name = "Optional Runtime"
version = "0.1.0"
"#,
        )
        .unwrap();
    }

    fn write_package(root: &Path, package: &str) {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            format!("[package]\nname = {package:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn marker() {}\n").unwrap();
    }

    fn write_project_fixture(root: &Path) {
        std::fs::create_dir_all(root.join("crates/game/src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            r#"[workspace]
members = ["crates/game"]
resolver = "3"

[workspace.dependencies]
az-gem-historical-input = "0.1.0"
az-gem-optional-runtime = "0.1.0"
az-core = "0.1.0"
renamed-input = { version = "0.1.0", package = "az-gem-historical-input" }
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("crates/game/Cargo.toml"),
            r#"[package]
name = "game"
version = "0.1.0"
edition = "2024"

[dependencies]
renamed-input = { workspace = true }
az-gem-optional-runtime = { workspace = true, optional = true }

[build-dependencies]
az-core = { workspace = true }
"#,
        )
        .unwrap();
        std::fs::write(root.join("crates/game/src/lib.rs"), "pub fn game() {}\n").unwrap();
        std::fs::write(
            root.join("azoth.lock"),
            "[engine]\nid = \"azoth\"\nrevision = \"engine-rev-29\"\n",
        )
        .unwrap();
    }

    #[test]
    fn real_graph_join_finds_member_only_renamed_build_and_transitive_engine_packages() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        write_project_fixture(project.path());

        let report = ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        let config = std::fs::read_to_string(&report.config_path).unwrap();

        assert_eq!(report.status, ProjectEnginePatchSyncStatus::Regenerated);
        assert!(config.contains("az-gem-historical-input ="));
        assert!(config.contains("az-gem-optional-runtime ="));
        assert!(config.contains("az-core ="));
        assert!(!config.contains("upstream-runtime ="));
        assert!(!config.contains("https://example.invalid/upstream"));
        assert!(
            config.find("az-core =").unwrap() < config.find("az-gem-historical-input =").unwrap()
        );
    }

    #[test]
    fn obsolete_independent_generated_workspace_is_ignored() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        write_project_fixture(project.path());

        let generated_root = project.path().join("target/azoth/targets");
        std::fs::create_dir_all(generated_root.join("server/src")).unwrap();
        std::fs::write(
            generated_root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"server\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        std::fs::write(
            generated_root.join("server/Cargo.toml"),
            r#"[package]
name = "stale-generated-server"
version = "0.1.0"
edition = "2024"

[dependencies]
az-gem-historical-input = { version = "0.1.0", features = ["removed-feature"] }
"#,
        )
        .unwrap();
        std::fs::write(
            generated_root.join("server/src/lib.rs"),
            "pub fn stale() {}\n",
        )
        .unwrap();

        let full = sync_project_engine_patch_table_to(project.path(), engine.path(), true).unwrap();
        assert!(full.config_path.is_file());

        let bootstrap =
            sync_project_engine_patch_table_to_scope(project.path(), engine.path(), true, false)
                .unwrap();
        let config = std::fs::read_to_string(bootstrap.config_path).unwrap();
        assert!(config.contains("az-gem-historical-input ="));
    }

    #[test]
    fn projects_only_dependency_overrides_used_by_the_resolved_graph() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        // The patch table records the engine root `canonical_engine_root`
        // resolved, so the expected patch path has to start from the same
        // canonical spelling rather than the raw temp path.
        let engine_root = az_filesystem::normalize(engine.path());
        write_engine_fixture(&engine_root);
        write_project_fixture(project.path());
        let root_manifest = project.path().join("Cargo.toml");
        let root_contents = std::fs::read_to_string(&root_manifest).unwrap().replace(
            "[workspace.dependencies]\n",
            concat!(
                "[workspace.dependencies]\n",
                "upstream-runtime = \"0.1.0\"\n",
            ),
        );
        std::fs::write(root_manifest, root_contents).unwrap();
        let game_manifest = project.path().join("crates/game/Cargo.toml");
        let game_contents = std::fs::read_to_string(&game_manifest).unwrap().replace(
            "[dependencies]\n",
            concat!(
                "[dependencies]\n",
                "upstream-runtime = { workspace = true }\n",
            ),
        );
        std::fs::write(game_manifest, game_contents).unwrap();

        let report = ensure_project_engine_patch_table_to(project.path(), &engine_root).unwrap();
        let config = std::fs::read_to_string(report.config_path).unwrap();
        let patches = parse_patch_paths(&config).unwrap();

        assert_eq!(
            patch_path(&patches["crates-io"]["upstream-runtime"]),
            Some(portable_path_string(&engine_root.join("vendor/upstream-runtime")).as_str())
        );
        assert!(!patches.contains_key("https://example.invalid/upstream"));
        assert!(!config.contains("upstream-git"));
    }

    #[test]
    fn used_patch_identity_distinguishes_duplicate_names_across_sources() {
        let root = tempfile::tempdir().unwrap();
        let crates_io_root = root.path().join("crates-io/duplicate");
        let git_root = root.path().join("git/duplicate");
        write_package(&crates_io_root, "duplicate");
        write_package(&git_root, "duplicate");
        let metadata = CargoMetadata {
            packages: vec![
                CargoMetadataPackage {
                    id: "parent".to_string(),
                    name: "parent".to_string(),
                    version: "0.1.0".to_string(),
                    source: None,
                    dependencies: vec![
                        CargoMetadataDependency {
                            name: "duplicate".to_string(),
                            source: Some(
                                "registry+https://github.com/rust-lang/crates.io-index".to_string(),
                            ),
                        },
                        CargoMetadataDependency {
                            name: "duplicate".to_string(),
                            source: Some("git+https://example.invalid/duplicate".to_string()),
                        },
                    ],
                    manifest_path: root.path().join("Cargo.toml"),
                },
                CargoMetadataPackage {
                    id: "duplicate-crates-io".to_string(),
                    name: "duplicate".to_string(),
                    version: "0.1.0".to_string(),
                    source: None,
                    dependencies: Vec::new(),
                    manifest_path: crates_io_root.join("Cargo.toml"),
                },
            ],
            workspace_members: vec!["parent".to_string()],
            workspace_root: root.path().to_path_buf(),
            resolve: Some(CargoMetadataResolve {
                nodes: vec![CargoMetadataNode {
                    id: "parent".to_string(),
                    dependencies: vec!["duplicate-crates-io".to_string()],
                }],
            }),
            used_patches: BTreeSet::new(),
        };
        let patches = BTreeMap::from([
            (
                "crates-io".to_string(),
                BTreeMap::from([("duplicate".to_string(), path_patch_value(&crates_io_root))]),
            ),
            (
                "https://example.invalid/duplicate".to_string(),
                BTreeMap::from([("duplicate".to_string(), path_patch_value(&git_root))]),
            ),
        ]);

        assert_eq!(
            used_patch_identities(&metadata, &patches),
            BTreeSet::from([CargoPatchIdentity {
                source: "crates-io".to_string(),
                package: "duplicate".to_string(),
            }])
        );
    }

    #[test]
    fn matching_fingerprint_is_a_no_op() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        write_project_fixture(project.path());
        ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        let before = std::fs::read(project_local_cargo_config_path(project.path())).unwrap();

        let report = ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();

        assert_eq!(report.status, ProjectEnginePatchSyncStatus::Unchanged);
        assert!(report.added_packages.is_empty());
        assert!(report.removed_packages.is_empty());
        assert_eq!(
            std::fs::read(project_local_cargo_config_path(project.path())).unwrap(),
            before
        );
    }

    #[test]
    fn forced_patch_sync_preserves_a_valid_workspace_lock_byte_for_byte() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        write_project_fixture(project.path());
        ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        let lock_path = project.path().join("Cargo.lock");
        let mut lock = std::fs::read(&lock_path).unwrap();
        lock.extend_from_slice(b"\n# host-shared lock sentinel\n");
        std::fs::write(&lock_path, &lock).unwrap();

        sync_project_engine_patch_table_to(project.path(), engine.path(), true).unwrap();

        assert_eq!(std::fs::read(lock_path).unwrap(), lock);
    }

    #[test]
    fn fingerprint_mismatch_regenerates_the_table() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        write_project_fixture(project.path());
        let first = ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        let config_path = first.config_path;
        let config = std::fs::read_to_string(&config_path).unwrap().replace(
            "# engine-lock-revision: engine-rev-29",
            "# engine-lock-revision: old-rev",
        );
        std::fs::write(&config_path, config).unwrap();

        let report = ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        let fingerprint =
            parse_project_engine_patch_fingerprint(&std::fs::read_to_string(config_path).unwrap())
                .unwrap();

        assert_eq!(report.status, ProjectEnginePatchSyncStatus::Regenerated);
        assert_eq!(fingerprint.engine_lock_revision, "engine-rev-29");
    }

    #[test]
    fn ensure_repairs_a_stale_cargo_lock_once() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        write_project_fixture(project.path());
        ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();

        let root_manifest = project.path().join("Cargo.toml");
        let root_contents = std::fs::read_to_string(&root_manifest).unwrap().replace(
            "[workspace.dependencies]\n",
            "[workspace.dependencies]\nupstream-runtime = \"0.1.0\"\n",
        );
        std::fs::write(&root_manifest, root_contents).unwrap();

        let game_manifest = project.path().join("crates/game/Cargo.toml");
        let game_contents = std::fs::read_to_string(&game_manifest).unwrap().replace(
            "[dependencies]\n",
            "[dependencies]\nupstream-runtime = { workspace = true }\n",
        );
        std::fs::write(&game_manifest, game_contents).unwrap();

        let report = ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        let lock = std::fs::read_to_string(project.path().join("Cargo.lock")).unwrap();

        assert_eq!(report.status, ProjectEnginePatchSyncStatus::Regenerated);
        assert!(lock.contains("name = \"upstream-runtime\""));
    }

    #[test]
    fn generated_header_fingerprint_round_trips_and_preserves_marker() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        write_project_fixture(project.path());

        let report = ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap();
        let config = std::fs::read_to_string(report.config_path).unwrap();
        let parsed = parse_project_engine_patch_fingerprint(&config).unwrap();

        assert!(config.starts_with(GENERATED_CARGO_CONFIG_HEADER));
        assert_eq!(parsed, report.fingerprint);
        assert_eq!(
            parsed.config_format_version,
            ENGINE_PATCH_CONFIG_FORMAT_VERSION
        );
        assert_eq!(parsed.engine_lock_revision, "engine-rev-29");
        assert!(!config.contains("VCPKGRS_TRIPLET"));
        let document = toml::from_str::<toml::Value>(&config).unwrap();
        assert!(document.get("target").is_none());
    }

    #[test]
    fn authored_cargo_config_is_never_overwritten() {
        let engine = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_engine_fixture(engine.path());
        write_project_fixture(project.path());
        let config_path = project_local_cargo_config_path(project.path());
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, "[build]\nrustflags = []\n").unwrap();

        let error =
            ensure_project_engine_patch_table_to(project.path(), engine.path()).unwrap_err();

        assert!(matches!(
            error,
            ProjectManifestError::LocalCargoConfigNotGenerated { path } if path == config_path
        ));
        assert_eq!(
            std::fs::read_to_string(config_path).unwrap(),
            "[build]\nrustflags = []\n"
        );
    }

    #[test]
    fn stale_table_diagnostic_names_revisions_path_and_remediation() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let error = ProjectManifestError::StaleEnginePatchTable(Box::new(StaleEnginePatchTable {
            selected_engine: "azoth@0.1.0".to_string(),
            engine_revision: "engine-new".to_string(),
            table_revision: "engine-old".to_string(),
            config_path: project_root.join(".cargo/config.toml"),
            project_root: project_root.clone(),
            reason: "missing engine package `az-gem-historical-input`".to_string(),
        }));
        let diagnostic = error.to_string();

        assert!(diagnostic.contains("stale engine patch table"));
        assert!(diagnostic.contains("engine rev `engine-new`, table rev `engine-old`"));
        assert!(diagnostic.contains("az-gem-historical-input"));
        assert!(diagnostic.contains(".cargo/config.toml"));
        assert!(diagnostic.contains(&format!(
            "run `azoth engine sync --project {}`",
            project_root.display()
        )));
    }

    #[test]
    fn atomic_write_replaces_complete_file_without_temp_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "old").unwrap();

        atomic_write(&path, b"new complete contents").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "new complete contents"
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}
