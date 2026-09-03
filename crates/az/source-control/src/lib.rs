//! Source-control provider boundary for Azoth workspaces.
//!
//! The only production provider today is Lore. This crate keeps Lore command
//! details out of session orchestration so editor/session code talks in source
//! control operations instead of Git command shapes.

use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use std::{env, thread};

use thiserror::Error;
use toml_edit::{DocumentMut, value};

#[derive(Debug, Error)]
pub enum SourceControlError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(
        "{} command failed in {}: {} {:?}; status={:?}; stdout={}; stderr={}",
        .0.plan.program, .0.plan.cwd.display(), .0.plan.program, .0.plan.args, .0.status,
        .0.output.stdout, .0.output.stderr
    )]
    CommandFailed(Box<CommandFailure>),

    #[error("could not parse {field} from source-control command output: {output}")]
    Parse { field: &'static str, output: String },

    #[error(
        "local Lore remote `{remote_url}` at `{endpoint}` was not reachable after {timeout_ms}ms"
    )]
    LocalLoreServerUnavailable {
        remote_url: String,
        endpoint: String,
        timeout_ms: u64,
    },

    #[error("could not parse Lore repository config {path}: {message}")]
    LoreConfigParse { path: PathBuf, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPlan {
    pub cwd: PathBuf,
    pub program: String,
    pub args: Vec<String>,
}

/// What was run and what came back when a source-control command exited
/// non-zero.
///
/// Carried behind a `Box` in [`SourceControlError::CommandFailed`]: inline it
/// is 128 bytes, which would make every `Result<_, SourceControlError>` in this
/// crate pay for the failure path (`clippy::result_large_err`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFailure {
    pub plan: CommandPlan,
    pub status: Option<i32>,
    pub output: CommandOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryInfo {
    pub repository_id: String,
    pub remote_url: String,
    pub default_branch: String,
    pub default_branch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRepositoryRequest {
    pub remote_url: String,
    pub path: PathBuf,
    pub description: Option<String>,
    pub use_shared_store: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub id: String,
    pub latest_revision: Option<String>,
    pub remote_latest_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    pub repository_id: String,
    pub branch: Option<String>,
    pub revision_number: Option<u64>,
    pub revision_id: Option<String>,
    pub remote_revision_number: Option<u64>,
    pub remote_revision_id: Option<String>,
    pub in_sync_with_remote: bool,
    pub changed_lines: Vec<String>,
    pub raw_output: String,
}

impl SourceStatus {
    #[must_use]
    pub const fn clean(&self) -> bool {
        self.changed_lines.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionSelector {
    Head,
    Branch(String),
    Revision(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneInstanceRequest {
    pub remote_url: String,
    pub destination: PathBuf,
    pub selector: RevisionSelector,
    pub use_shared_store: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageMode {
    KnownDirty,
    Scan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRequest {
    pub source_revision: Option<String>,
    pub target_revision: Option<String>,
    pub paths: Vec<String>,
    pub format: DiffFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffFormat {
    Patch,
    Stat,
    NameOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    pub target_branch: String,
    pub message: String,
}

/// The operations session orchestration needs from a source-control backend.
///
/// Every method here runs one provider command against an on-disk instance.
/// The shared failure paths are therefore the same throughout:
/// [`SourceControlError::Io`] when the provider binary cannot be spawned,
/// [`SourceControlError::LocalLoreServerUnavailable`] when the instance points
/// at a `lore://localhost` remote whose server does not answer before the start
/// timeout, and [`SourceControlError::CommandFailed`] when the command itself
/// exits non-zero. Methods that read structured data back add
/// [`SourceControlError::Parse`].
pub trait SourceControlProvider {
    /// Reconcile provider-owned repository metadata with portable project
    /// configuration before performing source-control operations.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`] if the instance's provider config
    /// cannot be read or rewritten, or [`SourceControlError::LoreConfigParse`]
    /// if that config is not valid TOML. The default implementation, which
    /// reconciles nothing, never fails.
    fn configure_repository_remote(
        &self,
        _instance: &Path,
        _remote_url: &str,
    ) -> Result<(), SourceControlError> {
        Ok(())
    }

    /// Create a new repository at `request.path` bound to `request.remote_url`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`] if the provider cannot be spawned,
    /// [`SourceControlError::LocalLoreServerUnavailable`] if the remote is a
    /// local `lore://` endpoint whose server does not start, or
    /// [`SourceControlError::CommandFailed`] if repository creation exits
    /// non-zero (for example the path is already a repository).
    fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<CommandOutput, SourceControlError>;

    /// Read the repository id, remote URL, and default branch of `instance`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] if the info command cannot run or
    /// exits non-zero — including when `instance` is not a repository — and
    /// [`SourceControlError::Parse`] if its output omits a field.
    fn repository_info(&self, instance: &Path) -> Result<RepositoryInfo, SourceControlError>;

    /// Report the working-copy status of `instance`, optionally rescanning the
    /// tree for changes the provider has not been told about.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] if the status command cannot run
    /// or exits non-zero, and [`SourceControlError::Parse`] if its output omits
    /// the repository id or carries an unparsable revision number.
    fn status(&self, instance: &Path, scan: bool) -> Result<SourceStatus, SourceControlError>;

    /// Name the branch `instance` is currently on, or `None` in a detached or
    /// branchless state.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::status`] returns.
    fn current_branch(&self, instance: &Path) -> Result<Option<String>, SourceControlError>;

    /// Describe `branch`, or return `Ok(None)` if no such branch exists.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] if the command cannot run or fails
    /// for a reason other than the branch being absent, and
    /// [`SourceControlError::Parse`] if the branch record omits a field.
    fn branch_info(
        &self,
        instance: &Path,
        branch: &str,
    ) -> Result<Option<BranchInfo>, SourceControlError>;

    /// Report whether `revision` resolves in `instance`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] if the command cannot run or fails
    /// for a reason other than the revision being absent, which is reported as
    /// `Ok(false)`.
    fn revision_exists(&self, instance: &Path, revision: &str) -> Result<bool, SourceControlError>;

    /// Clone `request.remote_url` into `request.destination` at the selected
    /// revision.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`] if the provider cannot be spawned,
    /// [`SourceControlError::LocalLoreServerUnavailable`] if the remote is a
    /// local `lore://` endpoint whose server does not start, or
    /// [`SourceControlError::CommandFailed`] if the clone exits non-zero (for
    /// example an unreachable remote or an unknown revision).
    fn clone_instance(
        &self,
        request: &CloneInstanceRequest,
    ) -> Result<CommandOutput, SourceControlError>;

    /// Create `branch` in `instance`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] — the last one notably when the
    /// branch already exists.
    fn create_branch(
        &self,
        instance: &Path,
        branch: &str,
    ) -> Result<CommandOutput, SourceControlError>;

    /// Switch `instance` to `branch`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] — the last one notably when the
    /// branch is unknown or the working copy blocks the switch.
    fn switch_branch(
        &self,
        instance: &Path,
        branch: &str,
    ) -> Result<CommandOutput, SourceControlError>;

    /// Tell the provider that `paths` changed without rescanning the tree.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] if the command exits non-zero.
    fn mark_dirty(
        &self,
        instance: &Path,
        paths: &[String],
    ) -> Result<CommandOutput, SourceControlError>;

    /// Stage `paths` — or the whole instance when `paths` is empty — under the
    /// given [`StageMode`].
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] if the command exits non-zero.
    fn stage(
        &self,
        instance: &Path,
        paths: &[String],
        mode: StageMode,
    ) -> Result<CommandOutput, SourceControlError>;

    /// Commit the staged contents of `instance` with `message`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] — the last one notably when there
    /// is nothing staged to commit.
    fn commit(&self, instance: &Path, message: &str) -> Result<CommandOutput, SourceControlError>;

    /// Produce a diff of `instance` per `request`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] — the last one notably when a
    /// requested revision does not resolve.
    fn diff(
        &self,
        instance: &Path,
        request: &DiffRequest,
    ) -> Result<CommandOutput, SourceControlError>;

    /// Bring `instance` up to `revision`, or to the remote head when `revision`
    /// is `None`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] — the last one notably when the
    /// sync conflicts with local changes.
    fn sync(
        &self,
        instance: &Path,
        revision: Option<&str>,
    ) -> Result<CommandOutput, SourceControlError>;

    /// Push `branch`, or the current branch when `branch` is `None`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] — the last one notably when the
    /// remote rejects the push as out of date.
    fn push(
        &self,
        instance: &Path,
        branch: Option<&str>,
    ) -> Result<CommandOutput, SourceControlError>;

    /// Merge `request.target_branch` into the current branch of `instance`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] — the last one notably when the
    /// merge stops on conflicts.
    fn merge_into(
        &self,
        instance: &Path,
        request: &MergeRequest,
    ) -> Result<CommandOutput, SourceControlError>;

    /// Mark `paths` as resolved in an in-progress merge.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] — the last one notably when no
    /// merge is in progress.
    fn resolve_merge(
        &self,
        instance: &Path,
        paths: &[String],
    ) -> Result<CommandOutput, SourceControlError>;

    /// Abandon the in-progress merge in `instance`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] — the last one notably when no
    /// merge is in progress.
    fn abort_merge(&self, instance: &Path) -> Result<CommandOutput, SourceControlError>;

    /// Report whether `instance` is mid-merge.
    ///
    /// # Errors
    ///
    /// Returns [`SourceControlError::Io`],
    /// [`SourceControlError::LocalLoreServerUnavailable`], or
    /// [`SourceControlError::CommandFailed`] if the probe cannot run or fails
    /// for a reason other than "no merge in progress", which is reported as
    /// `Ok(false)`.
    fn merge_in_progress(&self, instance: &Path) -> Result<bool, SourceControlError>;

    /// Describe, without running it, the command [`Self::push`] would run.
    fn push_plan(&self, instance: &Path, branch: Option<&str>) -> CommandPlan;
}

/// Forward every source-control operation to the pointed-to provider so a
/// `SessionManager<Arc<dyn SourceControlProvider + Send + Sync>>` can carry a
/// type-erased provider. This lets callers (e.g. `azd`) keep `LoreCli` as the
/// production default while injecting a test double behind the same `Arc`
/// without making the surrounding types generic.
impl<T> SourceControlProvider for std::sync::Arc<T>
where
    T: SourceControlProvider + ?Sized,
{
    fn configure_repository_remote(
        &self,
        instance: &Path,
        remote_url: &str,
    ) -> Result<(), SourceControlError> {
        (**self).configure_repository_remote(instance, remote_url)
    }
    fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).create_repository(request)
    }
    fn repository_info(&self, instance: &Path) -> Result<RepositoryInfo, SourceControlError> {
        (**self).repository_info(instance)
    }
    fn status(&self, instance: &Path, scan: bool) -> Result<SourceStatus, SourceControlError> {
        (**self).status(instance, scan)
    }
    fn current_branch(&self, instance: &Path) -> Result<Option<String>, SourceControlError> {
        (**self).current_branch(instance)
    }
    fn branch_info(
        &self,
        instance: &Path,
        branch: &str,
    ) -> Result<Option<BranchInfo>, SourceControlError> {
        (**self).branch_info(instance, branch)
    }
    fn revision_exists(&self, instance: &Path, revision: &str) -> Result<bool, SourceControlError> {
        (**self).revision_exists(instance, revision)
    }
    fn clone_instance(
        &self,
        request: &CloneInstanceRequest,
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).clone_instance(request)
    }
    fn create_branch(
        &self,
        instance: &Path,
        branch: &str,
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).create_branch(instance, branch)
    }
    fn switch_branch(
        &self,
        instance: &Path,
        branch: &str,
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).switch_branch(instance, branch)
    }
    fn mark_dirty(
        &self,
        instance: &Path,
        paths: &[String],
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).mark_dirty(instance, paths)
    }
    fn stage(
        &self,
        instance: &Path,
        paths: &[String],
        mode: StageMode,
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).stage(instance, paths, mode)
    }
    fn commit(&self, instance: &Path, message: &str) -> Result<CommandOutput, SourceControlError> {
        (**self).commit(instance, message)
    }
    fn diff(
        &self,
        instance: &Path,
        request: &DiffRequest,
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).diff(instance, request)
    }
    fn sync(
        &self,
        instance: &Path,
        revision: Option<&str>,
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).sync(instance, revision)
    }
    fn push(
        &self,
        instance: &Path,
        branch: Option<&str>,
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).push(instance, branch)
    }
    fn merge_into(
        &self,
        instance: &Path,
        request: &MergeRequest,
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).merge_into(instance, request)
    }
    fn resolve_merge(
        &self,
        instance: &Path,
        paths: &[String],
    ) -> Result<CommandOutput, SourceControlError> {
        (**self).resolve_merge(instance, paths)
    }
    fn abort_merge(&self, instance: &Path) -> Result<CommandOutput, SourceControlError> {
        (**self).abort_merge(instance)
    }
    fn merge_in_progress(&self, instance: &Path) -> Result<bool, SourceControlError> {
        (**self).merge_in_progress(instance)
    }
    fn push_plan(&self, instance: &Path, branch: Option<&str>) -> CommandPlan {
        (**self).push_plan(instance, branch)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LoreCli;

impl LoreCli {
    #[must_use]
    pub fn command_plan(cwd: impl Into<PathBuf>, args: Vec<String>) -> CommandPlan {
        CommandPlan {
            cwd: cwd.into(),
            program: "lore".to_string(),
            args: lore_args(args),
        }
    }

    fn run(cwd: &Path, args: Vec<String>) -> Result<CommandOutput, SourceControlError> {
        ensure_local_lore_server(cwd, &args)?;
        Self::run_once(cwd, args)
    }

    fn run_once(cwd: &Path, args: Vec<String>) -> Result<CommandOutput, SourceControlError> {
        let planned_args = lore_args(args);
        let output = Command::new("lore")
            .current_dir(cwd)
            .args(&planned_args)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if output.status.success() {
            Ok(CommandOutput { stdout, stderr })
        } else {
            Err(SourceControlError::CommandFailed(Box::new(
                CommandFailure {
                    plan: CommandPlan {
                        cwd: cwd.to_path_buf(),
                        program: "lore".to_string(),
                        args: planned_args,
                    },
                    status: output.status.code(),
                    output: CommandOutput { stdout, stderr },
                },
            )))
        }
    }
}

fn ensure_local_lore_server(cwd: &Path, args: &[String]) -> Result<(), SourceControlError> {
    let Some((remote_url, endpoint)) = local_lore_target(cwd, args)? else {
        return Ok(());
    };
    if local_lore_server_is_reachable(&endpoint) {
        return Ok(());
    }

    let mut command = Command::new(lore_server_program());
    if let Some(config_path) = lore_server_config_path() {
        command.arg("--config").arg(config_path);
    }
    command.current_dir(cwd);
    hide_background_process_window(&mut command);
    command.spawn()?;

    let timeout = local_lore_server_start_timeout();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if local_lore_server_is_reachable(&endpoint) {
            tracing::info!(
                remote_url,
                endpoint,
                "started local loreserver for Lore repository remote"
            );
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }

    Err(SourceControlError::LocalLoreServerUnavailable {
        remote_url,
        endpoint,
        timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
    })
}

fn local_lore_target(
    cwd: &Path,
    args: &[String],
) -> Result<Option<(String, String)>, SourceControlError> {
    if let Some(remote_url) = args
        .iter()
        .find(|arg| parse_local_lore_endpoint(arg).is_some())
    {
        let endpoint = parse_local_lore_endpoint(remote_url).expect("checked above");
        return Ok(Some((remote_url.clone(), endpoint)));
    }
    let Some(remote_url) = read_lore_remote_url(cwd)? else {
        return Ok(None);
    };
    Ok(parse_local_lore_endpoint(&remote_url).map(|endpoint| (remote_url, endpoint)))
}

fn read_lore_remote_url(cwd: &Path) -> Result<Option<String>, SourceControlError> {
    let config_path = cwd.join(".lore").join("config.toml");
    if !config_path.exists() {
        return Ok(None);
    }
    let config = std::fs::read_to_string(config_path)?;
    Ok(parse_lore_remote_url(&config))
}

fn parse_lore_remote_url(config: &str) -> Option<String> {
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return None;
        }
        let Some(value) = line.strip_prefix("remote_url") else {
            continue;
        };
        let value = value.trim_start();
        let value = value.strip_prefix('=')?.trim();
        let value = value.strip_prefix('"')?.split_once('"')?.0.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// Update Lore's ignored instance config from the portable project manifest.
///
/// Returns `false` when the path is not a Lore repository yet. Repository
/// creation writes the same remote URL itself.
///
/// # Errors
///
/// Returns [`SourceControlError::Io`] if `.lore/config.toml` exists but cannot
/// be read or written back, or [`SourceControlError::LoreConfigParse`] if that
/// file is not valid TOML.
pub fn reconcile_lore_remote_url(
    instance: &Path,
    remote_url: &str,
) -> Result<bool, SourceControlError> {
    let config_path = instance.join(".lore").join("config.toml");
    if !config_path.exists() {
        return Ok(false);
    }

    let config = std::fs::read_to_string(&config_path)?;
    if parse_lore_remote_url(&config).as_deref() == Some(remote_url) {
        return Ok(true);
    }

    let mut document =
        config
            .parse::<DocumentMut>()
            .map_err(|source| SourceControlError::LoreConfigParse {
                path: config_path.clone(),
                message: source.to_string(),
            })?;
    document["remote_url"] = value(remote_url);
    std::fs::write(&config_path, document.to_string())?;
    tracing::info!(
        path = %config_path.display(),
        remote_url,
        "reconciled Lore repository remote from project configuration"
    );
    Ok(true)
}

fn parse_local_lore_endpoint(remote_url: &str) -> Option<String> {
    let authority_and_path = remote_url.strip_prefix("lore://")?;
    let authority = authority_and_path
        .split_once('/')
        .map_or(authority_and_path, |(authority, _)| authority);
    let (host, port) = if let Some(host_and_port) = authority.strip_prefix('[') {
        let (host, rest) = host_and_port.split_once(']')?;
        let port = rest
            .strip_prefix(':')
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or(41337);
        (host, port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        (host, port.parse::<u16>().ok()?)
    } else {
        (authority, 41337)
    };
    let normalized_host = match host {
        "localhost" | "127.0.0.1" | "::1" => "127.0.0.1",
        _ => return None,
    };
    Some(format!("{normalized_host}:{port}"))
}

fn local_lore_server_is_reachable(endpoint: &str) -> bool {
    let Ok(addresses) = endpoint.to_socket_addrs() else {
        return false;
    };
    addresses.into_iter().any(|address| {
        std::net::TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok()
    })
}

fn lore_server_program() -> String {
    env::var("AZOTH_LORE_SERVER_BIN")
        .or_else(|_| env::var("LORE_SERVER_BIN"))
        .unwrap_or_else(|_| "loreserver".to_string())
}

fn lore_server_config_path() -> Option<PathBuf> {
    env::var_os("AZOTH_LORE_SERVER_CONFIG")
        .or_else(|| env::var_os("LORE_CONFIG_PATH"))
        .map(PathBuf::from)
        .or_else(default_user_lore_server_config_path)
        .filter(|path| path.exists())
}

fn default_user_lore_server_config_path() -> Option<PathBuf> {
    let home = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"))?;
    Some(PathBuf::from(home).join("loreserver").join("config"))
}

fn local_lore_server_start_timeout() -> Duration {
    env::var("AZOTH_LORE_SERVER_START_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or_else(|| Duration::from_mins(1), Duration::from_millis)
}

#[cfg(windows)]
fn hide_background_process_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_background_process_window(_command: &mut Command) {}

impl SourceControlProvider for LoreCli {
    fn configure_repository_remote(
        &self,
        instance: &Path,
        remote_url: &str,
    ) -> Result<(), SourceControlError> {
        reconcile_lore_remote_url(instance, remote_url).map(|_| ())
    }

    fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<CommandOutput, SourceControlError> {
        let mut args = vec!["repository".to_string(), "create".to_string()];
        if request.use_shared_store {
            args.push("--use-shared-store".to_string());
        }
        args.push("--repository".to_string());
        args.push(request.path.to_string_lossy().to_string());
        if let Some(description) = &request.description {
            args.push("--description".to_string());
            args.push(description.clone());
        }
        args.push(request.remote_url.clone());
        Self::run(
            request.path.parent().unwrap_or_else(|| Path::new(".")),
            args,
        )
    }

    fn repository_info(&self, instance: &Path) -> Result<RepositoryInfo, SourceControlError> {
        let output = Self::run(instance, vec!["repository".to_string(), "info".to_string()])?;
        parse_repository_info(&output.stdout)
    }

    fn status(&self, instance: &Path, scan: bool) -> Result<SourceStatus, SourceControlError> {
        let mut args = vec!["status".to_string()];
        if scan {
            args.push("--scan".to_string());
        }
        let output = Self::run(instance, args)?;
        parse_source_status(&output.stdout)
    }

    fn current_branch(&self, instance: &Path) -> Result<Option<String>, SourceControlError> {
        Ok(self.status(instance, false)?.branch)
    }

    fn branch_info(
        &self,
        instance: &Path,
        branch: &str,
    ) -> Result<Option<BranchInfo>, SourceControlError> {
        match Self::run(
            instance,
            vec!["branch".to_string(), "info".to_string(), branch.to_string()],
        ) {
            Ok(output) => parse_branch_info(&output.stdout).map(Some),
            Err(SourceControlError::CommandFailed(failure))
                if failure.status == Some(1)
                    && command_says_not_found(&failure.output.stdout, &failure.output.stderr) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn revision_exists(&self, instance: &Path, revision: &str) -> Result<bool, SourceControlError> {
        match Self::run(
            instance,
            vec![
                "revision".to_string(),
                "info".to_string(),
                revision.to_string(),
            ],
        ) {
            Ok(_) => Ok(true),
            Err(SourceControlError::CommandFailed(failure))
                if failure.status == Some(1)
                    && command_says_not_found(&failure.output.stdout, &failure.output.stderr) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn clone_instance(
        &self,
        request: &CloneInstanceRequest,
    ) -> Result<CommandOutput, SourceControlError> {
        let mut args = vec!["repository".to_string(), "clone".to_string()];
        if request.use_shared_store {
            args.push("--use-shared-store".to_string());
        }
        match &request.selector {
            RevisionSelector::Head => {}
            RevisionSelector::Branch(branch) => {
                args.push("--branch".to_string());
                args.push(branch.clone());
            }
            RevisionSelector::Revision(revision) => {
                args.push("--revision".to_string());
                args.push(revision.clone());
            }
        }
        args.push(request.remote_url.clone());
        args.push(request.destination.to_string_lossy().to_string());
        let cwd = request
            .destination
            .parent()
            .unwrap_or_else(|| Path::new("."));
        Self::run(cwd, args)
    }

    fn create_branch(
        &self,
        instance: &Path,
        branch: &str,
    ) -> Result<CommandOutput, SourceControlError> {
        Self::run(
            instance,
            vec![
                "branch".to_string(),
                "create".to_string(),
                branch.to_string(),
            ],
        )
    }

    fn switch_branch(
        &self,
        instance: &Path,
        branch: &str,
    ) -> Result<CommandOutput, SourceControlError> {
        Self::run(
            instance,
            vec![
                "branch".to_string(),
                "switch".to_string(),
                branch.to_string(),
            ],
        )
    }

    fn mark_dirty(
        &self,
        instance: &Path,
        paths: &[String],
    ) -> Result<CommandOutput, SourceControlError> {
        let mut args = vec!["dirty".to_string()];
        args.extend(paths.iter().cloned());
        Self::run(instance, args)
    }

    fn stage(
        &self,
        instance: &Path,
        paths: &[String],
        mode: StageMode,
    ) -> Result<CommandOutput, SourceControlError> {
        let mut args = vec!["stage".to_string()];
        if mode == StageMode::Scan {
            args.push("--scan".to_string());
        }
        if paths.is_empty() {
            args.push(".".to_string());
        } else {
            args.extend(paths.iter().cloned());
        }
        Self::run(instance, args)
    }

    fn commit(&self, instance: &Path, message: &str) -> Result<CommandOutput, SourceControlError> {
        Self::run(instance, vec!["commit".to_string(), message.to_string()])
    }

    fn diff(
        &self,
        instance: &Path,
        request: &DiffRequest,
    ) -> Result<CommandOutput, SourceControlError> {
        let mut args = vec!["diff".to_string()];
        if let Some(source) = &request.source_revision {
            args.push("--source".to_string());
            args.push(source.clone());
        }
        if let Some(target) = &request.target_revision {
            args.push("--target".to_string());
            args.push(target.clone());
        }
        // Lore's public diff command currently exposes unified diff output only,
        // so no format adds arguments; these requests still go through Lore and
        // are filtered by callers. Spelled out variant by variant so a new
        // `DiffFormat` fails to compile here instead of being silently ignored.
        match request.format {
            DiffFormat::Patch | DiffFormat::Stat | DiffFormat::NameOnly => {}
        }
        args.extend(request.paths.iter().cloned());
        Self::run(instance, args)
    }

    fn sync(
        &self,
        instance: &Path,
        revision: Option<&str>,
    ) -> Result<CommandOutput, SourceControlError> {
        let mut args = vec!["sync".to_string()];
        if let Some(revision) = revision {
            args.push(revision.to_string());
        }
        Self::run(instance, args)
    }

    fn push(
        &self,
        instance: &Path,
        branch: Option<&str>,
    ) -> Result<CommandOutput, SourceControlError> {
        let mut args = vec!["push".to_string()];
        if let Some(branch) = branch {
            args.push(branch.to_string());
        }
        Self::run(instance, args)
    }

    fn merge_into(
        &self,
        instance: &Path,
        request: &MergeRequest,
    ) -> Result<CommandOutput, SourceControlError> {
        Self::run(
            instance,
            vec![
                "branch".to_string(),
                "merge".to_string(),
                "into".to_string(),
                request.target_branch.clone(),
                request.message.clone(),
            ],
        )
    }

    fn resolve_merge(
        &self,
        instance: &Path,
        paths: &[String],
    ) -> Result<CommandOutput, SourceControlError> {
        let mut args = vec![
            "branch".to_string(),
            "merge".to_string(),
            "resolve".to_string(),
        ];
        args.extend(paths.iter().cloned());
        Self::run(instance, args)
    }

    fn abort_merge(&self, instance: &Path) -> Result<CommandOutput, SourceControlError> {
        Self::run(
            instance,
            vec![
                "branch".to_string(),
                "merge".to_string(),
                "abort".to_string(),
            ],
        )
    }

    fn merge_in_progress(&self, instance: &Path) -> Result<bool, SourceControlError> {
        match Self::run(
            instance,
            vec![
                "branch".to_string(),
                "merge".to_string(),
                "resolve".to_string(),
                "--dry-run".to_string(),
            ],
        ) {
            Ok(_) => Ok(true),
            Err(SourceControlError::CommandFailed(failure))
                if command_says_no_merge(&failure.output.stdout, &failure.output.stderr) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn push_plan(&self, instance: &Path, branch: Option<&str>) -> CommandPlan {
        let mut args = vec!["push".to_string()];
        if let Some(branch) = branch {
            args.push(branch.to_string());
        }
        Self::command_plan(instance, args)
    }
}

fn lore_args(args: Vec<String>) -> Vec<String> {
    let mut planned = vec!["--no-pager".to_string(), "--non-interactive".to_string()];
    planned.extend(args);
    planned
}

fn parse_repository_info(output: &str) -> Result<RepositoryInfo, SourceControlError> {
    let first_line = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| parse_error("repository id", output))?;
    let repository_id = first_line
        .trim()
        .rsplit_once('(')
        .and_then(|(_, rest)| rest.strip_suffix(')'))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| parse_error("repository id", output))?
        .to_string();
    let remote_url = value_after_prefix(output, "Remote URL:")
        .ok_or_else(|| parse_error("remote URL", output))?
        .to_string();
    let default_branch_line = value_after_prefix(output, "Default branch:")
        .ok_or_else(|| parse_error("default branch", output))?;
    let (default_branch, default_branch_id) = parse_branch_with_id(default_branch_line)
        .ok_or_else(|| parse_error("default branch", output))?;

    Ok(RepositoryInfo {
        repository_id,
        remote_url,
        default_branch,
        default_branch_id,
    })
}

fn parse_branch_info(output: &str) -> Result<BranchInfo, SourceControlError> {
    let name = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Branch "))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| parse_error("branch name", output))?
        .to_string();
    let id = value_after_prefix(output, "ID:")
        .ok_or_else(|| parse_error("branch id", output))?
        .to_string();
    let latest_revision = value_after_prefix(output, "Latest:").map(str::to_string);
    let remote_latest_revision = value_after_prefix(output, "Remote Latest:").map(str::to_string);
    Ok(BranchInfo {
        name,
        id,
        latest_revision,
        remote_latest_revision,
    })
}

fn parse_source_status(output: &str) -> Result<SourceStatus, SourceControlError> {
    let repository_id = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Repository "))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| parse_error("status repository id", output))?
        .to_string();
    let mut branch = None;
    let mut revision_number = None;
    let mut revision_id = None;
    let mut remote_revision_number = None;
    let mut remote_revision_id = None;
    let mut in_sync_with_remote = false;
    let mut changed_lines = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("On branch ") {
            if let Some((branch_name, number, revision)) = parse_status_revision_line(rest) {
                branch = Some(branch_name);
                revision_number = Some(number);
                revision_id = Some(revision);
            }
        } else if let Some(rest) = trimmed.strip_prefix("Remote revision ") {
            if let Some((number, revision)) = parse_revision_number_and_id(rest) {
                remote_revision_number = Some(number);
                remote_revision_id = Some(revision);
            }
        } else if trimmed == "Local branch in sync with remote" {
            in_sync_with_remote = true;
        } else if is_lore_change_line(trimmed) {
            changed_lines.push(trimmed.to_string());
        }
    }

    Ok(SourceStatus {
        repository_id,
        branch,
        revision_number,
        revision_id,
        remote_revision_number,
        remote_revision_id,
        in_sync_with_remote,
        changed_lines,
        raw_output: output.to_string(),
    })
}

fn value_after_prefix<'a>(output: &'a str, prefix: &str) -> Option<&'a str> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix(prefix)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn parse_branch_with_id(line: &str) -> Option<(String, String)> {
    let (name, id) = line.rsplit_once('(')?;
    Some((
        name.trim().to_string(),
        id.strip_suffix(')')?.trim().to_string(),
    ))
}

fn parse_status_revision_line(rest: &str) -> Option<(String, u64, String)> {
    let (branch, revision) = rest.split_once(" revision ")?;
    let (number, revision_id) = parse_revision_number_and_id(revision.trim())?;
    Some((branch.trim().to_string(), number, revision_id))
}

fn parse_revision_number_and_id(rest: &str) -> Option<(u64, String)> {
    let (number, revision_id) = rest.split_once(" -> ")?;
    Some((number.trim().parse().ok()?, revision_id.trim().to_string()))
}

fn is_lore_change_line(line: &str) -> bool {
    let mut chars = line.chars();
    let Some(status) = chars.next() else {
        return false;
    };
    matches!(status, 'A' | 'M' | 'D' | 'R' | 'C' | '?' | '!')
        && chars.next().is_some_and(char::is_whitespace)
}

fn command_says_not_found(stdout: &str, stderr: &str) -> bool {
    stdout.contains("Not found") || stderr.contains("Not found")
}

fn command_says_no_merge(stdout: &str, stderr: &str) -> bool {
    let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    text.contains("no merge")
        || text.contains("not currently merging")
        || text.contains("not found")
}

fn parse_error(field: &'static str, output: &str) -> SourceControlError {
    SourceControlError::Parse {
        field,
        output: output.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repository_info() {
        let info = parse_repository_info(
            "az-rs (019ee1086fac7a109cfc4b1f460d363e)\n\nRemote URL: lore://127.0.0.1:41337\nDefault branch: main (e726318bbc3fd75ac8733a7e030cc35b)\n",
        )
        .unwrap();

        assert_eq!(info.repository_id, "019ee1086fac7a109cfc4b1f460d363e");
        assert_eq!(info.remote_url, "lore://127.0.0.1:41337");
        assert_eq!(info.default_branch, "main");
    }

    #[test]
    fn parses_status() {
        let status = parse_source_status(
            "Repository 019ee1086fac7a109cfc4b1f460d363e\nOn branch main revision 1 -> abc\nRemote revision 1 -> abc\nLocal branch in sync with remote\nM src/lib.rs\n",
        )
        .unwrap();

        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.revision_id.as_deref(), Some("abc"));
        assert_eq!(status.changed_lines, vec!["M src/lib.rs"]);
        assert!(!status.clean());
    }

    #[test]
    fn parses_loopback_lore_remote_endpoints() {
        assert_eq!(
            parse_local_lore_endpoint("lore://127.0.0.1:41337"),
            Some("127.0.0.1:41337".to_string())
        );
        assert_eq!(
            parse_local_lore_endpoint("lore://localhost"),
            Some("127.0.0.1:41337".to_string())
        );
        assert_eq!(
            parse_local_lore_endpoint("lore://[::1]:41338/repository"),
            Some("127.0.0.1:41338".to_string())
        );
    }

    #[test]
    fn ignores_non_loopback_lore_remote_endpoints() {
        assert_eq!(parse_local_lore_endpoint("https://example.com"), None);
        assert_eq!(parse_local_lore_endpoint("lore://192.0.2.10:41337"), None);
        assert_eq!(parse_local_lore_endpoint("lore://example.com:41337"), None);
    }

    #[test]
    fn finds_loopback_lore_target_from_command_args() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("example");
        let args = vec![
            "repository".to_string(),
            "clone".to_string(),
            "lore://127.0.0.1:41339".to_string(),
            destination.to_string_lossy().into_owned(),
        ];

        assert_eq!(
            local_lore_target(temp.path(), &args).unwrap(),
            Some((
                "lore://127.0.0.1:41339".to_string(),
                "127.0.0.1:41339".to_string()
            ))
        );
    }

    #[test]
    fn parses_lore_remote_url_without_owning_full_lore_config_schema() {
        let config = r#"
remote_url = "lore://127.0.0.1:41337"
identity = "118081862+themixednuts@users.noreply.github.com"

[store]
max_capacity = 10485760
eviction_delay = 10
max_size = 10737418240
compaction_delay = 30

[file]
direct_write = false
direct_io = false
flush_write = false
"#;

        assert_eq!(
            parse_lore_remote_url(config),
            Some("lore://127.0.0.1:41337".to_string())
        );
    }

    #[test]
    fn reconciles_lore_remote_without_replacing_instance_settings() {
        let temp = tempfile::tempdir().unwrap();
        let lore_dir = temp.path().join(".lore");
        std::fs::create_dir(&lore_dir).unwrap();
        std::fs::write(
            lore_dir.join("config.toml"),
            r#"remote_url = "lore://127.0.0.1:41337"
identity = "developer@example.com"

[store]
max_capacity = 10485760
"#,
        )
        .unwrap();

        assert!(reconcile_lore_remote_url(temp.path(), "lore://192.0.2.10:41337").unwrap());

        let config = std::fs::read_to_string(lore_dir.join("config.toml")).unwrap();
        assert!(config.contains("remote_url = \"lore://192.0.2.10:41337\""));
        assert!(config.contains("identity = \"developer@example.com\""));
        assert!(config.contains("[store]"));
        assert!(config.contains("max_capacity = 10485760"));
    }

    #[test]
    fn recognizes_lore_0_8_no_merge_diagnostic() {
        assert!(command_says_no_merge(
            "No conflicts resolved",
            "[Error] No merge is in progress\n  at lore-revision/src/branch/merge.rs:303:1"
        ));
    }
}
