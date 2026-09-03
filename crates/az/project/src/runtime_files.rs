//! Stage authored build-target runtime files next to Cargo build output.
//!
//! Projects and gems may declare owner-root-relative sidecar files on a build
//! target (`runtime_files`). After a successful cargo build, those files are
//! copied into the resolved profile output directory (`target/<profile>/`) so
//! binaries that load peer DLLs/resources by adjacency keep working on clean
//! builds without ad-hoc copy scripts.

use std::fs::{self, File, OpenOptions};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(windows, test))]
use std::time::Duration;

use thiserror::Error;
use tracing::info;

use crate::manifest::{
    ProjectBuildTarget, ProjectManifestError, ResolvedProjectGraph, load_resolved_project_graph,
};
use crate::target_generation::{
    ProjectBuildSelectorError, project_build_selector_candidates,
    resolve_project_build_selector_indices,
};

const ATOMIC_REPLACE_ATTEMPTS: usize = 8;
#[cfg(windows)]
const ATOMIC_REPLACE_RETRY_DELAY: Duration = Duration::from_millis(10);

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Outcome of staging one runtime file for a build target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeFileStagingAction {
    /// Destination did not exist or differed; source was copied.
    Staged,
    /// Destination already matched source content; copy was skipped.
    AlreadyFresh,
}

/// One staged (or already-fresh) runtime file entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFileStagingEntry {
    pub relative_source: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub action: RuntimeFileStagingAction,
}

/// Aggregate report for staging all runtime files of one build target.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeFileStagingReport {
    pub target_name: String,
    pub output_dir: PathBuf,
    pub entries: Vec<RuntimeFileStagingEntry>,
}

impl RuntimeFileStagingReport {
    #[must_use]
    pub fn staged_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.action == RuntimeFileStagingAction::Staged)
            .count()
    }

    #[must_use]
    pub fn already_fresh_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.action == RuntimeFileStagingAction::AlreadyFresh)
            .count()
    }
}

/// Errors raised while staging runtime files next to build output.
#[derive(Debug, Error)]
pub enum RuntimeFileStagingError {
    #[error(transparent)]
    Project(#[from] ProjectManifestError),

    #[error(transparent)]
    Selector(#[from] ProjectBuildSelectorError),

    #[error("build target `{target}` runtime file `{relative_source}` is missing at {source_path}")]
    MissingSource {
        target: String,
        relative_source: String,
        source_path: PathBuf,
    },

    #[error("build target `{target}` runtime file `{relative_source}` has no file name component")]
    MissingFileName {
        target: String,
        relative_source: String,
    },

    #[error(
        "build target `{target}` runtime file `{relative_source}` could not be staged to {destination}: {source}"
    )]
    Io {
        target: String,
        relative_source: String,
        destination: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Resolve the cargo profile name used for on-disk build output.
///
/// Package profiles may rename the cargo profile (`cargo_profile` field); bare
/// profile names pass through unchanged.
#[must_use]
pub fn resolve_cargo_profile_name(graph: &ResolvedProjectGraph, requested_profile: &str) -> String {
    let requested_profile = requested_profile.trim();
    graph
        .lock
        .packaging
        .profiles
        .iter()
        .find(|profile| profile.name == requested_profile)
        .map_or_else(
            || requested_profile.to_string(),
            |profile| profile.cargo_profile.clone(),
        )
}

/// Stage runtime files for every selected authored build target.
///
/// Selection mirrors project-build package selectors: empty selectors stage only
/// default authored targets; explicit selectors use the same candidate catalog as
/// build planning. Generated runtime targets never declare runtime files.
///
/// Output directory is `{owner_root}/target/<cargo_profile_dir>/`. Callers that
/// override `CARGO_TARGET_DIR` should stage through
/// [`stage_build_target_runtime_files`] with the resolved output directory.
///
/// # Errors
///
/// Returns any error [`stage_selected_authored_runtime_files_for_target`]
/// returns; this is that function with no Rust target triple.
pub fn stage_selected_authored_runtime_files(
    project_root: &Path,
    package_selectors: &[String],
    requested_profile: &str,
) -> Result<Vec<RuntimeFileStagingReport>, RuntimeFileStagingError> {
    stage_selected_authored_runtime_files_for_target(
        project_root,
        package_selectors,
        requested_profile,
        None,
    )
}

/// Stage runtime files for selected authored targets under an optional Rust
/// target triple.
///
/// Cross-target Cargo output lives under
/// `target/<target-triple>/<cargo-profile>/`; native output omits the target
/// triple. This keeps required runtime sidecars adjacent to the executable in
/// both layouts.
///
/// # Errors
///
/// Returns [`RuntimeFileStagingError::Project`] if the project graph at
/// `project_root` cannot be loaded or resolved,
/// [`RuntimeFileStagingError::Selector`] if `package_selectors` names no
/// buildable target or resolves ambiguously, and any error
/// [`stage_build_target_runtime_files`] returns for a selected target.
pub fn stage_selected_authored_runtime_files_for_target(
    project_root: &Path,
    package_selectors: &[String],
    requested_profile: &str,
    target_triple: Option<&str>,
) -> Result<Vec<RuntimeFileStagingReport>, RuntimeFileStagingError> {
    let graph = load_resolved_project_graph(project_root)?;
    let cargo_profile = resolve_cargo_profile_name(&graph, requested_profile);
    let selected = selected_authored_build_targets(&graph, package_selectors)?;
    let mut reports = Vec::new();
    for (owner_root, target) in selected {
        if target.runtime_files.is_empty() {
            continue;
        }
        let target_dir = owner_root.join("target");
        let output_dir =
            cargo_profile_output_dir_for_target(&target_dir, &cargo_profile, target_triple);
        reports.push(stage_build_target_runtime_files(
            &owner_root,
            target,
            &output_dir,
        )?);
    }
    Ok(reports)
}

fn selected_authored_build_targets<'a>(
    graph: &'a ResolvedProjectGraph,
    package_selectors: &[String],
) -> Result<Vec<(PathBuf, &'a ProjectBuildTarget)>, RuntimeFileStagingError> {
    let project_root = graph.root.clone();
    let mut authored: Vec<(PathBuf, String, &ProjectBuildTarget)> = Vec::new();
    for target in &graph.manifest.tools.build_targets {
        authored.push((
            project_root.clone(),
            graph.manifest.project.id.clone(),
            target,
        ));
    }
    for gem in &graph.gems {
        for target in &gem.manifest.tools.build_targets {
            authored.push((gem.root.clone(), gem.manifest.gem.id.clone(), target));
        }
    }

    if package_selectors.is_empty() {
        // Mirror daemon build selection: primary-gem projects only build generated
        // runtime targets by default, so they have nothing authored to stage.
        if graph.manifest.project.primary_gem.is_some() {
            return Ok(Vec::new());
        }
        return Ok(authored
            .into_iter()
            .filter(|(_, _, target)| target.default)
            .map(|(root, _, target)| (root, target))
            .collect());
    }

    // Match the build-plan selector catalog (generated + authored) so package
    // names and owner:target aliases resolve the same way as `azoth build`.
    let candidates = project_build_selector_candidates(graph);
    let selected_indices = resolve_project_build_selector_indices(&candidates, package_selectors)?;
    let authored_by_key = authored
        .into_iter()
        .map(|(root, owner_id, target)| ((owner_id, target.name.clone()), (root, target)))
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut selected = Vec::new();
    for index in selected_indices {
        let candidate = &candidates[index];
        let key = (candidate.owner_id.clone(), candidate.target_name.clone());
        if let Some((root, target)) = authored_by_key.get(&key) {
            selected.push((root.clone(), *target));
        }
    }
    Ok(selected)
}

/// Resolve Cargo's profile output directory under a target directory.
///
/// Matches Cargo's on-disk layout: `debug`/`release` stay as named, while custom
/// profiles live under `target/<profile>/`.
#[must_use]
pub fn cargo_profile_output_dir(target_dir: impl AsRef<Path>, cargo_profile: &str) -> PathBuf {
    let profile = cargo_profile.trim();
    let dir_name = match profile {
        "" | "debug" | "dev" => "debug",
        other => other,
    };
    target_dir.as_ref().join(dir_name)
}

/// Resolve a native or cross-target Cargo profile output directory.
#[must_use]
pub fn cargo_profile_output_dir_for_target(
    target_dir: impl AsRef<Path>,
    cargo_profile: &str,
    target_triple: Option<&str>,
) -> PathBuf {
    let target_dir = target_triple
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map_or_else(
            || target_dir.as_ref().to_path_buf(),
            |target| target_dir.as_ref().join(target),
        );
    cargo_profile_output_dir(target_dir, cargo_profile)
}

/// Stage each `target.runtime_files` entry from `owner_root` into `output_dir`.
///
/// - Source paths are owner-root-relative (validated at manifest parse time).
/// - Destination is `output_dir/<file_name>` (directory components are dropped).
/// - Existing destinations with identical content (length + blake3) are left
///   untouched so repeated builds stay timestamp-stable.
/// - Writes go through a same-directory temp file + atomic replace.
///
/// # Errors
///
/// Returns [`RuntimeFileStagingError::MissingSource`] if a declared entry names
/// no file under `owner_root`, [`RuntimeFileStagingError::MissingFileName`] if
/// it has no final path component, and [`RuntimeFileStagingError::Io`] if
/// `output_dir` cannot be created, the freshness comparison cannot read either
/// side, or the atomic temp-file write and replace fails.
pub fn stage_build_target_runtime_files(
    owner_root: &Path,
    target: &ProjectBuildTarget,
    output_dir: &Path,
) -> Result<RuntimeFileStagingReport, RuntimeFileStagingError> {
    let mut report = RuntimeFileStagingReport {
        target_name: target.name.clone(),
        output_dir: output_dir.to_path_buf(),
        entries: Vec::with_capacity(target.runtime_files.len()),
    };

    if target.runtime_files.is_empty() {
        return Ok(report);
    }

    fs::create_dir_all(output_dir).map_err(|source| RuntimeFileStagingError::Io {
        target: target.name.clone(),
        relative_source: String::new(),
        destination: output_dir.to_path_buf(),
        source,
    })?;

    for relative_source in &target.runtime_files {
        let source = owner_root.join(relative_source);
        if !source.is_file() {
            return Err(RuntimeFileStagingError::MissingSource {
                target: target.name.clone(),
                relative_source: relative_source.clone(),
                source_path: source,
            });
        }

        let file_name =
            source
                .file_name()
                .ok_or_else(|| RuntimeFileStagingError::MissingFileName {
                    target: target.name.clone(),
                    relative_source: relative_source.clone(),
                })?;
        let destination = output_dir.join(file_name);

        let action = if destination_matches_source(&source, &destination).map_err(|source| {
            RuntimeFileStagingError::Io {
                target: target.name.clone(),
                relative_source: relative_source.clone(),
                destination: destination.clone(),
                source,
            }
        })? {
            RuntimeFileStagingAction::AlreadyFresh
        } else {
            stage_file_atomically(&source, &destination).map_err(|source| {
                RuntimeFileStagingError::Io {
                    target: target.name.clone(),
                    relative_source: relative_source.clone(),
                    destination: destination.clone(),
                    source,
                }
            })?;
            info!(
                target = %target.name,
                source = %source.display(),
                destination = %destination.display(),
                "staged build target runtime file"
            );
            RuntimeFileStagingAction::Staged
        };

        report.entries.push(RuntimeFileStagingEntry {
            relative_source: relative_source.clone(),
            source,
            destination,
            action,
        });
    }

    Ok(report)
}

fn destination_matches_source(source: &Path, destination: &Path) -> Result<bool, std::io::Error> {
    let Ok(destination_meta) = fs::metadata(destination) else {
        return Ok(false);
    };
    let source_meta = fs::metadata(source)?;
    if source_meta.len() != destination_meta.len() {
        return Ok(false);
    }
    Ok(file_blake3(source)? == file_blake3(destination)?)
}

fn file_blake3(path: &Path) -> Result<[u8; 32], std::io::Error> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    // Heap-allocated: a 64 KiB stack array trips `clippy::large_stack_arrays`.
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(*hasher.finalize().as_bytes())
}

fn stage_file_atomically(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    let (temporary_path, mut temporary) = create_atomic_temp_file(destination)?;
    let result = (|| {
        let mut input = File::open(source)?;
        std::io::copy(&mut input, &mut temporary)?;
        temporary.sync_all()?;
        drop(temporary);
        atomic_replace(&temporary_path, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_atomic_temp_file(path: &Path) -> Result<(PathBuf, File), std::io::Error> {
    for _ in 0..ATOMIC_REPLACE_ATTEMPTS {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary_path = path.with_extension(format!(
            "azoth-runtime-{}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique runtime-file staging temporary file",
    ))
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut last_error = None;
    for attempt in 0..ATOMIC_REPLACE_ATTEMPTS {
        // SAFETY: both buffers are owned, NUL-terminated UTF-16 paths and stay
        // alive for the duration of the Win32 call.
        let result = unsafe {
            MoveFileExW(
                PCWSTR(source_wide.as_ptr()),
                PCWSTR(destination_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        match result {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < ATOMIC_REPLACE_ATTEMPTS {
            std::thread::sleep(ATOMIC_REPLACE_RETRY_DELAY);
        }
    }
    Err(std::io::Error::other(last_error.map_or_else(
        || "atomic replacement failed".to_string(),
        |error| error.to_string(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ProjectBuildTarget;

    fn target_with_runtime_files(files: &[&str]) -> ProjectBuildTarget {
        let mut target = ProjectBuildTarget::package("tool", "tool_pkg");
        target.runtime_files = files.iter().map(|path| (*path).to_string()).collect();
        target
    }

    #[test]
    fn cargo_profile_output_dir_maps_debug_aliases() {
        let root = PathBuf::from("target");
        assert_eq!(cargo_profile_output_dir(&root, "debug"), root.join("debug"));
        assert_eq!(cargo_profile_output_dir(&root, "dev"), root.join("debug"));
        assert_eq!(
            cargo_profile_output_dir(&root, "release"),
            root.join("release")
        );
        assert_eq!(cargo_profile_output_dir(&root, "dist"), root.join("dist"));
    }

    #[test]
    fn cargo_profile_output_dir_places_cross_target_before_profile() {
        let root = PathBuf::from("target");

        assert_eq!(
            cargo_profile_output_dir_for_target(&root, "debug", Some("x86_64-pc-windows-msvc")),
            root.join("x86_64-pc-windows-msvc").join("debug")
        );
        assert_eq!(
            cargo_profile_output_dir_for_target(&root, "release", None),
            root.join("release")
        );
    }

    #[test]
    fn stage_build_target_runtime_files_errors_when_source_missing() {
        let temp = tempfile::tempdir().unwrap();
        let owner = temp.path();
        let output = owner.join("target").join("debug");
        let target = target_with_runtime_files(&["resources/missing.dll"]);

        let err = stage_build_target_runtime_files(owner, &target, &output).unwrap_err();
        assert!(matches!(
            err,
            RuntimeFileStagingError::MissingSource {
                target,
                relative_source,
                ..
            } if target == "tool" && relative_source == "resources/missing.dll"
        ));
    }

    #[test]
    fn stage_build_target_runtime_files_copies_fresh_source() {
        let temp = tempfile::tempdir().unwrap();
        let owner = temp.path();
        let source_dir = owner.join("resources");
        fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("steam_api64.dll");
        fs::write(&source, b"steam-api-bytes").unwrap();

        let output = owner.join("target").join("debug");
        let target = target_with_runtime_files(&["resources/steam_api64.dll"]);

        let report = stage_build_target_runtime_files(owner, &target, &output).unwrap();
        assert_eq!(report.staged_count(), 1);
        assert_eq!(report.already_fresh_count(), 0);
        assert_eq!(
            fs::read(output.join("steam_api64.dll")).unwrap(),
            b"steam-api-bytes"
        );
    }

    #[test]
    fn stage_build_target_runtime_files_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let owner = temp.path();
        let source_dir = owner.join("resources");
        fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("sidecar.bin");
        fs::write(&source, b"payload-v1").unwrap();

        let output = owner.join("out");
        let target = target_with_runtime_files(&["resources/sidecar.bin"]);

        let first = stage_build_target_runtime_files(owner, &target, &output).unwrap();
        assert_eq!(first.staged_count(), 1);
        let destination = output.join("sidecar.bin");
        let first_modified = fs::metadata(&destination).unwrap().modified().unwrap();

        // Ensure timestamp resolution does not race the second staging pass.
        std::thread::sleep(Duration::from_millis(20));

        let second = stage_build_target_runtime_files(owner, &target, &output).unwrap();
        assert_eq!(second.staged_count(), 0);
        assert_eq!(second.already_fresh_count(), 1);
        assert_eq!(fs::read(&destination).unwrap(), b"payload-v1");
        let second_modified = fs::metadata(&destination).unwrap().modified().unwrap();
        assert_eq!(
            first_modified, second_modified,
            "idempotent staging must not churn destination timestamps"
        );
    }

    #[test]
    fn stage_build_target_runtime_files_replaces_stale_destination() {
        let temp = tempfile::tempdir().unwrap();
        let owner = temp.path();
        let source_dir = owner.join("resources");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("sidecar.bin"), b"new-bytes").unwrap();

        let output = owner.join("out");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("sidecar.bin"), b"old-bytes").unwrap();

        let target = target_with_runtime_files(&["resources/sidecar.bin"]);
        let report = stage_build_target_runtime_files(owner, &target, &output).unwrap();
        assert_eq!(report.staged_count(), 1);
        assert_eq!(fs::read(output.join("sidecar.bin")).unwrap(), b"new-bytes");
    }
}
