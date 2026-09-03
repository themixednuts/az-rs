use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use az_filesystem::{
    AzothDataHome, FileIdentity, FileTransaction, FileWrite, file_identity, normalize,
};
use az_observability::{
    DiagnosticBundleSourceFile, STRUCTURED_LOG_EXTENSION, write_diagnostic_bundle,
};
use az_proto_asset::{
    ASSET_JOBS_PERMISSION, ASSET_PROCESSOR_AUDIENCE, ASSET_PROCESSOR_NAMESPACE,
    ASSET_PROCESSOR_SERVICE_NAME, ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION,
};
pub use az_proto_asset::{ASSET_WORKER_SERVICE_NAME, ASSET_WORKER_SERVICE_NAMESPACE};
use az_proto_core::{
    Capability, CapabilityGrantRequirement, CapabilityGrantSet, Endpoint, EndpointKind,
    SERVICE_LIFECYCLE_AUDIENCE, SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION, ServiceDescriptor,
    ServiceId, ServiceRole, encode_capability_grant_set,
};
pub use az_proto_core::{EDITOR_SERVICE_NAME, EDITOR_SERVICE_NAMESPACE};
use az_proto_observability::{OBSERVABILITY_AUDIENCE, OBSERVABILITY_CONTROL_PERMISSION};
use az_proto_project::{
    PROJECT_DOCUMENT_READ_PERMISSION, PROJECT_DOCUMENT_WRITE_PERMISSION, PROJECT_EDIT_PERMISSION,
    PROJECT_GAMEDATA_PERMISSION, PROJECT_GRAPH_CATALOG_PERMISSION, PROJECT_HOST_AUDIENCE,
    PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME, PROJECT_INVENTORY_PERMISSION,
    PROJECT_NODE_CATALOG_PERMISSION, PROJECT_RUNTIME_LAUNCH_PERMISSION, PROJECT_SCHEMA_PERMISSION,
    PROJECT_SOURCE_NAVIGATION_PERMISSION,
};
use az_proto_runtime::{
    RUNTIME_CONTROL_PERMISSION, RUNTIME_HOST_AUDIENCE, RUNTIME_HOST_NAMESPACE,
    RUNTIME_HOST_SERVICE_NAME, RUNTIME_READ_PERMISSION,
};
use az_proto_session::{
    SESSION_EXEC_PERMISSION, SESSION_MANAGE_PERMISSION, SESSION_READ_PERMISSION,
    SESSION_SAVE_PERMISSION, SESSION_SUPERVISOR_AUDIENCE, SESSION_SUPERVISOR_NAMESPACE,
    SESSION_SUPERVISOR_SERVICE_NAME,
};
use az_service_catalog::{
    DAEMON_SERVICE_NAME, DAEMON_SERVICE_NAMESPACE, add_observability_contract,
    add_service_lifecycle_contract, is_observability_control_grant, is_service_lifecycle_grant,
    runtime_host_service_descriptor, session_supervisor_service_descriptor,
};
use az_service_supervision::{
    ProcessIdentity, SERVICE_READY_SCHEMA_VERSION, ServiceEndpointKind, ServiceProcessKey,
    ServiceProcessRecord, ServiceProcessState, ServiceReadyRecord, ServiceRecord,
    SupervisedServiceRole, rotate_log_at_plan_time,
};
use thiserror::Error;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use crate::{
    SESSION_MANIFEST_FILE, SESSION_SCHEMA_VERSION, SessionId, SessionManifest, SessionState,
};

pub const AZOTH_IPC_DIR_ENV: &str = "AZOTH_IPC_DIR";

/// Return the short-lived IPC directory for one session.
///
/// Unix-domain sockets have a small platform path limit (108 bytes on Linux
/// and 104 bytes on macOS), so they cannot live under the durable project and
/// session metadata hierarchy. The directory remains session-scoped while the
/// durable manifest keeps the endpoint address needed for discovery.
///
/// # Errors
///
/// Returns [`SessionError::Io`] if the session's IPC directory cannot be
/// created, or — on Unix — if its permissions cannot be tightened to `0700`.
pub fn session_ipc_dir(session_id: SessionId) -> Result<PathBuf, SessionError> {
    let root = std::env::var_os(AZOTH_IPC_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map_or_else(default_session_ipc_root, PathBuf::from);
    let session_id = session_id.0.simple().to_string();
    let session_token = &session_id[session_id.len() - 24..];
    let session_dir = root.join(session_token);
    fs::create_dir_all(&session_dir)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o700))?;
    }

    Ok(session_dir)
}

#[cfg(unix)]
fn default_session_ipc_root() -> PathBuf {
    // SAFETY: `geteuid` has no preconditions and only reads process identity.
    let effective_user_id = unsafe { libc::geteuid() };
    PathBuf::from("/tmp").join(format!("az-{effective_user_id}"))
}

#[cfg(not(unix))]
fn default_session_ipc_root() -> PathBuf {
    std::env::temp_dir().join("azoth-ipc")
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML deserialize error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),

    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("project manifest error: {0}")]
    ProjectManifest(#[from] az_project::ProjectManifestError),

    #[error("package manifest error: {0}")]
    PackageManifest(#[from] az_asset::PackageManifestError),

    #[error("file transaction error: {0}")]
    FileTransaction(#[from] az_filesystem::FileTransactionError),

    #[error(transparent)]
    ServiceProcess(#[from] az_service_supervision::ServiceProcessError),

    #[error(
        "session `{session}` cannot recover service `{service}` because recorded-process cleanup was refused: {cleanup:?}"
    )]
    ServiceProcessCleanupRefused {
        session: String,
        service: String,
        cleanup: az_service_supervision::RecordedServiceProcessCleanup,
    },

    #[error("machine-local Azoth data-home error: {0}")]
    DataHome(#[from] az_filesystem::DataHomeError),

    #[error("capability grant encode error: {0}")]
    CapabilityGrantEncode(#[from] capnp::Error),

    #[error("invalid session name `{0}`")]
    InvalidSessionName(String),

    #[error("invalid session command: {message}")]
    InvalidSessionCommand { message: String },

    #[error(
        "session-supervisor command queue is busy; retry after the current operation completes"
    )]
    SupervisorCommandBusy,

    #[error("session-supervisor status broker is unavailable")]
    StatusBrokerUnavailable,

    #[error("session `{0}` already exists")]
    SessionAlreadyExists(String),

    #[error("session `{0}` was not found")]
    SessionNotFound(String),

    #[error(
        "session `{session}` belongs to project `{actual_project_id}`, expected `{expected_project_id}`"
    )]
    SessionProjectMismatch {
        session: String,
        expected_project_id: String,
        actual_project_id: String,
    },

    #[error(
        "session `{session}` manifest schema version `{actual}` is not supported; expected `{expected}`"
    )]
    UnsupportedManifestSchemaVersion {
        session: String,
        expected: u32,
        actual: u32,
    },

    #[error("session `{session}` is {state}; recovery only applies to failed-preserved sessions")]
    SessionNotFailedPreserved { session: String, state: String },

    #[error("session `{session}` is {state}; `{operation}` requires an active session")]
    SessionNotActive {
        session: String,
        state: String,
        operation: &'static str,
    },

    #[error("session `{session}` recovery is blocked by service `{service}`: {reason}")]
    SessionRecoveryBlocked {
        session: String,
        service: String,
        reason: String,
    },

    #[error(
        "session `{session}` has running services; stop the session supervisor before removing: {services:?}"
    )]
    SessionServicesRunning {
        session: String,
        services: Vec<String>,
    },

    #[error("session `{session}` workspace validation failed: {reason}")]
    SessionWorkspaceInvalid { session: String, reason: String },

    #[error("session `{session}` manifest validation failed: {reason}")]
    SessionManifestInvalid { session: String, reason: String },

    #[error("session `{session}` package output `{profile}` is invalid: {reason}")]
    InvalidPackageOutput {
        session: String,
        profile: String,
        reason: String,
    },

    #[error("service role `{role:?}` cannot be registered in a session manifest")]
    UnsupportedServiceRole { role: ServiceRole },

    #[error("session `{session}` service descriptor `{service}` is invalid: {reason}")]
    InvalidServiceDescriptor {
        session: String,
        service: String,
        reason: String,
    },

    #[error("session `{session}` service launch plan for `{service}` is invalid: {reason}")]
    InvalidServiceLaunchPlan {
        session: String,
        service: String,
        reason: String,
    },

    #[error(
        "{operation} cannot use endpoint kind {kind:?}; use platform IPC or explicit TCP debug endpoints"
    )]
    UnsupportedEndpointKind {
        operation: &'static str,
        kind: EndpointKind,
    },

    #[error("{operation} endpoint {kind:?} address `{address}` is invalid: {reason}")]
    InvalidEndpointAddress {
        operation: &'static str,
        kind: EndpointKind,
        address: String,
        reason: String,
    },

    #[error("session `{session}` has no descriptor for service `{service}`")]
    MissingServiceDescriptor { session: String, service: String },

    #[error("session `{session}` has no process record for service `{service}`")]
    MissingServiceProcess { session: String, service: String },

    #[error(
        "session `{session}` service `{service}` cannot start because dependency `{dependency}` is {dependency_state}"
    )]
    ServiceDependencyUnavailable {
        session: String,
        service: String,
        dependency: String,
        dependency_state: String,
    },

    #[error("session `{session}` service `{service}` has no readiness file")]
    MissingServiceReadyFile { session: String, service: String },

    #[error("session `{session}` service `{service}` readiness record is invalid: {reason}")]
    InvalidServiceReadyRecord {
        session: String,
        service: String,
        reason: String,
    },

    #[error("service `{service}` exited before publishing readiness; status={exit_code:?}")]
    ServiceProcessExitedBeforeReady {
        service: String,
        exit_code: Option<i32>,
    },

    #[error("service `{service}` did not publish readiness within {timeout_ms}ms")]
    ServiceProcessReadyTimeout { service: String, timeout_ms: u64 },

    #[error(
        "session `{session}` service `{service}` failed protocol boundary probe for role `{role:?}`: {reason}"
    )]
    ServiceBoundaryProbeFailed {
        session: String,
        service: String,
        role: ServiceRole,
        reason: String,
    },

    #[error("service `{service}` spawn failed for `{program}`: {source}")]
    ServiceProcessSpawnFailed {
        service: String,
        program: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSessionRequest {
    pub name: String,
}

impl CreateSessionRequest {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStatus {
    pub manifest: SessionManifest,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionServiceLaunchCommand {
    pub owner_id: String,
    pub owner_root: PathBuf,
    pub build_output_root: PathBuf,
    pub service_name: String,
    pub role: ServiceRole,
    pub endpoint: Endpoint,
    pub program: String,
    pub cwd: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionServiceLaunchContext {
    pub otlp_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredServiceDescriptor {
    pub manifest: SessionManifest,
    pub descriptor: ServiceDescriptor,
}

impl SessionServiceLaunchContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_otlp_endpoint(mut self, endpoint: Option<impl Into<String>>) -> Self {
        self.otlp_endpoint = endpoint.map(Into::into);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceDescriptorPlanMode {
    UseExisting,
    GenerateCanonical,
}

/// The state of a manifest file, as the filesystem reports it without reading
/// a byte of content.
///
/// [`FileTransaction`] publishes a newly staged file by rename. File identity,
/// rather than timestamp granularity, therefore distinguishes publications;
/// length and modification time cheaply detect in-place changes from external
/// tools. Platforms that cannot expose file identity do not use the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ManifestState {
    byte_length: u64,
    modified_unix_ns: u128,
    file_identity: Option<FileIdentity>,
}

impl ManifestState {
    fn capture(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::metadata(path)?;
        let modified = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|source| std::io::Error::new(std::io::ErrorKind::InvalidData, source))?;
        Ok(Self {
            byte_length: metadata.len(),
            modified_unix_ns: modified.as_nanos(),
            file_identity: file_identity(path, &metadata),
        })
    }

    const fn is_cacheable(self) -> bool {
        self.file_identity.is_some()
    }
}

/// Parsed session manifests keyed by manifest path, each paired with the
/// [`ManifestState`] it was parsed from.
type ManifestCache = Arc<Mutex<HashMap<PathBuf, (ManifestState, Arc<SessionManifest>)>>>;

#[derive(Debug, Clone)]
pub struct SessionManager {
    project_id: String,
    project_root: PathBuf,
    azoth_dir: PathBuf,
    sessions_dir: PathBuf,
    /// The manifests this manager has already parsed, each held against the
    /// file state it was parsed from.
    ///
    /// Locating a session is a keyed lookup over a directory of manifests, but
    /// the directory is keyed by session id, so the slug can only be read out
    /// of the manifests themselves. `az-sessiond` asks that question on every
    /// poll, and a manifest grows a `[[processes]]` row per launch, so parsing
    /// on every ask makes an idle loop cost more the longer the machine has
    /// been in use. Parsing is therefore driven by the file changing, not by
    /// being asked.
    manifests: ManifestCache,
}

impl SessionManager {
    /// Opens the manager for the project at `project_root`, resolving the
    /// machine-local Azoth data home from the environment.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::with_data_home`] returns.
    pub fn new(project_root: impl AsRef<Path>) -> Result<Self, SessionError> {
        Self::with_data_home(project_root, AzothDataHome::resolve())
    }

    /// Opens the manager for the project at `project_root` against an explicit
    /// data home.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::ProjectManifest`] if the project graph at
    /// `project_root` cannot be resolved, and [`SessionError::DataHome`] if the
    /// project's machine-local directories cannot be prepared or its
    /// pre-ADR-0030 state cannot be migrated.
    pub fn with_data_home(
        project_root: impl AsRef<Path>,
        data_home: impl Into<AzothDataHome>,
    ) -> Result<Self, SessionError> {
        let data_home = data_home.into();
        let project_root = normalize(project_root.as_ref());
        let project_graph = az_project::load_resolved_project_graph(&project_root)?;
        let data_paths = data_home.project(&project_graph.manifest.project.name, &project_root);
        let migration = data_paths.prepare()?;
        if migration.copied_files > 0 {
            info!(
                copied_files = migration.copied_files,
                project = %project_graph.manifest.project.id,
                "migrated legacy project state into the machine-local Azoth data home"
            );
        }
        let azoth_dir = data_paths.root().to_path_buf();
        let sessions_dir = data_paths.sessions_dir();

        Ok(Self {
            project_id: project_graph.manifest.project.id,
            project_root,
            azoth_dir,
            sessions_dir,
            manifests: Arc::default(),
        })
    }

    #[must_use]
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    #[must_use]
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    #[must_use]
    pub fn azoth_dir(&self) -> &Path {
        &self.azoth_dir
    }

    #[must_use]
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    /// Creates a new session workspace for `request` and activates it.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidSessionName`] if the requested name does
    /// not reduce to a valid slug, [`SessionError::SessionAlreadyExists`] if
    /// that slug is already taken, [`SessionError::Io`] if the sessions or run
    /// directory cannot be created, and the manifest read/write errors
    /// [`SessionError::TomlDeserialize`], [`SessionError::TomlSerialize`], and
    /// [`SessionError::FileTransaction`].
    #[instrument(skip(self))]
    pub fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<SessionManifest, SessionError> {
        let CreateSessionRequest { name } = request;
        let slug = session_slug(&name)?;
        fs::create_dir_all(&self.sessions_dir)?;

        if self.find_session(&slug)?.is_some() {
            return Err(SessionError::SessionAlreadyExists(slug));
        }

        let id = SessionId::new();
        let run_dir = self.sessions_dir.join(id.to_string());

        fs::create_dir_all(&run_dir)?;
        let mut manifest = SessionManifest::new(
            id,
            self.project_id.clone(),
            slug,
            self.project_root.clone(),
            self.project_root.clone(),
            run_dir,
            now_unix_ms(),
        );
        Self::write_manifest(&manifest)?;

        info!(
            session = %manifest.slug,
            workspace = %manifest.workspace_root.display(),
            "created session supervision scope for existing workspace"
        );

        manifest.activate(now_unix_ms());
        Self::write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Every session manifest under this project's sessions directory, ordered
    /// by slug.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Io`] if the sessions directory cannot be
    /// enumerated or a manifest cannot be read,
    /// [`SessionError::TomlDeserialize`] if a manifest is not valid TOML, and
    /// [`SessionError::UnsupportedManifestSchemaVersion`],
    /// [`SessionError::SessionProjectMismatch`], or
    /// [`SessionError::SessionManifestInvalid`] if a manifest fails identity
    /// validation.
    pub fn list_sessions(&self) -> Result<Vec<SessionManifest>, SessionError> {
        if !self.sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            let manifest_path = entry.path().join(SESSION_MANIFEST_FILE);
            if manifest_path.exists() {
                sessions.push((*self.read_manifest(&manifest_path)?).clone());
            }
        }
        sessions.sort_by(|a, b| a.slug.cmp(&b.slug));
        Ok(sessions)
    }

    /// The session's manifest paired with its preserved failure note, if any.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns, plus any error
    /// [`Self::failure_reason_for_manifest`] returns.
    pub fn status(&self, name: &str) -> Result<SessionStatus, SessionError> {
        let slug = session_slug(name)?;
        let manifest = self.session_by_slug(&slug)?;
        let failure_reason = Self::failure_reason_for_manifest(&manifest)?;
        Ok(SessionStatus {
            manifest,
            failure_reason,
        })
    }

    /// The manifest of the session `name` names.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidSessionName`] if `name` does not reduce
    /// to a valid slug, [`SessionError::SessionNotFound`] if no manifest
    /// carries that slug, [`SessionError::Io`] if the sessions directory or a
    /// manifest cannot be read, [`SessionError::TomlDeserialize`] if a manifest
    /// is not valid TOML, and
    /// [`SessionError::UnsupportedManifestSchemaVersion`],
    /// [`SessionError::SessionProjectMismatch`], or
    /// [`SessionError::SessionManifestInvalid`] if a manifest fails identity
    /// validation.
    pub fn session(&self, name: &str) -> Result<SessionManifest, SessionError> {
        Ok((*self.snapshot(name)?).clone())
    }

    /// The same manifest [`Self::session`] returns, shared rather than copied.
    ///
    /// Callers that only read it — polling loops above all — should take this
    /// one so that asking costs nothing beyond confirming the file has not
    /// changed.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns.
    pub fn snapshot(&self, name: &str) -> Result<Arc<SessionManifest>, SessionError> {
        let slug = session_slug(name)?;
        self.snapshot_by_slug(&slug)
    }

    /// Reads the session and confirms its workspace still resolves.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns, plus
    /// [`SessionError::SessionWorkspaceInvalid`] if the workspace root is
    /// missing, its project graph does not load, or that graph names a
    /// different project than the manifest records.
    pub fn validate_session_workspace(&self, name: &str) -> Result<SessionManifest, SessionError> {
        let manifest = self.session(name)?;
        Self::validate_existing_session_workspace(&manifest)?;
        Ok(manifest)
    }

    /// Reads the session and refuses `operation` unless it is active.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns, plus
    /// [`SessionError::SessionNotActive`] if the session is in any state other
    /// than [`SessionState::Active`].
    pub fn require_active_session(
        &self,
        name: &str,
        operation: &'static str,
    ) -> Result<SessionManifest, SessionError> {
        let manifest = self.session(name)?;
        ensure_session_active_for(&manifest, operation)?;
        Ok(manifest)
    }

    /// The preserved failure note for the session `name` names, if one exists.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns, plus any error
    /// [`Self::failure_reason_for_manifest`] returns.
    pub fn failure_reason(&self, name: &str) -> Result<Option<String>, SessionError> {
        let manifest = self.session(name)?;
        Self::failure_reason_for_manifest(&manifest)
    }

    /// Reads the `failure.txt` a preserved failure leaves in the run directory.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Io`] if the note exists but cannot be read; an
    /// absent note is reported as `Ok(None)`, not an error.
    pub fn failure_reason_for_manifest(
        manifest: &SessionManifest,
    ) -> Result<Option<String>, SessionError> {
        match fs::read_to_string(manifest.run_dir.join("failure.txt")) {
            Ok(reason) => Ok(Some(reason)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Removes the session's run directory and marks the manifest removed.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns,
    /// [`SessionError::SessionServicesRunning`] if the session still has
    /// starting or running process records,
    /// [`SessionError::SessionManifestInvalid`] if the manifest's run directory
    /// is not this manager's canonical path for that session id, and
    /// [`SessionError::Io`] if the run directory cannot be removed.
    pub fn retire_session(&self, name: &str) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;
        Self::ensure_no_running_services(&manifest)?;

        self.validate_session_run_dir(&manifest)?;
        if manifest.run_dir.exists()
            && let Err(error) = fs::remove_dir_all(&manifest.run_dir)
        {
            return Err(SessionError::Io(error));
        }

        manifest.state = SessionState::Removed;
        manifest.updated_unix_ms = now_unix_ms();
        Ok(manifest)
    }

    /// Moves the session to [`SessionState::FailedPreserved`] and writes
    /// `reason` beside the manifest.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns, [`SessionError::Io`] if the
    /// run directory or the failure note cannot be written, and the
    /// manifest-write errors [`SessionError::TomlSerialize`] and
    /// [`SessionError::FileTransaction`]. A diagnostic-bundle failure is logged
    /// rather than returned.
    pub fn mark_failed_preserved(
        &self,
        name: &str,
        reason: &str,
    ) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;

        Self::preserve_manifest_failure(&mut manifest, reason)?;
        Ok(manifest)
    }

    /// Marks the session removed without touching its run directory.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns, plus the manifest-write
    /// errors [`SessionError::Io`], [`SessionError::TomlSerialize`], and
    /// [`SessionError::FileTransaction`].
    pub fn mark_removed(&self, name: &str) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;
        manifest.state = SessionState::Removed;
        manifest.updated_unix_ms = now_unix_ms();
        Self::write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Returns a failed-preserved session to [`SessionState::Active`] and drops
    /// its failure note.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns,
    /// [`SessionError::SessionNotFailedPreserved`] unless the session is in
    /// [`SessionState::FailedPreserved`],
    /// [`SessionError::SessionWorkspaceInvalid`] if its workspace no longer
    /// resolves, [`SessionError::Io`] if the failure note cannot be removed,
    /// and the manifest-write errors [`SessionError::TomlSerialize`] and
    /// [`SessionError::FileTransaction`].
    pub fn recover_session(
        &self,
        name: &str,
        _force: bool,
    ) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;

        if manifest.state != SessionState::FailedPreserved {
            return Err(SessionError::SessionNotFailedPreserved {
                session: slug,
                state: session_state_label(manifest.state).to_string(),
            });
        }

        Self::validate_existing_session_workspace(&manifest)?;

        remove_file_if_exists(&manifest.run_dir.join("failure.txt"))?;
        manifest.activate(now_unix_ms());
        Self::write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Records `descriptor` in the session manifest, or returns the manifest
    /// unchanged when an equivalent publication is already durable.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns,
    /// [`SessionError::UnsupportedEndpointKind`] or
    /// [`SessionError::InvalidEndpointAddress`] if one of the descriptor's
    /// endpoints is not publicly reachable,
    /// [`SessionError::InvalidServiceDescriptor`] if the descriptor's service
    /// id or brokered capabilities do not match its role and this session,
    /// [`SessionError::UnsupportedServiceRole`] if the role cannot be recorded
    /// in a session manifest, and the manifest-write errors
    /// [`SessionError::Io`], [`SessionError::TomlSerialize`], and
    /// [`SessionError::FileTransaction`].
    pub fn register_service_descriptor(
        &self,
        name: &str,
        descriptor: &ServiceDescriptor,
    ) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;
        validate_session_service_descriptor(
            &manifest.slug,
            manifest.id,
            descriptor,
            "session service descriptor registration",
        )?;
        let record = ServiceRecord::from_descriptor(descriptor).ok_or(
            SessionError::UnsupportedServiceRole {
                role: descriptor.role,
            },
        )?;
        if manifest
            .services
            .iter()
            .any(|existing| existing.same_durable_publication(&record))
        {
            return Ok(manifest);
        }
        manifest
            .upsert_service_descriptor(descriptor, now_unix_ms())
            .ok_or(SessionError::UnsupportedServiceRole {
                role: descriptor.role,
            })?;
        Self::write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Mints the canonical runtime-host descriptor for `endpoint` and records
    /// it in the session manifest.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::UnsupportedEndpointKind`] or
    /// [`SessionError::InvalidEndpointAddress`] if `endpoint` is in-process,
    /// empty, or not a parsable TCP address; any error [`Self::session`]
    /// returns; [`SessionError::UnsupportedServiceRole`] if the minted
    /// descriptor's role cannot be recorded; and the manifest-write errors
    /// [`SessionError::Io`], [`SessionError::TomlSerialize`], and
    /// [`SessionError::FileTransaction`].
    pub fn register_runtime_host_descriptor(
        &self,
        name: &str,
        endpoint: Endpoint,
    ) -> Result<RegisteredServiceDescriptor, SessionError> {
        validate_public_endpoint(&endpoint, "session runtime-host descriptor registration")?;
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;
        let now = now_unix_ms();
        let run = Uuid::now_v7();
        let mut descriptor = runtime_host_service_descriptor(manifest.id.0, run, endpoint);
        add_observability_contract(&mut descriptor, Some(manifest.id.0));
        add_service_lifecycle_contract(
            &mut descriptor,
            Some(manifest.id.0),
            ServiceId::new(
                SESSION_SUPERVISOR_NAMESPACE,
                SESSION_SUPERVISOR_SERVICE_NAME,
            ),
            ServiceRole::SessionSupervisor,
        );
        manifest.upsert_service_descriptor(&descriptor, now).ok_or(
            SessionError::UnsupportedServiceRole {
                role: descriptor.role,
            },
        )?;
        Self::write_manifest(&manifest)?;
        Ok(RegisteredServiceDescriptor {
            manifest,
            descriptor,
        })
    }

    /// Mints the canonical session-supervisor descriptor for `run` at
    /// `endpoint` and records it in the session manifest.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::UnsupportedEndpointKind`] or
    /// [`SessionError::InvalidEndpointAddress`] if `endpoint` is in-process,
    /// empty, or not a parsable TCP address; any error [`Self::session`]
    /// returns; [`SessionError::UnsupportedServiceRole`] if the minted
    /// descriptor's role cannot be recorded; and the manifest-write errors
    /// [`SessionError::Io`], [`SessionError::TomlSerialize`], and
    /// [`SessionError::FileTransaction`].
    pub fn register_session_supervisor_descriptor(
        &self,
        name: &str,
        run: Uuid,
        endpoint: Endpoint,
    ) -> Result<RegisteredServiceDescriptor, SessionError> {
        validate_public_endpoint(&endpoint, "session supervisor descriptor registration")?;
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;
        let now = now_unix_ms();
        let descriptor = session_supervisor_service_descriptor(manifest.id.0, run, endpoint);
        manifest.upsert_service_descriptor(&descriptor, now).ok_or(
            SessionError::UnsupportedServiceRole {
                role: descriptor.role,
            },
        )?;
        Self::write_manifest(&manifest)?;
        Ok(RegisteredServiceDescriptor {
            manifest,
            descriptor,
        })
    }

    /// The descriptor the session published for `id` in `role`, if any.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns.
    pub fn service_descriptor(
        &self,
        name: &str,
        id: &ServiceId,
        role: ServiceRole,
    ) -> Result<Option<ServiceDescriptor>, SessionError> {
        Ok(self.session(name)?.service_descriptor(id, role))
    }

    /// Attaches project-owned service descriptors to an active session so its
    /// runtime host can reach them.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns,
    /// [`SessionError::SessionNotActive`] or
    /// [`SessionError::SessionWorkspaceInvalid`] if the session is not an
    /// active, resolvable workspace, [`SessionError::InvalidServiceLaunchPlan`]
    /// if the manifest already carries a project-owned process record,
    /// [`SessionError::UnsupportedEndpointKind`],
    /// [`SessionError::InvalidEndpointAddress`], or
    /// [`SessionError::InvalidServiceDescriptor`] if an attachment descriptor
    /// does not validate, [`SessionError::UnsupportedServiceRole`] if its role
    /// cannot be attached, and the manifest-write errors [`SessionError::Io`],
    /// [`SessionError::TomlSerialize`], and [`SessionError::FileTransaction`].
    pub fn attach_project_service_descriptors(
        &self,
        name: &str,
        descriptors: &[ServiceDescriptor],
    ) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.active_session_workspace(&slug, "attach project services")?;
        if let Some(process) = manifest.processes.iter().find(|process| {
            matches!(
                process.role,
                SupervisedServiceRole::ProjectHost
                    | SupervisedServiceRole::AssetProcessor
                    | SupervisedServiceRole::Worker
            )
        }) {
            return Err(SessionError::InvalidServiceLaunchPlan {
                session: slug,
                service: process.service_name.clone(),
                reason: "project-owned service process state cannot live in a session manifest"
                    .to_string(),
            });
        }
        let now = now_unix_ms();
        for descriptor in descriptors {
            validate_project_service_attachment_descriptor(
                &manifest.slug,
                descriptor,
                "attach project services",
            )?;
            manifest.attach_service_descriptor(descriptor, now).ok_or(
                SessionError::UnsupportedServiceRole {
                    role: descriptor.role,
                },
            )?;
        }
        Self::write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Records `commands` as the session's launch plan against the descriptors
    /// already registered for them.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::record_service_launch_plan_with_context`]
    /// returns.
    pub fn record_service_launch_plan(
        &self,
        name: &str,
        commands: &[SessionServiceLaunchCommand],
    ) -> Result<SessionManifest, SessionError> {
        self.record_service_launch_plan_with_context(
            name,
            commands,
            &SessionServiceLaunchContext::default(),
        )
    }

    /// Records `commands` as the session's launch plan, threading `context`
    /// into every child's argument vector.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns;
    /// [`SessionError::InvalidSessionCommand`] if `context` carries an
    /// unusable OTLP endpoint; [`SessionError::InvalidServiceLaunchPlan`] if a
    /// command names a role other than [`ServiceRole::RuntimeHost`], repeats a
    /// service name, or matches no registered descriptor;
    /// [`SessionError::UnsupportedEndpointKind`],
    /// [`SessionError::InvalidEndpointAddress`], or
    /// [`SessionError::InvalidServiceDescriptor`] if a command's endpoint or
    /// its descriptor's observability and lifecycle contracts do not validate;
    /// [`SessionError::UnsupportedServiceRole`] if a role cannot be recorded;
    /// [`SessionError::CapabilityGrantEncode`] if a capability grant set cannot
    /// be encoded; and the filesystem errors [`SessionError::Io`],
    /// [`SessionError::TomlSerialize`], and [`SessionError::FileTransaction`]
    /// raised while creating the log, ready, grant, and side-channel
    /// directories and committing the manifest.
    pub fn record_service_launch_plan_with_context(
        &self,
        name: &str,
        commands: &[SessionServiceLaunchCommand],
        context: &SessionServiceLaunchContext,
    ) -> Result<SessionManifest, SessionError> {
        self.record_service_launch_plan_inner(
            name,
            commands,
            context,
            ServiceDescriptorPlanMode::UseExisting,
        )
    }

    /// Mints canonical descriptors for `commands` and records them as the
    /// session's launch plan.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::record_planned_services_with_context`]
    /// returns.
    pub fn record_planned_services(
        &self,
        name: &str,
        commands: &[SessionServiceLaunchCommand],
    ) -> Result<SessionManifest, SessionError> {
        self.record_planned_services_with_context(
            name,
            commands,
            &SessionServiceLaunchContext::default(),
        )
    }

    /// Mints canonical descriptors for `commands`, rewrites their endpoints to
    /// match, and records the result as the session's launch plan.
    ///
    /// # Errors
    ///
    /// Returns every error
    /// [`Self::record_service_launch_plan_with_context`] returns; the minting
    /// step additionally raises [`SessionError::UnsupportedServiceRole`] and
    /// [`SessionError::InvalidServiceLaunchPlan`] when a command's role has no
    /// canonical session descriptor or its generated endpoint cannot be
    /// applied back to the command.
    pub fn record_planned_services_with_context(
        &self,
        name: &str,
        commands: &[SessionServiceLaunchCommand],
        context: &SessionServiceLaunchContext,
    ) -> Result<SessionManifest, SessionError> {
        self.record_service_launch_plan_inner(
            name,
            commands,
            context,
            ServiceDescriptorPlanMode::GenerateCanonical,
        )
    }

    fn record_service_launch_plan_inner(
        &self,
        name: &str,
        commands: &[SessionServiceLaunchCommand],
        context: &SessionServiceLaunchContext,
        descriptor_mode: ServiceDescriptorPlanMode,
    ) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;
        validate_service_launch_context(&slug, context)?;
        let now = now_unix_ms();
        validate_launch_plan_commands(&slug, commands)?;

        let mut commands = commands.to_vec();
        if descriptor_mode == ServiceDescriptorPlanMode::GenerateCanonical {
            let mut planned_manifest = manifest.clone();
            let descriptors =
                upsert_planned_service_descriptors(&mut planned_manifest, &commands, now)?;
            apply_planned_descriptor_endpoints(&slug, &mut commands, &descriptors)?;
            manifest = planned_manifest;
        }

        let dirs = LaunchPlanDirs::prepare(&manifest.run_dir)?;
        let mut capability_grant_writes = Vec::with_capacity(commands.len());
        for command in &commands {
            validate_public_endpoint(&command.endpoint, "session service launch plan")?;
            let role = SupervisedServiceRole::from_proto(command.role)
                .ok_or(SessionError::UnsupportedServiceRole { role: command.role })?;
            validate_service_launch_command(&slug, command)?;
            let descriptor = plan_service_descriptor(&slug, &mut manifest, command, role, now)?;
            let descriptor_record = ServiceRecord::from_descriptor(&descriptor).ok_or(
                SessionError::UnsupportedServiceRole {
                    role: descriptor.role,
                },
            )?;
            let paths = PlannedProcessPaths::new(&dirs, &command.service_name);
            stage_capability_grants(&paths, &descriptor_record, &mut capability_grant_writes)?;
            prepare_planned_process_logs(&paths)?;
            debug_assert_eq!(role, SupervisedServiceRole::RuntimeHost);
            let args =
                plan_service_launch_args(&slug, &manifest, command, &descriptor, &paths, context)?;
            let mut process = ServiceProcessRecord::planned(
                command.service_name.clone(),
                role,
                descriptor.run,
                &command.endpoint,
                command.program.clone(),
                manifest.workspace_root.clone(),
                args,
                paths.stdout_log,
                paths.stderr_log,
                paths.structured_log,
                Some(paths.ready_file),
                now,
            );
            process.owner_id.clone_from(&command.owner_id);
            process.owner_root.clone_from(&command.owner_root);
            manifest.upsert_process_record(process, now);
        }

        Self::write_manifest_with_files(&manifest, capability_grant_writes)?;
        Ok(manifest)
    }

    /// Records that the process `key` names is running as `identity`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns,
    /// [`SessionError::MissingServiceProcess`] if `key` names no current
    /// process record, [`SessionError::ServiceProcess`] if `identity` has a
    /// zero pid or start time, and the manifest-write errors
    /// [`SessionError::Io`], [`SessionError::TomlSerialize`], and
    /// [`SessionError::FileTransaction`].
    pub fn mark_service_running(
        &self,
        name: &str,
        key: &ServiceProcessKey,
        identity: ProcessIdentity,
    ) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;
        let now = now_unix_ms();
        manifest
            .service_process_mut(key)
            .ok_or_else(|| SessionError::MissingServiceProcess {
                session: slug.clone(),
                service: key.service_name.clone(),
            })?
            .mark_running(identity, now)?;
        manifest.updated_unix_ms = now;
        Self::write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Promotes the process `key` names to running by consuming the readiness
    /// record it published, and republishes its descriptor at the endpoints
    /// that record names.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns;
    /// [`SessionError::MissingServiceProcess`] if `key` names no current
    /// process record; [`SessionError::MissingServiceReadyFile`] if that record
    /// has no readiness path; [`SessionError::Io`] or
    /// [`SessionError::TomlDeserialize`] if the readiness file cannot be read
    /// or parsed; [`SessionError::InvalidServiceReadyRecord`] or
    /// [`SessionError::UnsupportedEndpointKind`] if the record disagrees with
    /// the planned process or names an in-process endpoint;
    /// [`SessionError::InvalidEndpointAddress`] if a published address does not
    /// parse; [`SessionError::MissingServiceDescriptor`] if the manifest has no
    /// descriptor for the service; [`SessionError::InvalidServiceDescriptor`]
    /// if the republished descriptor fails validation;
    /// [`SessionError::UnsupportedServiceRole`] if its role cannot be recorded;
    /// [`SessionError::ServiceProcess`] if `identity` has a zero pid or start
    /// time; and the manifest-write errors [`SessionError::TomlSerialize`] and
    /// [`SessionError::FileTransaction`].
    pub fn mark_service_ready(
        &self,
        name: &str,
        key: &ServiceProcessKey,
        identity: ProcessIdentity,
    ) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;
        let now = now_unix_ms();
        let process_index = manifest.current_service_process_index(key).ok_or_else(|| {
            SessionError::MissingServiceProcess {
                session: slug.clone(),
                service: key.service_name.clone(),
            }
        })?;
        let process = &manifest.processes[process_index];
        let ready_file =
            process
                .ready_file
                .clone()
                .ok_or_else(|| SessionError::MissingServiceReadyFile {
                    session: slug.clone(),
                    service: key.service_name.clone(),
                })?;
        let ready = read_service_ready_record(&ready_file)?;
        validate_service_ready_record(&slug, process, &ready, identity.process_id)?;

        let endpoint = ready.endpoint();
        validate_public_endpoint(&endpoint, "session service readiness")?;
        // `ServiceProcessRecord` names its service `service_name`; clippy's
        // `service.name == process.name` rewrite does not compile.
        #[allow(clippy::suspicious_operation_groupings)]
        let mut descriptor = manifest
            .services
            .iter()
            .find(|service| service.name == process.service_name && service.role == process.role)
            .ok_or_else(|| SessionError::MissingServiceDescriptor {
                session: slug.clone(),
                service: process.service_name.clone(),
            })?
            .to_descriptor();
        descriptor.endpoint = endpoint;
        descriptor.observability_endpoint = ready.observability_endpoint();
        descriptor.lifecycle_endpoint = ready.lifecycle_endpoint();
        validate_session_service_descriptor(
            &slug,
            manifest.id,
            &descriptor,
            "session service readiness",
        )?;
        manifest.upsert_service_descriptor(&descriptor, now).ok_or(
            SessionError::UnsupportedServiceRole {
                role: descriptor.role,
            },
        )?;

        let process = &mut manifest.processes[process_index];
        process.mark_running(identity, now)?;
        process.endpoint_kind = ready.endpoint_kind;
        process.endpoint_address = ready.endpoint_address;
        manifest.updated_unix_ms = now;
        Self::write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Clears the stale readiness file for the process `key` names, captures
    /// its program artifact, and marks the record starting.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns,
    /// [`SessionError::MissingServiceProcess`] if `key` names no current
    /// process record, [`SessionError::Io`] if the readiness file cannot be
    /// removed or the program artifact cannot be measured, and the
    /// manifest-write errors [`SessionError::TomlSerialize`] and
    /// [`SessionError::FileTransaction`].
    pub fn mark_service_starting(
        &self,
        name: &str,
        key: &ServiceProcessKey,
    ) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let mut manifest = self.session_by_slug(&slug)?;
        let now = now_unix_ms();
        let process = manifest.service_process_mut(key).ok_or_else(|| {
            SessionError::MissingServiceProcess {
                session: slug.clone(),
                service: key.service_name.clone(),
            }
        })?;
        if let Some(ready_file) = &process.ready_file {
            remove_file_if_exists(ready_file)?;
        }
        process.capture_program_artifact()?;
        process.mark_starting(now);
        manifest.updated_unix_ms = now;
        Self::write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Records that the process `key` names has exited, with `exit_code` and an
    /// optional `failure` note.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::session`] returns,
    /// [`SessionError::MissingServiceProcess`] if `key` names no current
    /// process record, and the manifest-write errors [`SessionError::Io`],
    /// [`SessionError::TomlSerialize`], and [`SessionError::FileTransaction`].
    pub fn mark_service_exited(
        &self,
        name: &str,
        key: &ServiceProcessKey,
        exit_code: Option<i32>,
        failure: Option<String>,
    ) -> Result<SessionManifest, SessionError> {
        let slug = session_slug(name)?;
        let manifest = self.session_by_slug(&slug)?;
        let process_index = manifest.current_service_process_index(key).ok_or_else(|| {
            SessionError::MissingServiceProcess {
                session: slug.clone(),
                service: key.service_name.clone(),
            }
        })?;
        Self::mark_service_process_index_exited(manifest, process_index, exit_code, failure)
    }

    fn mark_service_process_index_exited(
        mut manifest: SessionManifest,
        process_index: usize,
        exit_code: Option<i32>,
        failure: Option<String>,
    ) -> Result<SessionManifest, SessionError> {
        let now = now_unix_ms();
        manifest.processes[process_index].mark_exited(exit_code, failure, now);
        manifest.updated_unix_ms = now;
        Self::write_manifest(&manifest)?;
        Ok(manifest)
    }

    fn validate_existing_session_workspace(manifest: &SessionManifest) -> Result<(), SessionError> {
        if !manifest.workspace_root.is_dir() {
            return Err(SessionError::SessionWorkspaceInvalid {
                session: manifest.slug.clone(),
                reason: format!(
                    "workspace root `{}` does not exist",
                    manifest.workspace_root.display()
                ),
            });
        }
        let graph =
            az_project::load_resolved_project_graph(&manifest.workspace_root).map_err(|error| {
                SessionError::SessionWorkspaceInvalid {
                    session: manifest.slug.clone(),
                    reason: format!("workspace project graph is invalid: {error}"),
                }
            })?;
        if graph.manifest.project.id != manifest.project_id {
            return Err(SessionError::SessionWorkspaceInvalid {
                session: manifest.slug.clone(),
                reason: format!(
                    "expected project id `{}`, got `{}` from workspace project graph",
                    manifest.project_id, graph.manifest.project.id
                ),
            });
        }
        Ok(())
    }

    pub(crate) fn active_session_workspace(
        &self,
        name: &str,
        operation: &'static str,
    ) -> Result<SessionManifest, SessionError> {
        let manifest = self.session(name)?;
        ensure_session_active_for(&manifest, operation)?;
        Self::validate_existing_session_workspace(&manifest)?;
        Ok(manifest)
    }

    fn find_session(&self, slug: &str) -> Result<Option<Arc<SessionManifest>>, SessionError> {
        if !self.sessions_dir.exists() {
            return Ok(None);
        }
        for entry in fs::read_dir(&self.sessions_dir)? {
            let manifest_path = entry?.path().join(SESSION_MANIFEST_FILE);
            if !manifest_path.exists() {
                continue;
            }
            let manifest = self.read_manifest(&manifest_path)?;
            if manifest.slug == slug {
                return Ok(Some(manifest));
            }
        }
        Ok(None)
    }

    fn session_by_slug(&self, slug: &str) -> Result<SessionManifest, SessionError> {
        Ok((*self.snapshot_by_slug(slug)?).clone())
    }

    fn snapshot_by_slug(&self, slug: &str) -> Result<Arc<SessionManifest>, SessionError> {
        self.find_session(slug)?
            .ok_or_else(|| SessionError::SessionNotFound(slug.to_string()))
    }

    fn read_manifest(&self, path: &Path) -> Result<Arc<SessionManifest>, SessionError> {
        let state = ManifestState::capture(path)?;
        if let Some(manifest) = self.parsed(path, state) {
            return Ok(manifest);
        }

        let text = fs::read_to_string(path)?;
        let manifest: SessionManifest = toml::from_str(&text)?;
        self.validate_manifest_identity(&manifest)?;
        let manifest = Arc::new(manifest);

        // The text just read belongs to `state` only if the file did not move
        // under the read. When it did, the fresh parse is still returned and
        // simply not remembered; the next ask re-reads it.
        if state.is_cacheable() && ManifestState::capture(path)? == state {
            self.manifests
                .lock()
                .expect("session manifest cache mutex poisoned")
                .insert(path.to_path_buf(), (state, Arc::clone(&manifest)));
        }
        Ok(manifest)
    }

    fn parsed(&self, path: &Path, state: ManifestState) -> Option<Arc<SessionManifest>> {
        let manifests = self
            .manifests
            .lock()
            .expect("session manifest cache mutex poisoned");
        let (parsed_state, manifest) = manifests.get(path)?;
        let hit = (*parsed_state == state).then(|| Arc::clone(manifest));
        drop(manifests);
        hit
    }

    fn write_manifest(manifest: &SessionManifest) -> Result<(), SessionError> {
        Self::write_manifest_with_files(manifest, Vec::new())
    }

    fn write_manifest_with_files(
        manifest: &SessionManifest,
        mut writes: Vec<FileWrite>,
    ) -> Result<(), SessionError> {
        Self::validate_manifest_public_endpoints(manifest)?;
        fs::create_dir_all(&manifest.run_dir)?;
        let transaction_root = manifest_transaction_root(manifest);
        fs::create_dir_all(&transaction_root)?;
        recover_manifest_transactions(&manifest.run_dir)?;
        let text = toml::to_string_pretty(manifest)?;
        writes.push(FileWrite::new(manifest.manifest_path(), text.into_bytes()));
        FileTransaction::new(transaction_root).commit(writes)?;
        Ok(())
    }

    fn preserve_manifest_failure(
        manifest: &mut SessionManifest,
        reason: &str,
    ) -> Result<(), SessionError> {
        manifest.preserve_failure(now_unix_ms());
        fs::create_dir_all(&manifest.run_dir)?;
        fs::write(manifest.run_dir.join("failure.txt"), reason)?;
        Self::write_manifest(manifest)?;
        if let Err(error) = Self::write_failure_diagnostic_bundle(manifest, reason) {
            warn!(
                session = %manifest.slug,
                error = %error,
                "failed to write session failure diagnostic bundle"
            );
        }
        Ok(())
    }

    fn write_failure_diagnostic_bundle(
        manifest: &SessionManifest,
        reason: &str,
    ) -> Result<(), az_observability::DiagnosticBundleError> {
        let bundle_root = manifest
            .run_dir
            .join("diagnostics")
            .join(format!("failure-{}", manifest.updated_unix_ms));
        let mut sources = Vec::new();
        let mut bundle_paths = BTreeSet::new();
        push_diagnostic_bundle_source(
            &mut sources,
            &mut bundle_paths,
            DiagnosticBundleSourceFile::new(manifest.run_dir.join("failure.txt"), "failure")
                .with_bundle_path("failure.txt"),
        );
        push_diagnostic_bundle_source(
            &mut sources,
            &mut bundle_paths,
            DiagnosticBundleSourceFile::new(manifest.manifest_path(), "session")
                .with_bundle_path(SESSION_MANIFEST_FILE),
        );

        for process in &manifest.processes {
            for source in [
                process_log_source(&process.stdout_log),
                process_log_source(&process.stderr_log),
                process_log_source(&process.structured_log),
            ] {
                push_diagnostic_bundle_source(&mut sources, &mut bundle_paths, source);
            }
        }

        write_diagnostic_bundle(bundle_root, manifest.slug.as_str(), reason, sources)?;
        Ok(())
    }

    fn validate_manifest_identity(&self, manifest: &SessionManifest) -> Result<(), SessionError> {
        if manifest.schema_version != SESSION_SCHEMA_VERSION {
            return Err(SessionError::UnsupportedManifestSchemaVersion {
                session: manifest.slug.clone(),
                expected: SESSION_SCHEMA_VERSION,
                actual: manifest.schema_version,
            });
        }

        if manifest.project_id != self.project_id {
            return Err(SessionError::SessionProjectMismatch {
                session: manifest.slug.clone(),
                expected_project_id: self.project_id.clone(),
                actual_project_id: manifest.project_id.clone(),
            });
        }

        self.validate_manifest_owned_paths(manifest)?;
        Self::validate_manifest_public_endpoints(manifest)?;

        Ok(())
    }

    fn validate_manifest_public_endpoints(manifest: &SessionManifest) -> Result<(), SessionError> {
        let mut service_keys = BTreeSet::new();
        for service in &manifest.services {
            if !service_keys.insert((
                service.namespace.as_str(),
                service.name.as_str(),
                service.role,
            )) {
                return Err(SessionError::SessionManifestInvalid {
                    session: manifest.slug.clone(),
                    reason: format!(
                        "service descriptor `{}/{}` with role {:?} is duplicated",
                        service.namespace, service.name, service.role
                    ),
                });
            }
            let descriptor = service.to_descriptor();
            match descriptor.role {
                ServiceRole::ProjectHost | ServiceRole::AssetProcessor | ServiceRole::Worker => {
                    validate_project_service_attachment_descriptor(
                        &manifest.slug,
                        &descriptor,
                        "session manifest project-service attachment",
                    )?;
                }
                _ => validate_session_service_descriptor(
                    &manifest.slug,
                    manifest.id,
                    &descriptor,
                    "session manifest service descriptor",
                )?,
            }
        }

        let mut process_keys = BTreeSet::new();
        for process in &manifest.processes {
            if process.role != SupervisedServiceRole::RuntimeHost {
                return Err(SessionError::SessionManifestInvalid {
                    session: manifest.slug.clone(),
                    reason: format!(
                        "role {:?} cannot own process state in a session manifest",
                        process.role
                    ),
                });
            }
            let key = ServiceProcessKey::from_process(process);
            if !process_keys.insert(key.clone()) {
                return Err(SessionError::SessionManifestInvalid {
                    session: manifest.slug.clone(),
                    reason: format!(
                        "runtime process `{}` role {:?} is duplicated",
                        process.service_name, process.role
                    ),
                });
            }
            validate_public_endpoint(
                &Endpoint::new(process.endpoint_kind.to_proto(), &process.endpoint_address),
                "session manifest service process",
            )?;
            if !manifest
                .services
                .iter()
                .any(|service| ServiceProcessKey::new(&service.name, service.role) == key)
            {
                return Err(SessionError::SessionManifestInvalid {
                    session: manifest.slug.clone(),
                    reason: format!(
                        "runtime process `{}` has no matching descriptor",
                        process.service_name
                    ),
                });
            }
        }

        Ok(())
    }

    fn validate_manifest_owned_paths(
        &self,
        manifest: &SessionManifest,
    ) -> Result<(), SessionError> {
        Self::validate_manifest_path(
            manifest,
            "project root",
            &manifest.project_root,
            &self.project_root,
        )?;
        Self::validate_manifest_path(
            manifest,
            "workspace root",
            &manifest.workspace_root,
            &self.project_root,
        )?;
        Self::validate_manifest_path(
            manifest,
            "run dir",
            &manifest.run_dir,
            &self.sessions_dir.join(manifest.id.to_string()),
        )
    }

    fn validate_session_run_dir(&self, manifest: &SessionManifest) -> Result<(), SessionError> {
        Self::validate_manifest_path(
            manifest,
            "run dir",
            &manifest.run_dir,
            &self.sessions_dir.join(manifest.id.to_string()),
        )
    }

    fn validate_manifest_path(
        manifest: &SessionManifest,
        label: &str,
        actual: &Path,
        expected: &Path,
    ) -> Result<(), SessionError> {
        if same_path(actual, expected) {
            return Ok(());
        }

        Err(SessionError::SessionManifestInvalid {
            session: manifest.slug.clone(),
            reason: format!(
                "{label} must be `{}`, got `{}`",
                normalize(expected).display(),
                normalize(actual).display()
            ),
        })
    }

    fn ensure_no_running_services(manifest: &SessionManifest) -> Result<(), SessionError> {
        let running_services = running_session_services(manifest);
        if running_services.is_empty() {
            return Ok(());
        }

        Err(SessionError::SessionServicesRunning {
            session: manifest.slug.clone(),
            services: running_services,
        })
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = normalize(left);
    let right = normalize(right);
    #[cfg(windows)]
    {
        // Both sides already share one spelling; only case is still free to
        // differ, and comparing the `OsStr` keeps a non-Unicode component from
        // collapsing into a replacement character.
        left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_millis()
}

fn session_slug(name: &str) -> Result<String, SessionError> {
    let slug = name.trim().to_ascii_lowercase().replace([' ', '_'], "-");

    if slug.is_empty()
        || slug.starts_with('-')
        || slug.ends_with('-')
        || !slug
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(SessionError::InvalidSessionName(name.to_string()));
    }

    Ok(slug)
}

fn process_log_name(service_name: &str) -> String {
    let mut name = String::with_capacity(service_name.len());
    let mut last_was_separator = false;
    for ch in service_name.chars() {
        if ch.is_ascii_alphanumeric() {
            name.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            name.push('-');
            last_was_separator = true;
        }
    }

    while name.ends_with('-') {
        name.pop();
    }

    if name.is_empty() {
        "service".to_string()
    } else {
        name
    }
}

fn process_log_source(path: &Path) -> DiagnosticBundleSourceFile {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("service.log");
    DiagnosticBundleSourceFile::new(path.to_path_buf(), "log")
        .with_bundle_path(PathBuf::from("logs").join(file_name))
}

fn push_diagnostic_bundle_source(
    sources: &mut Vec<DiagnosticBundleSourceFile>,
    bundle_paths: &mut BTreeSet<PathBuf>,
    source: DiagnosticBundleSourceFile,
) {
    if bundle_paths.insert(source.bundle_path.clone()) {
        sources.push(source);
    }
}

fn remove_file_if_exists(path: &Path) -> Result<(), SessionError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

const fn session_state_label(state: SessionState) -> &'static str {
    match state {
        SessionState::Preparing => "preparing",
        SessionState::Active => "active",
        SessionState::FailedPreserved => "failed-preserved",
        SessionState::Removed => "removed",
    }
}

fn ensure_session_active_for(
    manifest: &SessionManifest,
    operation: &'static str,
) -> Result<(), SessionError> {
    if manifest.state == SessionState::Active {
        return Ok(());
    }

    Err(SessionError::SessionNotActive {
        session: manifest.slug.clone(),
        state: session_state_label(manifest.state).to_string(),
        operation,
    })
}

fn running_session_services(manifest: &SessionManifest) -> Vec<String> {
    manifest
        .processes
        .iter()
        .filter(|process| {
            matches!(
                process.state,
                ServiceProcessState::Starting | ServiceProcessState::Running
            )
        })
        .map(|process| process.service_name.clone())
        .collect()
}

fn upsert_planned_service_descriptors(
    manifest: &mut SessionManifest,
    commands: &[SessionServiceLaunchCommand],
    now_unix_ms: u128,
) -> Result<Vec<ServiceDescriptor>, SessionError> {
    let mut descriptors = Vec::with_capacity(commands.len());
    for command in commands {
        validate_public_endpoint(&command.endpoint, "session planned service descriptor")?;
        validate_service_launch_command(&manifest.slug, command)?;

        let expected_id = expected_session_service_id(command.role)?;
        if command.service_name != expected_id.name {
            return Err(SessionError::InvalidServiceLaunchPlan {
                session: manifest.slug.clone(),
                service: command.service_name.clone(),
                reason: format!(
                    "role {:?} must use canonical service name `{}`",
                    command.role, expected_id.name
                ),
            });
        }

        let run = Uuid::now_v7();
        let descriptor =
            planned_service_descriptor(manifest.id, command.role, run, command.endpoint.clone())?;
        validate_session_service_descriptor(
            &manifest.slug,
            manifest.id,
            &descriptor,
            "session planned service descriptor",
        )?;
        manifest
            .upsert_service_descriptor(&descriptor, now_unix_ms)
            .ok_or(SessionError::UnsupportedServiceRole {
                role: descriptor.role,
            })?;
        descriptors.push(descriptor);
    }

    Ok(descriptors)
}

fn apply_planned_descriptor_endpoints(
    session: &str,
    commands: &mut [SessionServiceLaunchCommand],
    descriptors: &[ServiceDescriptor],
) -> Result<(), SessionError> {
    for command in commands {
        let Some(descriptor) = descriptors.iter().find(|descriptor| {
            descriptor.id.name == command.service_name && descriptor.role == command.role
        }) else {
            return Err(SessionError::InvalidServiceLaunchPlan {
                session: session.to_string(),
                service: command.service_name.clone(),
                reason: "no generated descriptor matched launch command".to_string(),
            });
        };
        command.endpoint = descriptor.endpoint.clone();
    }
    Ok(())
}

fn planned_service_descriptor(
    session_id: SessionId,
    role: ServiceRole,
    run: Uuid,
    endpoint: Endpoint,
) -> Result<ServiceDescriptor, SessionError> {
    let mut descriptor = match role {
        ServiceRole::RuntimeHost => {
            Ok(runtime_host_service_descriptor(session_id.0, run, endpoint))
        }
        role => Err(SessionError::UnsupportedServiceRole { role }),
    }?;
    add_observability_contract(&mut descriptor, Some(session_id.0));
    add_service_lifecycle_contract(
        &mut descriptor,
        Some(session_id.0),
        ServiceId::new(
            SESSION_SUPERVISOR_NAMESPACE,
            SESSION_SUPERVISOR_SERVICE_NAME,
        ),
        ServiceRole::SessionSupervisor,
    );
    Ok(descriptor)
}

fn validate_service_launch_command(
    session: &str,
    command: &SessionServiceLaunchCommand,
) -> Result<(), SessionError> {
    if command.service_name.trim().is_empty() {
        return invalid_service_launch_plan(
            session,
            &command.service_name,
            "service name is empty",
        );
    }
    if command.owner_id.trim().is_empty() {
        return invalid_service_launch_plan(
            session,
            &command.service_name,
            "owner_id is empty; service launch plans must preserve daemon-planned project or gem ownership",
        );
    }
    if command.owner_root.as_os_str().is_empty() {
        return invalid_service_launch_plan(
            session,
            &command.service_name,
            "owner_root is empty; service launch plans must preserve daemon-planned project or gem ownership",
        );
    }
    if !command.owner_root.is_absolute() {
        return invalid_service_launch_plan(
            session,
            &command.service_name,
            format!(
                "owner_root `{}` is not absolute",
                command.owner_root.display()
            ),
        );
    }
    if command.build_output_root.as_os_str().is_empty() {
        return invalid_service_launch_plan(
            session,
            &command.service_name,
            "build_output_root is empty; service launch plans must preserve daemon-resolved Cargo output roots",
        );
    }
    if !command.build_output_root.is_absolute() {
        return invalid_service_launch_plan(
            session,
            &command.service_name,
            format!(
                "build_output_root `{}` is not absolute",
                command.build_output_root.display()
            ),
        );
    }
    if command.program.trim().is_empty() {
        return invalid_service_launch_plan(session, &command.service_name, "program is empty");
    }
    if !Path::new(&command.program).is_absolute() {
        return invalid_service_launch_plan(
            session,
            &command.service_name,
            format!(
                "program `{}` must be an absolute direct service binary path",
                command.program
            ),
        );
    }
    if !path_is_within_or_equal(Path::new(&command.program), &command.build_output_root) {
        return invalid_service_launch_plan(
            session,
            &command.service_name,
            format!(
                "program `{}` must live under build output root `{}`",
                command.program,
                command.build_output_root.display()
            ),
        );
    }
    Ok(())
}

fn validate_service_launch_context(
    session: &str,
    context: &SessionServiceLaunchContext,
) -> Result<(), SessionError> {
    if let Some(endpoint) = context.otlp_endpoint.as_deref() {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return invalid_service_launch_plan(
                session,
                "<launch-context>",
                "--otlp-endpoint must not be empty",
            );
        }
        if endpoint.starts_with('-') {
            return invalid_service_launch_plan(
                session,
                "<launch-context>",
                "--otlp-endpoint must be an endpoint value, not another option",
            );
        }
    }

    Ok(())
}

fn invalid_service_launch_plan(
    session: &str,
    service: &str,
    reason: impl Into<String>,
) -> Result<(), SessionError> {
    Err(SessionError::InvalidServiceLaunchPlan {
        session: session.to_string(),
        service: service.to_string(),
        reason: reason.into(),
    })
}

fn manifest_transaction_root(manifest: &SessionManifest) -> PathBuf {
    manifest.run_dir.join("transactions")
}

fn recover_manifest_transactions(run_dir: &Path) -> Result<(), SessionError> {
    FileTransaction::new(run_dir.join("transactions")).recover_pending()?;
    Ok(())
}

/// The four run-directory subtrees a session launch plan writes into.
struct LaunchPlanDirs {
    logs: PathBuf,
    ready: PathBuf,
    grants: PathBuf,
    side_channels: PathBuf,
}

impl LaunchPlanDirs {
    fn prepare(run_dir: &Path) -> Result<Self, SessionError> {
        let dirs = Self {
            logs: run_dir.join("logs"),
            ready: run_dir.join("ready"),
            grants: run_dir.join("capability-grants"),
            side_channels: run_dir.join("side-channels"),
        };
        fs::create_dir_all(&dirs.logs)?;
        fs::create_dir_all(&dirs.ready)?;
        fs::create_dir_all(&dirs.grants)?;
        fs::create_dir_all(&dirs.side_channels)?;
        Ok(dirs)
    }
}

/// Every path one planned service process owns, all derived from the sanitized
/// log name of its service.
struct PlannedProcessPaths {
    stdout_log: PathBuf,
    stderr_log: PathBuf,
    structured_log: PathBuf,
    ready_file: PathBuf,
    side_channel_root: PathBuf,
    capability_grants: PathBuf,
    observability_capability_grants: PathBuf,
    lifecycle_capability_grants: PathBuf,
}

impl PlannedProcessPaths {
    fn new(dirs: &LaunchPlanDirs, service_name: &str) -> Self {
        let log_name = process_log_name(service_name);
        Self {
            stdout_log: dirs.logs.join(format!("{log_name}.stdout.log")),
            stderr_log: dirs.logs.join(format!("{log_name}.stderr.log")),
            structured_log: dirs
                .logs
                .join(format!("{log_name}.{STRUCTURED_LOG_EXTENSION}")),
            ready_file: dirs.ready.join(format!("{log_name}.toml")),
            side_channel_root: dirs.side_channels.join(format!("{log_name}-side-channels")),
            capability_grants: dirs.grants.join(format!("{log_name}.capnp.packed")),
            observability_capability_grants: dirs
                .grants
                .join(format!("{log_name}.observability.capnp.packed")),
            lifecycle_capability_grants: dirs
                .grants
                .join(format!("{log_name}.lifecycle.capnp.packed")),
        }
    }
}

fn validate_launch_plan_commands(
    slug: &str,
    commands: &[SessionServiceLaunchCommand],
) -> Result<(), SessionError> {
    let mut seen_services = BTreeSet::new();
    for command in commands {
        if command.role != ServiceRole::RuntimeHost {
            return Err(SessionError::InvalidServiceLaunchPlan {
                session: slug.to_string(),
                service: command.service_name.clone(),
                reason: format!(
                    "role {:?} is project-instance owned; sessions may launch only RuntimeHost",
                    command.role
                ),
            });
        }
        if !seen_services.insert(command.service_name.as_str()) {
            return Err(SessionError::InvalidServiceLaunchPlan {
                session: slug.to_string(),
                service: command.service_name.clone(),
                reason: "service launch plan contains duplicate service entries".to_string(),
            });
        }
    }
    Ok(())
}

/// Resolve the registered descriptor a launch command names, extend it with the
/// observability and lifecycle contracts, and republish it in `manifest`.
fn plan_service_descriptor(
    slug: &str,
    manifest: &mut SessionManifest,
    command: &SessionServiceLaunchCommand,
    role: SupervisedServiceRole,
    now: u128,
) -> Result<ServiceDescriptor, SessionError> {
    let descriptor_index = manifest
        .services
        .iter()
        .position(|service| {
            service.name == command.service_name
                && service.role == role
                && service.endpoint_kind.to_proto() == command.endpoint.kind
                && service.endpoint_address == command.endpoint.address
        })
        .ok_or_else(|| SessionError::InvalidServiceLaunchPlan {
            session: slug.to_string(),
            service: command.service_name.clone(),
            reason: format!(
                "no registered descriptor matches role {:?} endpoint {:?} `{}`",
                command.role, command.endpoint.kind, command.endpoint.address
            ),
        })?;
    let mut descriptor = manifest.services[descriptor_index].to_descriptor();
    add_observability_contract(&mut descriptor, Some(manifest.id.0));
    add_service_lifecycle_contract(
        &mut descriptor,
        Some(manifest.id.0),
        ServiceId::new(
            SESSION_SUPERVISOR_NAMESPACE,
            SESSION_SUPERVISOR_SERVICE_NAME,
        ),
        ServiceRole::SessionSupervisor,
    );
    validate_session_service_descriptor(
        slug,
        manifest.id,
        &descriptor,
        "session service launch observability contract",
    )?;
    manifest.upsert_service_descriptor(&descriptor, now).ok_or(
        SessionError::UnsupportedServiceRole {
            role: descriptor.role,
        },
    )?;
    Ok(descriptor)
}

/// Stage the three capability-grant files one planned process reads at startup.
fn stage_capability_grants(
    paths: &PlannedProcessPaths,
    record: &ServiceRecord,
    writes: &mut Vec<FileWrite>,
) -> Result<(), SessionError> {
    let grant_set = CapabilityGrantSet::from_grants(capability_grants_for_service_process(record));
    writes.push(FileWrite::new(
        paths.capability_grants.clone(),
        encode_capability_grant_set(&grant_set)?,
    ));
    let observability_grant_set = CapabilityGrantSet::from_grants(
        observability_capability_grants_for_service_process(record),
    );
    writes.push(FileWrite::new(
        paths.observability_capability_grants.clone(),
        encode_capability_grant_set(&observability_grant_set)?,
    ));
    let lifecycle_grant_set =
        CapabilityGrantSet::from_grants(lifecycle_capability_grants_for_service_process(record));
    writes.push(FileWrite::new(
        paths.lifecycle_capability_grants.clone(),
        encode_capability_grant_set(&lifecycle_grant_set)?,
    ));
    Ok(())
}

/// Rotate the previous run's logs out of the way and create this run's.
fn prepare_planned_process_logs(paths: &PlannedProcessPaths) -> Result<(), SessionError> {
    let logs = [&paths.stdout_log, &paths.stderr_log, &paths.structured_log];
    for log in logs {
        rotate_log_at_plan_time(log)?;
    }
    for log in logs {
        fs::OpenOptions::new().create(true).append(true).open(log)?;
    }
    Ok(())
}

/// Build the argument vector one planned process is launched with.
fn plan_service_launch_args(
    slug: &str,
    manifest: &SessionManifest,
    command: &SessionServiceLaunchCommand,
    descriptor: &ServiceDescriptor,
    paths: &PlannedProcessPaths,
    context: &SessionServiceLaunchContext,
) -> Result<Vec<String>, SessionError> {
    let mut args = command.args.clone();
    apply_service_launch_context(&mut args, manifest, command);
    fs::create_dir_all(&paths.side_channel_root)?;
    set_launch_arg(
        &mut args,
        "--side-channel-root",
        &paths.side_channel_root.to_string_lossy(),
    );
    set_launch_arg(&mut args, "--run", &descriptor.run.to_string());
    set_launch_arg(
        &mut args,
        "--ready-file",
        &paths.ready_file.to_string_lossy(),
    );
    set_launch_arg(
        &mut args,
        "--capability-grants",
        &paths.capability_grants.to_string_lossy(),
    );
    set_launch_arg(
        &mut args,
        "--lifecycle-capability-grants",
        &paths.lifecycle_capability_grants.to_string_lossy(),
    );
    let lifecycle_endpoint = descriptor.lifecycle_endpoint.as_ref().ok_or_else(|| {
        invalid_service_descriptor(
            slug,
            &descriptor.id,
            "service lifecycle endpoint is required",
        )
    })?;
    set_launch_arg(
        &mut args,
        "--lifecycle-endpoint-kind",
        endpoint_kind_arg(lifecycle_endpoint.kind),
    );
    set_launch_arg(
        &mut args,
        "--lifecycle-endpoint",
        &lifecycle_endpoint.address,
    );
    if let Some(observability_endpoint) = descriptor.observability_endpoint.as_ref() {
        set_launch_arg(
            &mut args,
            "--observability-endpoint-kind",
            endpoint_kind_arg(observability_endpoint.kind),
        );
        set_launch_arg(
            &mut args,
            "--observability-endpoint",
            &observability_endpoint.address,
        );
        set_launch_arg(
            &mut args,
            "--observability-capability-grants",
            &paths.observability_capability_grants.to_string_lossy(),
        );
    }
    set_launch_arg(
        &mut args,
        "--structured-log",
        &paths.structured_log.to_string_lossy(),
    );
    set_optional_launch_arg(
        &mut args,
        "--otlp-endpoint",
        context.otlp_endpoint.as_deref().map(str::trim),
    );
    Ok(args)
}

fn apply_service_launch_context(
    args: &mut Vec<String>,
    manifest: &SessionManifest,
    command: &SessionServiceLaunchCommand,
) {
    let workspace_root = manifest.workspace_root.to_string_lossy().into_owned();
    set_launch_arg(
        args,
        "--endpoint-kind",
        endpoint_kind_arg(command.endpoint.kind),
    );
    set_launch_arg(args, "--endpoint", &command.endpoint.address);
    set_launch_arg(args, "--project", &workspace_root);
    set_launch_arg(args, "--project-id", &manifest.project_id);
    set_launch_arg(args, "--owner-id", &command.owner_id);
    set_launch_arg(args, "--owner-root", &command.owner_root.to_string_lossy());
    debug_assert_eq!(command.role, ServiceRole::RuntimeHost);
    set_launch_arg(args, "--session", &manifest.slug);
    set_launch_arg(args, "--session-id", &manifest.id.to_string());
    set_launch_arg(args, "--workspace-root", &workspace_root);
}

fn set_launch_arg(args: &mut Vec<String>, flag: &str, value: &str) {
    remove_launch_arg(args, flag);
    args.push(flag.to_string());
    args.push(value.to_string());
}

fn set_optional_launch_arg(args: &mut Vec<String>, flag: &str, value: Option<&str>) {
    remove_launch_arg(args, flag);
    if let Some(value) = value {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
}

fn remove_launch_arg(args: &mut Vec<String>, flag: &str) {
    let mut normalized = Vec::with_capacity(args.len() + 2);
    let mut index = 0;
    while index < args.len() {
        if args[index] == flag {
            index += if index + 1 < args.len() { 2 } else { 1 };
        } else if args[index]
            .strip_prefix(flag)
            .is_some_and(|rest| rest.starts_with('='))
        {
            index += 1;
        } else {
            normalized.push(args[index].clone());
            index += 1;
        }
    }
    *args = normalized;
}

fn launch_args_include_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| {
        arg == flag
            || arg
                .strip_prefix(flag)
                .is_some_and(|rest| rest.starts_with('='))
    })
}

const fn endpoint_kind_arg(kind: EndpointKind) -> &'static str {
    match kind {
        EndpointKind::WindowsNamedPipe => "windows-named-pipe",
        EndpointKind::UnixDomainSocket => "unix-domain-socket",
        EndpointKind::Tcp => "tcp",
        EndpointKind::InProcess => "in-process",
    }
}

fn path_is_within_or_equal(path: &Path, root: &Path) -> bool {
    let path = normalize(path);
    let root = normalize(root);
    // Component by component, so a shared textual prefix (`build-out2` against
    // `build-out`) never reads as containment and a trailing separator never
    // reads as a difference.
    let mut components = path.components();
    root.components().all(|expected| {
        components
            .next()
            .is_some_and(|actual| component_eq(actual, expected))
    })
}

#[cfg(windows)]
fn component_eq(left: Component<'_>, right: Component<'_>) -> bool {
    left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
}

#[cfg(not(windows))]
fn component_eq(left: Component<'_>, right: Component<'_>) -> bool {
    left == right
}

fn read_service_ready_record(path: &Path) -> Result<ServiceReadyRecord, SessionError> {
    let text = fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

fn validate_service_ready_record(
    session: &str,
    process: &ServiceProcessRecord,
    ready: &ServiceReadyRecord,
    pid: u32,
) -> Result<(), SessionError> {
    validate_ready_record_identity(session, process, ready)?;
    validate_ready_record_service_endpoint(session, process, ready)?;
    validate_ready_record_observability_endpoint(session, process, ready)?;
    validate_ready_record_lifecycle_endpoint(session, process, ready)?;
    validate_ready_endpoint_address(session, process, ready)?;
    if let Some(ready_pid) = ready.pid
        && ready_pid != pid
    {
        return invalid_ready_record(
            session,
            &process.service_name,
            format!("pid {ready_pid} does not match spawned pid {pid}"),
        );
    }
    if process.state != ServiceProcessState::Starting {
        return invalid_ready_record(
            session,
            &process.service_name,
            format!(
                "process state {:?} is not eligible for readiness",
                process.state
            ),
        );
    }
    Ok(())
}

/// Schema version, service name, and role must all match the planned process.
fn validate_ready_record_identity(
    session: &str,
    process: &ServiceProcessRecord,
    ready: &ServiceReadyRecord,
) -> Result<(), SessionError> {
    if ready.schema_version != SERVICE_READY_SCHEMA_VERSION {
        return invalid_ready_record(
            session,
            &process.service_name,
            format!(
                "schema_version {} does not match expected {}",
                ready.schema_version, SERVICE_READY_SCHEMA_VERSION
            ),
        );
    }
    if ready.service_name != process.service_name {
        return invalid_ready_record(
            session,
            &process.service_name,
            format!(
                "service_name `{}` does not match expected `{}`",
                ready.service_name, process.service_name
            ),
        );
    }
    if ready.role != process.role {
        return invalid_ready_record(
            session,
            &process.service_name,
            format!(
                "role {:?} does not match expected {:?}",
                ready.role, process.role
            ),
        );
    }
    Ok(())
}

/// The published service endpoint must be public and match the planned kind.
fn validate_ready_record_service_endpoint(
    session: &str,
    process: &ServiceProcessRecord,
    ready: &ServiceReadyRecord,
) -> Result<(), SessionError> {
    if ready.endpoint_kind == ServiceEndpointKind::InProcess {
        return Err(SessionError::UnsupportedEndpointKind {
            operation: "session service readiness",
            kind: EndpointKind::InProcess,
        });
    }
    if ready.endpoint_kind != process.endpoint_kind {
        return invalid_ready_record(
            session,
            &process.service_name,
            format!(
                "endpoint_kind {:?} does not match expected {:?}",
                ready.endpoint_kind, process.endpoint_kind
            ),
        );
    }
    if ready.endpoint_address.trim().is_empty() {
        return invalid_ready_record(session, &process.service_name, "endpoint_address is empty");
    }
    Ok(())
}

/// A service launched with `--observability-endpoint` must publish one back,
/// and a published one must be a complete, public kind/address pair.
fn validate_ready_record_observability_endpoint(
    session: &str,
    process: &ServiceProcessRecord,
    ready: &ServiceReadyRecord,
) -> Result<(), SessionError> {
    match (
        ready.observability_endpoint_kind,
        ready.observability_endpoint_address.as_deref(),
    ) {
        (Some(ServiceEndpointKind::InProcess), _) => {
            return Err(SessionError::UnsupportedEndpointKind {
                operation: "session service observability readiness",
                kind: EndpointKind::InProcess,
            });
        }
        (Some(kind), Some(address)) if !address.trim().is_empty() => {
            validate_public_endpoint(
                &Endpoint::new(kind.to_proto(), address),
                "session service observability readiness",
            )?;
        }
        (Some(_), Some(_)) => {
            return invalid_ready_record(
                session,
                &process.service_name,
                "observability_endpoint_address is empty",
            );
        }
        (Some(_), None) => {
            return invalid_ready_record(
                session,
                &process.service_name,
                "observability_endpoint_kind requires observability_endpoint_address",
            );
        }
        (None, Some(_)) => {
            return invalid_ready_record(
                session,
                &process.service_name,
                "observability_endpoint_address requires observability_endpoint_kind",
            );
        }
        (None, None) => {
            if launch_args_include_flag(&process.args, "--observability-endpoint") {
                return invalid_ready_record(
                    session,
                    &process.service_name,
                    "service was launched with --observability-endpoint but did not publish it in readiness",
                );
            }
        }
    }
    Ok(())
}

/// The lifecycle endpoint is mandatory and must be loopback-only TCP.
fn validate_ready_record_lifecycle_endpoint(
    session: &str,
    process: &ServiceProcessRecord,
    ready: &ServiceReadyRecord,
) -> Result<(), SessionError> {
    match (
        ready.lifecycle_endpoint_kind,
        ready.lifecycle_endpoint_address.as_deref(),
    ) {
        (Some(ServiceEndpointKind::Tcp), Some(address)) if !address.trim().is_empty() => {
            let socket = address.parse::<SocketAddr>().map_err(|_| {
                SessionError::InvalidServiceReadyRecord {
                    session: session.to_string(),
                    service: process.service_name.clone(),
                    reason: "lifecycle_endpoint_address must be a socket address".to_string(),
                }
            })?;
            if !socket.ip().is_loopback() {
                return invalid_ready_record(
                    session,
                    &process.service_name,
                    "lifecycle endpoint must be loopback-only",
                );
            }
        }
        (Some(ServiceEndpointKind::InProcess), _) => {
            return invalid_ready_record(
                session,
                &process.service_name,
                "lifecycle_endpoint_kind must not be in-process",
            );
        }
        (Some(_), Some(address)) if address.trim().is_empty() => {
            return invalid_ready_record(
                session,
                &process.service_name,
                "lifecycle_endpoint_address is empty",
            );
        }
        (Some(_), Some(_)) => {
            return invalid_ready_record(
                session,
                &process.service_name,
                "lifecycle endpoint must use loopback TCP",
            );
        }
        (Some(_), None) => {
            return invalid_ready_record(
                session,
                &process.service_name,
                "lifecycle_endpoint_kind requires lifecycle_endpoint_address",
            );
        }
        (None, Some(_)) => {
            return invalid_ready_record(
                session,
                &process.service_name,
                "lifecycle_endpoint_address requires lifecycle_endpoint_kind",
            );
        }
        (None, None) => {
            return invalid_ready_record(
                session,
                &process.service_name,
                "lifecycle endpoint is required",
            );
        }
    }
    Ok(())
}

fn validate_ready_endpoint_address(
    session: &str,
    process: &ServiceProcessRecord,
    ready: &ServiceReadyRecord,
) -> Result<(), SessionError> {
    match ready.endpoint_kind {
        ServiceEndpointKind::Tcp => {
            validate_ready_tcp_endpoint_address(session, process, &ready.endpoint_address)
        }
        ServiceEndpointKind::InProcess => unreachable!("in-process readiness was rejected earlier"),
        ServiceEndpointKind::WindowsNamedPipe | ServiceEndpointKind::UnixDomainSocket => {
            if ready.endpoint_address == process.endpoint_address {
                Ok(())
            } else {
                invalid_ready_record(
                    session,
                    &process.service_name,
                    format!(
                        "endpoint_address `{}` does not match planned `{}`",
                        ready.endpoint_address, process.endpoint_address
                    ),
                )
            }
        }
    }
}

fn validate_ready_tcp_endpoint_address(
    session: &str,
    process: &ServiceProcessRecord,
    ready_endpoint_address: &str,
) -> Result<(), SessionError> {
    let planned = parse_ready_tcp_endpoint_address(
        session,
        &process.service_name,
        "planned",
        &process.endpoint_address,
    )?;
    let ready = parse_ready_tcp_endpoint_address(
        session,
        &process.service_name,
        "readiness",
        ready_endpoint_address,
    )?;

    if ready.port() == 0 {
        return invalid_ready_record(
            session,
            &process.service_name,
            "tcp readiness endpoint still points at port 0",
        );
    }

    if planned.port() == 0 {
        if planned.ip() == ready.ip() {
            return Ok(());
        }

        return invalid_ready_record(
            session,
            &process.service_name,
            format!(
                "tcp readiness endpoint host `{}` does not match planned host `{}`",
                ready.ip(),
                planned.ip()
            ),
        );
    }

    if ready == planned {
        Ok(())
    } else {
        invalid_ready_record(
            session,
            &process.service_name,
            format!("tcp readiness endpoint `{ready}` does not match planned `{planned}`"),
        )
    }
}

fn parse_ready_tcp_endpoint_address(
    session: &str,
    service: &str,
    label: &str,
    endpoint_address: &str,
) -> Result<SocketAddr, SessionError> {
    endpoint_address.parse::<SocketAddr>().map_err(|error| {
        SessionError::InvalidServiceReadyRecord {
            session: session.to_string(),
            service: service.to_string(),
            reason: format!("{label} tcp endpoint `{endpoint_address}` is invalid: {error}"),
        }
    })
}

fn invalid_ready_record<T>(
    session: &str,
    service: &str,
    reason: impl Into<String>,
) -> Result<T, SessionError> {
    Err(SessionError::InvalidServiceReadyRecord {
        session: session.to_string(),
        service: service.to_string(),
        reason: reason.into(),
    })
}

const fn validate_public_endpoint_kind(
    kind: EndpointKind,
    operation: &'static str,
) -> Result<(), SessionError> {
    if matches!(kind, EndpointKind::InProcess) {
        return Err(SessionError::UnsupportedEndpointKind { operation, kind });
    }

    Ok(())
}

fn validate_public_endpoint(
    endpoint: &Endpoint,
    operation: &'static str,
) -> Result<(), SessionError> {
    validate_public_endpoint_kind(endpoint.kind, operation)?;
    validate_endpoint_address(endpoint, operation)
}

fn validate_endpoint_address(
    endpoint: &Endpoint,
    operation: &'static str,
) -> Result<(), SessionError> {
    if endpoint.address.trim().is_empty() {
        return Err(SessionError::InvalidEndpointAddress {
            operation,
            kind: endpoint.kind,
            address: endpoint.address.clone(),
            reason: "address is empty".to_string(),
        });
    }

    if endpoint.kind == EndpointKind::Tcp
        && let Err(error) = endpoint.address.parse::<SocketAddr>()
    {
        return Err(SessionError::InvalidEndpointAddress {
            operation,
            kind: endpoint.kind,
            address: endpoint.address.clone(),
            reason: error.to_string(),
        });
    }

    Ok(())
}

fn validate_session_service_descriptor(
    session_slug: &str,
    session_id: SessionId,
    descriptor: &ServiceDescriptor,
    operation: &'static str,
) -> Result<(), SessionError> {
    validate_public_endpoint(&descriptor.endpoint, operation)?;
    if let Some(endpoint) = &descriptor.observability_endpoint {
        validate_public_endpoint(endpoint, operation)?;
    }
    if let Some(endpoint) = &descriptor.lifecycle_endpoint {
        validate_public_endpoint(endpoint, operation)?;
    }
    validate_session_service_descriptor_identity(session_slug, descriptor)?;
    validate_session_service_descriptor_capabilities(session_slug, session_id, descriptor)?;
    Ok(())
}

fn validate_session_service_descriptor_identity(
    session_slug: &str,
    descriptor: &ServiceDescriptor,
) -> Result<(), SessionError> {
    let expected_id = expected_session_service_id(descriptor.role)?;
    if descriptor.id == expected_id {
        return Ok(());
    }

    Err(invalid_service_descriptor(
        session_slug,
        &descriptor.id,
        format!(
            "role {:?} must use canonical service id `{}`",
            descriptor.role,
            service_label(&expected_id)
        ),
    ))
}

fn validate_session_service_descriptor_capabilities(
    session_slug: &str,
    session_id: SessionId,
    descriptor: &ServiceDescriptor,
) -> Result<(), SessionError> {
    let requirements = session_service_descriptor_capability_requirements(descriptor.role)?;
    let main_capabilities: Vec<_> = descriptor
        .capabilities
        .iter()
        .filter(|capability| !is_observability_control_grant(capability))
        .cloned()
        .collect();
    let main_grants = CapabilityGrantSet::from_grants(main_capabilities);
    main_grants
        .validate_exact_brokered_for_session(session_id.0, requirements)
        .map_err(|error| {
            invalid_service_descriptor(session_slug, &descriptor.id, error.to_string())
        })?;

    let observability_capabilities = descriptor
        .capabilities
        .iter()
        .filter(|capability| is_observability_control_grant(capability))
        .cloned()
        .collect::<Vec<_>>();
    if !observability_capabilities.is_empty() {
        let observability_grants = CapabilityGrantSet::from_grants(observability_capabilities);
        observability_grants
            .validate_exact_brokered_for_session(
                session_id.0,
                OBSERVABILITY_DESCRIPTOR_REQUIREMENTS,
            )
            .map_err(|error| {
                invalid_service_descriptor(session_slug, &descriptor.id, error.to_string())
            })?;
    }

    Ok(())
}

fn validate_project_service_attachment_descriptor(
    session_slug: &str,
    descriptor: &ServiceDescriptor,
    operation: &'static str,
) -> Result<(), SessionError> {
    validate_public_endpoint(&descriptor.endpoint, operation)?;
    if let Some(endpoint) = &descriptor.observability_endpoint {
        validate_public_endpoint(endpoint, operation)?;
    }
    if let Some(endpoint) = &descriptor.lifecycle_endpoint {
        validate_public_endpoint(endpoint, operation)?;
    }
    let expected_id = expected_project_service_id(descriptor.role)?;
    if descriptor.id != expected_id {
        return Err(invalid_service_descriptor(
            session_slug,
            &descriptor.id,
            format!(
                "role {:?} must use canonical project-service id `{}`",
                descriptor.role,
                service_label(&expected_id)
            ),
        ));
    }
    if descriptor
        .capabilities
        .iter()
        .any(|capability| capability.session.is_some())
    {
        return Err(invalid_service_descriptor(
            session_slug,
            &descriptor.id,
            "project-instance descriptor contains a session-scoped capability",
        ));
    }

    let requirements = project_service_descriptor_capability_requirements(descriptor.role)?;
    let main_grants = CapabilityGrantSet::from_grants(
        descriptor
            .capabilities
            .iter()
            .filter(|capability| !is_observability_control_grant(capability))
            .cloned()
            .collect::<Vec<_>>(),
    );
    main_grants
        .validate_exact_brokered_for_project(requirements)
        .map_err(|error| {
            invalid_service_descriptor(session_slug, &descriptor.id, error.to_string())
        })?;

    let observability_capabilities = descriptor
        .capabilities
        .iter()
        .filter(|capability| is_observability_control_grant(capability))
        .cloned()
        .collect::<Vec<_>>();
    if !observability_capabilities.is_empty() {
        CapabilityGrantSet::from_grants(observability_capabilities)
            .validate_exact_brokered_for_project(OBSERVABILITY_DESCRIPTOR_REQUIREMENTS)
            .map_err(|error| {
                invalid_service_descriptor(session_slug, &descriptor.id, error.to_string())
            })?;
    }

    Ok(())
}

const WORKER_ASSET_PROCESSOR_PERMISSIONS: &[&str] = &[ASSET_JOBS_PERMISSION];
const EDITOR_ASSET_PROCESSOR_PERMISSIONS: &[&str] =
    &[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION];
const PROJECT_HOST_ASSET_PROCESSOR_PERMISSIONS: &[&str] =
    &[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION];
const SESSION_SUPERVISOR_ASSET_PROCESSOR_PERMISSIONS: &[&str] =
    &[ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION];
const EDITOR_PROJECT_HOST_PERMISSIONS: &[&str] = &[
    PROJECT_SCHEMA_PERMISSION,
    PROJECT_GAMEDATA_PERMISSION,
    PROJECT_NODE_CATALOG_PERMISSION,
    PROJECT_GRAPH_CATALOG_PERMISSION,
    PROJECT_INVENTORY_PERMISSION,
    PROJECT_EDIT_PERMISSION,
    PROJECT_DOCUMENT_READ_PERMISSION,
    PROJECT_DOCUMENT_WRITE_PERMISSION,
    PROJECT_RUNTIME_LAUNCH_PERMISSION,
    PROJECT_SOURCE_NAVIGATION_PERMISSION,
];
const SESSION_SUPERVISOR_PROJECT_HOST_PERMISSIONS: &[&str] = &[PROJECT_DOCUMENT_WRITE_PERMISSION];
const EDITOR_RUNTIME_HOST_PERMISSIONS: &[&str] =
    &[RUNTIME_READ_PERMISSION, RUNTIME_CONTROL_PERMISSION];
const PROJECT_HOST_RUNTIME_HOST_PERMISSIONS: &[&str] = &[RUNTIME_CONTROL_PERMISSION];
const EDITOR_SESSION_SUPERVISOR_PERMISSIONS: &[&str] = &[
    SESSION_READ_PERMISSION,
    SESSION_SAVE_PERMISSION,
    SESSION_EXEC_PERMISSION,
    SESSION_MANAGE_PERMISSION,
];
const DAEMON_SESSION_SUPERVISOR_PERMISSIONS: &[&str] =
    &[SESSION_READ_PERMISSION, SESSION_MANAGE_PERMISSION];
const OBSERVABILITY_CONTROL_PERMISSIONS: &[&str] = &[OBSERVABILITY_CONTROL_PERMISSION];
const SERVICE_LIFECYCLE_PERMISSIONS: &[&str] = &[SERVICE_LIFECYCLE_SHUTDOWN_PERMISSION];

const OBSERVABILITY_DESCRIPTOR_REQUIREMENTS: &[CapabilityGrantRequirement<'static>] = &[
    CapabilityGrantRequirement::new(
        EDITOR_SERVICE_NAMESPACE,
        EDITOR_SERVICE_NAME,
        ServiceRole::Editor,
        OBSERVABILITY_AUDIENCE,
        OBSERVABILITY_CONTROL_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
        ServiceRole::SessionSupervisor,
        OBSERVABILITY_AUDIENCE,
        OBSERVABILITY_CONTROL_PERMISSIONS,
    ),
];

const ASSET_WORKER_DESCRIPTOR_REQUIREMENTS: &[CapabilityGrantRequirement<'static>] = &[
    CapabilityGrantRequirement::new(
        ASSET_WORKER_SERVICE_NAMESPACE,
        ASSET_WORKER_SERVICE_NAME,
        ServiceRole::Worker,
        ASSET_PROCESSOR_AUDIENCE,
        WORKER_ASSET_PROCESSOR_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        DAEMON_SERVICE_NAMESPACE,
        DAEMON_SERVICE_NAME,
        ServiceRole::Daemon,
        SERVICE_LIFECYCLE_AUDIENCE,
        SERVICE_LIFECYCLE_PERMISSIONS,
    ),
];

const ASSET_PROCESSOR_DESCRIPTOR_REQUIREMENTS: &[CapabilityGrantRequirement<'static>] = &[
    CapabilityGrantRequirement::new(
        EDITOR_SERVICE_NAMESPACE,
        EDITOR_SERVICE_NAME,
        ServiceRole::Editor,
        ASSET_PROCESSOR_AUDIENCE,
        EDITOR_ASSET_PROCESSOR_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        PROJECT_HOST_NAMESPACE,
        PROJECT_HOST_SERVICE_NAME,
        ServiceRole::ProjectHost,
        ASSET_PROCESSOR_AUDIENCE,
        PROJECT_HOST_ASSET_PROCESSOR_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        ASSET_WORKER_SERVICE_NAMESPACE,
        ASSET_WORKER_SERVICE_NAME,
        ServiceRole::Worker,
        ASSET_PROCESSOR_AUDIENCE,
        WORKER_ASSET_PROCESSOR_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
        ServiceRole::SessionSupervisor,
        ASSET_PROCESSOR_AUDIENCE,
        SESSION_SUPERVISOR_ASSET_PROCESSOR_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        DAEMON_SERVICE_NAMESPACE,
        DAEMON_SERVICE_NAME,
        ServiceRole::Daemon,
        SERVICE_LIFECYCLE_AUDIENCE,
        SERVICE_LIFECYCLE_PERMISSIONS,
    ),
];

const PROJECT_HOST_DESCRIPTOR_REQUIREMENTS: &[CapabilityGrantRequirement<'static>] = &[
    CapabilityGrantRequirement::new(
        EDITOR_SERVICE_NAMESPACE,
        EDITOR_SERVICE_NAME,
        ServiceRole::Editor,
        PROJECT_HOST_AUDIENCE,
        EDITOR_PROJECT_HOST_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
        ServiceRole::SessionSupervisor,
        PROJECT_HOST_AUDIENCE,
        SESSION_SUPERVISOR_PROJECT_HOST_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        DAEMON_SERVICE_NAMESPACE,
        DAEMON_SERVICE_NAME,
        ServiceRole::Daemon,
        SERVICE_LIFECYCLE_AUDIENCE,
        SERVICE_LIFECYCLE_PERMISSIONS,
    ),
];

const RUNTIME_HOST_DESCRIPTOR_REQUIREMENTS: &[CapabilityGrantRequirement<'static>] = &[
    CapabilityGrantRequirement::new(
        EDITOR_SERVICE_NAMESPACE,
        EDITOR_SERVICE_NAME,
        ServiceRole::Editor,
        RUNTIME_HOST_AUDIENCE,
        EDITOR_RUNTIME_HOST_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        PROJECT_HOST_NAMESPACE,
        PROJECT_HOST_SERVICE_NAME,
        ServiceRole::ProjectHost,
        RUNTIME_HOST_AUDIENCE,
        PROJECT_HOST_RUNTIME_HOST_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
        ServiceRole::SessionSupervisor,
        SERVICE_LIFECYCLE_AUDIENCE,
        SERVICE_LIFECYCLE_PERMISSIONS,
    ),
];

const SESSION_SUPERVISOR_DESCRIPTOR_REQUIREMENTS: &[CapabilityGrantRequirement<'static>] = &[
    CapabilityGrantRequirement::new(
        EDITOR_SERVICE_NAMESPACE,
        EDITOR_SERVICE_NAME,
        ServiceRole::Editor,
        SESSION_SUPERVISOR_AUDIENCE,
        EDITOR_SESSION_SUPERVISOR_PERMISSIONS,
    ),
    CapabilityGrantRequirement::new(
        DAEMON_SERVICE_NAMESPACE,
        DAEMON_SERVICE_NAME,
        ServiceRole::Daemon,
        SESSION_SUPERVISOR_AUDIENCE,
        DAEMON_SESSION_SUPERVISOR_PERMISSIONS,
    ),
];

const fn session_service_descriptor_capability_requirements(
    role: ServiceRole,
) -> Result<&'static [CapabilityGrantRequirement<'static>], SessionError> {
    match role {
        ServiceRole::SessionSupervisor => Ok(SESSION_SUPERVISOR_DESCRIPTOR_REQUIREMENTS),
        ServiceRole::RuntimeHost => Ok(RUNTIME_HOST_DESCRIPTOR_REQUIREMENTS),
        role => Err(SessionError::UnsupportedServiceRole { role }),
    }
}

const fn project_service_descriptor_capability_requirements(
    role: ServiceRole,
) -> Result<&'static [CapabilityGrantRequirement<'static>], SessionError> {
    match role {
        ServiceRole::ProjectHost => Ok(PROJECT_HOST_DESCRIPTOR_REQUIREMENTS),
        ServiceRole::AssetProcessor => Ok(ASSET_PROCESSOR_DESCRIPTOR_REQUIREMENTS),
        ServiceRole::Worker => Ok(ASSET_WORKER_DESCRIPTOR_REQUIREMENTS),
        role => Err(SessionError::UnsupportedServiceRole { role }),
    }
}

fn expected_session_service_id(role: ServiceRole) -> Result<ServiceId, SessionError> {
    match role {
        ServiceRole::SessionSupervisor => Ok(ServiceId::new(
            SESSION_SUPERVISOR_NAMESPACE,
            SESSION_SUPERVISOR_SERVICE_NAME,
        )),
        ServiceRole::RuntimeHost => Ok(ServiceId::new(
            RUNTIME_HOST_NAMESPACE,
            RUNTIME_HOST_SERVICE_NAME,
        )),
        role => Err(SessionError::UnsupportedServiceRole { role }),
    }
}

fn expected_project_service_id(role: ServiceRole) -> Result<ServiceId, SessionError> {
    match role {
        ServiceRole::ProjectHost => Ok(ServiceId::new(
            PROJECT_HOST_NAMESPACE,
            PROJECT_HOST_SERVICE_NAME,
        )),
        ServiceRole::AssetProcessor => Ok(ServiceId::new(
            ASSET_PROCESSOR_NAMESPACE,
            ASSET_PROCESSOR_SERVICE_NAME,
        )),
        ServiceRole::Worker => Ok(ServiceId::new(
            ASSET_WORKER_SERVICE_NAMESPACE,
            ASSET_WORKER_SERVICE_NAME,
        )),
        role => Err(SessionError::UnsupportedServiceRole { role }),
    }
}

fn invalid_service_descriptor(
    session: &str,
    service_id: &ServiceId,
    reason: impl Into<String>,
) -> SessionError {
    SessionError::InvalidServiceDescriptor {
        session: session.to_string(),
        service: service_label(service_id),
        reason: reason.into(),
    }
}

fn service_label(service_id: &ServiceId) -> String {
    format!("{}/{}", service_id.namespace, service_id.name)
}

fn capability_grants_for_service_process(descriptor: &ServiceRecord) -> Vec<Capability> {
    descriptor
        .to_descriptor()
        .capabilities
        .into_iter()
        .filter(|capability| {
            !is_observability_control_grant(capability) && !is_service_lifecycle_grant(capability)
        })
        .collect()
}

fn observability_capability_grants_for_service_process(
    descriptor: &ServiceRecord,
) -> Vec<Capability> {
    descriptor
        .to_descriptor()
        .capabilities
        .into_iter()
        .filter(is_observability_control_grant)
        .collect()
}

fn lifecycle_capability_grants_for_service_process(descriptor: &ServiceRecord) -> Vec<Capability> {
    descriptor
        .to_descriptor()
        .capabilities
        .into_iter()
        .filter(is_service_lifecycle_grant)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("az-session-test-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        // `TEMP` can name an 8.3 short path (GitHub's Windows runner does).
        // `SessionManager::with_data_home` normalizes the project root it
        // records, so hand every test the same canonical spelling.
        let path = normalize(&path);
        let manifest = az_project::ProjectManifest::new(
            format!("local.az_session_{name}"),
            format!("Az Session {name}"),
            env!("CARGO_PKG_VERSION"),
        );
        az_project::write_project_manifest(&path, &manifest).unwrap();
        az_project::refresh_project_lock(&path).unwrap();
        path
    }

    fn remove_project(root: impl AsRef<Path>) {
        std::fs::remove_dir_all(root).unwrap();
    }

    fn test_manager(root: &Path) -> SessionManager {
        SessionManager::with_data_home(root, AzothDataHome::new(root.join(".azoth-test"))).unwrap()
    }

    fn test_launch_command(root: &Path) -> SessionServiceLaunchCommand {
        let build_output_root = root.join("target");
        SessionServiceLaunchCommand {
            owner_id: "local.test".to_string(),
            owner_root: root.to_path_buf(),
            build_output_root: build_output_root.clone(),
            service_name: RUNTIME_HOST_SERVICE_NAME.to_string(),
            role: ServiceRole::RuntimeHost,
            endpoint: Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            program: build_output_root
                .join("debug")
                .join("runtime-host")
                .to_string_lossy()
                .into_owned(),
            cwd: root.to_path_buf(),
            args: Vec::new(),
        }
    }

    fn runtime_host_process_key() -> ServiceProcessKey {
        ServiceProcessKey::new(
            RUNTIME_HOST_SERVICE_NAME,
            SupervisedServiceRole::RuntimeHost,
        )
    }

    #[cfg(unix)]
    #[test]
    fn default_session_ipc_path_fits_strict_unix_socket_limits() {
        use std::os::unix::ffi::OsStrExt;

        let session_id = SessionId::new();
        let path = session_ipc_dir(session_id)
            .unwrap()
            .join("asset-processor-g18446744073709551615-observability.sock");

        assert!(
            path.as_os_str().as_bytes().len() < 104,
            "Unix socket path must fit macOS and Linux sockaddr_un limits: {}",
            path.display()
        );
        assert_eq!(
            path.parent().unwrap().file_name().unwrap().as_bytes().len(),
            24
        );
    }

    fn launch_arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
            .or_else(|| {
                args.iter()
                    .find_map(|arg| arg.strip_prefix(&format!("{flag}=")))
            })
    }

    #[test]
    fn diagnostic_bundle_sources_are_unique_by_bundle_path() {
        let mut sources = Vec::new();
        let mut bundle_paths = BTreeSet::new();
        let first = DiagnosticBundleSourceFile::new("run/logs/project-host.stdout.log", "log")
            .with_bundle_path("logs/project-host.stdout.log");
        let duplicate = DiagnosticBundleSourceFile::new("run/logs/project-host.stdout.log", "log")
            .with_bundle_path("logs/project-host.stdout.log");

        push_diagnostic_bundle_source(&mut sources, &mut bundle_paths, first);
        push_diagnostic_bundle_source(&mut sources, &mut bundle_paths, duplicate);

        assert_eq!(sources.len(), 1);
        assert_eq!(
            sources[0].bundle_path,
            PathBuf::from("logs/project-host.stdout.log")
        );
    }

    #[test]
    fn record_planned_services_replaces_the_current_run_under_stable_paths() {
        let root = temp_project("planned-stable-endpoint");
        let manager = test_manager(&root);
        let manifest = manager
            .create_session(CreateSessionRequest::new("Planned Stable Endpoint"))
            .unwrap();
        let mut command = test_launch_command(&manifest.workspace_root);
        command.endpoint = Endpoint::new(
            EndpointKind::WindowsNamedPipe,
            r"\\.\pipe\azoth-planned-runtime-host",
        );

        let first = manager
            .record_planned_services("planned-stable-endpoint", &[command.clone()])
            .unwrap();
        let second = manager
            .record_planned_services("planned-stable-endpoint", &[command])
            .unwrap();

        let first_process = first
            .processes
            .iter()
            .find(|process| process.service_name == RUNTIME_HOST_SERVICE_NAME)
            .unwrap();
        let second_process = second
            .processes
            .get(
                second
                    .current_service_process_index(&runtime_host_process_key())
                    .unwrap(),
            )
            .unwrap();
        let second_descriptor = second
            .service_descriptor(
                &ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME),
                ServiceRole::RuntimeHost,
            )
            .unwrap();

        assert_ne!(first_process.run, second_process.run);
        assert_eq!(first_process.run.get_version_num(), 7);
        assert_eq!(second_process.run.get_version_num(), 7);
        assert_eq!(second_process.previous_run, Some(first_process.run));
        assert_eq!(second.processes.len(), 1);
        assert!(
            second
                .current_service_process_index(&ServiceProcessKey::new(
                    RUNTIME_HOST_SERVICE_NAME,
                    SupervisedServiceRole::ProjectHost,
                ))
                .is_none(),
            "a same-name process with another role is a different durable key"
        );
        assert_eq!(
            first_process.endpoint_address,
            r"\\.\pipe\azoth-planned-runtime-host"
        );
        assert_eq!(
            second_process.endpoint_address,
            r"\\.\pipe\azoth-planned-runtime-host"
        );
        assert_eq!(
            second_descriptor.endpoint.address,
            second_process.endpoint_address
        );
        assert!(second_process.args.windows(2).any(
            |pair| pair[0] == "--endpoint" && pair[1] == r"\\.\pipe\azoth-planned-runtime-host"
        ));
        let expected_run = second_process.run.to_string();
        assert_eq!(
            launch_arg_value(&second_process.args, "--run"),
            Some(expected_run.as_str())
        );

        remove_project(root);
    }

    #[test]
    fn record_planned_services_publishes_observability_endpoint_and_grants() {
        let root = temp_project("planned-observability-endpoint");
        let manager = test_manager(&root);
        let manifest = manager
            .create_session(CreateSessionRequest::new("Planned Observability Endpoint"))
            .unwrap();
        let planned = manager
            .record_planned_services(
                "planned-observability-endpoint",
                &[test_launch_command(&manifest.workspace_root)],
            )
            .unwrap();

        let process = planned
            .processes
            .iter()
            .find(|process| process.service_name == RUNTIME_HOST_SERVICE_NAME)
            .unwrap();
        let descriptor = planned
            .service_descriptor(
                &ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME),
                ServiceRole::RuntimeHost,
            )
            .unwrap();
        let observability_endpoint = descriptor.observability_endpoint.as_ref().unwrap();
        assert_eq!(observability_endpoint.kind, EndpointKind::Tcp);
        assert_eq!(
            launch_arg_value(&process.args, "--observability-endpoint"),
            Some(observability_endpoint.address.as_str())
        );
        assert_eq!(
            launch_arg_value(&process.args, "--observability-endpoint-kind"),
            Some("tcp")
        );

        let main_grants_path =
            PathBuf::from(launch_arg_value(&process.args, "--capability-grants").unwrap());
        let lifecycle_grants_path = PathBuf::from(
            launch_arg_value(&process.args, "--lifecycle-capability-grants").unwrap(),
        );
        let observability_grants_path = PathBuf::from(
            launch_arg_value(&process.args, "--observability-capability-grants").unwrap(),
        );
        assert!(main_grants_path.is_file());
        assert!(lifecycle_grants_path.is_file());
        assert!(observability_grants_path.is_file());

        let main_grants =
            az_proto_core::decode_capability_grant_set(&std::fs::read(main_grants_path).unwrap())
                .unwrap();
        assert!(main_grants.grants().iter().all(|capability| {
            !is_observability_control_grant(capability) && !is_service_lifecycle_grant(capability)
        }));

        let lifecycle_grants = az_proto_core::decode_capability_grant_set(
            &std::fs::read(lifecycle_grants_path).unwrap(),
        )
        .unwrap();
        lifecycle_grants
            .validate_exact_brokered_for_session(
                planned.id.0,
                az_service_catalog::SESSION_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS,
            )
            .unwrap();

        let observability_grants = az_proto_core::decode_capability_grant_set(
            &std::fs::read(observability_grants_path).unwrap(),
        )
        .unwrap();
        assert_eq!(observability_grants.grants().len(), 2);
        assert!(
            observability_grants
                .grants()
                .iter()
                .all(is_observability_control_grant)
        );

        remove_project(root);
    }

    #[test]
    fn mark_service_running_persists_the_spawn_identity_exactly() {
        let root = temp_project("running-spawn-identity");
        let manager = test_manager(&root);
        let created = manager
            .create_session(CreateSessionRequest::new("Running Spawn Identity"))
            .unwrap();
        manager
            .record_planned_services(
                &created.slug,
                &[test_launch_command(&created.workspace_root)],
            )
            .unwrap();
        let identity = ProcessIdentity {
            process_id: 42,
            process_start_time: 9_001,
        };

        let running = manager
            .mark_service_running(&created.slug, &runtime_host_process_key(), identity)
            .unwrap();

        let process = &running.processes[running
            .current_service_process_index(&runtime_host_process_key())
            .unwrap()];
        assert_eq!(process.pid, Some(identity.process_id));
        assert_eq!(
            process.process_start_time,
            Some(identity.process_start_time)
        );
        remove_project(root);
    }

    #[test]
    fn mark_service_ready_persists_the_spawn_identity_exactly() {
        let root = temp_project("ready-spawn-identity");
        let manager = test_manager(&root);
        let created = manager
            .create_session(CreateSessionRequest::new("Ready Spawn Identity"))
            .unwrap();
        let mut starting = manager
            .record_planned_services(
                &created.slug,
                &[test_launch_command(&created.workspace_root)],
            )
            .unwrap();
        let process_index = starting
            .current_service_process_index(&runtime_host_process_key())
            .unwrap();
        starting.processes[process_index].mark_starting(now_unix_ms());
        SessionManager::write_manifest(&starting).unwrap();

        let process = &starting.processes[process_index];
        let descriptor = starting
            .service_descriptor(
                &ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME),
                ServiceRole::RuntimeHost,
            )
            .unwrap();
        let identity = ProcessIdentity {
            process_id: 42,
            process_start_time: 9_001,
        };
        let mut ready = ServiceReadyRecord::new(
            &process.service_name,
            process.role,
            process.run,
            &Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41000"),
            Some(identity.process_id),
            now_unix_ms(),
        );
        let observability = descriptor.observability_endpoint.unwrap();
        ready.observability_endpoint_kind =
            Some(ServiceEndpointKind::from_proto(observability.kind));
        ready.observability_endpoint_address = Some(observability.address);
        let lifecycle = descriptor.lifecycle_endpoint.unwrap();
        ready.lifecycle_endpoint_kind = Some(ServiceEndpointKind::from_proto(lifecycle.kind));
        ready.lifecycle_endpoint_address = Some(lifecycle.address);
        let ready_file = process.ready_file.as_ref().unwrap();
        std::fs::create_dir_all(ready_file.parent().unwrap()).unwrap();
        std::fs::write(ready_file, toml::to_string_pretty(&ready).unwrap()).unwrap();

        let running = manager
            .mark_service_ready(&created.slug, &runtime_host_process_key(), identity)
            .unwrap();

        let process = &running.processes[running
            .current_service_process_index(&runtime_host_process_key())
            .unwrap()];
        assert_eq!(process.pid, Some(identity.process_id));
        assert_eq!(
            process.process_start_time,
            Some(identity.process_start_time)
        );
        remove_project(root);
    }

    #[test]
    fn service_launch_validation_accepts_program_under_build_output_root() {
        let root =
            std::env::temp_dir().join(format!("az-session-launch-root-{}", uuid::Uuid::new_v4()));
        let command = test_launch_command(&root);

        validate_service_launch_command("editor", &command).unwrap();
    }

    #[test]
    fn service_launch_validation_rejects_program_outside_build_output_root() {
        let root =
            std::env::temp_dir().join(format!("az-session-launch-root-{}", uuid::Uuid::new_v4()));
        let mut command = test_launch_command(&root);
        command.program = root
            .join("other-target")
            .join("debug")
            .join("project-host")
            .to_string_lossy()
            .into_owned();

        let error = validate_service_launch_command("editor", &command).unwrap_err();

        assert!(matches!(
            error,
            SessionError::InvalidServiceLaunchPlan { reason, .. }
                if reason.contains("build output root")
        ));
    }

    #[test]
    fn manifest_cache_observes_same_length_transaction_replacement() {
        let root = temp_project("manifest-cache-file-identity");
        let manager = test_manager(&root);
        let mut first = manager
            .create_session(CreateSessionRequest::new("Editor"))
            .unwrap();
        first.updated_unix_ms = 111;
        SessionManager::write_manifest(&first).unwrap();

        let cached = manager.snapshot_by_slug("editor").unwrap();
        assert_eq!(cached.updated_unix_ms, 111);
        let path = cached.manifest_path();
        let before = ManifestState::capture(&path).unwrap();

        let mut replacement = (*cached).clone();
        replacement.updated_unix_ms = 222;
        let replacement_text = toml::to_string_pretty(&replacement).unwrap();
        assert_eq!(replacement_text.len() as u64, before.byte_length);
        FileTransaction::new(manifest_transaction_root(&replacement))
            .commit([FileWrite::new(&path, replacement_text.into_bytes())])
            .unwrap();

        let after = ManifestState::capture(&path).unwrap();
        assert_ne!(
            after.file_identity, before.file_identity,
            "transaction publication must replace the staged file identity"
        );
        assert_eq!(after.byte_length, before.byte_length);
        assert_eq!(
            manager.snapshot_by_slug("editor").unwrap().updated_unix_ms,
            222,
            "same-length publication must not reuse the cached parse"
        );

        remove_project(root);
    }

    #[test]
    fn create_session_references_existing_project_workspace() {
        let root = temp_project("existing-workspace");
        let manager = test_manager(&root);

        let manifest = manager
            .create_session(CreateSessionRequest::new("Editor"))
            .unwrap();

        assert_eq!(manifest.slug, "editor");
        assert_eq!(manifest.workspace_root, root);
        assert!(manifest.run_dir.is_dir());
        assert!(manifest.workspace_root.join("azoth.toml").is_file());

        remove_project(root);
    }

    #[test]
    fn retire_session_removes_only_the_run_directory() {
        let root = temp_project("retire-run-scope");
        let manager = test_manager(&root);
        manager
            .create_session(CreateSessionRequest::new("Retire Run Scope"))
            .unwrap();
        let project_sentinel = root.join("project-survives.txt");
        std::fs::write(&project_sentinel, "workspace-owned").unwrap();

        let retired = manager.retire_session("retire-run-scope").unwrap();

        assert_eq!(retired.state, SessionState::Removed);
        assert!(!retired.run_dir.exists());
        assert_eq!(
            std::fs::read_to_string(project_sentinel).unwrap(),
            "workspace-owned"
        );
        assert!(root.join("azoth.toml").is_file());

        remove_project(root);
    }

    #[test]
    fn record_planned_services_rejects_bad_batch_without_manifest_change() {
        let root = temp_project("atomic-service-plan");
        let manager = test_manager(&root);
        let manifest = manager
            .create_session(CreateSessionRequest::new("Atomic Service Plan"))
            .unwrap();
        let mut project_host = test_launch_command(&manifest.workspace_root);
        project_host.endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:10001");
        let planned = manager
            .record_planned_services("atomic-service-plan", &[project_host.clone()])
            .unwrap();
        let manifest_path = planned.manifest_path();
        let before = std::fs::read_to_string(&manifest_path).unwrap();

        let mut bad_asset_processor = test_launch_command(&manifest.workspace_root);
        bad_asset_processor.service_name = "wrong-asset-processor".to_string();
        bad_asset_processor.role = ServiceRole::AssetProcessor;
        bad_asset_processor.endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:10002");
        bad_asset_processor.program = bad_asset_processor
            .build_output_root
            .join("debug")
            .join("asset-processor")
            .to_string_lossy()
            .into_owned();

        let error = manager
            .record_planned_services("atomic-service-plan", &[project_host, bad_asset_processor])
            .unwrap_err();

        assert!(matches!(
            error,
            SessionError::InvalidServiceLaunchPlan { service, reason, .. }
                if service == "wrong-asset-processor"
                    && reason.contains("project-instance owned")
        ));
        assert_eq!(std::fs::read_to_string(&manifest_path).unwrap(), before);
        assert_eq!(manager.session("atomic-service-plan").unwrap(), planned);

        remove_project(root);
    }
}
