//! Lore-materialized engine resolution.
//!
//! A project records *which* engine it builds against as a committed `[engine]`
//! Lore pointer in `azoth.toml` ("where") plus an exact revision pinned in
//! `azoth.lock` ("which"). This module turns that pin into an on-disk engine
//! root by materializing the revision through Lore — never git — and reusing an
//! already-materialized revision on subsequent runs.

use std::path::{Path, PathBuf};

use az_filesystem::AzothDataHome;
use az_source_control::{CloneInstanceRequest, RevisionSelector, SourceControlProvider};

use crate::manifest::{
    AZOTH_ENGINE_ROOT_ENV, LockedEngineSource, ProjectEngineSource, ProjectManifestError,
    engine_manifest_path, load_project_lock, resolve_engine_root, resolve_engine_root_at,
};

/// Overrides the directory under which Lore-materialized engines are cached.
pub const AZOTH_ENGINE_CACHE_ENV: &str = "AZOTH_ENGINE_CACHE";

/// Root directory under which Lore-materialized engines are cached, keyed by
/// engine id + revision so multiple pinned revisions coexist and sibling
/// projects share a single materialization. Overridable via
/// `AZOTH_ENGINE_CACHE`; otherwise `~/.azoth/runtime/engines`.
#[must_use]
pub fn engine_cache_root() -> PathBuf {
    if let Some(dir) = std::env::var_os(AZOTH_ENGINE_CACHE_ENV) {
        return PathBuf::from(dir);
    }
    AzothDataHome::resolve().engine_installations_dir()
}

/// Content-addressed materialization directory for one pinned engine revision.
#[must_use]
pub fn engine_revision_dir(cache_root: &Path, engine_id: &str, revision: &str) -> PathBuf {
    cache_root.join(engine_id).join(revision)
}

/// Ensure the engine pinned by `source` is present on disk at its exact Lore
/// revision, cloning it through `provider` (Lore) on first use and returning the
/// engine root. Idempotent: an already-materialized revision is reused without
/// touching Lore.
pub fn materialize_engine(
    engine_id: &str,
    source: &LockedEngineSource,
    cache_root: &Path,
    provider: &dyn SourceControlProvider,
) -> Result<PathBuf, ProjectManifestError> {
    let dest = engine_revision_dir(cache_root, engine_id, &source.revision);
    if engine_manifest_path(&dest).is_file() {
        return Ok(dest);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|err| ProjectManifestError::Write {
            path: parent.to_path_buf(),
            source: err,
        })?;
    }
    provider.clone_instance(&CloneInstanceRequest {
        remote_url: source.lore.clone(),
        destination: dest.clone(),
        selector: RevisionSelector::Revision(source.revision.clone()),
        use_shared_store: true,
    })?;
    Ok(dest)
}

/// Read the engine's current Lore revision from its working tree, producing the
/// `azoth.lock` pin. Called at lock-refresh time so the lock records the precise
/// revision the project's `[engine]` source currently resolves to.
pub fn pin_engine_revision(
    engine_root: &Path,
    source: &ProjectEngineSource,
    provider: &dyn SourceControlProvider,
) -> Result<LockedEngineSource, ProjectManifestError> {
    let status = provider.status(engine_root, false)?;
    let revision = status
        .revision_id
        .ok_or(ProjectManifestError::MissingLockedEngineRevision)?;
    Ok(LockedEngineSource {
        lore: source.lore.clone(),
        branch: source.branch.clone(),
        revision,
    })
}

/// Resolve the engine root for a project, honoring the Lore pin. Precedence:
/// 1. an explicit `AZOTH_ENGINE_ROOT` override (local-dev / CI escape hatch),
/// 2. the `azoth.lock` engine Lore pin, materialized through `provider`,
/// 3. the default walk-up from the current working directory.
pub fn resolve_engine_root_for_project(
    project_root: &Path,
    provider: &dyn SourceControlProvider,
) -> Result<PathBuf, ProjectManifestError> {
    if std::env::var_os(AZOTH_ENGINE_ROOT_ENV).is_some() {
        return resolve_engine_root();
    }
    // A missing lock means there is no engine pin yet (e.g. a project being
    // initialized, before its lock is committed). Fall back to env/default
    // resolution and let lock-aware callers surface the missing lock instead.
    let lock = match load_project_lock(project_root) {
        Ok(lock) => lock,
        Err(ProjectManifestError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            return resolve_engine_root();
        }
        Err(error) => return Err(error),
    };
    if let Some(source) = &lock.engine.source {
        let cache_root = if std::env::var_os(AZOTH_ENGINE_CACHE_ENV).is_some() {
            engine_cache_root()
        } else {
            let data_home = AzothDataHome::resolve();
            data_home.prepare()?;
            data_home.engine_installations_dir()
        };
        let root = materialize_engine(&lock.engine.id, source, &cache_root, provider)?;
        return resolve_engine_root_at(root);
    }
    resolve_engine_root()
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_source_control::{
        BranchInfo, CloneInstanceRequest, CommandOutput, CommandPlan, CreateRepositoryRequest,
        DiffRequest, MergeRequest, RepositoryInfo, SourceControlError, SourceStatus, StageMode,
    };
    use std::cell::RefCell;

    /// Records clone requests and fakes a Lore working tree by writing an
    /// `engine.toml` at the clone destination. `unimplemented!` guards the
    /// methods these tests never exercise.
    #[derive(Default)]
    struct FakeLore {
        clones: RefCell<Vec<CloneInstanceRequest>>,
        revision: String,
    }

    fn unsupported() -> SourceControlError {
        SourceControlError::Parse {
            field: "test",
            output: String::new(),
        }
    }

    impl SourceControlProvider for FakeLore {
        fn create_repository(
            &self,
            _request: &CreateRepositoryRequest,
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn repository_info(&self, _instance: &Path) -> Result<RepositoryInfo, SourceControlError> {
            Err(unsupported())
        }
        fn status(
            &self,
            _instance: &Path,
            _scan: bool,
        ) -> Result<SourceStatus, SourceControlError> {
            Ok(SourceStatus {
                repository_id: "engine".to_string(),
                branch: Some("main".to_string()),
                revision_number: Some(7),
                revision_id: Some(self.revision.clone()),
                remote_revision_number: None,
                remote_revision_id: None,
                in_sync_with_remote: true,
                changed_lines: Vec::new(),
                raw_output: String::new(),
            })
        }
        fn current_branch(&self, _instance: &Path) -> Result<Option<String>, SourceControlError> {
            Err(unsupported())
        }
        fn branch_info(
            &self,
            _instance: &Path,
            _branch: &str,
        ) -> Result<Option<BranchInfo>, SourceControlError> {
            Err(unsupported())
        }
        fn revision_exists(
            &self,
            _instance: &Path,
            _revision: &str,
        ) -> Result<bool, SourceControlError> {
            Err(unsupported())
        }
        fn clone_instance(
            &self,
            request: &CloneInstanceRequest,
        ) -> Result<CommandOutput, SourceControlError> {
            std::fs::create_dir_all(&request.destination).unwrap();
            std::fs::write(
                request.destination.join("engine.toml"),
                "[manifest]\nkind = \"engine\"\nschema = \"azoth.engine/v1\"\n\n[engine]\nid = \"azoth\"\nname = \"Azoth\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
            self.clones.borrow_mut().push(request.clone());
            Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }
        fn create_branch(
            &self,
            _instance: &Path,
            _branch: &str,
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn switch_branch(
            &self,
            _instance: &Path,
            _branch: &str,
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn mark_dirty(
            &self,
            _instance: &Path,
            _paths: &[String],
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn stage(
            &self,
            _instance: &Path,
            _paths: &[String],
            _mode: StageMode,
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn commit(
            &self,
            _instance: &Path,
            _message: &str,
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn diff(
            &self,
            _instance: &Path,
            _request: &DiffRequest,
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn sync(
            &self,
            _instance: &Path,
            _revision: Option<&str>,
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn push(
            &self,
            _instance: &Path,
            _branch: Option<&str>,
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn merge_into(
            &self,
            _instance: &Path,
            _request: &MergeRequest,
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn resolve_merge(
            &self,
            _instance: &Path,
            _paths: &[String],
        ) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn abort_merge(&self, _instance: &Path) -> Result<CommandOutput, SourceControlError> {
            Err(unsupported())
        }
        fn merge_in_progress(&self, _instance: &Path) -> Result<bool, SourceControlError> {
            Err(unsupported())
        }
        fn push_plan(&self, instance: &Path, _branch: Option<&str>) -> CommandPlan {
            CommandPlan {
                cwd: instance.to_path_buf(),
                program: "lore".to_string(),
                args: Vec::new(),
            }
        }
    }

    fn source(revision: &str) -> LockedEngineSource {
        LockedEngineSource {
            lore: "lore://127.0.0.1:41337".to_string(),
            branch: "main".to_string(),
            revision: revision.to_string(),
        }
    }

    #[test]
    fn materializes_pinned_revision_via_lore_once() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let provider = FakeLore {
            revision: "abc123".to_string(),
            ..Default::default()
        };
        let src = source("abc123");

        let root = materialize_engine("azoth", &src, &cache, &provider).unwrap();
        assert_eq!(root, engine_revision_dir(&cache, "azoth", "abc123"));
        assert!(engine_manifest_path(&root).is_file());
        assert_eq!(provider.clones.borrow().len(), 1);
        let request = provider.clones.borrow()[0].clone();
        assert_eq!(
            request.selector,
            RevisionSelector::Revision("abc123".to_string())
        );
        assert!(request.use_shared_store);

        // Second call reuses the materialized revision without touching Lore.
        let root_again = materialize_engine("azoth", &src, &cache, &provider).unwrap();
        assert_eq!(root_again, root);
        assert_eq!(provider.clones.borrow().len(), 1);
    }

    #[test]
    fn distinct_revisions_coexist() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let provider = FakeLore {
            revision: "rev".to_string(),
            ..Default::default()
        };

        let one = materialize_engine("azoth", &source("rev-one"), &cache, &provider).unwrap();
        let two = materialize_engine("azoth", &source("rev-two"), &cache, &provider).unwrap();
        assert_ne!(one, two);
        assert_eq!(provider.clones.borrow().len(), 2);
    }

    #[test]
    fn pins_revision_from_engine_working_tree() {
        let provider = FakeLore {
            revision: "deadbeef".to_string(),
            ..Default::default()
        };
        let manifest_source = ProjectEngineSource {
            lore: "lore://127.0.0.1:41337".to_string(),
            branch: "main".to_string(),
        };
        let pin = pin_engine_revision(Path::new("engine"), &manifest_source, &provider).unwrap();
        assert_eq!(
            pin,
            LockedEngineSource {
                lore: "lore://127.0.0.1:41337".to_string(),
                branch: "main".to_string(),
                revision: "deadbeef".to_string(),
            }
        );
    }
}
