//! UI-free core for the user-level Azoth daemon.
//!
//! `azd` owns global project discovery and project-instance services. Sessions
//! attach to the shared project host, asset processor, and worker pool while
//! supervising only their runtime host.

mod build_progress;
mod progress;
pub mod project_services;
mod transport;

use progress::{
    CapnpProgressSink, CapnpProjectBuildProgressSink, OpenProgress, OpenProjectPhase,
    PhaseRegistry, ProjectBuildPhaseRegistry, ProjectBuildProgress,
};

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
#[cfg(any(test, feature = "test-support"))]
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use az_endpoint_discovery::{endpoint_token, project_service_endpoint_in};
use az_filesystem::{
    AzothDataHome, FileTransaction, FileTransactionError, FileWrite, HostTool, HostToolBundle,
    canonical, normalize,
};
use az_project::{
    GeneratedTargetPackage, GeneratedTargetsSyncReport, ProjectBuildSelectorCandidate,
    ProjectBuildTarget, ProjectBuildTargetKind, ProjectManifestError, ProjectPackageCompression,
    ProjectPackageContainer, ProjectPackageOodleCompressor, ProjectPackageOodleEffort,
    ProjectPackageProfile, ProjectServiceRole, ProjectServiceTarget, ResolvedProjectGraph,
    ensure_project_generated_targets, generated_package_name, load_resolved_project_graph,
    project_build_selector_candidates, project_lock_path, project_manifest_path,
    resolve_project_build_selector_indices, validate_project_generated_target_workspaces,
};
use az_proto_asset::{
    ASSET_PROCESSOR_AUDIENCE, ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME,
};
use az_proto_core::{
    Capability, CapabilityGrantSet, Endpoint, EndpointKind, ProtocolVersion, ServiceDescriptor,
    ServiceHealth, ServiceHealthState, ServiceId, ServiceRole, encode_capability_grant_set,
};
#[cfg(test)]
use az_proto_daemon::ProcessIdentity as ProtoProcessIdentity;
use az_proto_daemon::{
    DAEMON_AUDIENCE, DAEMON_CONTROL_PERMISSION, DAEMON_LEASE_PERMISSION,
    DAEMON_PROJECTS_PERMISSION, DAEMON_READ_PERMISSION, DAEMON_SESSIONS_PERMISSION,
    EnsureProjectSessionRequest, EnsureProjectSessionServicesRequest, ExecuteProjectBuildRequest,
    ListProjectsRequest, ListProjectsResult, ListSessionSupervisorsRequest,
    ListSessionSupervisorsResult, PlanProjectBuildRequest, PlanProjectServicesRequest,
    PrepareProjectSessionServicesRequest, ProjectBuildCommand, ProjectBuildExecutionResult,
    ProjectBuildPackageProfile, ProjectBuildPlan, ProjectBuildProgressEvent, ProjectRecord,
    ProjectResult, ProjectServiceCommand, ProjectServicePlan, ProjectSessionResult,
    ProjectSessionServicesResult, ProjectSessionServicesStartResult, RegisterProjectRequest,
    RegisterProjectRootRequest, RegisterSessionSupervisorRequest, ResolveProjectRequest,
    ResolveSessionSupervisorRequest, SessionSupervisorDescriptor, SessionSupervisorResult,
    ShutdownDaemonRequest, ShutdownDaemonResult, TouchEditorLeaseRequest, TouchEditorLeaseResult,
    UnregisterSessionSupervisorRequest, UnregisterSessionSupervisorResult, daemon_capnp,
    editor_process_lease_id,
};
use az_proto_session::{
    SESSION_EXEC_PERMISSION, SESSION_MANAGE_PERMISSION, SESSION_READ_PERMISSION,
    SESSION_SAVE_PERMISSION, SESSION_SUPERVISOR_AUDIENCE, SESSION_SUPERVISOR_NAMESPACE,
    SESSION_SUPERVISOR_SERVICE_NAME, ServiceProcessState as ProtoServiceProcessState,
    SessionCapabilityRequest, SessionManifest as ProtoSessionManifest, SessionSupervisorEvent,
    SessionSupervisorEventSubscriptionRequest, SessionSupervisorEventSubscriptionResult,
    SessionSupervisorIdentity, StartServicesRequest, StartServicesResult, StopServicesRequest,
    StopServicesResult, session_capnp,
};
use az_service_catalog::{
    DAEMON_SERVICE_NAME, DAEMON_SERVICE_NAMESPACE, add_observability_contract,
    add_service_lifecycle_contract, asset_processor_service_descriptor,
    asset_worker_service_descriptor, is_observability_control_grant, is_service_lifecycle_grant,
    project_host_service_descriptor,
};
use az_service_supervision::{
    ProcessIdentity, RecordedProcess, RecordedServiceProcessCleanup, SERVICE_READY_SCHEMA_VERSION,
    ServiceLifecycleController, ServiceLifecycleEvent, ServiceLifecycleEvents, ServiceProcessKey,
    ServiceProcessLauncher, ServiceProcessRecord, ServiceProcessState, ServiceReadyRecord,
    ServiceRecord, SpawnedServiceProcess, StdServiceProcessLauncher, SupervisedServiceRole,
    request_service_lifecycle_shutdown, rotate_log_at_plan_time,
    terminate_recorded_service_process,
};
use capnp::Error;
use crossbeam_channel as channel;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

pub use az_proto_daemon::{
    ProjectBuildCommand as PlannedProjectBuildCommand,
    ProjectBuildPackageProfile as PlannedProjectBuildPackageProfile,
    ProjectBuildPlan as PlannedProjectBuildPlan,
    ProjectServiceCommand as PlannedProjectServiceCommand,
    ProjectServicePlan as PlannedProjectServicePlan,
};
pub use transport::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SessionSupervisorKey {
    project_id: String,
    session_slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorLeaseRecord {
    owner_process: ProcessIdentity,
    purpose: String,
    touched_unix_ms: u64,
}

#[derive(Debug)]
struct AzDaemonState {
    projects: BTreeMap<String, ProjectRecord>,
    session_supervisors: BTreeMap<SessionSupervisorKey, ServiceDescriptor>,
    session_supervisor_waiters: Vec<channel::Sender<(SessionSupervisorKey, ServiceDescriptor)>>,
    editor_leases: BTreeMap<String, EditorLeaseRecord>,
    editor_lease_admissions_open: bool,
}

impl Default for AzDaemonState {
    fn default() -> Self {
        Self {
            projects: BTreeMap::new(),
            session_supervisors: BTreeMap::new(),
            session_supervisor_waiters: Vec::new(),
            editor_leases: BTreeMap::new(),
            editor_lease_admissions_open: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSupervisorStopReport {
    pub stopped: u32,
    pub failed: u32,
}

#[derive(Debug, Clone)]
struct SessionSupervisorShutdownTarget {
    project_id: String,
    project_root: PathBuf,
    session_slug: String,
    descriptor: ServiceDescriptor,
}

const EDITOR_SERVICE_NAMESPACE: &str = "azoth";
const EDITOR_SERVICE_NAME: &str = "editor";
const DAEMON_SESSION_SERVICE_NAMESPACE: &str = "azoth";
const DAEMON_SESSION_SERVICE_NAME: &str = "azd";
const SESSION_SUPERVISOR_PROBE_RPC_TIMEOUT: Duration = Duration::from_secs(2);
const SESSION_SUPERVISOR_STOP_RPC_TIMEOUT: Duration = Duration::from_secs(3);
const SESSION_SUPERVISOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const PROJECT_SERVICE_GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
impl SessionSupervisorKey {
    fn new(project_id: impl Into<String>, session_slug: impl Into<String>) -> Self {
        Self {
            project_id: project_id.into(),
            session_slug: session_slug.into(),
        }
    }
}

#[derive(Clone)]
pub struct AzDaemon {
    state: Arc<Mutex<AzDaemonState>>,
    editor_lease_lifecycle: Arc<ServiceLifecycleEvents>,
    project_runtimes: Arc<Mutex<BTreeMap<String, Arc<Mutex<ProjectServiceRuntime>>>>>,
    project_registry_path: Option<PathBuf>,
    data_home: AzothDataHome,
}

/// The daemon-owned lifetime of one project's launched service children.
///
/// Field order is intentional: launcher termination runs before lifecycle
/// joins its identity-bound exit workers during runtime teardown.
struct ProjectServiceRuntime {
    launcher: StdServiceProcessLauncher,
    lifecycle: ServiceLifecycleEvents,
}

impl ProjectServiceRuntime {
    fn new() -> Self {
        Self {
            launcher: StdServiceProcessLauncher::new(),
            lifecycle: ServiceLifecycleEvents::new(),
        }
    }
}

impl Default for AzDaemon {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            editor_lease_lifecycle: Arc::new(ServiceLifecycleEvents::new()),
            project_runtimes: Arc::default(),
            project_registry_path: None,
            data_home: AzothDataHome::resolve(),
        }
    }
}

impl fmt::Debug for AzDaemon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state();
        f.debug_struct("AzDaemon")
            .field("project_count", &state.projects.len())
            .field("session_supervisor_count", &state.session_supervisors.len())
            .field("editor_lease_count", &state.editor_leases.len())
            .field("project_runtime_count", &self.project_runtimes().len())
            .field("project_registry_path", &self.project_registry_path)
            .field("data_home", &self.data_home)
            // The locked state, lifecycle broker, and runtime map are reported
            // as counts above rather than dumped through their guards.
            .finish_non_exhaustive()
    }
}

/// Which build to run: the project, profile, target triple, and package
/// selectors that together name one cargo build universe.
///
/// These four values are the whole answer to "what is being built"; planning
/// consumes exactly them, and the RPC layer receives exactly them in
/// `ExecuteProjectBuildRequest`. They are chosen once by the caller and travel
/// unchanged from the request through planning into execution, while the
/// reporter, phase registry, and cancellation token describe how the run is
/// observed rather than what it builds.
#[derive(Debug, Clone, Copy)]
pub struct ProjectBuildSelection<'a> {
    /// Registered project whose workspace is built.
    pub project_id: &'a str,
    /// Cargo profile the planned commands build with.
    pub profile: &'a str,
    /// Cargo `--target` triple, or `None` to build for the host triple.
    pub target_triple: Option<&'a str>,
    /// Package selectors narrowing the build; empty selects the whole plan.
    pub package_selectors: &'a [String],
}

/// One request to build and record a session's project-instance services.
///
/// This is the daemon-side shape of `PrepareProjectSessionServicesRequest`
/// minus its capability: the session being prepared, the endpoint layout its
/// services are addressed on, and the four knobs that qualify the preparation
/// (skip the build, restrict the service set, point the services at an OTLP
/// collector, accept a failed-preserved session). Preparation is only
/// well-defined for all of them at once, and the public entry point and its
/// progress-carrying inner form both take the identical set.
#[derive(Debug, Clone, Copy)]
pub struct ProjectSessionServicesRequest<'a> {
    /// Registered project that owns the session.
    pub project_id: &'a str,
    /// Session slug whose services are prepared.
    pub session_slug: &'a str,
    /// Public endpoint kind the prepared services are addressed on.
    pub endpoint_kind: EndpointKind,
    /// Trust the already-built service programs instead of rebuilding them.
    pub skip_build: bool,
    /// Service names to prepare; empty prepares every planned service.
    pub service_names: &'a [String],
    /// OTLP collector the prepared services report to, when configured.
    pub otlp_endpoint: Option<&'a str>,
    /// Also accept a failed-preserved session instead of only an active one.
    pub recover: bool,
}

impl AzDaemon {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a daemon bound to an explicit machine-local data home.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::DataHome`] when `data_home` cannot be prepared
    /// (its directories cannot be created or its layout is rejected).
    pub fn with_data_home(data_home: AzothDataHome) -> Result<Self, AzDaemonError> {
        data_home.prepare()?;
        Ok(Self {
            data_home,
            ..Self::default()
        })
    }

    /// Build a daemon whose project registrations persist at `path`.
    ///
    /// # Errors
    ///
    /// Returns any error [`read_project_registry`] returns while loading the
    /// existing registry: [`AzDaemonError::ProjectRegistryTransactionRecovery`]
    /// for an unrecoverable interrupted write,
    /// [`AzDaemonError::ProjectRegistryRead`] for an unreadable file,
    /// [`AzDaemonError::ProjectRegistryParse`] for malformed TOML, and
    /// [`AzDaemonError::UnsupportedProjectRegistrySchema`] for a registry
    /// written by a different schema version.
    pub fn with_project_registry_path(path: impl AsRef<Path>) -> Result<Self, AzDaemonError> {
        let path = path.as_ref().to_path_buf();
        let projects = read_project_registry(&path)?;
        Ok(Self {
            state: Arc::new(Mutex::new(AzDaemonState {
                projects,
                ..AzDaemonState::default()
            })),
            project_registry_path: Some(path),
            ..Self::default()
        })
    }

    /// Open a supervision manager for the existing project workspace.
    fn session_manager(
        &self,
        project_root: impl AsRef<Path>,
    ) -> Result<az_session::SessionManager, az_session::SessionError> {
        az_session::SessionManager::with_data_home(project_root, self.data_home.clone())
    }

    fn project_service_store(
        &self,
        project: &ProjectRecord,
    ) -> Result<project_services::ProjectServiceStore, AzDaemonError> {
        let project_root = PathBuf::from(&project.root);
        let paths = self.data_home.project(&project.name, &project_root);
        paths.prepare()?;
        Ok(project_services::ProjectServiceStore::new(
            paths,
            project.project_id.clone(),
            project_root,
        )?)
    }

    /// Build a daemon using the machine-global project registry.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::DataHome`] when the machine-local data home
    /// cannot be prepared, plus any error
    /// [`Self::with_project_registry_path`] returns for the default path.
    pub fn with_default_project_registry() -> Result<Self, AzDaemonError> {
        AzothDataHome::resolve().prepare()?;
        Self::with_project_registry_path(default_project_registry_path())
    }

    #[must_use]
    pub fn project_registry_path(&self) -> Option<&Path> {
        self.project_registry_path.as_deref()
    }

    fn state(&self) -> MutexGuard<'_, AzDaemonState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn project_runtimes(
        &self,
    ) -> MutexGuard<'_, BTreeMap<String, Arc<Mutex<ProjectServiceRuntime>>>> {
        self.project_runtimes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn project_runtime(&self, project_id: &str) -> Arc<Mutex<ProjectServiceRuntime>> {
        self.project_runtimes()
            .entry(project_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(ProjectServiceRuntime::new())))
            .clone()
    }

    /// Register the project whose manifest lives at `root`.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::ProjectPathCanonicalize`] when `root` cannot be
    /// canonicalized, [`AzDaemonError::ProjectManifest`] when the manifest is
    /// missing, unreadable, or declares generated targets that cannot be
    /// synchronized, and any error [`Self::register_project`] returns for the
    /// record built from that manifest.
    pub fn register_project_root(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<ProjectRecord, AzDaemonError> {
        let root = normalize_existing_path(root.as_ref())?;
        ensure_daemon_generated_targets(&root)?;
        let graph = load_resolved_project_graph(&root)?;
        let manifest = graph.manifest;
        let project = ProjectRecord {
            project_id: manifest.project.id,
            name: manifest.project.name,
            root: root.to_string_lossy().into_owned(),
            manifest_path: project_manifest_path(&root).to_string_lossy().into_owned(),
            engine_version: manifest.project.engine_version,
        };
        self.register_project(&project)?;
        Ok(project)
    }

    /// Record one project registration and persist the registry.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::MissingProjectId`],
    /// [`AzDaemonError::MissingProjectName`],
    /// [`AzDaemonError::MissingProjectRoot`], or
    /// [`AzDaemonError::MissingProjectManifestPath`] when `project` is
    /// incomplete, and any error [`write_project_registry`] returns when the
    /// updated registry cannot be committed.
    #[instrument(skip(self, project), fields(project_id = %project.project_id, root = %project.root))]
    pub fn register_project(
        &self,
        project: &ProjectRecord,
    ) -> Result<ProjectRecord, AzDaemonError> {
        validate_project_record(project)?;

        let mut projects = {
            let state = self.state();
            state.projects.clone()
        };
        projects.insert(project.project_id.clone(), project.clone());
        self.write_project_registry(&projects)?;
        let mut state = self.state();
        state.projects = projects;
        drop(state);
        info!("azd registered project");
        Ok(project.clone())
    }

    #[must_use]
    pub fn list_projects(&self) -> Vec<ProjectRecord> {
        self.state().projects.values().cloned().collect()
    }

    #[must_use]
    pub fn resolve_project(&self, project_id: &str) -> Option<ProjectRecord> {
        self.state().projects.get(project_id).cloned()
    }

    fn write_project_registry(
        &self,
        projects: &BTreeMap<String, ProjectRecord>,
    ) -> Result<(), AzDaemonError> {
        if let Some(path) = &self.project_registry_path {
            write_project_registry(path, projects)?;
        }
        Ok(())
    }

    /// Publish the session supervisor descriptor for one project session.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::UnknownProject`] when `project_id` is not
    /// registered, [`AzDaemonError::InvalidSessionSlug`] when `session_slug` is
    /// blank, and [`AzDaemonError::InvalidSessionSupervisorRole`] or
    /// [`AzDaemonError::InvalidSessionSupervisorDescriptor`] when `descriptor`
    /// does not describe a reachable session supervisor.
    #[instrument(skip(self, descriptor), fields(project_id = %project_id, session_slug = %session_slug))]
    pub fn register_session_supervisor(
        &self,
        project_id: &str,
        session_slug: &str,
        descriptor: &ServiceDescriptor,
    ) -> Result<ServiceDescriptor, AzDaemonError> {
        {
            let state = self.state();
            if !state.projects.contains_key(project_id) {
                return Err(AzDaemonError::UnknownProject {
                    project_id: project_id.to_string(),
                });
            }
        }
        if session_slug.trim().is_empty() {
            return Err(AzDaemonError::InvalidSessionSlug);
        }
        validate_session_supervisor_descriptor(descriptor, "azd session-supervisor registration")?;
        let mut state = self.state();
        let key = SessionSupervisorKey::new(project_id, session_slug);
        state
            .session_supervisors
            .insert(key.clone(), descriptor.clone());
        state
            .session_supervisor_waiters
            .retain(|waiter| waiter.send((key.clone(), descriptor.clone())).is_ok());
        drop(state);
        info!("azd registered session supervisor");
        Ok(descriptor.clone())
    }

    fn subscribe_session_supervisor_registration(
        &self,
        project_id: &str,
        session_slug: &str,
    ) -> (
        Option<ServiceDescriptor>,
        channel::Receiver<(SessionSupervisorKey, ServiceDescriptor)>,
    ) {
        let key = SessionSupervisorKey::new(project_id, session_slug);
        let (sender, receiver) = channel::unbounded();
        let mut state = self.state();
        let current = state.session_supervisors.get(&key).cloned();
        state.session_supervisor_waiters.push(sender);
        drop(state);
        (current, receiver)
    }

    /// Refresh (or open) the editor lease that keeps `azd` alive.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::InvalidCapability`] when the request does not
    /// carry the lease permission, [`AzDaemonError::InvalidEditorLease`] when
    /// the lease id, owner identity, or purpose is empty, does not match the
    /// canonical id for the owner process, names a process that is not alive,
    /// or arrives after shutdown closed lease admissions, and
    /// [`AzDaemonError::ProjectServiceProcess`] when the owner process cannot
    /// be bound to the lifecycle watcher.
    pub fn touch_editor_lease(
        &self,
        request: &TouchEditorLeaseRequest,
    ) -> Result<TouchEditorLeaseResult, AzDaemonError> {
        validate_capability(&request.capability, DAEMON_LEASE_PERMISSION)?;
        let owner_process = ProcessIdentity {
            process_id: request.owner_process.process_id,
            process_start_time: request.owner_process.process_start_time,
        };
        if request.lease_id.trim().is_empty() {
            return Err(AzDaemonError::InvalidEditorLease {
                reason: "lease id cannot be empty".to_string(),
            });
        }
        if request.owner_process.process_id == 0 || request.owner_process.process_start_time == 0 {
            return Err(AzDaemonError::InvalidEditorLease {
                reason: "owner process identity must be complete".to_string(),
            });
        }
        let canonical_lease_id = editor_process_lease_id(request.owner_process);
        if request.lease_id != canonical_lease_id {
            return Err(AzDaemonError::InvalidEditorLease {
                reason: format!("lease id must be `{canonical_lease_id}`"),
            });
        }
        if request.purpose.trim().is_empty() {
            return Err(AzDaemonError::InvalidEditorLease {
                reason: "purpose cannot be empty".to_string(),
            });
        }
        validate_editor_lease_owner_process(owner_process)?;
        let mut state = self.state();
        if !state.editor_lease_admissions_open {
            return Err(AzDaemonError::InvalidEditorLease {
                reason: "azd is shutting down and no longer accepts editor leases".to_string(),
            });
        }
        self.editor_lease_lifecycle.add_identity(owner_process)?;
        state.editor_leases.insert(
            request.lease_id.clone(),
            EditorLeaseRecord {
                owner_process,
                purpose: request.purpose.clone(),
                touched_unix_ms: current_unix_ms(),
            },
        );
        let active_lease_count = count_u32(state.editor_leases.len());
        // The admissions check and the insert above are the guarded window; the
        // log line and reply do not need the lock.
        drop(state);
        info!(
            lease_id = %request.lease_id,
            owner_process_id = request.owner_process.process_id,
            owner_process_start_time = request.owner_process.process_start_time,
            active_lease_count,
            "azd touched editor lease"
        );
        Ok(TouchEditorLeaseResult {
            accepted: true,
            lease_id: request.lease_id.clone(),
            active_lease_count,
        })
    }

    #[must_use]
    pub fn active_editor_lease_count(&self) -> usize {
        self.state().editor_leases.len()
    }

    /// Wait for explicit cancellation or the exact exit of the final editor
    /// process that owns this sidecar.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::ProjectServiceProcess`] when the lease
    /// lifecycle watcher fails: wrapping
    /// `ServiceProcessError::ProcessExitBindingUnavailable` when an owner
    /// process exit can no longer be observed, or
    /// `ServiceProcessError::LifecycleEventSourceClosed` when the event channel
    /// closes. Cancelling outstanding exit waits on the way out can surface the
    /// same error.
    pub fn wait_for_shutdown(
        &self,
        shutdown: &az_work::CancellationToken,
        shutdown_when_editor_leases_gone: bool,
    ) -> Result<(), AzDaemonError> {
        let close_for_initial_empty = if shutdown_when_editor_leases_gone {
            let mut state = self.state();
            if state.editor_leases.is_empty() {
                state.editor_lease_admissions_open = false;
                true
            } else {
                false
            }
        } else {
            false
        };
        if close_for_initial_empty {
            info!("azd sidecar has no active editor leases; shutting down");
            shutdown.cancel();
        }
        let cancellation = shutdown.cancellation_signal();
        loop {
            channel::select! {
                recv(cancellation.receiver()) -> _ => {
                    self.state().editor_lease_admissions_open = false;
                    self.editor_lease_lifecycle.cancel_all_exit_waits()?;
                    return Ok(());
                }
                recv(self.editor_lease_lifecycle.receiver()) -> event => match event {
                    Ok(ServiceLifecycleEvent::ProcessExited(identity)) => {
                        let close_for_final_exit = {
                            let mut state = self.state();
                            let removed = remove_editor_leases_for_identity(&mut state, identity);
                            if removed > 0
                                && shutdown_when_editor_leases_gone
                                && state.editor_leases.is_empty()
                            {
                                state.editor_lease_admissions_open = false;
                                true
                            } else {
                                false
                            }
                        };
                        self.editor_lease_lifecycle.consume_exit(identity)?;
                        if close_for_final_exit {
                            info!("azd sidecar has no active editor leases; shutting down");
                            shutdown.cancel();
                        }
                    }
                    Ok(ServiceLifecycleEvent::ProcessExitWaitFailed { identity, reason }) => {
                        self.state().editor_lease_admissions_open = false;
                        self.editor_lease_lifecycle.consume_exit(identity)?;
                        self.editor_lease_lifecycle.cancel_all_exit_waits()?;
                        return Err(az_service_supervision::ServiceProcessError::ProcessExitBindingUnavailable {
                            identity,
                            reason,
                        }.into());
                    }
                    Ok(ServiceLifecycleEvent::ReadyFileChanged) => {}
                    Err(_) => {
                        self.state().editor_lease_admissions_open = false;
                        self.editor_lease_lifecycle.cancel_all_exit_waits()?;
                        return Err(az_service_supervision::ServiceProcessError::LifecycleEventSourceClosed.into());
                    }
                }
            }
        }
    }

    pub fn stop_registered_session_supervisors(&self, reason: &str) -> SessionSupervisorStopReport {
        let supervisors = self.registered_session_supervisor_shutdown_targets();
        let mut stopped = 0;
        let mut failed = 0;
        for target in supervisors {
            let manager = match self.session_manager(&target.project_root) {
                Ok(manager) => manager,
                Err(error) => {
                    failed += 1;
                    info!(
                        project_id = %target.project_id,
                        session = %target.session_slug,
                        error = %error,
                        "azd could not open session manager while stopping session supervisor"
                    );
                    continue;
                }
            };
            let manifest = match manager.session(&target.session_slug) {
                Ok(manifest) => manifest,
                Err(error) => {
                    failed += 1;
                    info!(
                        project_id = %target.project_id,
                        session = %target.session_slug,
                        error = %error,
                        "azd could not load session manifest while stopping session supervisor"
                    );
                    continue;
                }
            };
            match request_session_service_stop(&manifest, &target.descriptor, reason) {
                Ok(_) => {
                    stopped += 1;
                    info!(
                        project_id = %target.project_id,
                        session = %target.session_slug,
                        "azd requested session-supervisor shutdown"
                    );
                }
                Err(error) => {
                    failed += 1;
                    info!(
                        project_id = %target.project_id,
                        session = %target.session_slug,
                        error = %error,
                        "azd failed to request session-supervisor shutdown"
                    );
                }
            }
        }
        SessionSupervisorStopReport { stopped, failed }
    }

    fn registered_session_supervisor_shutdown_targets(
        &self,
    ) -> Vec<SessionSupervisorShutdownTarget> {
        let state = self.state();
        state
            .session_supervisors
            .iter()
            .filter_map(|(key, descriptor)| {
                state
                    .projects
                    .get(&key.project_id)
                    .map(|project| SessionSupervisorShutdownTarget {
                        project_id: key.project_id.clone(),
                        project_root: PathBuf::from(&project.root),
                        session_slug: key.session_slug.clone(),
                        descriptor: descriptor.clone(),
                    })
            })
            .collect()
    }

    /// Withdraw a session supervisor descriptor, if it is still the current one.
    ///
    /// Returns `Ok(false)` when no matching descriptor is registered, so
    /// unregistering something already gone is success.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::UnknownProject`] when `project_id` is not
    /// registered, [`AzDaemonError::InvalidSessionSlug`] when `session_slug` is
    /// blank, and [`AzDaemonError::InvalidSessionSupervisorRole`] or
    /// [`AzDaemonError::InvalidSessionSupervisorDescriptor`] when `descriptor`
    /// is not a well-formed session-supervisor descriptor.
    #[instrument(skip(self, descriptor), fields(project_id = %project_id, session_slug = %session_slug))]
    pub fn unregister_session_supervisor(
        &self,
        project_id: &str,
        session_slug: &str,
        descriptor: &ServiceDescriptor,
    ) -> Result<bool, AzDaemonError> {
        {
            let state = self.state();
            if !state.projects.contains_key(project_id) {
                return Err(AzDaemonError::UnknownProject {
                    project_id: project_id.to_string(),
                });
            }
        }
        if session_slug.trim().is_empty() {
            return Err(AzDaemonError::InvalidSessionSlug);
        }
        validate_session_supervisor_descriptor(descriptor, "azd session-supervisor unregister")?;

        let key = SessionSupervisorKey::new(project_id, session_slug);
        let mut state = self.state();
        let supervisors = &mut state.session_supervisors;
        if !matches!(
            supervisors.get(&key),
            Some(current) if current.has_same_connection_contract(descriptor)
        ) {
            info!("azd skipped session supervisor unregister; descriptor changed or missing");
            return Ok(false);
        }

        supervisors.remove(&key);
        // The contract check and the removal are one guarded step; the log line
        // is not.
        drop(state);
        info!("azd unregistered session supervisor");
        Ok(true)
    }

    #[must_use]
    pub fn resolve_session_supervisor(
        &self,
        project_id: &str,
        session_slug: &str,
    ) -> Option<ServiceDescriptor> {
        self.state()
            .session_supervisors
            .get(&SessionSupervisorKey::new(project_id, session_slug))
            .cloned()
    }

    #[must_use]
    pub fn list_session_supervisors(&self, project_id: &str) -> Vec<SessionSupervisorDescriptor> {
        self.state()
            .session_supervisors
            .iter()
            .filter(|(key, _)| key.project_id == project_id)
            .map(|(key, descriptor)| SessionSupervisorDescriptor {
                session_slug: key.session_slug.clone(),
                descriptor: descriptor.clone(),
            })
            .collect()
    }

    /// Resolve, recover, or create one project session.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::InvalidSessionSlug`] for a blank
    /// `session_name`, [`AzDaemonError::UnknownProject`] when `project_id` is
    /// not registered, [`AzDaemonError::ProjectSession`] when the session store
    /// cannot be opened, recovered, or created, and any error
    /// [`Self::ensure_project_session_with_manager`] returns.
    pub fn ensure_project_session(
        &self,
        project_id: &str,
        session_name: &str,
    ) -> Result<ProjectSessionResult, AzDaemonError> {
        if session_name.trim().is_empty() {
            return Err(AzDaemonError::InvalidSessionSlug);
        }
        let project =
            self.resolve_project(project_id)
                .ok_or_else(|| AzDaemonError::UnknownProject {
                    project_id: project_id.to_string(),
                })?;
        let manager = self.session_manager(PathBuf::from(&project.root))?;
        Self::ensure_project_session_with_manager(project_id, session_name, &project, &manager)
    }

    /// Resolve, recover, or create a session against an already-open manager.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::InvalidSessionSlug`] for a blank
    /// `session_name`, [`AzDaemonError::ProjectSessionProjectMismatch`] when
    /// `manager` belongs to a different project, and
    /// [`AzDaemonError::ProjectSession`] when the session cannot be read,
    /// recovered from `FailedPreserved`, or created.
    fn ensure_project_session_with_manager(
        project_id: &str,
        session_name: &str,
        project: &ProjectRecord,
        manager: &az_session::SessionManager,
    ) -> Result<ProjectSessionResult, AzDaemonError> {
        if session_name.trim().is_empty() {
            return Err(AzDaemonError::InvalidSessionSlug);
        }
        if manager.project_id() != project_id {
            return Err(AzDaemonError::ProjectSessionProjectMismatch {
                requested_project_id: project_id.to_string(),
                actual_project_id: manager.project_id().to_string(),
                project_root: project.root.clone(),
            });
        }

        match manager.session(session_name) {
            Ok(manifest) => {
                let manifest = if manifest.state == az_session::SessionState::FailedPreserved {
                    manager.recover_session(session_name, true)?
                } else {
                    manifest
                };
                Ok(ProjectSessionResult {
                    manifest: az_session::session_manifest_to_proto(&manifest),
                    created: false,
                })
            }
            Err(az_session::SessionError::SessionNotFound(_)) => {
                let manifest = manager.create_session(az_session::CreateSessionRequest {
                    name: session_name.to_string(),
                })?;
                Ok(ProjectSessionResult {
                    manifest: az_session::session_manifest_to_proto(&manifest),
                    created: true,
                })
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Plan the default cargo commands for one project build.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_project_build_selected`] returns for an
    /// empty selector list.
    pub fn plan_project_build(
        &self,
        project_id: &str,
        profile: &str,
        target_triple: Option<&str>,
    ) -> Result<ProjectBuildPlan, AzDaemonError> {
        self.plan_project_build_selected(project_id, profile, target_triple, &[])
    }

    /// Plan the cargo commands for the named packages of one project build.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::UnknownProject`] when `project_id` is not
    /// registered, [`AzDaemonError::ProjectManifest`] when the project graph or
    /// its generated targets cannot be resolved,
    /// [`AzDaemonError::InvalidBuildPackageSelector`] when a selector matches no
    /// candidate package, and [`AzDaemonError::InvalidBuildProfile`],
    /// [`AzDaemonError::NoBuildTargets`], or
    /// [`AzDaemonError::MissingBuildTargetName`] when the resolved graph cannot
    /// be turned into a build plan.
    pub fn plan_project_build_selected(
        &self,
        project_id: &str,
        profile: &str,
        target_triple: Option<&str>,
        package_selectors: &[String],
    ) -> Result<ProjectBuildPlan, AzDaemonError> {
        let project =
            self.resolve_project(project_id)
                .ok_or_else(|| AzDaemonError::UnknownProject {
                    project_id: project_id.to_string(),
                })?;
        let root = PathBuf::from(&project.root);
        let graph = load_resolved_project_graph(&root)?;
        if !package_selectors.is_empty() {
            resolve_project_build_selector_indices(
                &project_build_selector_candidates(&graph),
                package_selectors,
            )
            .map_err(|error| invalid_build_selector(project_id, error))?;
        }
        let generated = ensure_daemon_generated_targets(&root)?;
        project_build_plan_from_graph(
            &root,
            project_id,
            profile,
            target_triple,
            package_selectors,
            &graph,
            &generated,
        )
    }

    /// Plan and run one project build, reporting progress as it goes.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::execute_project_build_selected`] returns for
    /// an empty selector list.
    #[instrument(skip(self, reporter, registry, cancel), fields(project_id = %project_id, profile = %profile, target_triple = ?target_triple))]
    pub fn execute_project_build(
        &self,
        project_id: &str,
        profile: &str,
        target_triple: Option<&str>,
        reporter: &az_work::Reporter,
        registry: &Arc<Mutex<ProjectBuildPhaseRegistry>>,
        cancel: &az_work::CancellationToken,
    ) -> Result<ProjectBuildExecutionResult, AzDaemonError> {
        self.execute_project_build_selected(
            &ProjectBuildSelection {
                project_id,
                profile,
                target_triple,
                package_selectors: &[],
            },
            reporter,
            registry,
            cancel,
        )
    }

    /// Plan and run one project build restricted to the named packages.
    ///
    /// A command that fails is reported as `Ok` with `success: false` and the
    /// captured diagnostics, not as an `Err`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_project_build_selected`] returns while
    /// planning, [`AzDaemonError::UnknownProject`] if the project disappears
    /// from the registry between planning and staging, and
    /// [`AzDaemonError::RuntimeFileStaging`] when the authored runtime sidecars
    /// cannot be staged after the cargo commands succeed.
    #[instrument(
        skip_all,
        fields(
            project_id = %selection.project_id,
            profile = %selection.profile,
            target_triple = ?selection.target_triple,
            selector_count = selection.package_selectors.len()
        )
    )]
    pub fn execute_project_build_selected(
        &self,
        selection: &ProjectBuildSelection<'_>,
        reporter: &az_work::Reporter,
        registry: &Arc<Mutex<ProjectBuildPhaseRegistry>>,
        cancel: &az_work::CancellationToken,
    ) -> Result<ProjectBuildExecutionResult, AzDaemonError> {
        info!("starting project build execution");
        let plan = self.plan_project_build_selected(
            selection.project_id,
            selection.profile,
            selection.target_triple,
            selection.package_selectors,
        )?;
        let progress = ProjectBuildProgress::new(reporter, registry, &plan.commands);
        let mut completed_command_count = 0_u32;
        info!(
            command_count = plan.commands.len(),
            "planned project build execution"
        );

        for index in 0..plan.commands.len() {
            match run_planned_build_command(&plan, &progress, index, cancel) {
                PlannedCommandOutcome::Completed { label } => {
                    completed_command_count = completed_command_count.saturating_add(1);
                    info!(
                        command = %label,
                        completed_command_count,
                        "finished project build command"
                    );
                }
                PlannedCommandOutcome::Failed {
                    command,
                    diagnostic_headline,
                    diagnostic_tail,
                } => {
                    return Ok(ProjectBuildExecutionResult {
                        success: false,
                        plan,
                        completed_command_count,
                        failing_command: Some(command),
                        diagnostic_headline,
                        diagnostic_tail,
                    });
                }
            }
        }

        // Stage authored runtime sidecars after every cargo command succeeds so
        // editor/daemon-executed builds match the CLI path without a protocol bump.
        let project = self.resolve_project(selection.project_id).ok_or_else(|| {
            AzDaemonError::UnknownProject {
                project_id: selection.project_id.to_string(),
            }
        })?;
        let staging_reports = az_project::stage_selected_authored_runtime_files(
            Path::new(&project.root),
            selection.package_selectors,
            selection.profile,
        )?;
        log_staged_runtime_files(&staging_reports);

        info!(completed_command_count, "finished project build execution");
        Ok(ProjectBuildExecutionResult {
            success: true,
            plan,
            completed_command_count,
            failing_command: None,
            diagnostic_headline: String::new(),
            diagnostic_tail: String::new(),
        })
    }

    /// Plan every project-instance service for one session.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_project_services_selected`] returns for
    /// an empty selection.
    pub fn plan_project_services(
        &self,
        project_id: &str,
        session_slug: &str,
        endpoint_kind: EndpointKind,
        workspace_root: Option<&Path>,
    ) -> Result<ProjectServicePlan, AzDaemonError> {
        self.plan_project_services_selected(
            project_id,
            session_slug,
            endpoint_kind,
            workspace_root,
            &[],
        )
    }

    /// Plan the named project-instance services for one session.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::InvalidSessionSlug`] for a blank
    /// `session_slug`, [`AzDaemonError::UnsupportedEndpointKind`] for a
    /// non-public `endpoint_kind`, [`AzDaemonError::UnknownProject`] when
    /// `project_id` is not registered,
    /// [`AzDaemonError::InvalidProjectServiceWorkspaceRoot`] or
    /// [`AzDaemonError::ProjectServiceWorkspaceMismatch`] when `workspace_root`
    /// does not belong to the project, [`AzDaemonError::ProjectManifest`] when
    /// the project graph or its generated targets cannot be resolved,
    /// [`AzDaemonError::DuplicateServiceTargetName`] when two owners declare the
    /// same service, [`AzDaemonError::HostTool`] when the engine
    /// asset-processor host binary is missing,
    /// [`AzDaemonError::NoServiceTargets`] when nothing is planned, and
    /// [`AzDaemonError::InvalidServicePlan`] when `selected_service_names` names
    /// a service the plan does not contain.
    pub fn plan_project_services_selected(
        &self,
        project_id: &str,
        session_slug: &str,
        endpoint_kind: EndpointKind,
        workspace_root: Option<&Path>,
        selected_service_names: &[String],
    ) -> Result<ProjectServicePlan, AzDaemonError> {
        if session_slug.trim().is_empty() {
            return Err(AzDaemonError::InvalidSessionSlug);
        }
        validate_public_endpoint_kind(endpoint_kind, "azd project service planning")?;

        let project =
            self.resolve_project(project_id)
                .ok_or_else(|| AzDaemonError::UnknownProject {
                    project_id: project_id.to_string(),
                })?;
        let (root, graph, generated) =
            service_plan_root_and_graph(project_id, &project, workspace_root)?;

        let (mut build_commands, mut commands) = if graph.manifest.project.primary_gem.is_some() {
            let generated = generated.as_ref().ok_or_else(|| {
                AzDaemonError::ProjectManifest(ProjectManifestError::InvalidGeneratedTargets {
                    path: root.clone(),
                    reason: "primary-gem project did not produce generated target packages"
                        .to_string(),
                })
            })?;
            self.plan_primary_gem_services(
                &root,
                &graph,
                generated,
                project_id,
                session_slug,
                endpoint_kind,
            )?
        } else {
            self.plan_workspace_services(&root, &graph, project_id, session_slug, endpoint_kind)?
        };

        if commands.is_empty() {
            return Err(AzDaemonError::NoServiceTargets {
                project_id: project_id.to_string(),
            });
        }

        commands.sort_by_key(|command| service_plan_priority(command.role));
        sort_build_commands_into_launch_order(&mut build_commands, &commands);

        let mut plan = ProjectServicePlan {
            build_commands,
            commands,
        };
        let planned_service_names = project_service_names(&plan);
        let service_names =
            requested_service_names(selected_service_names, &planned_service_names)?;
        retain_project_service_plan_services(&mut plan, &service_names)?;
        Ok(plan)
    }

    /// Plan the service commands of a primary-gem project.
    ///
    /// The asset-processor host is engine-owned: it has no project cargo
    /// package and no gem linkage, so it is launched from the host tool bundle
    /// and planning fails closed when that binary is missing.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::DuplicateServiceTargetName`] when two owners
    /// claim the same service name, [`AzDaemonError::HostTool`] when the engine
    /// asset-processor host binary cannot be resolved, and
    /// [`AzDaemonError::UnsupportedEndpointKind`] for a non-public
    /// `endpoint_kind`.
    fn plan_primary_gem_services(
        &self,
        root: &Path,
        graph: &ResolvedProjectGraph,
        generated: &GeneratedBuildContext,
        project_id: &str,
        session_slug: &str,
        endpoint_kind: EndpointKind,
    ) -> Result<(Vec<ProjectBuildCommand>, Vec<ProjectServiceCommand>), AzDaemonError> {
        let mut service_owners = BTreeMap::new();
        let mut build_commands = Vec::new();
        let mut commands = Vec::new();
        let owner_id = &graph.manifest.project.id;

        let engine_ap_target = engine_asset_processor_service_target();
        record_service_target_owner(&mut service_owners, owner_id, &engine_ap_target)?;
        commands.push(engine_asset_processor_service_command(
            &self.data_home,
            root,
            owner_id,
            project_id,
            session_slug,
            endpoint_kind,
        )?);
        for target in generated_service_targets() {
            record_service_target_owner(&mut service_owners, owner_id, &target)?;
            build_commands.push(generated_service_build_command(
                root, owner_id, generated, &target,
            ));
            commands.push(generated_service_command(
                &ServiceSite {
                    data_home: &self.data_home,
                    project_root: root,
                    // Generated services build into the project's own
                    // workspace, so the binary root is the project root.
                    binary_root: root,
                    owner_id,
                    project_id,
                    session_slug,
                },
                generated,
                &target,
                endpoint_kind,
            )?);
        }
        Ok((build_commands, commands))
    }

    /// Plan the service commands declared by a project workspace and its gems.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::DuplicateServiceTargetName`] when the project
    /// and one of its gems declare the same service name,
    /// [`AzDaemonError::MissingBuildTargetName`] or
    /// [`AzDaemonError::InvalidBuildProfile`] when a declared target cannot be
    /// turned into a cargo command, and
    /// [`AzDaemonError::UnsupportedEndpointKind`] for a non-public
    /// `endpoint_kind`.
    fn plan_workspace_services(
        &self,
        root: &Path,
        graph: &ResolvedProjectGraph,
        project_id: &str,
        session_slug: &str,
        endpoint_kind: EndpointKind,
    ) -> Result<(Vec<ProjectBuildCommand>, Vec<ProjectServiceCommand>), AzDaemonError> {
        let mut service_owners = BTreeMap::new();
        let mut build_commands = Vec::new();
        let mut commands = Vec::new();

        let owners = std::iter::once((
            root,
            &graph.manifest.project.id,
            &graph.manifest.tools.service_targets,
        ))
        .chain(graph.gems.iter().map(|gem| {
            (
                gem.root.as_path(),
                &gem.manifest.gem.id,
                &gem.manifest.tools.service_targets,
            )
        }));

        for (owner_root, owner_id, service_targets) in owners {
            for target in default_service_targets(service_targets) {
                record_service_target_owner(&mut service_owners, owner_id, target)?;
                build_commands.push(service_build_command(owner_root, owner_id, target)?);
                commands.push(service_command(
                    &ServiceSite {
                        data_home: &self.data_home,
                        project_root: root,
                        binary_root: owner_root,
                        owner_id,
                        project_id,
                        session_slug,
                    },
                    target,
                    endpoint_kind,
                )?);
            }
        }
        Ok((build_commands, commands))
    }

    /// Build and record every project-instance service for one session.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::prepare_project_session_services_selected`]
    /// returns for an empty selection and no OTLP endpoint.
    pub fn prepare_project_session_services(
        &self,
        project_id: &str,
        session_slug: &str,
        endpoint_kind: EndpointKind,
        skip_build: bool,
    ) -> Result<ProjectSessionServicesResult, AzDaemonError> {
        self.prepare_project_session_services_inner(
            &ProjectSessionServicesRequest {
                project_id,
                session_slug,
                endpoint_kind,
                skip_build,
                service_names: &[],
                otlp_endpoint: None,
                recover: false,
            },
            None,
        )
    }

    fn record_project_service_plan(
        &self,
        project: &ProjectRecord,
        commands: &[ProjectServiceCommand],
        otlp_endpoint: Option<&str>,
        program_freshness: ServiceProgramFreshnessPolicy,
    ) -> Result<project_services::ProjectServiceManifest, AzDaemonError> {
        let store = self.project_service_store(project)?;
        let now = u128::from(current_unix_ms());
        let mut manifest = store.load_or_create(now)?;
        fs::create_dir_all(store.logs_dir())?;
        fs::create_dir_all(store.ready_dir())?;
        fs::create_dir_all(store.grants_dir())?;
        fs::create_dir_all(store.side_channels_dir())?;
        fs::create_dir_all(store.asset_processing_staging_dir())?;
        fs::create_dir_all(store.product_cache_dir())?;

        if project_services_have_reusable_launch_plan(&manifest, commands, program_freshness) {
            return Ok(manifest);
        }

        self.retire_project_services_before_replan(project, &store, &mut manifest, commands)?;

        let mut writes = Vec::new();
        let mut seen = BTreeSet::new();
        for command in commands
            .iter()
            .filter(|command| command.role != ServiceRole::RuntimeHost)
        {
            let descriptor = project_service_descriptor_for_command(command, Uuid::now_v7())?;
            if !seen.insert(command.service_name.as_str()) {
                return Err(AzDaemonError::InvalidServicePlan {
                    service: command.service_name.clone(),
                    reason: "project service launch plan contains a duplicate service".to_string(),
                });
            }
            let descriptor_record = manifest.upsert_service_descriptor(&descriptor, now)?;

            let files = ProjectServiceFiles::new(&store, &command.service_name);
            rotate_log_at_plan_time(&files.stdout_log)?;
            rotate_log_at_plan_time(&files.stderr_log)?;
            rotate_log_at_plan_time(&files.structured_log)?;

            let role_grants = project_service_role_grants(&manifest, command, &descriptor)?;
            let observability_grants = descriptor
                .capabilities
                .iter()
                .filter(|capability| is_observability_control_grant(capability))
                .cloned()
                .collect::<Vec<_>>();
            let lifecycle_grants = descriptor
                .capabilities
                .iter()
                .filter(|capability| is_service_lifecycle_grant(capability))
                .cloned()
                .collect::<Vec<_>>();
            writes.push(FileWrite::new(
                files.grants_file.clone(),
                encode_capability_grant_set(&CapabilityGrantSet::from_grants(role_grants))?,
            ));
            writes.push(FileWrite::new(
                files.observability_grants_file.clone(),
                encode_capability_grant_set(&CapabilityGrantSet::from_grants(
                    observability_grants,
                ))?,
            ));
            writes.push(FileWrite::new(
                files.lifecycle_grants_file.clone(),
                encode_capability_grant_set(&CapabilityGrantSet::from_grants(lifecycle_grants))?,
            ));

            let args = project_service_launch_args(
                &store,
                &manifest,
                project,
                command,
                &descriptor,
                &files,
                otlp_endpoint,
            )?;

            let mut process = ServiceProcessRecord::planned(
                command.service_name.clone(),
                descriptor_record.role,
                descriptor.run,
                &command.endpoint,
                command.program.clone(),
                PathBuf::from(&command.cwd),
                args,
                files.stdout_log,
                files.stderr_log,
                files.structured_log,
                Some(files.ready_file),
                now,
            );
            process.owner_id.clone_from(&command.owner_id);
            process.owner_root = PathBuf::from(&command.owner_root);
            manifest.upsert_process(process, now);
        }

        store.write_with_files(&manifest, writes)?;
        Ok(manifest)
    }

    // `launcher` and `lifecycle` borrow out of the guard for the whole body, so
    // clippy's suggested early `drop(runtime)` would not compile; the lock also
    // has to span the retire sequence to serialize it against a concurrent
    // start of the same project's services.
    #[allow(clippy::significant_drop_tightening)]
    fn retire_project_services_before_replan(
        &self,
        project: &ProjectRecord,
        store: &project_services::ProjectServiceStore,
        manifest: &mut project_services::ProjectServiceManifest,
        commands: &[ProjectServiceCommand],
    ) -> Result<(), AzDaemonError> {
        let runtime = self.project_runtime(&project.project_id);
        let runtime = runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let launcher = &runtime.launcher;
        let lifecycle = &runtime.lifecycle;
        for command in commands {
            let key = project_service_process_key(command)?;
            let Some(process) = manifest.current_process(&key).cloned() else {
                continue;
            };
            if matches!(
                process.state,
                ServiceProcessState::Starting | ServiceProcessState::Running
            ) {
                let graceful_exit = if process.state == ServiceProcessState::Running {
                    request_project_service_graceful_exit(
                        project, manifest, &process, &key, launcher, lifecycle,
                    )
                } else {
                    None
                };
                let exit_code = if let Some(exit) = graceful_exit {
                    exit.exit_code
                } else {
                    force_project_service_exit(project, &process, &key, launcher, lifecycle)?
                };
                manifest
                    .current_process_mut(&key)
                    .expect("current process was cloned from this manifest")
                    .mark_exited(
                        exit_code,
                        Some("retired before project-service replanning".to_string()),
                        u128::from(current_unix_ms()),
                    );
                store.write(manifest)?;
            }
            remove_stale_project_service_unix_endpoint(&process)?;
        }
        Ok(())
    }

    // `launcher` and `lifecycle` borrow out of the guard for the whole body, so
    // clippy's suggested early `drop(runtime)` would not compile; the lock also
    // has to span every start wave to serialize it against a concurrent retire
    // of the same project's services.
    #[allow(clippy::significant_drop_tightening)]
    fn start_project_services(
        &self,
        project: &ProjectRecord,
        requested: &[String],
        ready_timeout: Duration,
    ) -> Result<project_services::ProjectServiceManifest, AzDaemonError> {
        let store = self.project_service_store(project)?;
        let runtime = self.project_runtime(&project.project_id);
        let runtime = runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let launcher = &runtime.launcher;
        let lifecycle = &runtime.lifecycle;
        let mut manifest = store.load_or_create(u128::from(current_unix_ms()))?;
        let mut started = Vec::new();

        for wave in project_service_start_waves(&manifest.processes, requested) {
            let mut pending = Vec::new();
            for mut process in wave {
                let key = ServiceProcessKey::from_process(&process);
                if !prepare_project_service_for_start(
                    project,
                    &store,
                    &mut manifest,
                    launcher,
                    &mut process,
                    &key,
                )? {
                    continue;
                }
                let spawned = match launcher.spawn(&process) {
                    Ok(spawned) => spawned,
                    Err(error) => {
                        mark_project_process_failed(
                            &store,
                            &mut manifest,
                            &key,
                            format!("spawn failed: {error}"),
                        )?;
                        rollback_project_service_starts(
                            &store,
                            launcher,
                            lifecycle,
                            &mut manifest,
                            &started,
                            "project service startup rolled back after spawn failure",
                        )?;
                        return Err(error.into());
                    }
                };
                started.push(spawned.clone());
                bind_project_service_exit_or_rollback(
                    &store,
                    launcher,
                    lifecycle,
                    &mut manifest,
                    &started,
                    &process,
                    &spawned,
                )?;
                pending.push(PendingProjectService { process, spawned });
            }
            wait_for_project_service_wave(
                &ProjectServiceWave {
                    project_id: &project.project_id,
                    store: &store,
                    launcher,
                    lifecycle,
                    ready_reader: &FilesystemProjectServiceReadyReader,
                    pending: &pending,
                    started: &started,
                    ready_timeout,
                },
                &mut manifest,
            )?;
        }
        Ok(manifest)
    }

    /// Build and record the named project-instance services for one session.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_project_services_selected`] returns while
    /// planning, [`AzDaemonError::ServiceBuildSpawn`] or
    /// [`AzDaemonError::ServiceBuildFailed`] when a planned cargo command
    /// cannot be spawned or exits non-zero,
    /// [`AzDaemonError::ProjectServiceCleanupRefused`] or
    /// [`AzDaemonError::ProjectServiceProcess`] when a recorded process from a
    /// previous run cannot be retired, [`AzDaemonError::ProjectServices`] when
    /// the project service manifest cannot be loaded or committed, and
    /// [`AzDaemonError::ProjectSession`] when the session store rejects the
    /// attached descriptors or the planned runtime-host launch.
    pub fn prepare_project_session_services_selected(
        &self,
        request: &ProjectSessionServicesRequest<'_>,
    ) -> Result<ProjectSessionServicesResult, AzDaemonError> {
        self.prepare_project_session_services_inner(request, None)
    }

    fn prepare_project_session_services_inner(
        &self,
        request: &ProjectSessionServicesRequest<'_>,
        build_progress: Option<&az_work::Progress>,
    ) -> Result<ProjectSessionServicesResult, AzDaemonError> {
        let &ProjectSessionServicesRequest {
            project_id,
            session_slug,
            endpoint_kind,
            skip_build,
            service_names: selected_service_names,
            otlp_endpoint,
            recover,
        } = request;
        if session_slug.trim().is_empty() {
            return Err(AzDaemonError::InvalidSessionSlug);
        }
        validate_public_endpoint_kind(endpoint_kind, "azd project session service preparation")?;

        let project =
            self.resolve_project(project_id)
                .ok_or_else(|| AzDaemonError::UnknownProject {
                    project_id: project_id.to_string(),
                })?;
        let manager = self.session_manager(PathBuf::from(&project.root))?;
        if manager.project_id() != project_id {
            return Err(AzDaemonError::ProjectSessionProjectMismatch {
                requested_project_id: project_id.to_string(),
                actual_project_id: manager.project_id().to_string(),
                project_root: project.root,
            });
        }
        let session = manager.session(session_slug)?;
        if session.state != az_session::SessionState::Active
            && !(recover && session.state == az_session::SessionState::FailedPreserved)
        {
            let state = match session.state {
                az_session::SessionState::Preparing => "preparing",
                az_session::SessionState::Active => "active",
                az_session::SessionState::FailedPreserved => "failed-preserved",
                az_session::SessionState::Removed => "removed",
            };
            return Err(az_session::SessionError::SessionNotActive {
                session: session.slug,
                state: state.to_string(),
                operation: "prepare project services",
            }
            .into());
        }
        let mut plan = self.plan_project_services_selected(
            project_id,
            &session.slug,
            endpoint_kind,
            Some(&session.workspace_root),
            selected_service_names,
        )?;

        if !skip_build {
            // Coalesce the per-target build commands into one cargo invocation
            // per (cwd, target-dir, program) group so we drive a SINGLE cargo
            // process (which parallelizes internally) and parse ONE progress
            // stream instead of contending on the shared target dir.
            for command in coalesce_build_commands(&plan.build_commands) {
                run_build_command_with_progress(&command, build_progress)?;
            }
        }
        apply_session_endpoint_layout(&mut plan.commands, &session, endpoint_kind)?;
        validate_planned_service_owner_context(&plan.commands)?;

        let service_names = plan
            .commands
            .iter()
            .map(|command| command.service_name.clone())
            .collect::<Vec<_>>();
        let project_service_manifest = self.record_project_service_plan(
            &project,
            &plan.commands,
            otlp_endpoint,
            if skip_build {
                ServiceProgramFreshnessPolicy::TrustPrebuilt
            } else {
                ServiceProgramFreshnessPolicy::Verify
            },
        )?;
        let manifest = attach_and_record_session_services(
            &manager,
            &session.slug,
            &project_service_manifest.services,
            &plan.commands,
            otlp_endpoint,
        )?;

        Ok(ProjectSessionServicesResult {
            manifest: az_session::session_manifest_to_proto(&manifest),
            service_names,
            build_command_count: count_u32(plan.build_commands.len()),
            prepared_process_count: count_u32(plan.commands.len()),
            built: !skip_build,
        })
    }

    /// Reuse the recorded launch plans, or rebuild and re-record the services
    /// whose plans went stale.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::prepare_project_session_services_selected`]
    /// returns, plus [`AzDaemonError::ProjectSession`] when the refreshed
    /// session manifest cannot be read back.
    fn prepare_or_reuse_session_services(
        &self,
        prepare: &EnsurePrepare<'_>,
    ) -> Result<ProjectSessionServicesResult, AzDaemonError> {
        let build_command_count = count_u32(prepare.plan.build_commands.len());
        let prepared_process_count = count_u32(prepare.plan.commands.len());
        if prepare.project_plan_reusable && prepare.session_plan_reusable {
            info!(
                project_id = prepare.project_id,
                session = %prepare.session_slug,
                services = ?prepare.requested_running_service_names,
                "reusing project-instance and session service launch plans without rebuild"
            );
            prepare
                .open_progress
                .phase(OpenProjectPhase::Build)
                .finish();
            return Ok(ProjectSessionServicesResult {
                manifest: az_session::session_manifest_to_proto(prepare.manifest),
                service_names: prepare.planned_service_names.clone(),
                build_command_count,
                prepared_process_count,
                built: false,
            });
        }

        let mut services_to_prepare = Vec::new();
        if !prepare.project_plan_reusable {
            services_to_prepare.extend(prepare.requested_project_services.iter().cloned());
        }
        if !prepare.session_plan_reusable {
            services_to_prepare.extend(prepare.requested_session_services.iter().cloned());
        }
        let build_phase = prepare.open_progress.phase(OpenProjectPhase::Build);
        self.prepare_project_session_services_inner(
            &ProjectSessionServicesRequest {
                project_id: prepare.project_id,
                session_slug: prepare.session_slug,
                endpoint_kind: prepare.endpoint_kind,
                skip_build: prepare.skip_build,
                service_names: &services_to_prepare,
                otlp_endpoint: None,
                recover: false,
            },
            Some(build_phase),
        )?;
        build_phase.finish();
        Ok(ProjectSessionServicesResult {
            manifest: az_session::session_manifest_to_proto(
                &prepare.manager.session(prepare.session_slug)?,
            ),
            service_names: prepare.planned_service_names.clone(),
            build_command_count,
            prepared_process_count,
            built: !prepare.skip_build,
        })
    }

    /// Attach to a reachable session supervisor, or launch `az-sessiond` and
    /// wait for it to register one.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::SessionSupervisorRpc`] when a reachable
    /// supervisor rejects or fails the start request,
    /// [`AzDaemonError::SessiondSpawn`], [`AzDaemonError::SessiondLogOpen`], or
    /// [`AzDaemonError::SessiondLogRotate`] when the supervisor process cannot
    /// be launched, [`AzDaemonError::SessiondExited`] when it exits before
    /// registering, and [`AzDaemonError::SessionServicesStartTimedOut`] when it
    /// never registers within the deadline.
    fn start_or_reuse_session_supervisor(
        &self,
        existing: Option<ReachableSessionSupervisor>,
        launch: &SessionSupervisorLaunch<'_>,
    ) -> Result<SessionSupervisorStart, AzDaemonError> {
        let Some(reachable) = existing else {
            let command = daemon_registered_sessiond_launch_command(
                Path::new(&launch.project.root),
                &launch.manifest.slug,
                launch.endpoint_kind,
                launch.daemon_endpoint,
                true,
                launch.timeout_ms,
                launch.requested_session_services,
            )?;
            let mut child = spawn_sessiond_process(&command, launch.manifest)?;
            let descriptor = wait_for_session_supervisor_start(
                self,
                &launch.project.project_id,
                &launch.manifest.slug,
                launch.manager,
                &mut child,
                &command,
                launch.timeout_ms,
                launch.log_path,
            )?;
            return Ok(SessionSupervisorStart {
                descriptor,
                reused: false,
                start_requested: true,
                sessiond_pid: child.id(),
                terminal_manifest: None,
            });
        };

        if session_services_are_running(&reachable.manifest, launch.requested_session_services) {
            return Ok(SessionSupervisorStart {
                descriptor: reachable.descriptor,
                reused: true,
                start_requested: false,
                sessiond_pid: 0,
                terminal_manifest: Some(reachable.manifest),
            });
        }

        let result = request_session_service_start(
            launch.manifest,
            &reachable.descriptor,
            "azd ensure project session services",
            launch.requested_session_services.to_vec(),
        )?;
        validate_terminal_start_status_for_session(
            &launch.manifest.slug,
            &result,
            "azd ensure project session services",
        )?;
        Ok(SessionSupervisorStart {
            descriptor: reachable.descriptor,
            reused: true,
            start_requested: !result.started.is_empty(),
            sessiond_pid: 0,
            terminal_manifest: Some(result.status.manifest),
        })
    }

    /// Bring one session's project services all the way to running, reporting
    /// open-project phase progress as each stage completes.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::InvalidServicePlan`] when `timeout_ms` is zero
    /// or `daemon_endpoint` has an empty address,
    /// [`AzDaemonError::UnsupportedEndpointKind`] when `endpoint_kind` or the
    /// daemon endpoint kind is not a public transport,
    /// [`AzDaemonError::UnknownProject`] when `project_id` is not registered,
    /// plus any error [`Self::ensure_project_session`],
    /// [`Self::plan_project_services`], and
    /// [`Self::prepare_project_session_services_selected`] return. Starting the
    /// session supervisor can additionally return
    /// [`AzDaemonError::SessiondSpawn`], [`AzDaemonError::SessiondLogOpen`],
    /// [`AzDaemonError::SessiondLogRotate`], [`AzDaemonError::SessiondExited`],
    /// [`AzDaemonError::SessionServicesStartTimedOut`],
    /// [`AzDaemonError::SessionServiceNotRunning`],
    /// [`AzDaemonError::SessionSupervisorRpc`], or
    /// [`AzDaemonError::SessionSupervisorShutdownTimedOut`].
    #[allow(clippy::too_many_arguments)]
    #[instrument(
        skip(self, start_service_names, daemon_endpoint, open_progress),
        fields(
            project_id,
            session = session_name,
            services = ?start_service_names,
            endpoint_kind = ?endpoint_kind
        )
    )]
    pub fn ensure_project_session_services_with_progress(
        &self,
        project_id: &str,
        session_name: &str,
        endpoint_kind: EndpointKind,
        skip_build: bool,
        start_service_names: &[String],
        timeout_ms: u64,
        daemon_endpoint: &Endpoint,
        open_progress: &OpenProgress,
    ) -> Result<ProjectSessionServicesStartResult, AzDaemonError> {
        let ensure_started = Instant::now();
        validate_ensure_services_request(session_name, endpoint_kind, daemon_endpoint, timeout_ms)?;
        let program_freshness = if skip_build {
            ServiceProgramFreshnessPolicy::TrustPrebuilt
        } else {
            ServiceProgramFreshnessPolicy::Verify
        };

        let session_resolve_started = Instant::now();
        let project =
            self.resolve_project(project_id)
                .ok_or_else(|| AzDaemonError::UnknownProject {
                    project_id: project_id.to_string(),
                })?;
        let manager = self.session_manager(PathBuf::from(&project.root))?;
        let session = Self::ensure_project_session_with_manager(
            project_id,
            session_name,
            &project,
            &manager,
        )?;
        let session_slug = session.manifest.slug.clone();
        let manifest = manager.session(&session_slug)?;
        let session_resolve_ms = duration_millis_u64(session_resolve_started.elapsed());

        // Session reachability is probed independently from project-instance
        // service state. The session supervisor can own only RuntimeHost;
        // project services are resolved from the project service store below.
        let supervisor_probe_started = Instant::now();
        let existing_supervisor = self.existing_reachable_session_supervisor_snapshot(&manifest)?;
        let supervisor_probe_ms = duration_millis_u64(supervisor_probe_started.elapsed());
        let freshness_started = Instant::now();

        let plan_started = Instant::now();
        let (plan, planned_service_names, requested_running_service_names) = self
            .plan_ensure_session_services(
                project_id,
                &session_slug,
                endpoint_kind,
                &manifest,
                start_service_names,
            )?;
        // Resolve + plan are done.
        open_progress.phase(OpenProjectPhase::ResolvePlan).finish();
        let plan_ms = duration_millis_u64(plan_started.elapsed());

        let LaunchPlanFreshness {
            requested_project_services,
            requested_session_services,
            project_plan_reusable,
            session_plan_reusable,
        } = self.launch_plan_freshness(
            &project,
            &manifest,
            &plan,
            &requested_running_service_names,
            program_freshness,
        )?;
        let freshness_ms = duration_millis_u64(freshness_started.elapsed());

        if !session_plan_reusable && let Some(reachable) = existing_supervisor.as_ref() {
            self.shutdown_session_supervisor_before_prepare(
                &manifest,
                &reachable.descriptor,
                reachable.process,
            )?;
        }
        let build_prepare_started = Instant::now();
        let prepared = self.prepare_or_reuse_session_services(&EnsurePrepare {
            project_id,
            session_slug: &session_slug,
            endpoint_kind,
            skip_build,
            manager: &manager,
            manifest: &manifest,
            plan: &plan,
            planned_service_names,
            requested_project_services: &requested_project_services,
            requested_session_services: &requested_session_services,
            requested_running_service_names: &requested_running_service_names,
            project_plan_reusable,
            session_plan_reusable,
            open_progress,
        })?;
        let build_prepare_ms = duration_millis_u64(build_prepare_started.elapsed());

        self.start_ensured_session_services(EnsureStart {
            project: &project,
            project_id,
            session_slug: &session_slug,
            manager: &manager,
            endpoint_kind,
            daemon_endpoint,
            timeout_ms,
            requested_project_services: &requested_project_services,
            requested_session_services: &requested_session_services,
            requested_running_service_names: &requested_running_service_names,
            open_progress,
            session_created: session.created,
            prepared,
            timing: EnsureTiming {
                ensure_started,
                session_resolve_ms,
                supervisor_probe_ms,
                freshness_ms,
                plan_ms,
                build_prepare_ms,
            },
        })
    }

    /// Plan this session's services, apply its endpoint layout, and narrow the
    /// plan to the services the caller asked to run.
    ///
    /// Returns the narrowed plan, its service names, and the requested service
    /// names resolved against it.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_project_services`] returns,
    /// [`AzDaemonError::SessionServiceEndpointLayout`] when the session's
    /// endpoint directory cannot be prepared,
    /// [`AzDaemonError::InvalidServicePlan`] when a planned command has no
    /// owner context, and [`AzDaemonError::InvalidServicePlan`] when a
    /// requested service is not in the plan.
    fn plan_ensure_session_services(
        &self,
        project_id: &str,
        session_slug: &str,
        endpoint_kind: EndpointKind,
        manifest: &az_session::SessionManifest,
        start_service_names: &[String],
    ) -> Result<(ProjectServicePlan, Vec<String>, Vec<String>), AzDaemonError> {
        let mut plan = self.plan_project_services(
            project_id,
            session_slug,
            endpoint_kind,
            Some(&manifest.workspace_root),
        )?;
        apply_session_endpoint_layout(&mut plan.commands, manifest, endpoint_kind)?;
        validate_planned_service_owner_context(&plan.commands)?;
        let planned_service_names = project_service_names(&plan);
        let requested_running_service_names =
            requested_service_names(start_service_names, &planned_service_names)?;
        retain_project_service_plan_services(&mut plan, &requested_running_service_names)?;
        let planned_service_names = project_service_names(&plan);
        Ok((plan, planned_service_names, requested_running_service_names))
    }

    /// Which of this pass's recorded launch plans can still be reused.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::DataHome`] or [`AzDaemonError::ProjectServices`]
    /// when the project service store cannot be opened or its manifest cannot
    /// be loaded.
    fn launch_plan_freshness(
        &self,
        project: &ProjectRecord,
        manifest: &az_session::SessionManifest,
        plan: &ProjectServicePlan,
        requested_running_service_names: &[String],
        program_freshness: ServiceProgramFreshnessPolicy,
    ) -> Result<LaunchPlanFreshness, AzDaemonError> {
        let requested_project_services = requested_running_service_names
            .iter()
            .filter(|service| service.as_str() != az_proto_runtime::RUNTIME_HOST_SERVICE_NAME)
            .cloned()
            .collect::<Vec<_>>();
        let requested_session_services = requested_running_service_names
            .iter()
            .filter(|service| service.as_str() == az_proto_runtime::RUNTIME_HOST_SERVICE_NAME)
            .cloned()
            .collect::<Vec<_>>();
        let project_commands = plan
            .commands
            .iter()
            .filter(|command| command.role != ServiceRole::RuntimeHost)
            .cloned()
            .collect::<Vec<_>>();
        let project_store = self.project_service_store(project)?;
        let project_manifest = project_store.load_or_create(u128::from(current_unix_ms()))?;
        let project_plan_reusable = project_services_have_reusable_launch_plan(
            &project_manifest,
            &project_commands,
            program_freshness,
        );
        let session_plan_reusable = requested_session_services.is_empty()
            || session_services_have_persisted_reusable_launch_plan(
                manifest,
                &requested_session_services,
                &plan.commands,
                program_freshness,
            );
        Ok(LaunchPlanFreshness {
            requested_project_services,
            requested_session_services,
            project_plan_reusable,
            session_plan_reusable,
        })
    }

    /// Start the project-instance services, then the session supervisor, and
    /// wait until every requested service reports running.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::start_project_services`] and
    /// [`Self::start_or_reuse_session_supervisor`] return, plus
    /// [`AzDaemonError::ProjectSession`] when the refreshed session manifest or
    /// its attached descriptors cannot be written,
    /// [`AzDaemonError::SessionServicesStartTimedOut`] when the supervisor's
    /// services never reach running, and
    /// [`AzDaemonError::SessionServiceNotRunning`] when one of them settles in
    /// a non-running state.
    fn start_ensured_session_services(
        &self,
        start: EnsureStart<'_>,
    ) -> Result<ProjectSessionServicesStartResult, AzDaemonError> {
        let project_services = self.start_project_services(
            start.project,
            start.requested_project_services,
            Duration::from_millis(start.timeout_ms),
        )?;
        start.manager.attach_project_service_descriptors(
            start.session_slug,
            &project_services.running_descriptors(),
        )?;
        let manifest = start.manager.session(start.session_slug)?;
        let log_path = az_session::sessiond_output_log_path(&manifest);

        let start_phase = start.open_progress.phase(OpenProjectPhase::StartServices);
        start_phase.set_total(progress_units(start.requested_running_service_names.len()));
        start_phase.message("starting services");

        let sessiond_start_started = Instant::now();
        let existing_supervisor = self.existing_reachable_session_supervisor_snapshot(&manifest)?;
        let SessionSupervisorStart {
            descriptor: supervisor,
            reused: reused_supervisor,
            start_requested,
            sessiond_pid,
            terminal_manifest,
        } = self.start_or_reuse_session_supervisor(
            existing_supervisor,
            &SessionSupervisorLaunch {
                project: start.project,
                manager: start.manager,
                manifest: &manifest,
                endpoint_kind: start.endpoint_kind,
                daemon_endpoint: start.daemon_endpoint,
                requested_session_services: start.requested_session_services,
                timeout_ms: start.timeout_ms,
                log_path: &log_path,
            },
        )?;
        let sessiond_start_ms = duration_millis_u64(sessiond_start_started.elapsed());

        let services_ready_started = Instant::now();
        let supervisor_manifest = match terminal_manifest {
            Some(manifest) => manifest,
            None => wait_for_session_services_running(
                &manifest,
                &supervisor,
                start.requested_session_services,
                start.timeout_ms,
                &log_path,
            )?,
        };
        if let Some(blocker) = first_unready_session_service(
            &supervisor_manifest,
            start.requested_session_services,
            None,
        ) {
            return Err(AzDaemonError::SessionServiceNotRunning {
                session: supervisor_manifest.slug.clone(),
                service: blocker.service,
                state: blocker.state,
            });
        }
        let all_running_service_names = running_service_names_after_start(
            &project_services,
            start.requested_project_services,
            &supervisor_manifest,
            start.requested_session_services,
        );
        start_phase.advance(progress_units(all_running_service_names.len()));
        start_phase.finish();
        log_ensure_services_timing(
            start.project_id,
            start.session_slug,
            if start.prepared.built {
                "build-and-start"
            } else {
                "persisted-plan-start"
            },
            start.timing.ensure_started,
            start.timing.session_resolve_ms,
            start.timing.supervisor_probe_ms,
            start.timing.freshness_ms,
            start.timing.plan_ms,
            start.timing.build_prepare_ms,
            sessiond_start_ms,
            duration_millis_u64(services_ready_started.elapsed()),
        );

        Ok(ProjectSessionServicesStartResult {
            manifest: supervisor_manifest,
            service_names: start.prepared.service_names,
            running_service_names: all_running_service_names,
            build_command_count: start.prepared.build_command_count,
            prepared_process_count: start.prepared.prepared_process_count,
            built: start.prepared.built,
            session_created: start.session_created,
            supervisor,
            reused_supervisor,
            start_requested,
            sessiond_pid,
            log_path: log_path.to_string_lossy().into_owned(),
        })
    }
}

/// Which recorded launch plans an ensure pass can reuse, and the requested
/// services split by who owns them.
struct LaunchPlanFreshness {
    /// Requested services owned by the project instance.
    requested_project_services: Vec<String>,
    /// Requested services owned by the session supervisor.
    requested_session_services: Vec<String>,
    project_plan_reusable: bool,
    session_plan_reusable: bool,
}

/// Stage timings accumulated before the start phase, in milliseconds.
struct EnsureTiming {
    ensure_started: Instant,
    session_resolve_ms: u64,
    supervisor_probe_ms: u64,
    freshness_ms: u64,
    plan_ms: u64,
    build_prepare_ms: u64,
}

/// Everything [`AzDaemon::start_ensured_session_services`] needs once the
/// launch plans are prepared.
struct EnsureStart<'a> {
    project: &'a ProjectRecord,
    project_id: &'a str,
    session_slug: &'a str,
    manager: &'a az_session::SessionManager,
    endpoint_kind: EndpointKind,
    daemon_endpoint: &'a Endpoint,
    timeout_ms: u64,
    requested_project_services: &'a [String],
    requested_session_services: &'a [String],
    requested_running_service_names: &'a [String],
    open_progress: &'a OpenProgress,
    session_created: bool,
    prepared: ProjectSessionServicesResult,
    timing: EnsureTiming,
}

/// A service count as progress units, never zero so the bar has a denominator.
fn progress_units(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX).max(1)
}

/// One ensure pass's decision inputs for the build-and-record stage.
struct EnsurePrepare<'a> {
    project_id: &'a str,
    session_slug: &'a str,
    endpoint_kind: EndpointKind,
    skip_build: bool,
    manager: &'a az_session::SessionManager,
    manifest: &'a az_session::SessionManifest,
    plan: &'a ProjectServicePlan,
    planned_service_names: Vec<String>,
    requested_project_services: &'a [String],
    requested_session_services: &'a [String],
    requested_running_service_names: &'a [String],
    /// True when the recorded project-service launch plan can be reused as-is.
    project_plan_reusable: bool,
    /// True when the recorded session-service launch plan can be reused as-is.
    session_plan_reusable: bool,
    open_progress: &'a OpenProgress,
}

/// Reject an ensure request that can never produce running services.
///
/// # Errors
///
/// Returns [`AzDaemonError::InvalidServicePlan`] when `timeout_ms` is zero or
/// `daemon_endpoint` has a blank address, and
/// [`AzDaemonError::UnsupportedEndpointKind`] when either endpoint kind is not
/// a public transport.
fn validate_ensure_services_request(
    session_name: &str,
    endpoint_kind: EndpointKind,
    daemon_endpoint: &Endpoint,
    timeout_ms: u64,
) -> Result<(), AzDaemonError> {
    if timeout_ms == 0 {
        return Err(AzDaemonError::InvalidServicePlan {
            service: session_name.to_string(),
            reason: "service startup timeout must be positive".to_string(),
        });
    }
    validate_public_endpoint_kind(endpoint_kind, "azd project session service startup")?;
    validate_public_endpoint_kind(daemon_endpoint.kind, "azd sessiond registration endpoint")?;
    if daemon_endpoint.address.trim().is_empty() {
        return Err(AzDaemonError::InvalidServicePlan {
            service: session_name.to_string(),
            reason: "daemon endpoint address cannot be empty".to_string(),
        });
    }
    Ok(())
}

/// The requested services that are actually running, project-instance services
/// first and session-owned services after them.
fn running_service_names_after_start(
    project_services: &project_services::ProjectServiceManifest,
    requested_project_services: &[String],
    supervisor_manifest: &ProtoSessionManifest,
    requested_session_services: &[String],
) -> Vec<String> {
    let mut running = project_services
        .processes
        .iter()
        .filter(|process| {
            process.state == ServiceProcessState::Running
                && requested_project_services.contains(&process.service_name)
        })
        .map(|process| process.service_name.clone())
        .collect::<Vec<_>>();
    running.extend(running_session_service_names(
        supervisor_manifest,
        requested_session_services,
    ));
    running
}

/// What [`AzDaemon::start_or_reuse_session_supervisor`] needs to launch a
/// session supervisor when no reachable one exists.
struct SessionSupervisorLaunch<'a> {
    project: &'a ProjectRecord,
    manager: &'a az_session::SessionManager,
    manifest: &'a az_session::SessionManifest,
    endpoint_kind: EndpointKind,
    daemon_endpoint: &'a Endpoint,
    requested_session_services: &'a [String],
    timeout_ms: u64,
    log_path: &'a Path,
}

/// The session supervisor this ensure pass ended up talking to.
struct SessionSupervisorStart {
    descriptor: ServiceDescriptor,
    /// True when an already-running supervisor was reused.
    reused: bool,
    /// True when this pass asked the supervisor to start services.
    start_requested: bool,
    /// The `az-sessiond` process id, or `0` when a supervisor was reused.
    sessiond_pid: u32,
    /// The supervisor manifest already known to be terminal, when the start
    /// path produced one and no readiness wait is needed.
    terminal_manifest: Option<ProtoSessionManifest>,
}

fn require_project_process_gone(
    project: &ProjectRecord,
    process: &ServiceProcessRecord,
    cleanup: RecordedServiceProcessCleanup,
) -> Result<RecordedServiceProcessCleanup, AzDaemonError> {
    if cleanup.proves_recorded_process_gone() {
        Ok(cleanup)
    } else {
        Err(AzDaemonError::ProjectServiceCleanupRefused {
            project_id: project.project_id.clone(),
            service: process.service_name.clone(),
            cleanup,
        })
    }
}

/// Remove a stable Unix endpoint only from the project supervisor, after the
/// caller has established that the prior recorded owner is terminal.
fn remove_stale_project_service_unix_endpoint(
    process: &ServiceProcessRecord,
) -> Result<(), AzDaemonError> {
    if process.endpoint_kind != az_service_supervision::ServiceEndpointKind::UnixDomainSocket {
        return Ok(());
    }
    match fs::remove_file(&process.endpoint_address) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Remove the session supervisor's stable Unix endpoint only after the daemon
/// has established that the recorded process identity no longer owns it.
fn remove_stale_session_supervisor_unix_endpoint(
    descriptor: &ServiceDescriptor,
) -> Result<(), AzDaemonError> {
    if descriptor.endpoint.kind != EndpointKind::UnixDomainSocket {
        return Ok(());
    }
    match fs::remove_file(&descriptor.endpoint.address) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn log_ensure_services_timing(
    project_id: &str,
    session: &str,
    path: &str,
    started: Instant,
    session_resolve_ms: u64,
    supervisor_probe_ms: u64,
    freshness_ms: u64,
    plan_ms: u64,
    build_prepare_ms: u64,
    sessiond_start_ms: u64,
    services_ready_ms: u64,
) {
    let total_ms = duration_millis_u64(started.elapsed());
    info!(
        project_id,
        session,
        path,
        total_ms,
        session_resolve_ms,
        supervisor_probe_ms,
        freshness_ms,
        plan_ms,
        build_prepare_ms,
        sessiond_start_ms,
        services_ready_ms,
        timing_table = %format!(
            "stage                 ms\nsession resolve    {session_resolve_ms:>6}\nsupervisor probe   {supervisor_probe_ms:>6}\nfreshness          {freshness_ms:>6}\nservice plan       {plan_ms:>6}\nbuild+prepare      {build_prepare_ms:>6}\nsessiond start     {sessiond_start_ms:>6}\nservices ready     {services_ready_ms:>6}\ntotal              {total_ms:>6}"
        ),
        "project service ensure timing summary"
    );
}

#[derive(Debug, Error)]
pub enum AzDaemonError {
    #[error("project manifest error: {0}")]
    ProjectManifest(#[from] ProjectManifestError),

    #[error(transparent)]
    ProjectServices(#[from] project_services::ProjectServiceError),

    #[error("project service protocol encoding failed: {0}")]
    ProjectServiceProtocol(#[from] capnp::Error),

    #[error("project service process operation failed: {0}")]
    ProjectServiceProcess(#[from] az_service_supervision::ServiceProcessError),

    #[error("failed to rotate az-sessiond launch logs: {0}")]
    SessiondLogRotate(#[source] az_service_supervision::ServiceProcessError),

    #[error(
        "project `{project_id}` cannot replace service `{service}` because recorded-process cleanup was refused: {cleanup:?}"
    )]
    ProjectServiceCleanupRefused {
        project_id: String,
        service: String,
        cleanup: RecordedServiceProcessCleanup,
    },

    #[error("project service `{service}` did not publish readiness within {timeout_ms}ms")]
    ProjectServiceReadyTimeout { service: String, timeout_ms: u64 },

    #[error(transparent)]
    RuntimeFileStaging(#[from] az_project::RuntimeFileStagingError),

    #[error("machine-local Azoth data-home error: {0}")]
    DataHome(#[from] az_filesystem::DataHomeError),

    // Boxed: `DaemonEndpointRecordError` is 128 bytes on its own, which made
    // every `Result<_, AzDaemonError>` in this crate an oversized `Err`.
    #[error(transparent)]
    EndpointDiscovery(Box<az_endpoint_discovery::DaemonEndpointRecordError>),

    #[error(transparent)]
    HostTool(#[from] az_filesystem::HostToolError),

    #[error("invalid daemon capability: {reason}")]
    InvalidCapability { reason: String },

    #[error("project id cannot be empty")]
    MissingProjectId,

    #[error("project name cannot be empty")]
    MissingProjectName,

    #[error("project root cannot be empty")]
    MissingProjectRoot,

    #[error("project manifest path cannot be empty")]
    MissingProjectManifestPath,

    #[error("project record `{project_id}` is invalid: {reason}")]
    InvalidProjectRecord { project_id: String, reason: String },

    #[error("project path `{path}` cannot be canonicalized: {source}")]
    ProjectPathCanonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("project `{project_id}` is not registered")]
    UnknownProject { project_id: String },

    #[error("session slug cannot be empty")]
    InvalidSessionSlug,

    #[error("invalid editor lease: {reason}")]
    InvalidEditorLease { reason: String },

    #[error("project session error: {0}")]
    ProjectSession(#[from] az_session::SessionError),

    #[error(transparent)]
    SessionSupervisorLease(#[from] az_session::SessionSupervisorLeaseError),

    #[error("failed to read the identity of process {process_id}: {source}")]
    ProcessIdentity {
        process_id: u32,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "project session root `{project_root}` belongs to project `{actual_project_id}`, expected `{requested_project_id}`"
    )]
    ProjectSessionProjectMismatch {
        requested_project_id: String,
        actual_project_id: String,
        project_root: String,
    },

    #[error("session-supervisor descriptor must use role SessionSupervisor, got {role:?}")]
    InvalidSessionSupervisorRole { role: ServiceRole },

    #[error("session-supervisor descriptor is invalid: {reason}")]
    InvalidSessionSupervisorDescriptor { reason: String },

    #[error("project `{project_id}` has no default build targets")]
    NoBuildTargets { project_id: String },

    #[error(
        "build package selector `{selector}` for project `{project_id}` is {reason}; candidates: {candidates}"
    )]
    InvalidBuildPackageSelector {
        project_id: String,
        selector: String,
        reason: String,
        candidates: String,
    },

    #[error("project `{project_id}` has no default service targets")]
    NoServiceTargets { project_id: String },

    #[error(
        "service target `{service_name}` is declared by both `{first_owner}` and `{second_owner}`"
    )]
    DuplicateServiceTargetName {
        service_name: String,
        first_owner: String,
        second_owner: String,
    },

    #[error("planned service `{service}` is invalid: {reason}")]
    InvalidServicePlan { service: String, reason: String },

    #[error("session service endpoint layout failed: {0}")]
    SessionServiceEndpointLayout(#[from] std::io::Error),

    #[error("service build command `{program}` failed to spawn: {source}")]
    ServiceBuildSpawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("service build command `{program} {args}` failed with status {status:?}{diagnostics}")]
    ServiceBuildFailed {
        program: String,
        args: String,
        status: Option<i32>,
        diagnostics: String,
    },

    #[error("az-sessiond command `{program}` failed to spawn: {source}")]
    SessiondSpawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("az-sessiond log `{path}` could not be opened: {source}")]
    SessiondLogOpen {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("az-sessiond command `{program} {args}` exited early with status {status:?}")]
    SessiondExited {
        program: String,
        args: String,
        cwd: PathBuf,
        status: Option<i32>,
    },

    #[error(
        "session `{session}` services did not become running within {timeout_ms}ms; log: {log_path}"
    )]
    SessionServicesStartTimedOut {
        session: String,
        timeout_ms: u64,
        log_path: PathBuf,
    },

    #[error(
        "session `{session}` service `{service}` is {state}; project services require it to be running"
    )]
    SessionServiceNotRunning {
        session: String,
        service: String,
        state: String,
    },

    #[error("session-supervisor RPC `{operation}` failed for session `{session}`: {reason}")]
    SessionSupervisorRpc {
        session: String,
        operation: &'static str,
        reason: String,
    },

    #[error(
        "session `{session}` supervisor run {run} did not stop within {timeout_ms}ms before project-service rebuild"
    )]
    SessionSupervisorShutdownTimedOut {
        session: String,
        run: Uuid,
        timeout_ms: u64,
    },

    #[error(
        "project service workspace root `{workspace_root}` belongs to project `{found_project_id}`, expected `{project_id}`"
    )]
    ProjectServiceWorkspaceMismatch {
        project_id: String,
        found_project_id: String,
        workspace_root: String,
    },

    #[error("project service workspace root `{workspace_root}` is invalid: {reason}")]
    InvalidProjectServiceWorkspaceRoot {
        workspace_root: String,
        reason: String,
    },

    #[error(
        "{operation} cannot use endpoint kind {kind:?}; use platform IPC or explicit TCP debug endpoints"
    )]
    UnsupportedEndpointKind {
        operation: &'static str,
        kind: EndpointKind,
    },

    #[error("unsupported build profile `{0}`")]
    InvalidBuildProfile(String),

    #[error("build target name cannot be empty")]
    MissingBuildTargetName,

    #[error("cargo metadata failed for project root {root}: status {status:?}; stderr: {stderr}")]
    CargoMetadataFailed {
        root: PathBuf,
        status: Option<i32>,
        stderr: String,
    },

    #[error("failed to run cargo metadata for project root {root}: {source}")]
    CargoMetadataIo {
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse cargo metadata for project root {root}: {source}")]
    CargoMetadataParse {
        root: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to read azd project registry {path}: {source}")]
    ProjectRegistryRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write azd project registry {path}: {source}")]
    ProjectRegistryWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse azd project registry {path}: {source}")]
    ProjectRegistryParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to encode azd project registry: {0}")]
    ProjectRegistryEncode(#[from] toml::ser::Error),

    #[error("failed to recover azd project registry transactions under {path}: {source}")]
    ProjectRegistryTransactionRecovery {
        path: PathBuf,
        #[source]
        source: FileTransactionError,
    },

    #[error("failed to commit azd project registry transaction for {path}: {source}")]
    ProjectRegistryTransaction {
        path: PathBuf,
        #[source]
        source: FileTransactionError,
    },

    #[error("azd project registry {path} has schema version `{actual}`, expected `{expected}`")]
    UnsupportedProjectRegistrySchema {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
}

// `EndpointDiscovery` carries a boxed payload, so `#[from]` would have derived
// `From<Box<DaemonEndpointRecordError>>` and broken `?` on the unboxed error.
impl From<az_endpoint_discovery::DaemonEndpointRecordError> for AzDaemonError {
    fn from(source: az_endpoint_discovery::DaemonEndpointRecordError) -> Self {
        Self::EndpointDiscovery(Box::new(source))
    }
}

/// Daemon failures cross the capnp boundary as a `failed` disconnect reason;
/// capnp carries only a message, so the chain is flattened into `Display`.
impl From<AzDaemonError> for Error {
    fn from(error: AzDaemonError) -> Self {
        Self::failed(format!("azd failed: {error}"))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ProjectRegistryFile {
    schema_version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    projects: Vec<ProjectRecord>,
}

const PROJECT_REGISTRY_SCHEMA_VERSION: u32 = 1;

fn read_project_registry(path: &Path) -> Result<BTreeMap<String, ProjectRecord>, AzDaemonError> {
    recover_project_registry_transactions(path)?;
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(source) => {
            return Err(AzDaemonError::ProjectRegistryRead {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let file: ProjectRegistryFile =
        toml::from_str(&contents).map_err(|source| AzDaemonError::ProjectRegistryParse {
            path: path.to_path_buf(),
            source,
        })?;
    if file.schema_version != PROJECT_REGISTRY_SCHEMA_VERSION {
        return Err(AzDaemonError::UnsupportedProjectRegistrySchema {
            path: path.to_path_buf(),
            expected: PROJECT_REGISTRY_SCHEMA_VERSION,
            actual: file.schema_version,
        });
    }

    let mut projects = BTreeMap::new();
    for project in file.projects {
        if let Err(error) = validate_project_record(&project) {
            warn!(
                path = %path.display(),
                project_id = %project.project_id,
                error = ?error,
                "dropping invalid azd project registry record"
            );
            continue;
        }
        projects.insert(project.project_id.clone(), project);
    }
    Ok(projects)
}

/// Forget one project registration, returning the record that was removed.
///
/// The registry only ever grew: [`register_project_root`] had no inverse, so a
/// project whose manifest was deleted kept its row forever. Those rows are not
/// inert — resolving a registration whose `azoth.toml` is gone fails, and every
/// sweep over the registry has to tolerate that.
///
/// Returns `Ok(None)` when no registration carries `project_id`, so forgetting
/// something already absent is success rather than an error.
///
/// # Errors
///
/// Returns any error [`read_project_registry`] returns while loading the
/// registry, plus any error [`write_project_registry`] returns while committing
/// the shortened registry.
pub fn forget_project_registration(
    path: &Path,
    project_id: &str,
) -> Result<Option<ProjectRecord>, AzDaemonError> {
    let mut projects = read_project_registry(path)?;
    let Some(removed) = projects.remove(project_id) else {
        return Ok(None);
    };
    write_project_registry(path, &projects)?;
    Ok(Some(removed))
}

fn write_project_registry(
    path: &Path,
    projects: &BTreeMap<String, ProjectRecord>,
) -> Result<(), AzDaemonError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| AzDaemonError::ProjectRegistryWrite {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let file = ProjectRegistryFile {
        schema_version: PROJECT_REGISTRY_SCHEMA_VERSION,
        projects: projects.values().cloned().collect(),
    };
    let contents = toml::to_string_pretty(&file)?;
    recover_project_registry_transactions(path)?;
    FileTransaction::new(project_registry_transaction_root(path))
        .commit([FileWrite::new(path, contents.into_bytes())])
        .map(|_| ())
        .map_err(|source| AzDaemonError::ProjectRegistryTransaction {
            path: path.to_path_buf(),
            source,
        })
}

fn project_registry_transaction_root(path: &Path) -> PathBuf {
    // The machine-global registry shares the data-home runtime directory with
    // endpoint-discovery's global record, so both intentionally use the same
    // control-record transaction root and its cross-process lock.
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(
            || PathBuf::from("control-record-transactions"),
            |parent| parent.join("control-record-transactions"),
        )
}

fn recover_project_registry_transactions(path: &Path) -> Result<(), AzDaemonError> {
    let transaction_root = project_registry_transaction_root(path);
    FileTransaction::new(&transaction_root)
        .recover_pending()
        .map(|_| ())
        .map_err(|source| AzDaemonError::ProjectRegistryTransactionRecovery {
            path: transaction_root,
            source,
        })
}

fn default_project_registry_path() -> PathBuf {
    AzothDataHome::resolve().daemon_project_registry_path()
}

pub struct AzDaemonRpc {
    daemon: AzDaemon,
    shutdown: az_work::CancellationToken,
    started_at: Instant,
    run: Uuid,
}

struct CancelOnDrop(az_work::CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl fmt::Debug for AzDaemonRpc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzDaemonRpc")
            .field("daemon", &self.daemon)
            .field("shutdown_requested", &self.shutdown_requested())
            .field("uptime_ms", &duration_millis_u64(self.started_at.elapsed()))
            // `shutdown` is reported as `shutdown_requested`; `run` identifies
            // the listener, not this RPC facade.
            .finish_non_exhaustive()
    }
}

impl AzDaemonRpc {
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn new(daemon: AzDaemon) -> Self {
        Self::with_shutdown(daemon, az_work::CancellationToken::new(), Uuid::now_v7())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn test_new(daemon: AzDaemon) -> Self {
        Self::new(daemon)
    }

    #[must_use]
    pub(crate) fn with_shutdown(
        daemon: AzDaemon,
        shutdown: az_work::CancellationToken,
        run: Uuid,
    ) -> Self {
        Self {
            daemon,
            shutdown,
            started_at: Instant::now(),
            run,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub const fn daemon(&self) -> &AzDaemon {
        &self.daemon
    }

    #[must_use]
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown.is_cancelled()
    }

    #[must_use]
    pub(crate) fn into_client(self) -> daemon_capnp::az_daemon::Client {
        capnp_rpc::new_client(self)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn client_from_rc(this: &Rc<Self>) -> daemon_capnp::az_daemon::Client {
        capnp_rpc::new_client_from_rc(Rc::clone(this))
    }

    #[must_use]
    fn health_snapshot(&self) -> ServiceHealth {
        let state = if self.shutdown_requested() {
            ServiceHealthState::Stopping
        } else {
            ServiceHealthState::Ready
        };
        let message = if self.shutdown_requested() {
            "azd shutdown requested"
        } else {
            "azd reachable"
        };
        ServiceHealth::ready(
            ServiceId::new(
                DAEMON_SESSION_SERVICE_NAMESPACE,
                DAEMON_SESSION_SERVICE_NAME,
            ),
            ServiceRole::Daemon,
            self.run,
            az_proto_core::ProtocolVersion::CURRENT,
        )
        .with_state(state)
        .with_uptime_ms(duration_millis_u64(self.started_at.elapsed()))
        .with_active_operation(if self.shutdown_requested() {
            "shutdown"
        } else {
            ""
        })
        .with_message(message)
    }
}

// Every method below is a capnp-rpc server handler. capnp-rpc keeps its
// connection state behind `Rc<RefCell<..>>` — the `Rc<Self>` receiver and the
// `Params`/`Results` hooks are `!Send` by construction — so these futures can
// never be `Send`, and the dispatcher only ever polls them on the connection's
// own `LocalSet`.
#[allow(clippy::future_not_send)]
impl daemon_capnp::az_daemon::Server for AzDaemonRpc {
    async fn health(
        self: capnp::capability::Rc<Self>,
        _params: daemon_capnp::az_daemon::HealthParams,
        mut results: daemon_capnp::az_daemon::HealthResults,
    ) -> Result<(), Error> {
        (self.health_snapshot()).to_capnp(results.get().init_health())?;
        Ok(())
    }

    async fn register_project(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::RegisterProjectParams,
        mut results: daemon_capnp::az_daemon::RegisterProjectResults,
    ) -> Result<(), Error> {
        let request = RegisterProjectRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_PROJECTS_PERMISSION)
            .map_err(Error::from)?;
        let project = self
            .daemon
            .register_project(&request.project)
            .map_err(Error::from)?;
        (project).to_capnp(results.get().init_project());
        Ok(())
    }

    async fn list_projects(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::ListProjectsParams,
        mut results: daemon_capnp::az_daemon::ListProjectsResults,
    ) -> Result<(), Error> {
        let request = ListProjectsRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_READ_PERMISSION).map_err(Error::from)?;
        (ListProjectsResult {
            projects: self.daemon.list_projects(),
            protocol_version: az_proto_core::ProtocolVersion::CURRENT,
        })
        .to_capnp(results.get().init_result());
        Ok(())
    }

    async fn resolve_project(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::ResolveProjectParams,
        mut results: daemon_capnp::az_daemon::ResolveProjectResults,
    ) -> Result<(), Error> {
        let request = ResolveProjectRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_READ_PERMISSION).map_err(Error::from)?;
        (ProjectResult {
            project: self.daemon.resolve_project(&request.project_id),
        })
        .to_capnp(results.get().init_result());
        Ok(())
    }

    async fn register_session_supervisor(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::RegisterSessionSupervisorParams,
        mut results: daemon_capnp::az_daemon::RegisterSessionSupervisorResults,
    ) -> Result<(), Error> {
        let request = RegisterSessionSupervisorRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_SESSIONS_PERMISSION)
            .map_err(Error::from)?;
        let descriptor = self
            .daemon
            .register_session_supervisor(
                &request.project_id,
                &request.session_slug,
                &request.descriptor,
            )
            .map_err(Error::from)?;
        az_proto_core::ServiceDescriptor::to_capnp(&descriptor, results.get().init_descriptor())?;
        Ok(())
    }

    async fn unregister_session_supervisor(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::UnregisterSessionSupervisorParams,
        mut results: daemon_capnp::az_daemon::UnregisterSessionSupervisorResults,
    ) -> Result<(), Error> {
        let request = UnregisterSessionSupervisorRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_SESSIONS_PERMISSION)
            .map_err(Error::from)?;
        let removed = self
            .daemon
            .unregister_session_supervisor(
                &request.project_id,
                &request.session_slug,
                &request.descriptor,
            )
            .map_err(Error::from)?;
        (UnregisterSessionSupervisorResult { removed }).to_capnp(results.get().init_result());
        Ok(())
    }

    async fn resolve_session_supervisor(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::ResolveSessionSupervisorParams,
        mut results: daemon_capnp::az_daemon::ResolveSessionSupervisorResults,
    ) -> Result<(), Error> {
        let request = ResolveSessionSupervisorRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_READ_PERMISSION).map_err(Error::from)?;
        (SessionSupervisorResult {
            descriptor: self
                .daemon
                .resolve_session_supervisor(&request.project_id, &request.session_slug),
        })
        .to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn list_session_supervisors(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::ListSessionSupervisorsParams,
        mut results: daemon_capnp::az_daemon::ListSessionSupervisorsResults,
    ) -> Result<(), Error> {
        let request = ListSessionSupervisorsRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_READ_PERMISSION).map_err(Error::from)?;
        (ListSessionSupervisorsResult {
            supervisors: self.daemon.list_session_supervisors(&request.project_id),
        })
        .to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn ensure_project_session(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::EnsureProjectSessionParams,
        mut results: daemon_capnp::az_daemon::EnsureProjectSessionResults,
    ) -> Result<(), Error> {
        let request = EnsureProjectSessionRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_SESSIONS_PERMISSION)
            .map_err(Error::from)?;
        let result = self
            .daemon
            .ensure_project_session(&request.project_id, &request.session_name)
            .map_err(Error::from)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn prepare_project_session_services(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::PrepareProjectSessionServicesParams,
        mut results: daemon_capnp::az_daemon::PrepareProjectSessionServicesResults,
    ) -> Result<(), Error> {
        let request =
            PrepareProjectSessionServicesRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_SESSIONS_PERMISSION)
            .map_err(Error::from)?;
        let result = self
            .daemon
            .prepare_project_session_services_selected(&ProjectSessionServicesRequest {
                project_id: &request.project_id,
                session_slug: &request.session_slug,
                endpoint_kind: request.endpoint_kind,
                skip_build: request.skip_build,
                service_names: &request.service_names,
                otlp_endpoint: request.otlp_endpoint.as_deref(),
                recover: request.recover,
            })
            .map_err(Error::from)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn ensure_project_session_services_with_progress(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::EnsureProjectSessionServicesWithProgressParams,
        mut results: daemon_capnp::az_daemon::EnsureProjectSessionServicesWithProgressResults,
    ) -> Result<(), Error> {
        let outer = params.get()?.get_request()?;
        let request = EnsureProjectSessionServicesRequest::from_capnp(outer.get_request()?)?;
        let sink_client = outer.get_progress_sink()?;
        validate_capability(&request.capability, DAEMON_SESSIONS_PERMISSION)
            .map_err(Error::from)?;

        // The blocking open work (cargo build, service start, readiness
        // waits) runs on a blocking thread so it never stalls the capnp
        // event loop. Progress events stream back over an unbounded channel
        // to a drain task on THIS (LocalSet) thread, which is the only place
        // the editor's sink capability can be driven.
        let (tx, mut rx) =
            tokio::sync::mpsc::unbounded_channel::<az_proto_daemon::ProjectOpenProgressEvent>();
        let registry = Arc::new(Mutex::new(PhaseRegistry::new()));
        let sink = Arc::new(CapnpProgressSink::new(
            Arc::clone(&registry),
            move |event| {
                // A closed receiver (editor dropped the sink) is non-fatal.
                let _ = tx.send(event);
            },
        ));
        let reporter = az_work::Reporter::new(sink);
        let open_progress = OpenProgress::new(&reporter, &registry);

        let _progress_drain = tokio::task::spawn_local(async move {
            while let Some(event) = rx.recv().await {
                let mut update = sink_client.update_request();
                (event).to_capnp(update.get().init_event());
                let promise = update.send().promise;
                tokio::task::spawn_local(async move {
                    let _ = promise.await;
                });
            }
        });

        let daemon = self.daemon.clone();
        let result = tokio::task::spawn_blocking(move || {
            daemon.ensure_project_session_services_with_progress(
                &request.project_id,
                &request.session_name,
                request.endpoint_kind,
                request.skip_build,
                &request.start_service_names,
                request.timeout_ms,
                &request.daemon_endpoint,
                &open_progress,
            )
        })
        .await
        .map_err(|join| Error::failed(format!("project open task panicked: {join}")))
        .and_then(|result| result.map_err(Error::from));

        // Dropping the reporter closes the event channel after the blocking
        // work drops its phase tree. The detached drain forwards queued
        // progress best-effort, but the authoritative result must never wait
        // on progress callback lifetime or transport health.
        drop(reporter);

        let result = result?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn plan_project_build(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::PlanProjectBuildParams,
        mut results: daemon_capnp::az_daemon::PlanProjectBuildResults,
    ) -> Result<(), Error> {
        let request = PlanProjectBuildRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_PROJECTS_PERMISSION)
            .map_err(Error::from)?;
        let plan = self
            .daemon
            .plan_project_build_selected(
                &request.project_id,
                &request.profile,
                request.target_triple.as_deref(),
                &request.package_selectors,
            )
            .map_err(Error::from)?;
        (plan).to_capnp(results.get().init_plan())?;
        Ok(())
    }

    async fn execute_project_build(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::ExecuteProjectBuildParams,
        mut results: daemon_capnp::az_daemon::ExecuteProjectBuildResults,
    ) -> Result<(), Error> {
        let outer = params.get()?.get_request()?;
        let request = ExecuteProjectBuildRequest::from_capnp(outer.get_request()?)?;
        let sink_client = outer.get_progress_sink()?;
        validate_capability(&request.capability, DAEMON_PROJECTS_PERMISSION)
            .map_err(Error::from)?;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProjectBuildProgressEvent>();
        let registry = Arc::new(Mutex::new(ProjectBuildPhaseRegistry::new()));
        let sink = Arc::new(CapnpProjectBuildProgressSink::new(
            Arc::clone(&registry),
            move |event| {
                let _ = tx.send(event);
            },
        ));
        let reporter = az_work::Reporter::new(sink);
        let cancel = az_work::CancellationToken::new();
        let _cancel_on_drop = CancelOnDrop(cancel.clone());

        let sink_cancel = cancel.clone();
        let _progress_drain = tokio::task::spawn_local(async move {
            while let Some(event) = rx.recv().await {
                let mut update = sink_client.update_request();
                (event).to_capnp(update.get().init_event())?;
                let promise = update.send().promise;
                let cancel = sink_cancel.clone();
                tokio::task::spawn_local(async move {
                    if promise.await.is_err() {
                        cancel.cancel();
                    }
                });
            }
            Ok::<(), Error>(())
        });

        let ExecuteProjectBuildRequest {
            project_id,
            profile,
            target_triple,
            package_selectors,
            ..
        } = request;
        let daemon = self.daemon.clone();
        let result = tokio::task::spawn_blocking(move || {
            daemon.execute_project_build_selected(
                &ProjectBuildSelection {
                    project_id: &project_id,
                    profile: &profile,
                    target_triple: target_triple.as_deref(),
                    package_selectors: &package_selectors,
                },
                &reporter,
                &registry,
                &cancel,
            )
        })
        .await
        .map_err(|join| Error::failed(format!("project build task panicked: {join}")))
        .and_then(|result| result.map_err(Error::from));

        let result = result?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }

    async fn plan_project_services(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::PlanProjectServicesParams,
        mut results: daemon_capnp::az_daemon::PlanProjectServicesResults,
    ) -> Result<(), Error> {
        let request = PlanProjectServicesRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_PROJECTS_PERMISSION)
            .map_err(Error::from)?;
        let plan = self
            .daemon
            .plan_project_services_selected(
                &request.project_id,
                &request.session_slug,
                request.endpoint_kind,
                request.workspace_root.as_deref().map(Path::new),
                &request.service_names,
            )
            .map_err(Error::from)?;
        (plan).to_capnp(results.get().init_plan())?;
        Ok(())
    }

    async fn register_project_root(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::RegisterProjectRootParams,
        mut results: daemon_capnp::az_daemon::RegisterProjectRootResults,
    ) -> Result<(), Error> {
        let request = RegisterProjectRootRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_PROJECTS_PERMISSION)
            .map_err(Error::from)?;
        let project = self
            .daemon
            .register_project_root(&request.root)
            .map_err(Error::from)?;
        (project).to_capnp(results.get().init_project());
        Ok(())
    }

    async fn shutdown(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::ShutdownParams,
        mut results: daemon_capnp::az_daemon::ShutdownResults,
    ) -> Result<(), Error> {
        let request = ShutdownDaemonRequest::from_capnp(params.get()?.get_request()?)?;
        validate_capability(&request.capability, DAEMON_CONTROL_PERMISSION).map_err(Error::from)?;
        info!(reason = %request.reason, "azd shutdown requested");
        (ShutdownDaemonResult {
            accepted: true,
            reason: request.reason,
        })
        .to_capnp(results.get().init_result());
        self.shutdown.cancel();
        Ok(())
    }

    async fn touch_editor_lease(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::az_daemon::TouchEditorLeaseParams,
        mut results: daemon_capnp::az_daemon::TouchEditorLeaseResults,
    ) -> Result<(), Error> {
        let request = TouchEditorLeaseRequest::from_capnp(params.get()?.get_request()?)?;
        let result = self
            .daemon
            .touch_editor_lease(&request)
            .map_err(Error::from)?;
        (result).to_capnp(results.get().init_result())?;
        Ok(())
    }
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Narrow a collection length onto a `u32` telemetry/wire counter.
///
/// Saturating is the honest behaviour here: these counters describe plan and
/// lease sizes that are orders of magnitude below `u32::MAX`, and a wrapped
/// count would be worse than a clamped one.
fn count_u32(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn validate_project_record(project: &ProjectRecord) -> Result<(), AzDaemonError> {
    if project.project_id.trim().is_empty() {
        return Err(AzDaemonError::MissingProjectId);
    }
    if project.name.trim().is_empty() {
        return Err(AzDaemonError::MissingProjectName);
    }
    if project.root.trim().is_empty() {
        return Err(AzDaemonError::MissingProjectRoot);
    }
    if project.manifest_path.trim().is_empty() {
        return Err(AzDaemonError::MissingProjectManifestPath);
    }
    let root = Path::new(&project.root);
    if !root.is_absolute() {
        return Err(AzDaemonError::InvalidProjectRecord {
            project_id: project.project_id.clone(),
            reason: format!("root `{}` is not absolute", project.root),
        });
    }
    let manifest_path = Path::new(&project.manifest_path);
    if !manifest_path.is_absolute() {
        return Err(AzDaemonError::InvalidProjectRecord {
            project_id: project.project_id.clone(),
            reason: format!("manifest_path `{}` is not absolute", project.manifest_path),
        });
    }
    let expected_manifest_path = project_manifest_path(root);
    if !same_path(manifest_path, &expected_manifest_path) {
        return Err(AzDaemonError::InvalidProjectRecord {
            project_id: project.project_id.clone(),
            reason: format!(
                "manifest_path `{}` must match `{}`",
                project.manifest_path,
                expected_manifest_path.display()
            ),
        });
    }
    Ok(())
}

fn normalize_existing_path(path: &Path) -> Result<PathBuf, AzDaemonError> {
    canonical(path).map_err(|source| AzDaemonError::ProjectPathCanonicalize {
        path: path.to_path_buf(),
        source,
    })
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

fn validate_session_supervisor_descriptor(
    descriptor: &ServiceDescriptor,
    endpoint_operation: &'static str,
) -> Result<(), AzDaemonError> {
    descriptor
        .require_protocol_version(ProtocolVersion::CURRENT)
        .map_err(|error| {
            invalid_session_supervisor_descriptor(format!(
                "session-supervisor unavailable until restarted: {error}"
            ))
        })?;
    if descriptor.role != ServiceRole::SessionSupervisor {
        return Err(AzDaemonError::InvalidSessionSupervisorRole {
            role: descriptor.role,
        });
    }

    let expected = ServiceId::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
    );
    if descriptor.id != expected {
        return Err(invalid_session_supervisor_descriptor(format!(
            "expected canonical service id `{}/{}`, got `{}/{}`",
            expected.namespace, expected.name, descriptor.id.namespace, descriptor.id.name
        )));
    }

    validate_public_endpoint_kind(descriptor.endpoint.kind, endpoint_operation)?;

    descriptor
        .validate_brokered_capability_templates()
        .map_err(|error| {
            invalid_session_supervisor_descriptor(format!(
                "descriptor capability templates are invalid: {error}"
            ))
        })?;

    for capability in &descriptor.capabilities {
        validate_session_supervisor_capability_shape(capability)?;
    }

    let daemon_capability = required_session_supervisor_capability(
        descriptor,
        &ServiceId::new(
            DAEMON_SESSION_SERVICE_NAMESPACE,
            DAEMON_SESSION_SERVICE_NAME,
        ),
        ServiceRole::Daemon,
        &[SESSION_READ_PERMISSION, SESSION_MANAGE_PERMISSION],
        "daemon",
    )?;
    let editor_capability = required_session_supervisor_capability(
        descriptor,
        &ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
        ServiceRole::Editor,
        &[
            SESSION_READ_PERMISSION,
            SESSION_SAVE_PERMISSION,
            SESSION_EXEC_PERMISSION,
            SESSION_MANAGE_PERMISSION,
        ],
        "editor",
    )?;
    if daemon_capability.session != editor_capability.session {
        return Err(invalid_session_supervisor_descriptor(
            "editor and daemon capability templates must be scoped to the same session",
        ));
    }

    Ok(())
}

fn required_session_supervisor_capability(
    descriptor: &ServiceDescriptor,
    service: &ServiceId,
    role: ServiceRole,
    required_permissions: &[&str],
    caller: &'static str,
) -> Result<Capability, AzDaemonError> {
    descriptor
        .capabilities
        .iter()
        .find(|capability| {
            capability.service == *service
                && capability.matches_brokered_template_request(
                    role,
                    SESSION_SUPERVISOR_AUDIENCE,
                    required_permissions,
                    None,
                )
        })
        .cloned()
        .ok_or_else(|| {
            invalid_session_supervisor_descriptor(format!(
                "descriptor must grant `{}` capability `{}` to `{}/{}`",
                caller,
                required_permissions.join(", "),
                service.namespace,
                service.name
            ))
        })
}

fn validate_session_supervisor_capability_shape(
    capability: &Capability,
) -> Result<(), AzDaemonError> {
    if !is_session_supervisor_descriptor_caller(capability) {
        return Err(invalid_session_supervisor_descriptor(format!(
            "capability for `{}/{}` with role {:?} is not a valid session-supervisor caller",
            capability.service.namespace, capability.service.name, capability.role
        )));
    }
    if capability.audience != SESSION_SUPERVISOR_AUDIENCE {
        return Err(invalid_session_supervisor_descriptor(format!(
            "capability for `{}/{}` must target audience `{}`, got `{}`",
            capability.service.namespace,
            capability.service.name,
            SESSION_SUPERVISOR_AUDIENCE,
            capability.audience
        )));
    }
    if capability.session.is_none() {
        return Err(invalid_session_supervisor_descriptor(format!(
            "capability for `{}/{}` must be scoped to a session",
            capability.service.namespace, capability.service.name
        )));
    }
    Ok(())
}

fn is_session_supervisor_descriptor_caller(capability: &Capability) -> bool {
    matches!(
        (&capability.service.namespace, &capability.service.name, capability.role),
        (namespace, name, ServiceRole::Editor)
            if namespace == EDITOR_SERVICE_NAMESPACE && name == EDITOR_SERVICE_NAME
    ) || matches!(
        (&capability.service.namespace, &capability.service.name, capability.role),
        (namespace, name, ServiceRole::Daemon)
            if namespace == DAEMON_SESSION_SERVICE_NAMESPACE && name == DAEMON_SESSION_SERVICE_NAME
    )
}

fn invalid_session_supervisor_descriptor(reason: impl Into<String>) -> AzDaemonError {
    AzDaemonError::InvalidSessionSupervisorDescriptor {
        reason: reason.into(),
    }
}

fn remove_editor_leases_for_identity(
    state: &mut AzDaemonState,
    identity: ProcessIdentity,
) -> usize {
    let before = state.editor_leases.len();
    state.editor_leases.retain(|lease_id, lease| {
        let keep = lease.owner_process != identity;
        if !keep {
            info!(
                lease_id = %lease_id,
                owner_process_id = lease.owner_process.process_id,
                owner_process_start_time = lease.owner_process.process_start_time,
                purpose = %lease.purpose,
                touched_unix_ms = lease.touched_unix_ms,
                "azd retired editor lease after exact owner-process exit"
            );
        }
        keep
    });
    before.saturating_sub(state.editor_leases.len())
}

fn validate_editor_lease_owner_process(
    owner_process: ProcessIdentity,
) -> Result<(), AzDaemonError> {
    match owner_process.assess() {
        Ok(RecordedProcess::Live { .. }) => Ok(()),
        Ok(assessment) => Err(AzDaemonError::InvalidEditorLease {
            reason: format!("owner process is not live: {assessment:?}"),
        }),
        Err(error) => Err(AzDaemonError::InvalidEditorLease {
            reason: format!("owner process identity could not be assessed: {error}"),
        }),
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn validate_capability(
    capability: &Capability,
    required_permission: &str,
) -> Result<(), AzDaemonError> {
    if let Err(error) = capability.validate_lifetime() {
        return Err(AzDaemonError::InvalidCapability {
            reason: error.to_string(),
        });
    }

    if capability.audience != DAEMON_AUDIENCE {
        return Err(AzDaemonError::InvalidCapability {
            reason: format!(
                "expected audience `{DAEMON_AUDIENCE}` but got `{}`",
                capability.audience
            ),
        });
    }
    if !matches!(
        capability.role,
        ServiceRole::Editor | ServiceRole::Daemon | ServiceRole::SessionSupervisor
    ) {
        return Err(AzDaemonError::InvalidCapability {
            reason: format!("unsupported role {:?}", capability.role),
        });
    }
    let has_required = capability
        .permissions
        .iter()
        .any(|permission| permission == required_permission);
    let has_projects = capability
        .permissions
        .iter()
        .any(|permission| permission == DAEMON_PROJECTS_PERMISSION);
    let has_sessions = capability
        .permissions
        .iter()
        .any(|permission| permission == DAEMON_SESSIONS_PERMISSION);
    if has_required
        || (required_permission == DAEMON_READ_PERMISSION && (has_projects || has_sessions))
    {
        Ok(())
    } else {
        Err(AzDaemonError::InvalidCapability {
            reason: format!("missing required permission `{required_permission}`"),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildProfile<'a> {
    Debug,
    Release,
    Custom(&'a str),
}

impl<'a> BuildProfile<'a> {
    fn parse(profile: &'a str) -> Result<Self, AzDaemonError> {
        let profile = profile.trim();
        match profile {
            "" => Err(AzDaemonError::InvalidBuildProfile(profile.to_string())),
            "debug" => Ok(Self::Debug),
            "release" => Ok(Self::Release),
            custom
                if custom
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') =>
            {
                Ok(Self::Custom(custom))
            }
            _ => Err(AzDaemonError::InvalidBuildProfile(profile.to_string())),
        }
    }
}

fn resolve_project_build_profile<'a>(
    requested_profile: &'a str,
    graph: &'a ResolvedProjectGraph,
) -> Result<(BuildProfile<'a>, Option<ProjectBuildPackageProfile>), AzDaemonError> {
    let requested_profile = requested_profile.trim();
    let package_profile = graph
        .lock
        .packaging
        .profiles
        .iter()
        .find(|profile| profile.name == requested_profile);
    let cargo_profile =
        package_profile.map_or(requested_profile, |profile| profile.cargo_profile.as_str());
    Ok((
        BuildProfile::parse(cargo_profile)?,
        package_profile.map(project_build_package_profile),
    ))
}

fn project_build_package_profile(profile: &ProjectPackageProfile) -> ProjectBuildPackageProfile {
    ProjectBuildPackageProfile {
        name: profile.name.clone(),
        asset_platform: profile.asset_platform.clone(),
        cargo_profile: profile.cargo_profile.clone(),
        container: package_container_name(profile.container).to_string(),
        compression: package_compression_name(profile.compression).to_string(),
        oodle_compressor: profile
            .oodle
            .as_ref()
            .map(|oodle| oodle_compressor_name(oodle.compressor).to_string()),
        oodle_effort: profile
            .oodle
            .as_ref()
            .map(|oodle| oodle_effort_name(oodle.effort).to_string()),
    }
}

const fn package_container_name(container: ProjectPackageContainer) -> &'static str {
    match container {
        ProjectPackageContainer::Loose => "loose",
        ProjectPackageContainer::AzPack => "azpack",
        ProjectPackageContainer::Pak => "pak",
    }
}

const fn package_compression_name(compression: ProjectPackageCompression) -> &'static str {
    match compression {
        ProjectPackageCompression::None => "none",
        ProjectPackageCompression::Oodle => "oodle",
    }
}

const fn oodle_compressor_name(compressor: ProjectPackageOodleCompressor) -> &'static str {
    match compressor {
        ProjectPackageOodleCompressor::Kraken => "kraken",
        ProjectPackageOodleCompressor::Mermaid => "mermaid",
        ProjectPackageOodleCompressor::Selkie => "selkie",
        ProjectPackageOodleCompressor::Leviathan => "leviathan",
        ProjectPackageOodleCompressor::Hydra => "hydra",
    }
}

const fn oodle_effort_name(effort: ProjectPackageOodleEffort) -> &'static str {
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

#[derive(Debug, Clone, Copy)]
enum SelectedProjectBuildTarget<'a> {
    Generated(&'a GeneratedTargetPackage),
    Authored {
        root: &'a Path,
        owner_id: &'a str,
        target: &'a ProjectBuildTarget,
    },
}

impl SelectedProjectBuildTarget<'_> {
    fn selector_candidate(&self, project_id: &str) -> ProjectBuildSelectorCandidate {
        match self {
            Self::Generated(target) => ProjectBuildSelectorCandidate {
                owner_id: project_id.to_string(),
                target_name: target.name.clone(),
                package_name: target.package.clone(),
            },
            Self::Authored {
                owner_id, target, ..
            } => ProjectBuildSelectorCandidate {
                owner_id: (*owner_id).to_string(),
                target_name: target.name.clone(),
                package_name: target
                    .package
                    .clone()
                    .unwrap_or_else(|| target.name.clone()),
            },
        }
    }

    const fn requires_runtime_products(self) -> bool {
        matches!(self, Self::Generated(_))
    }
}

fn authored_build_targets<'a>(
    project_root: &'a Path,
    graph: &'a ResolvedProjectGraph,
) -> Vec<SelectedProjectBuildTarget<'a>> {
    let mut targets = graph
        .manifest
        .tools
        .build_targets
        .iter()
        .map(|target| SelectedProjectBuildTarget::Authored {
            root: project_root,
            owner_id: graph.manifest.project.id.as_str(),
            target,
        })
        .collect::<Vec<_>>();
    for gem in &graph.gems {
        targets.extend(gem.manifest.tools.build_targets.iter().map(|target| {
            SelectedProjectBuildTarget::Authored {
                root: gem.root.as_path(),
                owner_id: gem.manifest.gem.id.as_str(),
                target,
            }
        }));
    }
    targets
}

fn selected_project_build_targets<'a>(
    project_root: &'a Path,
    graph: &'a ResolvedProjectGraph,
    generated: &'a GeneratedTargetsSyncReport,
    selectors: &[String],
    project_id: &str,
) -> Result<Vec<SelectedProjectBuildTarget<'a>>, AzDaemonError> {
    let primary_gem_project = graph.manifest.project.primary_gem.is_some();
    // Schema 8 generates only the four ship crates; no generated target is a
    // service (ADR 0040 — service hosts arrive prebuilt in Push 2).
    let generated_target_count = if primary_gem_project {
        generated.targets.len()
    } else {
        0
    };
    let mut targets = generated
        .targets
        .iter()
        .filter(|_| primary_gem_project)
        .map(SelectedProjectBuildTarget::Generated)
        .collect::<Vec<_>>();
    targets.extend(authored_build_targets(project_root, graph));

    if selectors.is_empty() {
        if primary_gem_project {
            targets.truncate(generated_target_count);
            return Ok(targets);
        }
        return Ok(targets
            .into_iter()
            .filter(|target| {
                matches!(
                    target,
                    SelectedProjectBuildTarget::Authored { target, .. } if target.default
                )
            })
            .collect());
    }

    let candidates = targets
        .iter()
        .map(|target| target.selector_candidate(&graph.manifest.project.id))
        .collect::<Vec<_>>();
    let selected = resolve_project_build_selector_indices(&candidates, selectors)
        .map_err(|error| invalid_build_selector(project_id, error))?;
    Ok(selected.into_iter().map(|index| targets[index]).collect())
}

fn project_build_plan_from_graph(
    project_root: &Path,
    project_id: &str,
    requested_profile: &str,
    target_triple: Option<&str>,
    selectors: &[String],
    graph: &ResolvedProjectGraph,
    generated: &GeneratedTargetsSyncReport,
) -> Result<ProjectBuildPlan, AzDaemonError> {
    let (profile, package_profile) = resolve_project_build_profile(requested_profile, graph)?;
    let selected =
        selected_project_build_targets(project_root, graph, generated, selectors, project_id)?;
    let requires_runtime_products = graph.manifest.project.primary_gem.is_none()
        || selected
            .iter()
            .copied()
            .any(SelectedProjectBuildTarget::requires_runtime_products);
    let should_process_runtime_assets = requires_runtime_products && package_profile.is_some();
    let mut commands = Vec::with_capacity(selected.len() + 2);
    for target in selected {
        commands.push(match target {
            SelectedProjectBuildTarget::Generated(target) => generated_build_command(
                project_root,
                &graph.manifest.project.id,
                generated,
                target,
                profile,
                target_triple,
            )?,
            SelectedProjectBuildTarget::Authored {
                root,
                owner_id,
                target,
            } => build_command(root, owner_id, target, profile, target_triple)?,
        });
    }
    if should_process_runtime_assets {
        commands.extend(asset_processing_build_commands(
            project_root,
            graph,
            generated,
        )?);
    }
    if commands.is_empty() {
        return Err(AzDaemonError::NoBuildTargets {
            project_id: project_id.to_string(),
        });
    }

    Ok(ProjectBuildPlan {
        commands: coalesce_build_commands(&commands),
        package_profile: if requires_runtime_products {
            package_profile
        } else {
            None
        },
    })
}

fn asset_processing_build_commands(
    project_root: &Path,
    graph: &ResolvedProjectGraph,
    generated: &GeneratedTargetsSyncReport,
) -> Result<Vec<ProjectBuildCommand>, AzDaemonError> {
    // Engine AP host is never built as a project package. Only the project
    // asset-worker (static builder closure) is prepared from the project
    // workspace. Missing engine host binary is a fail-closed launch error.
    if graph.manifest.project.primary_gem.is_some() {
        let context = GeneratedBuildContext::from_report(generated)?;
        return Ok(generated_service_targets()
            .into_iter()
            .filter(|target| matches!(target.role, ProjectServiceRole::AssetWorker))
            .map(|target| {
                generated_service_build_command(
                    project_root,
                    &graph.manifest.project.id,
                    &context,
                    &target,
                )
            })
            .collect());
    }

    let mut commands = Vec::new();
    for target in default_service_targets(&graph.manifest.tools.service_targets)
        .into_iter()
        .filter(|target| matches!(target.role, ProjectServiceRole::AssetWorker))
    {
        commands.push(service_build_command(
            project_root,
            &graph.manifest.project.id,
            target,
        )?);
    }
    for gem in &graph.gems {
        for target in default_service_targets(&gem.manifest.tools.service_targets)
            .into_iter()
            .filter(|target| matches!(target.role, ProjectServiceRole::AssetWorker))
        {
            commands.push(service_build_command(
                &gem.root,
                &gem.manifest.gem.id,
                target,
            )?);
        }
    }
    Ok(commands)
}

fn invalid_build_selector(
    project_id: &str,
    error: az_project::ProjectBuildSelectorError,
) -> AzDaemonError {
    AzDaemonError::InvalidBuildPackageSelector {
        project_id: project_id.to_string(),
        selector: error.selector,
        reason: error.reason.to_string(),
        candidates: error.candidates,
    }
}

fn default_service_targets(targets: &[ProjectServiceTarget]) -> Vec<&ProjectServiceTarget> {
    targets.iter().filter(|target| target.default).collect()
}

fn service_plan_root_and_graph(
    project_id: &str,
    project: &ProjectRecord,
    workspace_root: Option<&Path>,
) -> Result<
    (
        PathBuf,
        az_project::ResolvedProjectGraph,
        Option<GeneratedBuildContext>,
    ),
    AzDaemonError,
> {
    let root = if let Some(root) = workspace_root.filter(|root| !root.as_os_str().is_empty()) {
        if !root.is_absolute() {
            return Err(AzDaemonError::InvalidProjectServiceWorkspaceRoot {
                workspace_root: root.to_string_lossy().into_owned(),
                reason: "workspace root must be absolute".to_string(),
            });
        }
        normalize_existing_path(root)?
    } else {
        PathBuf::from(&project.root)
    };
    let generated = ensure_daemon_generated_targets(&root)?;
    let graph = load_resolved_project_graph(&root)?;
    if graph.manifest.project.id != project_id {
        return Err(AzDaemonError::ProjectServiceWorkspaceMismatch {
            project_id: project_id.to_string(),
            found_project_id: graph.manifest.project.id,
            workspace_root: root.to_string_lossy().into_owned(),
        });
    }
    let generated = graph
        .manifest
        .project
        .primary_gem
        .is_some()
        .then(|| GeneratedBuildContext::from_report(&generated))
        .transpose()?;
    Ok((root, graph, generated))
}

#[derive(Debug, Clone)]
struct GeneratedBuildContext {
    target_directory: PathBuf,
    workspace_root: PathBuf,
}

fn ensure_daemon_generated_targets(
    root: &Path,
) -> Result<GeneratedTargetsSyncReport, AzDaemonError> {
    let report = ensure_project_generated_targets(root)?;
    validate_daemon_generated_targets(root, &report)?;
    Ok(report)
}

fn validate_daemon_generated_targets(
    root: &Path,
    report: &GeneratedTargetsSyncReport,
) -> Result<(), AzDaemonError> {
    validate_project_generated_target_workspaces(root, report)?;
    Ok(())
}

impl GeneratedBuildContext {
    fn from_report(report: &GeneratedTargetsSyncReport) -> Result<Self, AzDaemonError> {
        let workspace_root = report.workspace_root.as_ref().ok_or_else(|| {
            AzDaemonError::ProjectManifest(ProjectManifestError::InvalidGeneratedTargets {
                path: report.target_directory.clone(),
                reason: "generated target report has no package root".to_string(),
            })
        })?;
        Ok(Self {
            target_directory: report.target_directory.clone(),
            workspace_root: workspace_root.clone(),
        })
    }
}

/// Reorder build commands to match the runtime-dependency order of the launch
/// commands.
///
/// Coalesced cargo invocations do not care, but per-target build waves and
/// progress reporting mirror launch order.
fn sort_build_commands_into_launch_order(
    build_commands: &mut [ProjectBuildCommand],
    commands: &[ProjectServiceCommand],
) {
    let launch_order: BTreeMap<&str, usize> = commands
        .iter()
        .enumerate()
        .map(|(index, command)| (command.service_name.as_str(), index))
        .collect();
    build_commands.sort_by_key(|command| {
        launch_order
            .get(command.target_name.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
}

/// The per-service files the daemon plans for one project service.
struct ProjectServiceFiles {
    file_stem: String,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
    structured_log: PathBuf,
    ready_file: PathBuf,
    grants_file: PathBuf,
    observability_grants_file: PathBuf,
    lifecycle_grants_file: PathBuf,
}

impl ProjectServiceFiles {
    fn new(store: &project_services::ProjectServiceStore, service_name: &str) -> Self {
        let file_stem = endpoint_token(service_name);
        Self {
            stdout_log: store.logs_dir().join(format!("{file_stem}.stdout.log")),
            stderr_log: store.logs_dir().join(format!("{file_stem}.stderr.log")),
            structured_log: store.logs_dir().join(format!("{file_stem}.capnp.log")),
            ready_file: store.ready_dir().join(format!("{file_stem}.toml")),
            grants_file: store.grants_dir().join(format!("{file_stem}.capnp")),
            observability_grants_file: store
                .grants_dir()
                .join(format!("{file_stem}.observability.capnp")),
            lifecycle_grants_file: store
                .grants_dir()
                .join(format!("{file_stem}.lifecycle.capnp")),
            file_stem,
        }
    }
}

/// Build the service descriptor for one planned project-service command,
/// including its observability and lifecycle contracts.
///
/// # Errors
///
/// Returns [`AzDaemonError::InvalidServicePlan`] when `command` carries a role
/// the project-instance supervisor cannot own.
fn project_service_descriptor_for_command(
    command: &ProjectServiceCommand,
    run: Uuid,
) -> Result<ServiceDescriptor, AzDaemonError> {
    let endpoint = command.endpoint.clone();
    let mut descriptor = match command.role {
        ServiceRole::ProjectHost => project_host_service_descriptor(run, endpoint),
        ServiceRole::AssetProcessor => asset_processor_service_descriptor(run, endpoint),
        ServiceRole::Worker => asset_worker_service_descriptor(run, endpoint),
        _ => {
            return Err(AzDaemonError::InvalidServicePlan {
                service: command.service_name.clone(),
                reason: format!(
                    "role {:?} cannot be owned by the project-instance supervisor",
                    command.role
                ),
            });
        }
    };
    add_observability_contract(&mut descriptor, None);
    add_service_lifecycle_contract(
        &mut descriptor,
        None,
        ServiceId::new(DAEMON_SERVICE_NAMESPACE, DAEMON_SERVICE_NAME),
        ServiceRole::Daemon,
    );
    Ok(descriptor)
}

/// The capability grants handed to one project service at launch.
///
/// A worker inherits the worker-role grants the asset processor published; every
/// other role carries its own non-control grants, and the project host also
/// receives the asset processor's authoring grant.
///
/// # Errors
///
/// Returns [`AzDaemonError::InvalidServicePlan`] when a worker is planned
/// without the project asset-processor descriptor it must attach to.
///
/// # Panics
///
/// Panics when a project host is planned without the asset-processor descriptor
/// or that descriptor grants no `ProjectHost` authoring; the launch order plans
/// the asset processor first, so both are already established here.
fn project_service_role_grants(
    manifest: &project_services::ProjectServiceManifest,
    command: &ProjectServiceCommand,
    descriptor: &ServiceDescriptor,
) -> Result<Vec<Capability>, AzDaemonError> {
    let mut role_grants: Vec<Capability> = if command.role == ServiceRole::Worker {
        let processor = manifest
            .services
            .iter()
            .find(|service| service.role == SupervisedServiceRole::AssetProcessor)
            .ok_or_else(|| AzDaemonError::InvalidServicePlan {
                service: command.service_name.clone(),
                reason: "asset worker requires the project asset-processor descriptor".to_string(),
            })?;
        processor
            .to_descriptor()
            .capabilities
            .into_iter()
            .filter(|capability| capability.role == ServiceRole::Worker)
            .collect()
    } else {
        descriptor
            .capabilities
            .iter()
            .filter(|capability| {
                !is_observability_control_grant(capability)
                    && !is_service_lifecycle_grant(capability)
            })
            .cloned()
            .collect()
    };
    if command.role == ServiceRole::ProjectHost {
        let processor = manifest
            .service_descriptor(
                &ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
                ServiceRole::AssetProcessor,
            )
            .expect("asset-processor descriptor checked above");
        let capability = processor
            .capabilities
            .iter()
            .find(|capability| {
                capability.role == ServiceRole::ProjectHost
                    && capability.audience == ASSET_PROCESSOR_AUDIENCE
            })
            .cloned()
            .expect("asset-processor descriptor grants ProjectHost authoring");
        role_grants.push(capability);
    }
    Ok(role_grants)
}

/// Add the flags that only one project-service role takes: the asset DB, the
/// worker's staging and cache roots, and the project host's side-channel root,
/// all pointed at the asset processor already planned for this project.
///
/// # Errors
///
/// Returns [`AzDaemonError::UnsupportedEndpointKind`] when the asset
/// processor's endpoint uses a non-public transport, and
/// [`AzDaemonError::SessionServiceEndpointLayout`] when a staging or
/// side-channel directory cannot be created.
///
/// # Panics
///
/// Panics when a worker or project host is planned without the asset-processor
/// descriptor; the launch order plans the asset processor first.
fn apply_role_specific_service_args(
    store: &project_services::ProjectServiceStore,
    manifest: &project_services::ProjectServiceManifest,
    command: &ProjectServiceCommand,
    files: &ProjectServiceFiles,
    args: &mut Vec<String>,
) -> Result<(), AzDaemonError> {
    match command.role {
        ServiceRole::AssetProcessor => {
            set_project_service_arg(args, "--asset-db", &store.asset_db_path().to_string_lossy());
        }
        ServiceRole::Worker => {
            let processor = manifest
                .service_descriptor(
                    &ServiceId::new("azoth", "asset-processor"),
                    ServiceRole::AssetProcessor,
                )
                .expect("asset-processor descriptor checked above");
            set_project_service_arg(
                args,
                "--asset-processor-endpoint-kind",
                endpoint_kind_arg(processor.endpoint.kind)?,
            );
            set_project_service_arg(
                args,
                "--asset-processor-endpoint",
                &processor.endpoint.address,
            );
            let staging_root = store
                .asset_processing_staging_dir()
                .join(endpoint_token(&command.service_name));
            fs::create_dir_all(&staging_root)?;
            set_project_service_arg(args, "--staging-root", &staging_root.to_string_lossy());
            set_project_service_arg(
                args,
                "--cache-root",
                &store.product_cache_dir().to_string_lossy(),
            );
        }
        ServiceRole::ProjectHost => {
            remove_project_service_arg(args, "--asset-db");
            let processor = manifest
                .service_descriptor(
                    &ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
                    ServiceRole::AssetProcessor,
                )
                .expect("asset-processor descriptor checked above");
            set_project_service_arg(
                args,
                "--asset-processor-endpoint-kind",
                endpoint_kind_arg(processor.endpoint.kind)?,
            );
            set_project_service_arg(
                args,
                "--asset-processor-endpoint",
                &processor.endpoint.address,
            );
            let side_channel_root = store
                .side_channels_dir()
                .join(format!("{}-side-channels", files.file_stem));
            fs::create_dir_all(&side_channel_root)?;
            set_project_service_arg(
                args,
                "--side-channel-root",
                &side_channel_root.to_string_lossy(),
            );
        }
        _ => unreachable!("project role validated above"),
    }
    Ok(())
}

/// The launch arguments for one planned project service.
///
/// Starts from the planned command's own arguments and overwrites every
/// daemon-owned flag: endpoints, project identity, capability-grant files,
/// logs, and the role-specific wiring (asset DB, staging and cache roots,
/// side-channel root, asset-processor endpoint).
///
/// # Errors
///
/// Returns [`AzDaemonError::UnsupportedEndpointKind`] when the service, its
/// asset processor, or its lifecycle endpoint uses a non-public transport,
/// [`AzDaemonError::InvalidServicePlan`] when the descriptor has no lifecycle
/// endpoint, and [`AzDaemonError::SessionServiceEndpointLayout`] when a staging
/// or side-channel directory cannot be created.
///
/// # Panics
///
/// Panics when a worker or project host is planned without the asset-processor
/// descriptor; the launch order plans the asset processor first.
fn project_service_launch_args(
    store: &project_services::ProjectServiceStore,
    manifest: &project_services::ProjectServiceManifest,
    project: &ProjectRecord,
    command: &ProjectServiceCommand,
    descriptor: &ServiceDescriptor,
    files: &ProjectServiceFiles,
    otlp_endpoint: Option<&str>,
) -> Result<Vec<String>, AzDaemonError> {
    let endpoint = &command.endpoint;
    let mut args = command.args.clone();
    set_project_service_arg(
        &mut args,
        "--endpoint-kind",
        endpoint_kind_arg(endpoint.kind)?,
    );
    set_project_service_arg(&mut args, "--endpoint", &endpoint.address);
    set_project_service_arg(&mut args, "--project", &project.root);
    set_project_service_arg(&mut args, "--project-id", &project.project_id);
    set_project_service_arg(&mut args, "--workspace-root", &project.root);
    set_project_service_arg(&mut args, "--owner-id", &command.owner_id);
    set_project_service_arg(&mut args, "--owner-root", &command.owner_root);
    remove_project_service_arg(&mut args, "--session");
    remove_project_service_arg(&mut args, "--session-id");
    apply_role_specific_service_args(store, manifest, command, files, &mut args)?;
    set_project_service_arg(&mut args, "--run", &descriptor.run.to_string());
    set_project_service_arg(
        &mut args,
        "--ready-file",
        &files.ready_file.to_string_lossy(),
    );
    set_project_service_arg(
        &mut args,
        "--capability-grants",
        &files.grants_file.to_string_lossy(),
    );
    set_project_service_arg(
        &mut args,
        "--lifecycle-capability-grants",
        &files.lifecycle_grants_file.to_string_lossy(),
    );
    let lifecycle_endpoint = descriptor.lifecycle_endpoint.as_ref().ok_or_else(|| {
        AzDaemonError::InvalidServicePlan {
            service: command.service_name.clone(),
            reason: "service lifecycle endpoint is required".to_string(),
        }
    })?;
    set_project_service_arg(
        &mut args,
        "--lifecycle-endpoint-kind",
        endpoint_kind_arg(lifecycle_endpoint.kind)?,
    );
    set_project_service_arg(
        &mut args,
        "--lifecycle-endpoint",
        &lifecycle_endpoint.address,
    );
    if let Some(endpoint) = descriptor.observability_endpoint.as_ref() {
        set_project_service_arg(
            &mut args,
            "--observability-endpoint-kind",
            endpoint_kind_arg(endpoint.kind)?,
        );
        set_project_service_arg(&mut args, "--observability-endpoint", &endpoint.address);
        set_project_service_arg(
            &mut args,
            "--observability-capability-grants",
            &files.observability_grants_file.to_string_lossy(),
        );
    }
    set_project_service_arg(
        &mut args,
        "--structured-log",
        &files.structured_log.to_string_lossy(),
    );
    if let Some(endpoint) = otlp_endpoint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        set_project_service_arg(&mut args, "--otlp-endpoint", endpoint);
    } else {
        remove_project_service_arg(&mut args, "--otlp-endpoint");
    }
    Ok(args)
}

fn record_service_target_owner(
    service_owners: &mut BTreeMap<String, String>,
    owner_id: &str,
    target: &ProjectServiceTarget,
) -> Result<(), AzDaemonError> {
    match service_owners.entry(target.name.clone()) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(owner_id.to_string());
            Ok(())
        }
        std::collections::btree_map::Entry::Occupied(entry) => {
            Err(AzDaemonError::DuplicateServiceTargetName {
                service_name: target.name.clone(),
                first_owner: entry.get().clone(),
                second_owner: owner_id.to_string(),
            })
        }
    }
}

fn build_command(
    root: &Path,
    owner_id: &str,
    target: &ProjectBuildTarget,
    profile: BuildProfile<'_>,
    target_triple: Option<&str>,
) -> Result<ProjectBuildCommand, AzDaemonError> {
    if target.name.trim().is_empty() {
        return Err(AzDaemonError::MissingBuildTargetName);
    }

    let mut args = cargo_build_args(target_triple);
    match target.kind {
        ProjectBuildTargetKind::Package => {
            let package = target.package.as_deref().ok_or_else(|| {
                ProjectManifestError::MissingBuildPackage {
                    target: target.name.clone(),
                }
            })?;
            args.push("-p".to_string());
            args.push(package.to_string());
        }
        ProjectBuildTargetKind::Bin => {
            if let Some(package) = target.package.as_deref() {
                args.push("-p".to_string());
                args.push(package.to_string());
            }
            args.push("--bin".to_string());
            args.push(target.name.clone());
        }
        ProjectBuildTargetKind::Example => {
            if let Some(package) = target.package.as_deref() {
                args.push("-p".to_string());
                args.push(package.to_string());
            }
            args.push("--example".to_string());
            args.push(target.name.clone());
        }
    }

    match profile {
        BuildProfile::Debug => {}
        BuildProfile::Release => args.push("--release".to_string()),
        BuildProfile::Custom(profile) => {
            args.push("--profile".to_string());
            args.push(profile.to_string());
        }
    }

    if let Some(target_triple) = target_triple.filter(|value| !value.trim().is_empty()) {
        args.push("--target".to_string());
        args.push(target_triple.to_string());
    }

    if !target.features.is_empty() {
        args.push("--features".to_string());
        args.push(target.features.join(","));
    }
    push_locked_if_workspace_lock_exists(&mut args, root);

    Ok(ProjectBuildCommand {
        owner_id: owner_id.to_string(),
        owner_root: root.to_string_lossy().into_owned(),
        target_name: target.name.clone(),
        program: "cargo".to_string(),
        cwd: root.to_string_lossy().into_owned(),
        args,
        cargo_target_dir: None,
    })
}

fn generated_build_command(
    root: &Path,
    owner_id: &str,
    report: &GeneratedTargetsSyncReport,
    target: &GeneratedTargetPackage,
    profile: BuildProfile<'_>,
    target_triple: Option<&str>,
) -> Result<ProjectBuildCommand, AzDaemonError> {
    let context = GeneratedBuildContext::from_report(report)?;
    let role_root = context.workspace_root.join(&target.name);
    let manifest_path = role_root.join("Cargo.toml");
    let mut args = cargo_build_args(target_triple);
    args.extend([
        "--manifest-path".to_string(),
        manifest_path.to_string_lossy().into_owned(),
    ]);
    push_build_profile_args(&mut args, profile);
    if let Some(target_triple) = target_triple.filter(|value| !value.trim().is_empty()) {
        args.push("--target".to_string());
        args.push(target_triple.to_string());
    }
    push_locked_if_workspace_lock_exists(&mut args, &role_root);
    Ok(ProjectBuildCommand {
        owner_id: owner_id.to_string(),
        owner_root: root.to_string_lossy().into_owned(),
        target_name: target.name.clone(),
        program: "cargo".to_string(),
        cwd: role_root.to_string_lossy().into_owned(),
        args,
        cargo_target_dir: Some(context.target_directory.to_string_lossy().into_owned()),
    })
}

fn cargo_build_args(target_triple: Option<&str>) -> Vec<String> {
    let mut args = Vec::with_capacity(2);
    if cfg!(not(target_os = "windows"))
        && target_triple
            .map(str::trim)
            .is_some_and(|target| target.ends_with("-pc-windows-msvc"))
    {
        args.push("xwin".to_string());
    }
    args.push("build".to_string());
    args
}

fn push_build_profile_args(args: &mut Vec<String>, profile: BuildProfile<'_>) {
    match profile {
        BuildProfile::Debug => {}
        BuildProfile::Release => args.push("--release".to_string()),
        BuildProfile::Custom(profile) => {
            args.push("--profile".to_string());
            args.push(profile.to_string());
        }
    }
}

/// Build waves run after Azoth has synchronized their workspace contract. Keep
/// that contract authoritative when Cargo can locate its lockfile, while still
/// allowing legacy/uninitialized projects (which have no lock yet) to create
/// one through their explicit synchronization flow.
fn push_locked_if_workspace_lock_exists(args: &mut Vec<String>, root: &Path) {
    if root
        .ancestors()
        .any(|ancestor| ancestor.join("Cargo.lock").is_file())
    {
        args.push("--locked".to_string());
    }
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    target_directory: PathBuf,
}

fn load_cargo_metadata(root: &Path) -> Result<CargoMetadata, AzDaemonError> {
    load_cargo_metadata_with_args(root, &["metadata", "--format-version", "1", "--no-deps"])
}

fn load_cargo_metadata_with_args(
    root: &Path,
    args: &[&str],
) -> Result<CargoMetadata, AzDaemonError> {
    let mut command = Command::new("cargo");
    command.args(args).current_dir(root);
    let output = az_work::owned_command_output(&mut command).map_err(|source| {
        AzDaemonError::CargoMetadataIo {
            root: root.to_path_buf(),
            source,
        }
    })?;
    if !output.status.success() {
        return Err(AzDaemonError::CargoMetadataFailed {
            root: root.to_path_buf(),
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    serde_json::from_slice(&output.stdout).map_err(|source| AzDaemonError::CargoMetadataParse {
        root: root.to_path_buf(),
        source,
    })
}

fn service_build_command(
    root: &Path,
    owner_id: &str,
    target: &ProjectServiceTarget,
) -> Result<ProjectBuildCommand, AzDaemonError> {
    if target.name.trim().is_empty() {
        return Err(AzDaemonError::MissingBuildTargetName);
    }

    let mut args = vec![
        "build".to_string(),
        "-p".to_string(),
        target.package.clone(),
        "--bin".to_string(),
        target.bin.clone(),
    ];
    if !target.features.is_empty() {
        args.push("--features".to_string());
        args.push(target.features.join(","));
    }
    push_locked_if_workspace_lock_exists(&mut args, root);

    Ok(ProjectBuildCommand {
        owner_id: owner_id.to_string(),
        owner_root: root.to_string_lossy().into_owned(),
        target_name: target.name.clone(),
        program: "cargo".to_string(),
        cwd: root.to_string_lossy().into_owned(),
        args,
        cargo_target_dir: None,
    })
}

fn generated_service_build_command(
    root: &Path,
    owner_id: &str,
    context: &GeneratedBuildContext,
    target: &ProjectServiceTarget,
) -> ProjectBuildCommand {
    let role_root = context.workspace_root.join(&target.name);
    let manifest_path = role_root.join("Cargo.toml");
    let mut args = vec![
        "build".to_string(),
        "--manifest-path".to_string(),
        manifest_path.to_string_lossy().into_owned(),
    ];
    push_locked_if_workspace_lock_exists(&mut args, &role_root);
    ProjectBuildCommand {
        owner_id: owner_id.to_string(),
        owner_root: root.to_string_lossy().into_owned(),
        target_name: target.name.clone(),
        program: "cargo".to_string(),
        cwd: role_root.to_string_lossy().into_owned(),
        args,
        cargo_target_dir: Some(context.target_directory.to_string_lossy().into_owned()),
    }
}

/// Project-generated service targets only. The asset-processor host is an
/// engine host tool and is planned separately via
/// [`engine_asset_processor_service_command`].
fn generated_service_targets() -> [ProjectServiceTarget; 3] {
    [
        ProjectServiceTarget::cargo_bin(
            "project-host",
            ProjectServiceRole::ProjectHost,
            generated_package_name("project-host"),
            "project-host",
        ),
        ProjectServiceTarget::cargo_bin(
            "asset-worker",
            ProjectServiceRole::AssetWorker,
            generated_package_name("asset-worker"),
            "asset-worker",
        ),
        ProjectServiceTarget::cargo_bin(
            "runtime-host",
            ProjectServiceRole::RuntimeHost,
            generated_package_name("runtime-host"),
            "runtime-host",
        ),
    ]
}

fn engine_asset_processor_service_target() -> ProjectServiceTarget {
    ProjectServiceTarget::cargo_bin(
        "asset-processor",
        ProjectServiceRole::AssetProcessor,
        HostTool::AssetProcessor.cargo_package(),
        HostTool::AssetProcessor.cargo_binary(),
    )
}

fn resolve_engine_asset_processor_binary_from_bundle(
    bundle: &HostToolBundle,
) -> Result<(PathBuf, PathBuf), AzDaemonError> {
    let program = bundle
        .resolve(HostTool::AssetProcessor)
        .map_err(|source| AzDaemonError::InvalidServicePlan {
            service: "asset-processor".to_string(),
            reason: format!(
                "engine asset-processor host is missing from the host tool bundle ({source}); build the engine host tools first (never auto-spawning Cargo from the project lease)"
            ),
        })?;
    let owner_root = bundle.directory().to_path_buf();
    Ok((program, owner_root))
}

/// Where one planned project service is rooted and who it belongs to.
///
/// Every service command — engine-owned, generated, or workspace-declared —
/// is derived from this same context: the data home that issues its endpoint,
/// the project workspace it runs from, the root its binary is built under, and
/// the owner/project/session triple it is planned for. The six values are
/// resolved together while walking the project graph and are passed on
/// untouched, so only the service target and the endpoint kind distinguish one
/// planned command from the next.
#[derive(Debug, Clone, Copy)]
struct ServiceSite<'a> {
    /// Azoth data home whose endpoint layout addresses the service.
    data_home: &'a AzothDataHome,
    /// Project workspace root the service process runs from.
    project_root: &'a Path,
    /// Root whose build output holds the service binary: the project root for
    /// generated services, the gem or engine bundle root otherwise.
    binary_root: &'a Path,
    /// Manifest id of the project or gem that owns the service target.
    owner_id: &'a str,
    /// Registered project id the service is planned for.
    project_id: &'a str,
    /// Session slug the service is planned for.
    session_slug: &'a str,
}

fn engine_asset_processor_service_command(
    data_home: &AzothDataHome,
    project_root: &Path,
    owner_id: &str,
    project_id: &str,
    session_slug: &str,
    endpoint_kind: EndpointKind,
) -> Result<ProjectServiceCommand, AzDaemonError> {
    let bundle = HostToolBundle::current().map_err(|source| AzDaemonError::InvalidServicePlan {
        service: "asset-processor".to_string(),
        reason: format!(
            "engine host tool bundle could not be resolved for asset-processor: {source}"
        ),
    })?;
    engine_asset_processor_service_command_from_bundle(
        data_home,
        project_root,
        owner_id,
        project_id,
        session_slug,
        endpoint_kind,
        &bundle,
    )
}

fn engine_asset_processor_service_command_from_bundle(
    data_home: &AzothDataHome,
    project_root: &Path,
    owner_id: &str,
    project_id: &str,
    session_slug: &str,
    endpoint_kind: EndpointKind,
    bundle: &HostToolBundle,
) -> Result<ProjectServiceCommand, AzDaemonError> {
    let (program, owner_root) = resolve_engine_asset_processor_binary_from_bundle(bundle)?;
    let target = engine_asset_processor_service_target();
    let mut command = service_command(
        &ServiceSite {
            data_home,
            project_root,
            binary_root: &owner_root,
            owner_id,
            project_id,
            session_slug,
        },
        &target,
        endpoint_kind,
    )?;
    // service_command would look under owner_root/target/debug; override with
    // the real engine host binary resolved from the host-tool bundle.
    command.program = program.to_string_lossy().into_owned();
    command.build_output_root = owner_root.to_string_lossy().into_owned();
    command.owner_root = owner_root.to_string_lossy().into_owned();
    Ok(command)
}

fn generated_service_command(
    site: &ServiceSite<'_>,
    context: &GeneratedBuildContext,
    target: &ProjectServiceTarget,
    endpoint_kind: EndpointKind,
) -> Result<ProjectServiceCommand, AzDaemonError> {
    let mut command = service_command(site, target, endpoint_kind)?;
    command.build_output_root = context.target_directory.to_string_lossy().into_owned();
    command.program = service_binary_path(&context.target_directory, target)
        .to_string_lossy()
        .into_owned();
    Ok(command)
}

fn service_command(
    site: &ServiceSite<'_>,
    target: &ProjectServiceTarget,
    endpoint_kind: EndpointKind,
) -> Result<ProjectServiceCommand, AzDaemonError> {
    let &ServiceSite {
        data_home,
        project_root,
        binary_root,
        owner_id,
        project_id,
        session_slug,
    } = site;
    let build_output_root = cargo_target_dir_or_default(binary_root);
    let endpoint = service_endpoint(
        data_home,
        project_root,
        project_id,
        session_slug,
        &target.name,
        service_role(target.role),
        endpoint_kind,
    )?;
    let mut args = vec![
        "--endpoint-kind".to_string(),
        endpoint_kind_arg(endpoint_kind)?.to_string(),
        "--endpoint".to_string(),
        endpoint.address.clone(),
        "--project".to_string(),
        project_root.to_string_lossy().into_owned(),
        "--project-id".to_string(),
        project_id.to_string(),
        "--owner-root".to_string(),
        binary_root.to_string_lossy().into_owned(),
    ];
    if target.role == ProjectServiceRole::RuntimeHost {
        args.extend(["--session".to_string(), session_slug.to_string()]);
    }
    args.extend(["--service".to_string(), target.name.clone()]);
    args.extend(target.args.iter().cloned());

    Ok(ProjectServiceCommand {
        owner_id: owner_id.to_string(),
        owner_root: binary_root.to_string_lossy().into_owned(),
        build_output_root: build_output_root.to_string_lossy().into_owned(),
        service_name: target.name.clone(),
        role: service_role(target.role),
        endpoint,
        program: service_binary_path(&build_output_root, target)
            .to_string_lossy()
            .into_owned(),
        cwd: project_root.to_string_lossy().into_owned(),
        args,
    })
}

/// Merge build commands that target the same Cargo build universe into a
/// single invocation carrying every compatible `-p`/`--bin` selector. Cargo already
/// parallelizes internally, so one process avoids target-dir lock contention
/// and yields one coherent progress stream. Commands that do not fit the
/// supported build shape pass through unchanged.
fn coalesce_build_commands(commands: &[ProjectBuildCommand]) -> Vec<ProjectBuildCommand> {
    type GroupKey = (String, String, String);

    struct Group {
        template: ProjectBuildCommand,
        cargo_prefix: Vec<String>,
        packages: Vec<String>,
        bins: Vec<String>,
        manifest_path: Option<String>,
        target_dir: Option<String>,
        features: Option<String>,
        common_args: Vec<String>,
    }

    let mut order: Vec<GroupKey> = Vec::new();
    let mut groups: BTreeMap<GroupKey, Group> = BTreeMap::new();
    let mut passthrough: Vec<ProjectBuildCommand> = Vec::new();

    for command in commands {
        let Some(ParsedCargoBuildArgs {
            cargo_prefix,
            packages,
            bins,
            manifest_path,
            target_dir,
            features,
            common_args,
        }) = parse_cargo_build_args(&command.args)
        else {
            passthrough.push(command.clone());
            continue;
        };
        let key = (
            command.cwd.clone(),
            command.program.clone(),
            format!(
                "{}\n{}\n{}\n{}\n{}\n{}\n{}",
                command.cargo_target_dir.clone().unwrap_or_default(),
                cargo_prefix.join("\u{0}"),
                manifest_path.clone().unwrap_or_default(),
                target_dir.clone().unwrap_or_default(),
                features.clone().unwrap_or_default(),
                if bins.is_empty() { "packages" } else { "bins" },
                common_args.join("\u{0}")
            ),
        );
        let group = groups.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Group {
                template: command.clone(),
                cargo_prefix,
                packages: Vec::new(),
                bins: Vec::new(),
                manifest_path,
                target_dir,
                features,
                common_args,
            }
        });
        for package in packages {
            if !group.packages.contains(&package) {
                group.packages.push(package);
            }
        }
        for bin in bins {
            if !group.bins.contains(&bin) {
                group.bins.push(bin);
            }
        }
    }

    let mut result = Vec::new();
    for key in &order {
        let group = &groups[key];
        let mut args = group.cargo_prefix.clone();
        args.push("build".to_string());
        if let Some(manifest_path) = &group.manifest_path {
            args.push("--manifest-path".to_string());
            args.push(manifest_path.clone());
        }
        for package in &group.packages {
            args.push("-p".to_string());
            args.push(package.clone());
        }
        for bin in &group.bins {
            args.push("--bin".to_string());
            args.push(bin.clone());
        }
        if let Some(target_dir) = &group.target_dir {
            args.push("--target-dir".to_string());
            args.push(target_dir.clone());
        }
        args.extend(group.common_args.iter().cloned());
        if let Some(features) = &group.features {
            args.push("--features".to_string());
            args.push(features.clone());
        }
        result.push(ProjectBuildCommand {
            args,
            ..group.template.clone()
        });
    }
    result.extend(passthrough);
    result
}

/// The mergeable parts of one recognized cargo build invocation.
struct ParsedCargoBuildArgs {
    /// Wrapper words before `build`, e.g. `xwin`.
    cargo_prefix: Vec<String>,
    packages: Vec<String>,
    bins: Vec<String>,
    manifest_path: Option<String>,
    target_dir: Option<String>,
    features: Option<String>,
    /// Flags that must match exactly for two commands to merge.
    common_args: Vec<String>,
}

/// Parse `[xwin] build [--manifest-path PATH] -p P [-p P...] [--bin B...]
/// [--target-dir DIR] [--features F] [--locked] [--release|--profile P] [--target T]`
/// into its mergeable parts. Returns `None`
/// for any other shape.
fn parse_cargo_build_args(args: &[String]) -> Option<ParsedCargoBuildArgs> {
    let (cargo_prefix, remaining) = match args {
        [build, remaining @ ..] if build == "build" => (Vec::new(), remaining),
        [xwin, build, remaining @ ..] if xwin == "xwin" && build == "build" => {
            (vec![xwin.clone()], remaining)
        }
        _ => return None,
    };
    let mut iter = remaining.iter();
    let mut packages = Vec::new();
    let mut bins = Vec::new();
    let mut manifest_path = None;
    let mut target_dir = None;
    let mut features = None;
    let mut common_args = Vec::new();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "-p" | "--package" => packages.push(iter.next()?.clone()),
            "--bin" => bins.push(iter.next()?.clone()),
            "--manifest-path" => manifest_path = Some(iter.next()?.clone()),
            "--target-dir" => target_dir = Some(iter.next()?.clone()),
            "--features" => features = Some(iter.next()?.clone()),
            "--locked" | "--release" => common_args.push(flag.clone()),
            "--profile" | "--target" => {
                common_args.push(flag.clone());
                common_args.push(iter.next()?.clone());
            }
            _ => return None,
        }
    }
    Some(ParsedCargoBuildArgs {
        cargo_prefix,
        packages,
        bins,
        manifest_path,
        target_dir,
        features,
        common_args,
    })
}

/// Resolve the target directory used by a cargo build command.
fn build_command_target_dir(command: &ProjectBuildCommand) -> Option<PathBuf> {
    if let Some(target_dir) = &command.cargo_target_dir {
        return Some(PathBuf::from(target_dir));
    }
    if let Some(target_dir) = command
        .args
        .iter()
        .position(|arg| arg == "--target-dir")
        .and_then(|index| command.args.get(index + 1))
        .map(PathBuf::from)
    {
        return Some(target_dir);
    }

    parse_cargo_build_args(&command.args)
        .is_some()
        .then(|| cargo_target_dir_or_default(Path::new(&command.cwd)))
}

const BUILD_DIAGNOSTIC_TAIL_MAX_LINES: usize = 160;
const BUILD_DIAGNOSTIC_LINE_MAX_CHARS: usize = 220;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BuildCommandOutcome {
    success: bool,
    diagnostic_headline: String,
    diagnostic_tail: String,
}

impl BuildCommandOutcome {
    const fn success() -> Self {
        Self {
            success: true,
            diagnostic_headline: String::new(),
            diagnostic_tail: String::new(),
        }
    }

    fn failed(headline: impl Into<String>, tail: impl Into<String>) -> Self {
        Self {
            success: false,
            diagnostic_headline: headline.into(),
            diagnostic_tail: tail.into(),
        }
    }

    fn spawn_failed(command: &ProjectBuildCommand, source: &std::io::Error) -> Self {
        Self::failed(
            format!(
                "failed to spawn build command `{}`: {source}",
                project_build_command_label(command)
            ),
            String::new(),
        )
    }

    fn io_failed(
        command: &ProjectBuildCommand,
        operation: &'static str,
        source: &std::io::Error,
        tail: String,
    ) -> Self {
        Self::failed(
            format!(
                "{operation} for build command `{}` failed: {source}",
                project_build_command_label(command)
            ),
            tail,
        )
    }

    fn cancelled(command: &ProjectBuildCommand) -> Self {
        Self::failed(
            format!(
                "project build cancelled before command `{}` completed",
                project_build_command_label(command)
            ),
            String::new(),
        )
    }
}

#[derive(Debug)]
enum CommandOutputEvent {
    Stdout(String),
    Stderr(String),
    CargoUnitFinished { target: Option<String> },
    CargoDiagnostic { rendered: String },
    CargoFinished { success: bool },
}

#[derive(Debug, Default, Clone)]
struct OutputTail {
    lines: VecDeque<String>,
}

impl OutputTail {
    fn push_line(&mut self, line: &str) {
        let line = line.trim_end();
        if line.trim().is_empty() {
            return;
        }
        self.lines.push_back(truncate_build_diagnostic_line(
            line,
            BUILD_DIAGNOSTIC_LINE_MAX_CHARS,
        ));
        while self.lines.len() > BUILD_DIAGNOSTIC_TAIL_MAX_LINES {
            self.lines.pop_front();
        }
    }

    fn finish(&self) -> Option<String> {
        (!self.lines.is_empty()).then(|| self.lines.iter().cloned().collect::<Vec<_>>().join("\n"))
    }
}

#[derive(Debug, Default, Clone)]
struct CommandOutputTails {
    stdout: OutputTail,
    stderr: OutputTail,
    cargo: build_progress::DiagnosticTail,
}

impl CommandOutputTails {
    fn push_stdout(&mut self, line: &str) {
        self.stdout.push_line(line);
    }

    fn push_stderr(&mut self, line: &str) {
        self.stderr.push_line(line);
    }

    fn push_cargo_diagnostic(&mut self, rendered: &str) {
        self.cargo.push_rendered(rendered);
    }

    fn finish(&self) -> String {
        let mut sections = Vec::new();
        let cargo = self.cargo.finish();
        if !cargo.trim().is_empty() {
            sections.push(cargo.trim().to_owned());
        }
        if let Some(stdout) = self.stdout.finish() {
            sections.push(format!("stdout tail:\n{stdout}"));
        }
        if let Some(stderr) = self.stderr.finish() {
            sections.push(format!("stderr tail:\n{stderr}"));
        }
        sections.join("\n\n")
    }
}

fn truncate_build_diagnostic_line(line: &str, max_chars: usize) -> String {
    let mut chars = line.chars();
    let mut out = String::new();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return line.to_string();
        };
        out.push(ch);
    }
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

fn project_build_command_label(command: &ProjectBuildCommand) -> String {
    format!(
        "{}:{}",
        empty_daemon_label(&command.owner_id),
        empty_daemon_label(&command.target_name)
    )
}

fn empty_daemon_label(value: &str) -> &str {
    if value.trim().is_empty() {
        "unknown"
    } else {
        value
    }
}

fn is_cargo_program(program: &str) -> bool {
    Path::new(program)
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("cargo"))
}

/// The result of running one command out of a project build plan.
enum PlannedCommandOutcome {
    Completed {
        label: String,
    },
    Failed {
        command: ProjectBuildCommand,
        diagnostic_headline: String,
        diagnostic_tail: String,
    },
}

/// Run the command at `index` of `plan`, driving its progress phase.
///
/// A command that fails is reported as [`PlannedCommandOutcome::Failed`] with
/// the captured diagnostics; the caller decides what the plan-level result is.
fn run_planned_build_command(
    plan: &ProjectBuildPlan,
    progress: &ProjectBuildProgress,
    index: usize,
    cancel: &az_work::CancellationToken,
) -> PlannedCommandOutcome {
    let command = plan.commands[index].clone();
    let command_count = plan.commands.len();
    let Some(command_progress) = progress.command(index) else {
        debug_assert!(
            false,
            "project build progress phase exists for each planned command"
        );
        return PlannedCommandOutcome::Failed {
            command,
            diagnostic_headline: "project build progress phase missing for planned command"
                .to_string(),
            diagnostic_tail: String::new(),
        };
    };
    let command_number = index.saturating_add(1);
    let label = project_build_command_label(&command);
    info!(
        command = %label,
        command_number,
        command_count,
        "starting project build command"
    );
    command_progress.message(format!(
        "Starting command {command_number}/{command_count}: {label}"
    ));

    let outcome = if cancel.is_cancelled() {
        BuildCommandOutcome::cancelled(&command)
    } else {
        run_project_build_command_with_capture(&command, command_progress, cancel)
    };

    if outcome.success {
        command_progress.message(format!(
            "Finished command {command_number}/{command_count}: {label}"
        ));
        command_progress.finish();
        return PlannedCommandOutcome::Completed { label };
    }

    command_progress.message(format!(
        "Failed command {command_number}/{command_count}: {}",
        outcome.diagnostic_headline
    ));
    warn!(
        command = %label,
        diagnostic = %outcome.diagnostic_headline,
        "project build command failed"
    );
    PlannedCommandOutcome::Failed {
        command,
        diagnostic_headline: outcome.diagnostic_headline,
        diagnostic_tail: outcome.diagnostic_tail,
    }
}

/// Report every runtime sidecar the post-build staging pass actually copied.
fn log_staged_runtime_files(reports: &[az_project::RuntimeFileStagingReport]) {
    for report in reports {
        for entry in &report.entries {
            if entry.action == az_project::RuntimeFileStagingAction::Staged {
                info!(
                    target = %report.target_name,
                    source = %entry.source.display(),
                    destination = %entry.destination.display(),
                    "staged build target runtime file"
                );
            }
        }
    }
}

/// The mutable folding state of one build command's output stream.
struct BuildOutputState {
    counter: build_progress::UnitCounter,
    tails: CommandOutputTails,
    diagnostic_headline: Option<String>,
    /// Non-cargo commands count as reporting success up front; cargo sets this
    /// from its own `build-finished` record.
    cargo_reported_success: bool,
}

/// The immutable handles [`wait_for_build_command_exit`] selects over.
struct BuildCommandWait<'a> {
    command: &'a ProjectBuildCommand,
    progress: &'a az_work::Progress,
    lifecycle: &'a ServiceLifecycleEvents,
    process_identity: ProcessIdentity,
    output: &'a channel::Receiver<CommandOutputEvent>,
    cancel: &'a az_work::CancellationToken,
}

/// Configure the child process for one build command, asking cargo for its
/// JSON stream when the caller has not already chosen a message format.
fn build_command_process(command: &ProjectBuildCommand, is_cargo: bool) -> Command {
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_project_build_environment(&mut process, command);
    if is_cargo
        && !command
            .args
            .iter()
            .any(|arg| arg.starts_with("--message-format"))
    {
        process.arg("--message-format=json-render-diagnostics");
    }
    process
}

/// Bind the spawned build process's exact exit to a fresh lifecycle watcher.
///
/// # Errors
///
/// Returns a failed [`BuildCommandOutcome`] — after terminating the child —
/// when the process disappears before its identity can be captured, the
/// identity probe fails, or the watcher refuses the binding.
fn bind_build_process_exit(
    command: &ProjectBuildCommand,
    child: &mut az_work::OwnedSynchronousChild,
) -> Result<(ProcessIdentity, ServiceLifecycleEvents), BuildCommandOutcome> {
    let process_identity = match ProcessIdentity::capture(child.id()) {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            let _ = child.terminate_and_wait();
            return Err(BuildCommandOutcome::io_failed(
                command,
                "binding build process exit",
                &std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "spawned build process disappeared before its exit could be bound",
                ),
                String::new(),
            ));
        }
        Err(source) => {
            let _ = child.terminate_and_wait();
            return Err(BuildCommandOutcome::io_failed(
                command,
                "capturing build process identity",
                &source,
                String::new(),
            ));
        }
    };
    let lifecycle = ServiceLifecycleEvents::new();
    if let Err(error) = lifecycle.add_identity(process_identity) {
        let _ = child.terminate_and_wait();
        return Err(BuildCommandOutcome::io_failed(
            command,
            "binding build process exit",
            &std::io::Error::other(error),
            String::new(),
        ));
    }
    Ok((process_identity, lifecycle))
}

/// Drain the child's stdout and stderr on their own threads, parsing cargo's
/// JSON records into progress events where the program is cargo.
fn spawn_build_output_readers(
    child: &mut az_work::OwnedSynchronousChild,
    is_cargo: bool,
    tx: &channel::Sender<CommandOutputEvent>,
) -> Vec<std::thread::JoinHandle<()>> {
    let mut readers = Vec::new();
    if let Some(stdout) = child.take_stdout() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if is_cargo {
                    match build_progress::parse_cargo_line(&line) {
                        build_progress::CargoBuildEvent::UnitFinished { target } => {
                            let _ = tx.send(CommandOutputEvent::CargoUnitFinished { target });
                        }
                        build_progress::CargoBuildEvent::Diagnostic { rendered } => {
                            let _ = tx.send(CommandOutputEvent::CargoDiagnostic { rendered });
                        }
                        build_progress::CargoBuildEvent::Finished { success } => {
                            let _ = tx.send(CommandOutputEvent::CargoFinished { success });
                        }
                        build_progress::CargoBuildEvent::Other => {
                            if !line.trim_start().starts_with('{') {
                                let _ = tx.send(CommandOutputEvent::Stdout(line));
                            }
                        }
                    }
                } else {
                    let _ = tx.send(CommandOutputEvent::Stdout(line));
                }
            }
        }));
    }
    if let Some(stderr) = child.take_stderr() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx.send(CommandOutputEvent::Stderr(line));
            }
        }));
    }
    readers
}

/// Fold output events until the build process exits or the caller cancels,
/// returning its exit status and whether cancellation caused it.
///
/// # Errors
///
/// Returns a failed [`BuildCommandOutcome`] when terminating or waiting on the
/// process tree fails, when the exit cannot be retired or consumed, or when the
/// lifecycle event source closes before the exit arrives.
fn wait_for_build_command_exit(
    wait: &BuildCommandWait<'_>,
    child: &mut az_work::OwnedSynchronousChild,
    state: &mut BuildOutputState,
) -> Result<(std::process::ExitStatus, bool), BuildCommandOutcome> {
    let command = wait.command;
    let lifecycle = wait.lifecycle;
    let process_identity = wait.process_identity;
    let cancellation = wait.cancel.cancellation_signal();
    let no_output = channel::never();
    let mut output_events = wait.output;
    let mut cancelled = false;

    let status = loop {
        channel::select! {
            recv(output_events) -> event => match event {
                Ok(event) => handle_project_build_output_event(event, wait.progress, state),
                Err(_) => output_events = &no_output,
            },
            recv(cancellation.receiver()) -> _ => {
                cancelled = true;
                let status = match child.terminate_and_wait() {
                    Ok(status) => status,
                    Err(source) => {
                        return Err(BuildCommandOutcome::io_failed(
                            command,
                            "terminating and waiting for cancelled process tree",
                            &source,
                            state.tails.finish()));
                    }
                };
                if let Err(error) = lifecycle.retire_exit(process_identity) {
                    return Err(BuildCommandOutcome::io_failed(
                        command,
                        "retiring cancelled build process exit",
                        &std::io::Error::other(error),
                        state.tails.finish()));
                }
                break status;
            },
            recv(lifecycle.receiver()) -> event => match event {
                Ok(ServiceLifecycleEvent::ProcessExited(identity)) if identity == process_identity => {
                    let status = match child.wait() {
                        Ok(status) => status,
                        Err(source) => {
                            return Err(BuildCommandOutcome::io_failed(
                                command,
                                "waiting for completed build process tree",
                                &source,
                                state.tails.finish()));
                        }
                    };
                    if let Err(error) = lifecycle.consume_exit(identity) {
                        return Err(BuildCommandOutcome::io_failed(
                            command,
                            "consuming build process exit",
                            &std::io::Error::other(error),
                            state.tails.finish()));
                    }
                    break status;
                }
                Ok(ServiceLifecycleEvent::ProcessExitWaitFailed { identity, reason })
                    if identity == process_identity =>
                {
                    let _ = child.terminate_and_wait();
                    return Err(BuildCommandOutcome::io_failed(
                        command,
                        "waiting for build process exit",
                        &std::io::Error::other(reason),
                        state.tails.finish()));
                }
                Ok(ServiceLifecycleEvent::ReadyFileChanged
                    | ServiceLifecycleEvent::ProcessExited(_)
                    | ServiceLifecycleEvent::ProcessExitWaitFailed { .. }) => {}
                Err(_) => {
                    let _ = child.terminate_and_wait();
                    return Err(BuildCommandOutcome::io_failed(
                        command,
                        "receiving build process lifecycle event",
                        &std::io::Error::other("build lifecycle event source closed"),
                        state.tails.finish()));
                }
            }
        }
    };
    Ok((status, cancelled))
}

fn run_project_build_command_with_capture(
    command: &ProjectBuildCommand,
    progress: &az_work::Progress,
    cancel: &az_work::CancellationToken,
) -> BuildCommandOutcome {
    let is_cargo = is_cargo_program(&command.program);
    let target_dir = is_cargo
        .then(|| build_command_target_dir(command))
        .flatten();
    let cargo_version = build_progress::cargo_version();
    let cached_total = target_dir
        .as_deref()
        .and_then(|dir| build_progress::read_cached_unit_count(dir, &cargo_version));
    if let Some(total) = cached_total {
        progress.set_total(total);
    } else if !is_cargo {
        progress.set_total(1);
    }

    let mut process = build_command_process(command, is_cargo);
    let owner = match az_work::OwnedSynchronousCommandTree::new() {
        Ok(owner) => owner,
        Err(source) => return BuildCommandOutcome::spawn_failed(command, &source),
    };
    let mut child = match owner.spawn(&mut process) {
        Ok(child) => child,
        Err(source) => return BuildCommandOutcome::spawn_failed(command, &source),
    };
    let (process_identity, lifecycle) = match bind_build_process_exit(command, &mut child) {
        Ok(bound) => bound,
        Err(outcome) => return outcome,
    };

    let (tx, rx) = channel::unbounded::<CommandOutputEvent>();
    let readers = spawn_build_output_readers(&mut child, is_cargo, &tx);
    drop(tx);

    let mut state = BuildOutputState {
        counter: build_progress::UnitCounter::new(cached_total),
        tails: CommandOutputTails::default(),
        diagnostic_headline: None,
        cargo_reported_success: !is_cargo,
    };
    let (status, cancelled) = match wait_for_build_command_exit(
        &BuildCommandWait {
            command,
            progress,
            lifecycle: &lifecycle,
            process_identity,
            output: &rx,
            cancel,
        },
        &mut child,
        &mut state,
    ) {
        Ok(exit) => exit,
        Err(outcome) => return outcome,
    };

    for reader in readers {
        let _ = reader.join();
    }
    while let Ok(event) = rx.try_recv() {
        handle_project_build_output_event(event, progress, &mut state);
    }

    if cancelled {
        return BuildCommandOutcome::failed(
            format!(
                "project build cancelled while running `{}`",
                project_build_command_label(command)
            ),
            state.tails.finish(),
        );
    }
    if !status.success() || !state.cargo_reported_success {
        let headline = state.diagnostic_headline.unwrap_or_else(|| {
            format!(
                "build command `{}` failed with status {:?}",
                project_build_command_label(command),
                status.code()
            )
        });
        return BuildCommandOutcome::failed(headline, state.tails.finish());
    }

    if is_cargo {
        if let Some(dir) = target_dir.as_deref() {
            build_progress::write_cached_unit_count(dir, state.counter.done(), &cargo_version);
        }
    } else {
        progress.advance(1);
    }
    BuildCommandOutcome::success()
}

fn handle_project_build_output_event(
    event: CommandOutputEvent,
    progress: &az_work::Progress,
    state: &mut BuildOutputState,
) {
    match event {
        CommandOutputEvent::Stdout(line) => {
            debug!(line = %line, "project build command stdout");
            state.tails.push_stdout(&line);
        }
        CommandOutputEvent::Stderr(line) => {
            debug!(line = %line, "project build command stderr");
            state.tails.push_stderr(&line);
        }
        CommandOutputEvent::CargoUnitFinished { target } => {
            let _fraction = state.counter.advance();
            if let Some(target) = target {
                debug!(target = %target, "cargo build unit finished");
            }
            progress.advance(1);
        }
        CommandOutputEvent::CargoDiagnostic { rendered } => {
            if rendered.is_empty() {
                return;
            }
            state.tails.push_cargo_diagnostic(&rendered);
            if let Some(headline) = build_progress::rendered_diagnostic_headline(&rendered) {
                state
                    .diagnostic_headline
                    .get_or_insert_with(|| headline.clone());
                progress.message(headline);
            }
        }
        CommandOutputEvent::CargoFinished { success } => {
            state.cargo_reported_success = success;
        }
    }
}

/// Run a cargo build, streaming its JSON output to count finished compilation
/// units and report a real percentage through `build_progress` (the Build
/// phase node). Human-readable diagnostics are forwarded verbatim so build
/// errors surface exactly as before.
fn run_build_command_with_progress(
    command: &ProjectBuildCommand,
    build_progress: Option<&az_work::Progress>,
) -> Result<(), AzDaemonError> {
    // Only cargo emits the JSON stream we parse; anything else falls back to
    // the original inherited-stdio status run.
    let is_cargo = Path::new(&command.program)
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("cargo"));
    let Some(progress) = build_progress.filter(|_| is_cargo) else {
        return run_build_command_status(command);
    };

    let target_dir = build_command_target_dir(command);
    let cargo_version = build_progress::cargo_version();
    let cached_total = target_dir
        .as_deref()
        .and_then(|dir| build_progress::read_cached_unit_count(dir, &cargo_version));
    let mut counter = build_progress::UnitCounter::new(cached_total);
    if let Some(total) = cached_total {
        progress.set_total(total);
    }
    progress.message("building project services");

    // `--message-format=json-render-diagnostics` keeps structured records AND
    // populates `message.rendered` so diagnostics stay human-readable. stderr
    // is inherited so cargo's own error output still reaches the daemon log.
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .arg("--message-format=json-render-diagnostics")
        .current_dir(&command.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    apply_project_build_environment(&mut process, command);
    let owner = az_work::OwnedSynchronousCommandTree::new().map_err(|source| {
        AzDaemonError::ServiceBuildSpawn {
            program: command.program.clone(),
            source,
        }
    })?;
    let mut child =
        owner
            .spawn(&mut process)
            .map_err(|source| AzDaemonError::ServiceBuildSpawn {
                program: command.program.clone(),
                source,
            })?;

    let mut cargo_reported_success = false;
    let mut diagnostics = build_progress::DiagnosticTail::default();
    if let Some(stdout) = child.take_stdout() {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            match build_progress::parse_cargo_line(&line) {
                build_progress::CargoBuildEvent::UnitFinished { target } => {
                    let fraction = counter.advance();
                    if let Some(target) = target {
                        progress.message(format!("compiling {target}"));
                    }
                    let (done, total) = fraction.raw();
                    progress.advance(1);
                    let _ = (done, total);
                }
                build_progress::CargoBuildEvent::Diagnostic { rendered } => {
                    if !rendered.is_empty() {
                        eprint!("{rendered}");
                        diagnostics.push_rendered(&rendered);
                        if let Some(headline) =
                            build_progress::rendered_diagnostic_headline(&rendered)
                        {
                            progress.message(headline);
                        }
                    }
                }
                build_progress::CargoBuildEvent::Finished { success } => {
                    cargo_reported_success = success;
                }
                build_progress::CargoBuildEvent::Other => {}
            }
        }
    }

    let status = child
        .wait()
        .map_err(|source| AzDaemonError::ServiceBuildSpawn {
            program: command.program.clone(),
            source,
        })?;
    if !status.success() || !cargo_reported_success {
        return Err(AzDaemonError::ServiceBuildFailed {
            program: command.program.clone(),
            args: command.args.join(" "),
            status: status.code(),
            diagnostics: diagnostics.finish(),
        });
    }

    // Persist this build's unit count as the next build's denominator. Never
    // overwrite on failure (handled above by the early return).
    if let Some(dir) = target_dir.as_deref() {
        build_progress::write_cached_unit_count(dir, counter.done(), &cargo_version);
    }
    Ok(())
}

fn run_build_command_status(command: &ProjectBuildCommand) -> Result<(), AzDaemonError> {
    let mut process = Command::new(&command.program);
    process.args(&command.args).current_dir(&command.cwd);
    apply_project_build_environment(&mut process, command);
    let owner = az_work::OwnedSynchronousCommandTree::new().map_err(|source| {
        AzDaemonError::ServiceBuildSpawn {
            program: command.program.clone(),
            source,
        }
    })?;
    let mut child =
        owner
            .spawn(&mut process)
            .map_err(|source| AzDaemonError::ServiceBuildSpawn {
                program: command.program.clone(),
                source,
            })?;
    let status = child
        .wait()
        .map_err(|source| AzDaemonError::ServiceBuildSpawn {
            program: command.program.clone(),
            source,
        })?;
    if !status.success() {
        return Err(AzDaemonError::ServiceBuildFailed {
            program: command.program.clone(),
            args: command.args.join(" "),
            status: status.code(),
            diagnostics: String::new(),
        });
    }
    Ok(())
}

fn apply_project_build_environment(process: &mut Command, command: &ProjectBuildCommand) {
    if let Some(target_dir) = &command.cargo_target_dir {
        process.env("CARGO_TARGET_DIR", target_dir);
    }
}

fn session_launch_command(
    command: &ProjectServiceCommand,
) -> az_session::SessionServiceLaunchCommand {
    az_session::SessionServiceLaunchCommand {
        owner_id: command.owner_id.clone(),
        owner_root: PathBuf::from(&command.owner_root),
        build_output_root: PathBuf::from(&command.build_output_root),
        service_name: command.service_name.clone(),
        role: command.role,
        endpoint: command.endpoint.clone(),
        program: command.program.clone(),
        cwd: PathBuf::from(&command.cwd),
        args: command.args.clone(),
    }
}

fn set_project_service_arg(args: &mut Vec<String>, flag: &str, value: &str) {
    remove_project_service_arg(args, flag);
    args.extend([flag.to_string(), value.to_string()]);
}

fn remove_project_service_arg(args: &mut Vec<String>, flag: &str) {
    let mut normalized = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        if args[index] == flag {
            index += usize::from(index + 1 < args.len()) + 1;
        } else if args[index]
            .strip_prefix(flag)
            .is_some_and(|suffix| suffix.starts_with('='))
        {
            index += 1;
        } else {
            normalized.push(args[index].clone());
            index += 1;
        }
    }
    *args = normalized;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ProjectServiceStartTier {
    AssetProcessor,
    ProjectHost,
    Worker,
}

fn project_service_start_waves(
    processes: &[ServiceProcessRecord],
    requested: &[String],
) -> Vec<Vec<ServiceProcessRecord>> {
    let requested = requested
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    processes
        .iter()
        .filter(|process| requested.contains(process.service_name.as_str()))
        .filter_map(|process| project_service_start_tier(process.role))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|tier| {
            processes
                .iter()
                .filter(|process| requested.contains(process.service_name.as_str()))
                .filter(|process| project_service_start_tier(process.role) == Some(tier))
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect()
}

const fn project_service_start_tier(
    role: SupervisedServiceRole,
) -> Option<ProjectServiceStartTier> {
    match role {
        SupervisedServiceRole::AssetProcessor => Some(ProjectServiceStartTier::AssetProcessor),
        SupervisedServiceRole::ProjectHost => Some(ProjectServiceStartTier::ProjectHost),
        SupervisedServiceRole::Worker => Some(ProjectServiceStartTier::Worker),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct PendingProjectService {
    process: ServiceProcessRecord,
    spawned: SpawnedServiceProcess,
}

trait ProjectServiceReadySubscription {
    fn finish(&mut self) -> Result<(), az_service_supervision::ServiceProcessError>;
}

impl ProjectServiceReadySubscription for az_service_supervision::ServiceReadySubscription {
    fn finish(&mut self) -> Result<(), az_service_supervision::ServiceProcessError> {
        Self::finish(self)
    }
}

trait ProjectServiceLifecycle {
    type ReadySubscription: ProjectServiceReadySubscription;

    fn bind_exit(
        &self,
        identity: ProcessIdentity,
    ) -> Result<(), az_service_supervision::ServiceProcessError>;
    fn subscribe_ready(
        &self,
        ready_paths: &[PathBuf],
    ) -> Result<Self::ReadySubscription, az_service_supervision::ServiceProcessError>;
    fn wait_until(
        &self,
        deadline: Instant,
    ) -> Result<Option<ServiceLifecycleEvent>, az_service_supervision::ServiceProcessError>;
    fn wait_for_exit_until(
        &self,
        identity: ProcessIdentity,
        deadline: Instant,
    ) -> Result<Option<ServiceLifecycleEvent>, az_service_supervision::ServiceProcessError>;
    fn consume_exit(
        &self,
        identity: ProcessIdentity,
    ) -> Result<(), az_service_supervision::ServiceProcessError>;
    fn retire_exit(
        &self,
        identity: ProcessIdentity,
    ) -> Result<(), az_service_supervision::ServiceProcessError>;
}

impl ProjectServiceLifecycle for ServiceLifecycleEvents {
    type ReadySubscription = az_service_supervision::ServiceReadySubscription;

    fn bind_exit(
        &self,
        identity: ProcessIdentity,
    ) -> Result<(), az_service_supervision::ServiceProcessError> {
        self.add_identity(identity)
    }

    fn subscribe_ready(
        &self,
        ready_paths: &[PathBuf],
    ) -> Result<Self::ReadySubscription, az_service_supervision::ServiceProcessError> {
        Self::subscribe_ready(self, ready_paths.iter().map(PathBuf::as_path))
    }

    fn wait_until(
        &self,
        deadline: Instant,
    ) -> Result<Option<ServiceLifecycleEvent>, az_service_supervision::ServiceProcessError> {
        Self::wait_until(self, deadline)
    }

    fn wait_for_exit_until(
        &self,
        identity: ProcessIdentity,
        deadline: Instant,
    ) -> Result<Option<ServiceLifecycleEvent>, az_service_supervision::ServiceProcessError> {
        Self::wait_for_exit_until(self, identity, deadline)
    }

    fn consume_exit(
        &self,
        identity: ProcessIdentity,
    ) -> Result<(), az_service_supervision::ServiceProcessError> {
        Self::consume_exit(self, identity)
    }

    fn retire_exit(
        &self,
        identity: ProcessIdentity,
    ) -> Result<(), az_service_supervision::ServiceProcessError> {
        Self::retire_exit(self, identity)
    }
}

/// Wait for the one exact identity that acknowledged a normal project-service
/// shutdown request. A timeout or failed OS wait deliberately leaves force
/// termination to the caller; startup rollback remains a separate force-only
/// path because it has no ready lifecycle endpoint to contact.
/// Ask one running project service to shut itself down and wait for its exact
/// exit, returning `None` when the caller must force termination instead.
///
/// Every failure here is a fall-back signal, not an error: an unreachable
/// lifecycle endpoint, a missing recorded identity, a deadline, or a failed
/// wait all resolve to `None` so the caller terminates the process tree.
fn request_project_service_graceful_exit<L, C>(
    project: &ProjectRecord,
    manifest: &project_services::ProjectServiceManifest,
    process: &ServiceProcessRecord,
    key: &ServiceProcessKey,
    launcher: &L,
    lifecycle: &C,
) -> Option<az_service_supervision::ServiceProcessExit>
where
    L: ServiceProcessLauncher<Error = az_service_supervision::ServiceProcessError>,
    C: ProjectServiceLifecycle,
{
    let descriptor = project_service_descriptor_for_process(manifest, process)?;
    let controller = ServiceLifecycleController::new(
        ServiceId::new(DAEMON_SERVICE_NAMESPACE, DAEMON_SERVICE_NAME),
        ServiceRole::Daemon,
    );
    if let Err(error) = request_service_lifecycle_shutdown(&descriptor, &controller) {
        warn!(
            project_id = %project.project_id,
            service = %process.service_name,
            error = %error,
            "project service lifecycle control request failed; forcing exact process termination"
        );
        return None;
    }
    let identity = ProcessIdentity {
        process_id: process.pid?,
        process_start_time: process.process_start_time?,
    };
    match wait_for_project_service_graceful_exit(launcher, lifecycle, key, identity) {
        Ok(Some(exit)) => Some(exit),
        Ok(None) => {
            warn!(
                project_id = %project.project_id,
                service = %process.service_name,
                "project service did not exit through lifecycle control before the shutdown deadline; forcing exact process termination"
            );
            None
        }
        Err(error) => {
            warn!(
                project_id = %project.project_id,
                service = %process.service_name,
                error = %error,
                "project service lifecycle exit wait failed; forcing exact process termination"
            );
            None
        }
    }
}

/// Terminate a project service the daemon still tracks, or prove that an
/// untracked recorded process is already gone. Returns its exit code when the
/// launcher observed one.
///
/// # Errors
///
/// Returns [`AzDaemonError::InvalidServicePlan`] when a tracked running process
/// has no recorded process id or start time,
/// [`AzDaemonError::ProjectServiceProcess`] when termination or exit retirement
/// fails, and [`AzDaemonError::ProjectServiceCleanupRefused`] when a recorded
/// process cannot be proven gone.
fn force_project_service_exit<L, C>(
    project: &ProjectRecord,
    process: &ServiceProcessRecord,
    key: &ServiceProcessKey,
    launcher: &L,
    lifecycle: &C,
) -> Result<Option<i32>, AzDaemonError>
where
    L: ServiceProcessLauncher<Error = az_service_supervision::ServiceProcessError>,
    C: ProjectServiceLifecycle,
{
    if launcher.is_tracking(key)
        && let Some(exit) = launcher.terminate(key)?
    {
        lifecycle.retire_exit(ProcessIdentity {
            process_id: process
                .pid
                .ok_or_else(|| AzDaemonError::InvalidServicePlan {
                    service: process.service_name.clone(),
                    reason: "running project service has no process id".to_string(),
                })?,
            process_start_time: process.process_start_time.ok_or_else(|| {
                AzDaemonError::InvalidServicePlan {
                    service: process.service_name.clone(),
                    reason: "running project service has no process start time".to_string(),
                }
            })?,
        })?;
        return Ok(exit.exit_code);
    }
    require_project_process_gone(
        project,
        process,
        terminate_recorded_service_process(process)?,
    )?;
    Ok(None)
}

fn wait_for_project_service_graceful_exit<L, C>(
    launcher: &L,
    lifecycle: &C,
    key: &ServiceProcessKey,
    identity: ProcessIdentity,
) -> Result<Option<az_service_supervision::ServiceProcessExit>, AzDaemonError>
where
    L: ServiceProcessLauncher<Error = az_service_supervision::ServiceProcessError>,
    C: ProjectServiceLifecycle,
{
    match lifecycle.wait_for_exit_until(
        identity,
        Instant::now() + PROJECT_SERVICE_GRACEFUL_SHUTDOWN_TIMEOUT,
    )? {
        Some(ServiceLifecycleEvent::ProcessExited(exit_identity)) => {
            let exit = launcher.try_wait(key)?;
            lifecycle.consume_exit(exit_identity)?;
            Ok(exit)
        }
        Some(ServiceLifecycleEvent::ProcessExitWaitFailed {
            identity: exit_identity,
            ..
        }) => {
            lifecycle.consume_exit(exit_identity)?;
            Ok(None)
        }
        Some(ServiceLifecycleEvent::ReadyFileChanged) | None => Ok(None),
    }
}

trait ProjectServiceReadyReader {
    fn read(&self, path: &Path) -> Result<String, std::io::Error>;
}

struct FilesystemProjectServiceReadyReader;

impl ProjectServiceReadyReader for FilesystemProjectServiceReadyReader {
    fn read(&self, path: &Path) -> Result<String, std::io::Error> {
        fs::read_to_string(path)
    }
}

fn bind_project_service_exit_or_rollback<L, C>(
    store: &project_services::ProjectServiceStore,
    launcher: &L,
    lifecycle: &C,
    manifest: &mut project_services::ProjectServiceManifest,
    started: &[SpawnedServiceProcess],
    process: &ServiceProcessRecord,
    spawned: &SpawnedServiceProcess,
) -> Result<(), AzDaemonError>
where
    L: ServiceProcessLauncher<Error = az_service_supervision::ServiceProcessError>,
    C: ProjectServiceLifecycle,
{
    if let Err(error) = lifecycle.bind_exit(spawned.identity) {
        let reason = format!(
            "could not bind identity-bound exit notification for `{}`: {error}",
            process.service_name
        );
        mark_project_process_failed(
            store,
            manifest,
            &ServiceProcessKey::from_process(process),
            reason.clone(),
        )?;
        rollback_project_service_starts(store, launcher, lifecycle, manifest, started, &reason)?;
        return Err(error.into());
    }
    Ok(())
}

/// Reclaim whatever a previous run left behind and mark one project service
/// `Starting` so it can be spawned.
///
/// Returns `false` when the service is already running under this daemon's own
/// launcher, in which case the caller skips it.
///
/// # Errors
///
/// Returns [`AzDaemonError::ProjectServiceCleanupRefused`] when a recorded
/// process from a previous run cannot be proven gone,
/// [`AzDaemonError::ProjectServiceProcess`] when terminating it or reading the
/// program artifact fails, [`AzDaemonError::SessionServiceEndpointLayout`] when
/// a stale ready file cannot be removed, and
/// [`AzDaemonError::ProjectServices`] when the manifest cannot be committed.
fn prepare_project_service_for_start<L>(
    project: &ProjectRecord,
    store: &project_services::ProjectServiceStore,
    manifest: &mut project_services::ProjectServiceManifest,
    launcher: &L,
    process: &mut ServiceProcessRecord,
    key: &ServiceProcessKey,
) -> Result<bool, AzDaemonError>
where
    L: ServiceProcessLauncher<Error = az_service_supervision::ServiceProcessError>,
{
    if process.state == ServiceProcessState::Running && launcher.is_tracking(key) {
        return Ok(false);
    }
    if matches!(
        process.state,
        ServiceProcessState::Starting | ServiceProcessState::Running
    ) && !launcher.is_tracking(key)
    {
        let cleanup = require_project_process_gone(
            project,
            process,
            terminate_recorded_service_process(process)?,
        )?;
        info!(
            project_id = %project.project_id,
            service = %process.service_name,
            ?cleanup,
            "reclaimed stale project-service process before restart"
        );
        remove_stale_project_service_unix_endpoint(process)?;
        if let Some(current) = manifest.current_process_mut(key) {
            current.mark_exited(
                None,
                Some("stale project-service ownership recovered".to_string()),
                u128::from(current_unix_ms()),
            );
        }
        store.write(manifest)?;
    }

    if matches!(
        process.state,
        ServiceProcessState::Exited | ServiceProcessState::Failed
    ) {
        remove_stale_project_service_unix_endpoint(process)?;
    }

    if let Some(ready_file) = process.ready_file.as_ref() {
        match fs::remove_file(ready_file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    process.capture_program_artifact()?;
    if let Some(current) = manifest.current_process_mut(key) {
        current.program_artifact = process.program_artifact;
        current.mark_starting(u128::from(current_unix_ms()));
    }
    store.write(manifest)?;
    Ok(true)
}

/// Everything one start wave needs to watch its services become ready.
struct ProjectServiceWave<'a, L, C, R> {
    project_id: &'a str,
    store: &'a project_services::ProjectServiceStore,
    launcher: &'a L,
    lifecycle: &'a C,
    ready_reader: &'a R,
    /// The services spawned in this wave, in launch order.
    pending: &'a [PendingProjectService],
    /// Every service started so far, rolled back together on failure.
    started: &'a [SpawnedServiceProcess],
    ready_timeout: Duration,
}

impl<L, C, R> ProjectServiceWave<'_, L, C, R>
where
    L: ServiceProcessLauncher<Error = az_service_supervision::ServiceProcessError>,
    C: ProjectServiceLifecycle,
    R: ProjectServiceReadyReader,
{
    /// Undo every service started so far, reporting `reason` on each record.
    ///
    /// # Errors
    ///
    /// Returns any error [`rollback_project_service_starts`] returns.
    fn roll_back(
        &self,
        manifest: &mut project_services::ProjectServiceManifest,
        reason: &str,
    ) -> Result<(), AzDaemonError> {
        rollback_project_service_starts(
            self.store,
            self.launcher,
            self.lifecycle,
            manifest,
            self.started,
            reason,
        )
    }

    /// Commit the ready records of every pending service whose ready file has
    /// appeared, marking those entries of `ready`.
    ///
    /// # Errors
    ///
    /// Rolls the wave back and returns the failure when a ready file cannot be
    /// read, parsed, validated against the spawned process, or committed.
    fn collect_ready(
        &self,
        manifest: &mut project_services::ProjectServiceManifest,
        ready: &mut [bool],
    ) -> Result<(), AzDaemonError> {
        for (index, pending) in self.pending.iter().enumerate() {
            if ready[index] {
                continue;
            }
            let process = &pending.process;
            let Some(ready_file) = process.ready_file.as_ref().filter(|path| path.is_file()) else {
                continue;
            };
            let readiness = (|| -> Result<(), AzDaemonError> {
                let text = self.ready_reader.read(ready_file)?;
                let record: ServiceReadyRecord =
                    toml::from_str(&text).map_err(project_services::ProjectServiceError::from)?;
                validate_project_ready_record(
                    process,
                    pending.spawned.identity.process_id,
                    &record,
                )?;
                commit_project_service_ready(
                    self.store,
                    manifest,
                    process,
                    pending.spawned.identity,
                    &record,
                )
            })();
            if let Err(error) = readiness {
                let reason = format!(
                    "project-service readiness processing failed for `{}`: {error}",
                    process.service_name
                );
                self.roll_back(manifest, &reason)?;
                return Err(error);
            }
            ready[index] = true;
        }
        Ok(())
    }

    /// Roll the wave back after one of its services exited before reporting
    /// readiness. Always returns `Err`.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::InvalidServicePlan`] naming the service that
    /// exited, or the rollback/reap failure that displaced it.
    fn fail_on_process_exit(
        &self,
        manifest: &mut project_services::ProjectServiceManifest,
        identity: ProcessIdentity,
    ) -> Result<(), AzDaemonError> {
        let process = self
            .pending
            .iter()
            .find(|pending| pending.spawned.identity == identity)
            .map(|pending| pending.process.clone())
            .or_else(|| project_process_for_identity(manifest, identity));
        let Some(process) = process else {
            let reason =
                format!("project lifecycle exit arrived for an untracked identity {identity:?}");
            self.roll_back(manifest, &reason)?;
            return Err(AzDaemonError::InvalidServicePlan {
                service: self.project_id.to_string(),
                reason,
            });
        };
        let key = ServiceProcessKey::from_process(&process);
        let exit = match self.launcher.try_wait(&key) {
            Ok(Some(exit)) => exit,
            Ok(None) => {
                let reason = format!(
                    "project lifecycle exit for `{}` could not reap its owned child",
                    process.service_name
                );
                self.roll_back(manifest, &reason)?;
                return Err(AzDaemonError::InvalidServicePlan {
                    service: process.service_name,
                    reason,
                });
            }
            Err(error) => {
                let reason = format!(
                    "project lifecycle exit for `{}` could not be reaped: {error}",
                    process.service_name
                );
                self.roll_back(manifest, &reason)?;
                return Err(error.into());
            }
        };
        if let Err(error) = self.lifecycle.consume_exit(identity) {
            let reason = format!(
                "project lifecycle exit for `{}` could not be consumed: {error}",
                process.service_name
            );
            self.roll_back(manifest, &reason)?;
            return Err(error.into());
        }
        let reason = format!(
            "project service `{}` exited during readiness with status {:?}",
            process.service_name, exit.exit_code
        );
        mark_project_process_failed(self.store, manifest, &key, reason.clone())?;
        self.roll_back(manifest, &reason)?;
        Err(AzDaemonError::InvalidServicePlan {
            service: process.service_name,
            reason,
        })
    }

    /// Roll the wave back after the identity-bound exit wait itself failed.
    /// Always returns `Err`.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::InvalidServicePlan`] describing the failed
    /// wait, or the rollback failure that displaced it.
    fn fail_on_exit_wait_failure(
        &self,
        manifest: &mut project_services::ProjectServiceManifest,
        identity: ProcessIdentity,
        wait_reason: &str,
    ) -> Result<(), AzDaemonError> {
        if let Err(error) = self.lifecycle.consume_exit(identity) {
            let rollback_reason = format!(
                "project lifecycle wait failure for {identity:?} could not be consumed: {error}"
            );
            self.roll_back(manifest, &rollback_reason)?;
            return Err(error.into());
        }
        let reason = format!(
            "identity-bound project-service exit wait failed for {identity:?}: {wait_reason}"
        );
        if let Some(key) = self
            .pending
            .iter()
            .find(|pending| pending.spawned.identity == identity)
            .map(|pending| ServiceProcessKey::from_process(&pending.process))
            .or_else(|| {
                project_process_for_identity(manifest, identity)
                    .as_ref()
                    .map(ServiceProcessKey::from_process)
            })
        {
            mark_project_process_failed(self.store, manifest, &key, reason.clone())?;
        }
        self.roll_back(manifest, &reason)?;
        Err(AzDaemonError::InvalidServicePlan {
            service: self.project_id.to_string(),
            reason,
        })
    }

    /// Roll the wave back after the readiness deadline passed. Always returns
    /// `Err`.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::ProjectServiceReadyTimeout`] naming the first
    /// service that never reported ready, or the rollback failure that
    /// displaced it.
    ///
    /// # Panics
    ///
    /// Panics if every service is already ready; the caller only reaches the
    /// deadline arm while at least one is still pending.
    fn fail_on_timeout(
        &self,
        manifest: &mut project_services::ProjectServiceManifest,
        ready: &[bool],
    ) -> Result<(), AzDaemonError> {
        let timeout_ms = duration_millis_u64(self.ready_timeout);
        let pending = self
            .pending
            .iter()
            .enumerate()
            .find(|(index, _)| !ready[*index])
            .map(|(_, pending)| pending)
            .expect("readiness loop has one pending service");
        let key = ServiceProcessKey::from_spawned(&pending.spawned);
        let reason = format!("readiness timed out after {timeout_ms}ms");
        mark_project_process_failed(self.store, manifest, &key, reason.clone())?;
        self.roll_back(manifest, &reason)?;
        Err(AzDaemonError::ProjectServiceReadyTimeout {
            service: pending.process.service_name.clone(),
            timeout_ms,
        })
    }
}

/// Wait until every service spawned in this wave has published a valid ready
/// record, rolling the whole wave back on any failure.
///
/// # Errors
///
/// Returns [`AzDaemonError::ProjectServiceProcess`] when the readiness watcher
/// cannot be bound, closed, or waited on, plus any error the wave's
/// [`ProjectServiceWave::collect_ready`],
/// [`ProjectServiceWave::fail_on_process_exit`],
/// [`ProjectServiceWave::fail_on_exit_wait_failure`], or
/// [`ProjectServiceWave::fail_on_timeout`] steps return.
fn wait_for_project_service_wave<L, C, R>(
    wave: &ProjectServiceWave<'_, L, C, R>,
    manifest: &mut project_services::ProjectServiceManifest,
) -> Result<(), AzDaemonError>
where
    L: ServiceProcessLauncher<Error = az_service_supervision::ServiceProcessError>,
    C: ProjectServiceLifecycle,
    R: ProjectServiceReadyReader,
{
    if wave.pending.is_empty() {
        return Ok(());
    }

    let ready_paths = wave
        .pending
        .iter()
        .filter_map(|pending| pending.process.ready_file.clone())
        .collect::<Vec<_>>();
    let mut ready_subscription = match wave.lifecycle.subscribe_ready(&ready_paths) {
        Ok(subscription) => subscription,
        Err(error) => {
            let reason = format!("could not bind project-service readiness watcher: {error}");
            wave.roll_back(manifest, &reason)?;
            return Err(error.into());
        }
    };
    let mut ready = vec![false; wave.pending.len()];
    let deadline = Instant::now() + wave.ready_timeout;

    while ready.iter().any(|ready| !ready) {
        wave.collect_ready(manifest, &mut ready)?;

        if ready.iter().all(|ready| *ready) {
            if let Err(error) = ready_subscription.finish() {
                let reason = format!("project-service readiness watcher failed to close: {error}");
                wave.roll_back(manifest, &reason)?;
                return Err(error.into());
            }
            return Ok(());
        }

        let event = match wave.lifecycle.wait_until(deadline) {
            Ok(event) => event,
            Err(error) => {
                let reason = format!("project-service readiness wait failed: {error}");
                wave.roll_back(manifest, &reason)?;
                return Err(error.into());
            }
        };
        match event {
            Some(ServiceLifecycleEvent::ReadyFileChanged) => {}
            Some(ServiceLifecycleEvent::ProcessExited(identity)) => {
                return wave.fail_on_process_exit(manifest, identity);
            }
            Some(ServiceLifecycleEvent::ProcessExitWaitFailed { identity, reason }) => {
                return wave.fail_on_exit_wait_failure(manifest, identity, &reason);
            }
            None => return wave.fail_on_timeout(manifest, &ready),
        }
    }
    Ok(())
}

fn project_process_for_identity(
    manifest: &project_services::ProjectServiceManifest,
    identity: ProcessIdentity,
) -> Option<ServiceProcessRecord> {
    manifest
        .processes
        .iter()
        .find(|process| {
            process.pid == Some(identity.process_id)
                && process.process_start_time == Some(identity.process_start_time)
        })
        .cloned()
}

fn project_service_descriptor_for_process(
    manifest: &project_services::ProjectServiceManifest,
    process: &ServiceProcessRecord,
) -> Option<ServiceDescriptor> {
    manifest
        .services
        .iter()
        .find(|service| service.role == process.role)
        .map(ServiceRecord::to_descriptor)
}

fn validate_project_ready_record(
    process: &ServiceProcessRecord,
    pid: u32,
    ready: &ServiceReadyRecord,
) -> Result<(), AzDaemonError> {
    let invalid = |reason: String| AzDaemonError::InvalidServicePlan {
        service: process.service_name.clone(),
        reason,
    };
    if ready.schema_version != SERVICE_READY_SCHEMA_VERSION {
        return Err(invalid(format!(
            "readiness schema {} does not match {}",
            ready.schema_version, SERVICE_READY_SCHEMA_VERSION
        )));
    }
    if ready.service_name != process.service_name {
        return Err(invalid(format!(
            "readiness service `{}` does not match `{}`",
            ready.service_name, process.service_name
        )));
    }
    if ready.role != process.role {
        return Err(invalid(format!(
            "readiness role {:?} does not match {:?}",
            ready.role, process.role
        )));
    }
    if ready.pid.is_some_and(|ready_pid| ready_pid != pid) {
        return Err(invalid(format!(
            "readiness pid {:?} does not match spawned pid {pid}",
            ready.pid
        )));
    }
    if ready.endpoint_kind != process.endpoint_kind {
        return Err(invalid(format!(
            "readiness endpoint kind {:?} does not match {:?}",
            ready.endpoint_kind, process.endpoint_kind
        )));
    }
    match ready.endpoint_kind {
        az_service_supervision::ServiceEndpointKind::Tcp => {
            let planned = process
                .endpoint_address
                .parse::<std::net::SocketAddr>()
                .map_err(|error| invalid(format!("planned TCP endpoint is invalid: {error}")))?;
            let actual = ready
                .endpoint_address
                .parse::<std::net::SocketAddr>()
                .map_err(|error| invalid(format!("readiness TCP endpoint is invalid: {error}")))?;
            if actual.port() == 0
                || (planned.port() == 0 && planned.ip() != actual.ip())
                || (planned.port() != 0 && planned != actual)
            {
                return Err(invalid(format!(
                    "readiness TCP endpoint `{actual}` does not satisfy planned `{planned}`"
                )));
            }
        }
        az_service_supervision::ServiceEndpointKind::WindowsNamedPipe
        | az_service_supervision::ServiceEndpointKind::UnixDomainSocket => {
            if ready.endpoint_address != process.endpoint_address {
                return Err(invalid(format!(
                    "readiness endpoint `{}` does not match planned `{}`",
                    ready.endpoint_address, process.endpoint_address
                )));
            }
        }
        az_service_supervision::ServiceEndpointKind::InProcess => {
            return Err(invalid(
                "in-process endpoints cannot cross the project supervision boundary".to_string(),
            ));
        }
    }
    Ok(())
}

fn project_service_process_key(
    command: &ProjectServiceCommand,
) -> Result<ServiceProcessKey, AzDaemonError> {
    ServiceProcessKey::from_proto(&command.service_name, command.role).ok_or_else(|| {
        AzDaemonError::InvalidServicePlan {
            service: command.service_name.clone(),
            reason: format!(
                "role {:?} cannot own a supervised service process",
                command.role
            ),
        }
    })
}

fn commit_project_service_ready(
    store: &project_services::ProjectServiceStore,
    manifest: &mut project_services::ProjectServiceManifest,
    process: &ServiceProcessRecord,
    identity: ProcessIdentity,
    ready: &ServiceReadyRecord,
) -> Result<(), AzDaemonError> {
    let now = u128::from(current_unix_ms());
    // `ServiceProcessRecord` has no `name` field, so clippy's suggested
    // `service.name == process.name` would not compile.
    #[allow(clippy::suspicious_operation_groupings)]
    let service_index = manifest
        .services
        .iter()
        .position(|service| service.name == process.service_name && service.role == process.role)
        .ok_or_else(|| AzDaemonError::InvalidServicePlan {
            service: process.service_name.clone(),
            reason: "project-service descriptor disappeared during readiness".to_string(),
        })?;
    let current = manifest
        .current_process_mut(&ServiceProcessKey::from_process(process))
        .ok_or_else(|| AzDaemonError::InvalidServicePlan {
            service: process.service_name.clone(),
            reason: "current project-service process disappeared during readiness".to_string(),
        })?;
    current.mark_running(identity, now)?;
    current.endpoint_address.clone_from(&ready.endpoint_address);
    let service = &mut manifest.services[service_index];
    service.endpoint_address.clone_from(&ready.endpoint_address);
    service.observability_endpoint_kind = ready.observability_endpoint_kind;
    service
        .observability_endpoint_address
        .clone_from(&ready.observability_endpoint_address);
    service.lifecycle_endpoint_kind = ready.lifecycle_endpoint_kind;
    service
        .lifecycle_endpoint_address
        .clone_from(&ready.lifecycle_endpoint_address);
    manifest.updated_unix_ms = now;
    store.write(manifest)?;
    Ok(())
}

fn mark_project_process_failed(
    store: &project_services::ProjectServiceStore,
    manifest: &mut project_services::ProjectServiceManifest,
    key: &ServiceProcessKey,
    reason: String,
) -> Result<(), AzDaemonError> {
    if let Some(process) = manifest.current_process_mut(key) {
        process.mark_exited(None, Some(reason), u128::from(current_unix_ms()));
    }
    manifest.updated_unix_ms = u128::from(current_unix_ms());
    store.write(manifest)?;
    Ok(())
}

fn rollback_project_service_starts<L, C>(
    store: &project_services::ProjectServiceStore,
    launcher: &L,
    lifecycle: &C,
    manifest: &mut project_services::ProjectServiceManifest,
    started: &[SpawnedServiceProcess],
    reason: &str,
) -> Result<(), AzDaemonError>
where
    L: ServiceProcessLauncher<Error = az_service_supervision::ServiceProcessError>,
    C: ProjectServiceLifecycle,
{
    for spawned in started.iter().rev() {
        let key = ServiceProcessKey::from_spawned(spawned);
        let exit_code = launcher.terminate(&key)?.and_then(|exit| exit.exit_code);
        lifecycle.retire_exit(spawned.identity)?;
        if let Some(process) = manifest.current_process_mut(&key) {
            process.mark_exited(
                exit_code,
                Some(reason.to_string()),
                u128::from(current_unix_ms()),
            );
        }
    }
    manifest.updated_unix_ms = u128::from(current_unix_ms());
    store.write(manifest)?;
    Ok(())
}

fn apply_session_endpoint_layout(
    commands: &mut [ProjectServiceCommand],
    manifest: &az_session::SessionManifest,
    endpoint_kind: EndpointKind,
) -> Result<(), AzDaemonError> {
    match endpoint_kind {
        EndpointKind::UnixDomainSocket => {
            let ipc_dir = az_session::session_ipc_dir(manifest.id)?;
            for command in commands
                .iter_mut()
                .filter(|command| command.role == ServiceRole::RuntimeHost)
            {
                let endpoint = ipc_dir
                    .join(format!("{}.sock", endpoint_token(&command.service_name)))
                    .to_string_lossy()
                    .into_owned();
                command.endpoint = Endpoint::new(EndpointKind::UnixDomainSocket, endpoint);
            }
        }
        EndpointKind::WindowsNamedPipe => {
            let project_token = endpoint_token(&manifest.project_id);
            let session_token = endpoint_token(&manifest.id.to_string());
            for command in commands
                .iter_mut()
                .filter(|command| command.role == ServiceRole::RuntimeHost)
            {
                let service_token = endpoint_token(&command.service_name);
                let endpoint =
                    format!(r"\\.\pipe\azoth-{project_token}-{session_token}-{service_token}");
                command.endpoint = Endpoint::new(EndpointKind::WindowsNamedPipe, endpoint);
            }
        }
        EndpointKind::Tcp => {}
        EndpointKind::InProcess => {
            validate_public_endpoint_kind(endpoint_kind, "azd project session endpoint layout")?;
        }
    }

    Ok(())
}

/// Attach the planned project-service descriptors to the session, then record
/// the runtime-host launch commands, and hand back the resulting manifest.
///
/// A plan with no runtime-host command records nothing and simply reads the
/// session back, so an unchanged session is not rewritten.
fn attach_and_record_session_services(
    manager: &az_session::SessionManager,
    session_slug: &str,
    services: &[ServiceRecord],
    commands: &[ProjectServiceCommand],
    otlp_endpoint: Option<&str>,
) -> Result<az_session::SessionManifest, AzDaemonError> {
    let attached_descriptors = services
        .iter()
        .map(ServiceRecord::to_descriptor)
        .collect::<Vec<_>>();
    manager.attach_project_service_descriptors(session_slug, &attached_descriptors)?;
    let launch_commands = commands
        .iter()
        .filter(|command| command.role == ServiceRole::RuntimeHost)
        .map(session_launch_command)
        .collect::<Vec<_>>();
    if launch_commands.is_empty() {
        return Ok(manager.session(session_slug)?);
    }
    let launch_context =
        az_session::SessionServiceLaunchContext::new().with_otlp_endpoint(otlp_endpoint);
    Ok(manager.record_planned_services_with_context(
        session_slug,
        &launch_commands,
        &launch_context,
    )?)
}

fn validate_planned_service_owner_context(
    commands: &[ProjectServiceCommand],
) -> Result<(), AzDaemonError> {
    for command in commands {
        if command.owner_id.trim().is_empty() {
            return Err(AzDaemonError::InvalidServicePlan {
                service: command.service_name.clone(),
                reason: "missing daemon-owned owner id".to_string(),
            });
        }
        if command.owner_root.trim().is_empty() {
            return Err(AzDaemonError::InvalidServicePlan {
                service: command.service_name.clone(),
                reason: "missing daemon-owned owner root".to_string(),
            });
        }
        if !Path::new(&command.owner_root).is_absolute() {
            return Err(AzDaemonError::InvalidServicePlan {
                service: command.service_name.clone(),
                reason: format!("owner root `{}` is not absolute", command.owner_root),
            });
        }
        if command.build_output_root.trim().is_empty() {
            return Err(AzDaemonError::InvalidServicePlan {
                service: command.service_name.clone(),
                reason: "missing daemon-resolved build output root".to_string(),
            });
        }
        if !Path::new(&command.build_output_root).is_absolute() {
            return Err(AzDaemonError::InvalidServicePlan {
                service: command.service_name.clone(),
                reason: format!(
                    "build output root `{}` is not absolute",
                    command.build_output_root
                ),
            });
        }
    }
    Ok(())
}

fn requested_service_names(
    requested: &[String],
    prepared: &[String],
) -> Result<Vec<String>, AzDaemonError> {
    requested_service_names_with_dependencies(requested, prepared, project_service_dependencies)
}

fn requested_service_names_with_dependencies(
    requested: &[String],
    prepared: &[String],
    dependencies: impl Fn(&str) -> &'static [&'static str],
) -> Result<Vec<String>, AzDaemonError> {
    if prepared.is_empty() {
        return Err(AzDaemonError::InvalidServicePlan {
            service: "session-services".to_string(),
            reason: "prepared service list cannot be empty".to_string(),
        });
    }
    if requested.is_empty() {
        return Ok(prepared.to_vec());
    }

    let prepared_set = prepared.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let mut selected = BTreeSet::new();
    let mut visiting = Vec::new();
    for service_name in requested {
        let service_name = service_name.trim();
        if service_name.is_empty() {
            return Err(AzDaemonError::InvalidServicePlan {
                service: "session-services".to_string(),
                reason: "requested service name cannot be empty".to_string(),
            });
        }
        if !prepared_set.contains(service_name) {
            return Err(AzDaemonError::InvalidServicePlan {
                service: service_name.to_string(),
                reason: "requested service is not part of the prepared session plan".to_string(),
            });
        }
        collect_service_dependencies(
            service_name,
            &prepared_set,
            &dependencies,
            &mut visiting,
            &mut selected,
        )?;
    }
    Ok(prepared
        .iter()
        .filter(|service_name| selected.contains(service_name.as_str()))
        .cloned()
        .collect())
}

fn collect_service_dependencies(
    service_name: &str,
    prepared: &BTreeSet<&str>,
    dependencies: &impl Fn(&str) -> &'static [&'static str],
    visiting: &mut Vec<String>,
    selected: &mut BTreeSet<String>,
) -> Result<(), AzDaemonError> {
    if selected.contains(service_name) {
        return Ok(());
    }
    if let Some(cycle_start) = visiting.iter().position(|name| name == service_name) {
        let mut cycle = visiting[cycle_start..].to_vec();
        cycle.push(service_name.to_string());
        return Err(AzDaemonError::InvalidServicePlan {
            service: service_name.to_string(),
            reason: format!("service dependency cycle: {}", cycle.join(" -> ")),
        });
    }

    visiting.push(service_name.to_string());
    for dependency in dependencies(service_name) {
        if !prepared.contains(dependency) {
            return Err(AzDaemonError::InvalidServicePlan {
                service: service_name.to_string(),
                reason: format!("required dependency `{dependency}` is not planned"),
            });
        }
        collect_service_dependencies(dependency, prepared, dependencies, visiting, selected)?;
    }
    let popped = visiting.pop();
    debug_assert_eq!(popped.as_deref(), Some(service_name));
    selected.insert(service_name.to_string());
    Ok(())
}

fn project_service_dependencies(service_name: &str) -> &'static [&'static str] {
    match service_name {
        "project-host" | "asset-worker" => &["asset-processor"],
        "runtime-host" => &["project-host", "asset-processor"],
        _ => &[],
    }
}

fn project_service_names(plan: &ProjectServicePlan) -> Vec<String> {
    plan.commands
        .iter()
        .map(|command| command.service_name.clone())
        .collect()
}

fn retain_project_service_plan_services(
    plan: &mut ProjectServicePlan,
    service_names: &[String],
) -> Result<(), AzDaemonError> {
    if service_names.is_empty() {
        return Err(AzDaemonError::InvalidServicePlan {
            service: "session-services".to_string(),
            reason: "resolved service selection cannot be empty".to_string(),
        });
    }

    let service_names = service_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    plan.commands
        .retain(|command| service_names.contains(command.service_name.as_str()));
    plan.build_commands
        .retain(|command| service_names.contains(command.target_name.as_str()));
    // Some executable services are prebuilt engine host tools. In particular,
    // a primary-gem project's asset-processor command intentionally has no
    // project Cargo build command. The launch command is the executable plan;
    // the build-command set may legitimately become empty after selection.
    if plan.commands.is_empty() {
        return Err(AzDaemonError::InvalidServicePlan {
            service: "session-services".to_string(),
            reason: "resolved service selection retained no executable service plan".to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessiondLaunchCommand {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
}

fn daemon_registered_sessiond_launch_command(
    project_path: &Path,
    session: &str,
    endpoint_kind: EndpointKind,
    daemon_endpoint: &Endpoint,
    keep_alive: bool,
    service_ready_timeout_ms: u64,
    start_service_names: &[String],
) -> Result<SessiondLaunchCommand, AzDaemonError> {
    let bundle = HostToolBundle::current()?;
    daemon_registered_sessiond_launch_command_from_bundle(
        &bundle,
        project_path,
        session,
        endpoint_kind,
        daemon_endpoint,
        keep_alive,
        service_ready_timeout_ms,
        start_service_names,
    )
}

#[allow(clippy::too_many_arguments)]
fn daemon_registered_sessiond_launch_command_from_bundle(
    bundle: &HostToolBundle,
    project_path: &Path,
    session: &str,
    endpoint_kind: EndpointKind,
    daemon_endpoint: &Endpoint,
    keep_alive: bool,
    service_ready_timeout_ms: u64,
    start_service_names: &[String],
) -> Result<SessiondLaunchCommand, AzDaemonError> {
    let sessiond = bundle.resolve(HostTool::SessionSupervisor)?;
    sessiond_launch_command_with_executable(
        &sessiond,
        project_path,
        session,
        endpoint_kind,
        Some(daemon_endpoint),
        keep_alive,
        service_ready_timeout_ms,
        start_service_names,
    )
}

#[allow(clippy::too_many_arguments)]
fn sessiond_launch_command_with_executable(
    sessiond_executable: &Path,
    project_path: &Path,
    session: &str,
    endpoint_kind: EndpointKind,
    daemon_endpoint: Option<&Endpoint>,
    keep_alive: bool,
    service_ready_timeout_ms: u64,
    start_service_names: &[String],
) -> Result<SessiondLaunchCommand, AzDaemonError> {
    let project_path = if project_path.is_absolute() {
        project_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(project_path)
    };
    let sessiond_args = sessiond_args(
        &project_path,
        session,
        endpoint_kind,
        daemon_endpoint,
        keep_alive,
        service_ready_timeout_ms,
        start_service_names,
    )?;
    Ok(SessiondLaunchCommand {
        program: sessiond_executable.to_string_lossy().into_owned(),
        args: sessiond_args,
        cwd: project_path,
    })
}

fn sessiond_args(
    project_path: &Path,
    session: &str,
    endpoint_kind: EndpointKind,
    daemon_endpoint: Option<&Endpoint>,
    keep_alive: bool,
    service_ready_timeout_ms: u64,
    start_service_names: &[String],
) -> Result<Vec<String>, AzDaemonError> {
    let mut args = vec![
        "--project".to_string(),
        project_path.to_string_lossy().into_owned(),
        "--session".to_string(),
        session.to_string(),
        "--endpoint-kind".to_string(),
        endpoint_kind_arg(endpoint_kind)?.to_string(),
        "--service-ready-timeout-ms".to_string(),
        service_ready_timeout_ms.to_string(),
    ];
    if let Some(daemon_endpoint) = daemon_endpoint {
        args.extend([
            "--daemon-endpoint-kind".to_string(),
            endpoint_kind_arg(daemon_endpoint.kind)?.to_string(),
            "--daemon-endpoint".to_string(),
            daemon_endpoint.address.clone(),
        ]);
    } else {
        args.push("--no-daemon-registration".to_string());
    }
    if keep_alive {
        args.push("--keep-alive".to_string());
    }
    for service_name in start_service_names {
        args.extend(["--start-service".to_string(), service_name.clone()]);
    }
    Ok(args)
}

fn spawn_sessiond_process(
    command: &SessiondLaunchCommand,
    manifest: &az_session::SessionManifest,
) -> Result<Child, AzDaemonError> {
    let log_path = az_session::sessiond_output_log_path(manifest);
    rotate_sessiond_launch_logs(manifest)?;
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|source| AzDaemonError::SessiondLogOpen {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let stdout = open_sessiond_log(&log_path).and_then(|stdout| {
        let stderr = stdout
            .try_clone()
            .map_err(|source| AzDaemonError::SessiondLogOpen {
                path: log_path.clone(),
                source,
            })?;
        Ok((stdout, stderr))
    })?;

    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.0))
        .stderr(Stdio::from(stdout.1));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    process
        .spawn()
        .map_err(|source| AzDaemonError::SessiondSpawn {
            program: command.program.clone(),
            source,
        })
}

fn rotate_sessiond_launch_logs(
    manifest: &az_session::SessionManifest,
) -> Result<(), AzDaemonError> {
    rotate_log_at_plan_time(&az_session::sessiond_output_log_path(manifest))
        .map_err(AzDaemonError::SessiondLogRotate)?;
    rotate_log_at_plan_time(&az_session::sessiond_structured_log_path(manifest))
        .map_err(AzDaemonError::SessiondLogRotate)?;
    Ok(())
}

fn open_sessiond_log(path: &Path) -> Result<File, AzDaemonError> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| AzDaemonError::SessiondLogOpen {
            path: path.to_path_buf(),
            source,
        })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the wait joins eight independently owned things for one spawned sessiond: the daemon that brokers registrations, the project id and session slug being waited on, the session manager that opens the lease store, the live child handle, the launch command echoed back in SessiondExited, the deadline, and the log path the timeout error points at. They are owned by different layers and are never carried together anywhere else, so a struct would only be a bag with this call as its sole member"
)]
fn wait_for_session_supervisor_start(
    daemon: &AzDaemon,
    project_id: &str,
    session_slug: &str,
    manager: &az_session::SessionManager,
    child: &mut Child,
    command: &SessiondLaunchCommand,
    timeout_ms: u64,
    log_path: &Path,
) -> Result<ServiceDescriptor, AzDaemonError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let Some(expected_process) = capture_process_identity(child.id())? else {
        return Err(AzDaemonError::SessiondExited {
            program: command.program.clone(),
            args: command.args.join(" "),
            cwd: command.cwd.clone(),
            status: child.try_wait()?.and_then(|status| status.code()),
        });
    };
    let lease_store =
        az_session::SessionSupervisorLeaseStore::new(manager.session(session_slug)?.run_dir);
    let (current, registrations) =
        daemon.subscribe_session_supervisor_registration(project_id, session_slug);
    let lifecycle = ServiceLifecycleEvents::new();
    lifecycle.add_identity(expected_process)?;

    if let Some(descriptor) = current
        && let Some(descriptor) = validate_spawned_session_supervisor_registration(
            manager,
            session_slug,
            &lease_store,
            expected_process,
            &descriptor,
        )?
    {
        return Ok(descriptor);
    }

    loop {
        let mut select = channel::Select::new();
        let registration_index = select.recv(&registrations);
        let exit_index = select.recv(lifecycle.receiver());
        let Ok(operation) = select.select_deadline(deadline) else {
            let _ = child.kill();
            return Err(AzDaemonError::SessionServicesStartTimedOut {
                session: session_slug.to_string(),
                timeout_ms,
                log_path: log_path.to_path_buf(),
            });
        };
        if operation.index() == registration_index {
            let (key, descriptor) = operation.recv(&registrations).map_err(|_| {
                az_service_supervision::ServiceProcessError::LifecycleEventSourceClosed
            })?;
            if key != SessionSupervisorKey::new(project_id, session_slug) {
                continue;
            }
            if let Some(descriptor) = validate_spawned_session_supervisor_registration(
                manager,
                session_slug,
                &lease_store,
                expected_process,
                &descriptor,
            )? {
                return Ok(descriptor);
            }
        } else if operation.index() == exit_index {
            let event = operation.recv(lifecycle.receiver()).map_err(|_| {
                az_service_supervision::ServiceProcessError::LifecycleEventSourceClosed
            })?;
            match event {
                ServiceLifecycleEvent::ProcessExited(identity) if identity == expected_process => {
                    lifecycle.consume_exit(identity)?;
                    return Err(AzDaemonError::SessiondExited {
                        program: command.program.clone(),
                        args: command.args.join(" "),
                        cwd: command.cwd.clone(),
                        status: child.try_wait()?.and_then(|status| status.code()),
                    });
                }
                ServiceLifecycleEvent::ProcessExitWaitFailed { identity, reason }
                    if identity == expected_process =>
                {
                    lifecycle.consume_exit(identity)?;
                    return Err(AzDaemonError::InvalidServicePlan {
                        service: session_slug.to_string(),
                        reason: format!(
                            "sessiond process-exit wait failed for {identity:?}: {reason}"
                        ),
                    });
                }
                event => {
                    return Err(AzDaemonError::InvalidServicePlan {
                        service: session_slug.to_string(),
                        reason: format!(
                            "sessiond startup observed unrelated lifecycle event {event:?}"
                        ),
                    });
                }
            }
        }
    }
}

fn validate_spawned_session_supervisor_registration(
    manager: &az_session::SessionManager,
    session_slug: &str,
    lease_store: &az_session::SessionSupervisorLeaseStore,
    expected_process: ProcessIdentity,
    registered: &ServiceDescriptor,
) -> Result<Option<ServiceDescriptor>, AzDaemonError> {
    let Some(lease) = lease_store.load()?.record else {
        return Ok(None);
    };
    if lease.process != expected_process {
        return Ok(None);
    }
    let descriptor = lease.descriptor();
    if !descriptor.has_same_connection_contract(registered) {
        return Err(AzDaemonError::InvalidSessionSupervisorDescriptor {
            reason: "azd registration did not match the spawned session-supervisor lease"
                .to_string(),
        });
    }
    challenge_session_supervisor_lease(&manager.session(session_slug)?, &lease)?;
    info!(
        session = %session_slug,
        run = %descriptor.run,
        process_id = lease.process.process_id,
        endpoint = %descriptor.endpoint.address,
        "session-supervisor registration observed from spawned az-sessiond"
    );
    Ok(Some(descriptor))
}

struct DaemonSessionSupervisorEventSink {
    events: tokio::sync::mpsc::UnboundedSender<SessionSupervisorEvent>,
}

// capnp-rpc keeps its connection state behind `Rc<RefCell<..>>`, so the
// `Rc<Self>` receiver and `Params` hook make this handler's future `!Send` by
// construction; it is only ever polled on the connection's own `LocalSet`.
#[allow(clippy::future_not_send)]
impl session_capnp::session_supervisor_event_sink::Server for DaemonSessionSupervisorEventSink {
    async fn update(
        self: capnp::capability::Rc<Self>,
        params: session_capnp::session_supervisor_event_sink::UpdateParams,
        _results: session_capnp::session_supervisor_event_sink::UpdateResults,
    ) -> Result<(), capnp::Error> {
        let event = SessionSupervisorEvent::from_capnp(params.get()?.get_event()?)?;
        self.events
            .send(event)
            .map_err(|_| capnp::Error::failed("azd session status receiver closed".to_string()))
    }
}

fn wait_for_session_services_running(
    manifest: &az_session::SessionManifest,
    descriptor: &ServiceDescriptor,
    service_names: &[String],
    timeout_ms: u64,
    log_path: &Path,
) -> Result<ProtoSessionManifest, AzDaemonError> {
    let manifest = manifest.clone();
    let descriptor = descriptor.clone();
    let service_names = service_names.to_vec();
    let log_path = log_path.to_path_buf();
    let session = manifest.slug.clone();
    run_session_supervisor_rpc(session, "subscribeEvents", move || async move {
        let (subscription, mut event_rx) =
            subscribe_session_supervisor_status(&descriptor, &manifest).await?;
        await_session_services_running(
            &manifest,
            &service_names,
            timeout_ms,
            log_path,
            subscription,
            &mut event_rx,
        )
        .await
    })
}

/// Open a session-supervisor status subscription and return its accepted
/// initial state plus the stream of later status events.
///
/// # Errors
///
/// Returns [`AzDaemonError::SessionSupervisorRpc`] when the supervisor cannot
/// be reached, the subscribe call cannot be encoded, sent, or decoded, or the
/// supervisor declines the subscription.
// capnp-rpc keeps its connection state behind `Rc<RefCell<..>>`, so the client
// handle held across the subscribe await is `!Send` by construction.
#[allow(clippy::future_not_send)]
async fn subscribe_session_supervisor_status(
    descriptor: &ServiceDescriptor,
    manifest: &az_session::SessionManifest,
) -> Result<
    (
        SessionSupervisorEventSubscriptionResult,
        tokio::sync::mpsc::UnboundedReceiver<SessionSupervisorEvent>,
    ),
    AzDaemonError,
> {
    let client = connect_session_supervisor(descriptor, manifest, "subscribeEvents").await?;
    let (events, event_rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = capnp_rpc::new_client(DaemonSessionSupervisorEventSink { events });
    let mut request = client.subscribe_events_request();
    {
        let mut params = request.get();
        SessionSupervisorEventSubscriptionRequest {
            capability: daemon_session_capability(
                descriptor,
                manifest.id,
                &[SESSION_READ_PERMISSION],
            )?,
            slug: manifest.slug.clone(),
        }
        .to_capnp(params.reborrow().init_request())
        .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "subscribeEvents", error))?;
        params.set_sink(sink);
    }
    let response =
        request.send().promise.await.map_err(|error| {
            session_supervisor_rpc_error(&manifest.slug, "subscribeEvents", error)
        })?;
    let subscription = SessionSupervisorEventSubscriptionResult::from_capnp(
        response
            .get()
            .map_err(|error| {
                session_supervisor_rpc_error(&manifest.slug, "subscribeEvents", error)
            })?
            .get_result()
            .map_err(|error| {
                session_supervisor_rpc_error(&manifest.slug, "subscribeEvents", error)
            })?,
    )
    .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "subscribeEvents", error))?;
    if !subscription.subscribed {
        return Err(session_supervisor_rpc_error(
            &manifest.slug,
            "subscribeEvents",
            "session-supervisor declined readiness subscription",
        ));
    }
    Ok((subscription, event_rx))
}

/// Fold status events until every requested service reports running.
///
/// # Errors
///
/// Returns [`AzDaemonError::SessionSupervisorRpc`] when the status identity
/// changes mid-wait, the subscription closes, or the status sequence regresses;
/// [`AzDaemonError::SessionServiceNotRunning`] when a service settles in a
/// terminal non-running state; and
/// [`AzDaemonError::SessionServicesStartTimedOut`] when `timeout_ms` elapses
/// first.
async fn await_session_services_running(
    manifest: &az_session::SessionManifest,
    service_names: &[String],
    timeout_ms: u64,
    log_path: PathBuf,
    subscription: SessionSupervisorEventSubscriptionResult,
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SessionSupervisorEvent>,
) -> Result<ProtoSessionManifest, AzDaemonError> {
    let mut sequence = subscription.initial_sequence;
    let mut status = subscription.initial_status;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if status.manifest.id != manifest.id.0
            || status.manifest.slug != manifest.slug
            || status.manifest.project_id != manifest.project_id
        {
            return Err(session_supervisor_rpc_error(
                &manifest.slug,
                "subscribeEvents",
                "session status identity changed while waiting for readiness",
            ));
        }
        match first_unready_session_service(&status.manifest, service_names, None) {
            None => return Ok(status.manifest),
            Some(blocker) if blocker.terminal => {
                return Err(AzDaemonError::SessionServiceNotRunning {
                    session: status.manifest.slug,
                    service: blocker.service,
                    state: blocker.state,
                });
            }
            Some(_) => {}
        }
        let event = tokio::select! {
            event = event_rx.recv() => event.ok_or_else(|| {
                session_supervisor_rpc_error(
                    &manifest.slug,
                    "subscribeEvents",
                    "session status subscription closed before readiness",
                )
            })?,
            () = tokio::time::sleep_until(deadline) => {
                return Err(AzDaemonError::SessionServicesStartTimedOut {
                    session: manifest.slug.clone(),
                    timeout_ms,
                    log_path,
                });
            }
        };
        if event.sequence <= sequence {
            return Err(session_supervisor_rpc_error(
                &manifest.slug,
                "subscribeEvents",
                format!(
                    "session status sequence regressed from {sequence} to {}",
                    event.sequence
                ),
            ));
        }
        sequence = event.sequence;
        status = event.status;
    }
}

fn session_services_are_running(manifest: &ProtoSessionManifest, service_names: &[String]) -> bool {
    service_names.iter().all(|service_name| {
        current_process_for_service(manifest, service_name)
            .is_some_and(|process| service_process_ready_state(process, None).is_none())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceProgramFreshnessPolicy {
    Verify,
    TrustPrebuilt,
}

impl ServiceProgramFreshnessPolicy {
    const fn verifies_program(self) -> bool {
        matches!(self, Self::Verify)
    }
}

fn session_services_have_reusable_launch_plan(
    manifest: &ProtoSessionManifest,
    service_names: &[String],
    commands: &[ProjectServiceCommand],
    program_freshness: ServiceProgramFreshnessPolicy,
) -> bool {
    service_names.iter().all(|service_name| {
        let Some(command) = commands
            .iter()
            .find(|command| command.service_name == *service_name)
        else {
            return false;
        };

        current_process_for_service(manifest, service_name).is_some_and(|process| {
            service_process_matches_expected_launch_plan(
                manifest,
                process,
                command,
                program_freshness,
            )
        })
    })
}

fn session_services_have_persisted_reusable_launch_plan(
    manifest: &az_session::SessionManifest,
    service_names: &[String],
    commands: &[ProjectServiceCommand],
    program_freshness: ServiceProgramFreshnessPolicy,
) -> bool {
    let manifest = az_session::session_manifest_to_proto(manifest);
    session_services_have_reusable_launch_plan(
        &manifest,
        service_names,
        commands,
        program_freshness,
    )
}

fn project_services_have_reusable_launch_plan(
    manifest: &project_services::ProjectServiceManifest,
    commands: &[ProjectServiceCommand],
    program_freshness: ServiceProgramFreshnessPolicy,
) -> bool {
    commands
        .iter()
        .filter(|command| command.role != ServiceRole::RuntimeHost)
        .all(|command| {
            ServiceProcessKey::from_proto(&command.service_name, command.role)
                .and_then(|key| manifest.current_process(&key))
                .is_some_and(|process| {
                    project_service_process_matches_expected_launch_plan(
                        manifest,
                        process,
                        command,
                        program_freshness,
                    )
                })
        })
}

// `ServiceProcessRecord` has no `name` field, so clippy's suggested
// `descriptor.name == process.name` would not compile.
#[allow(clippy::suspicious_operation_groupings)]
fn project_service_process_matches_expected_launch_plan(
    manifest: &project_services::ProjectServiceManifest,
    process: &ServiceProcessRecord,
    command: &ProjectServiceCommand,
    program_freshness: ServiceProgramFreshnessPolicy,
) -> bool {
    matches!(
        process.state,
        ServiceProcessState::Planned
            | ServiceProcessState::Starting
            | ServiceProcessState::Running
            | ServiceProcessState::Exited
    ) && process.service_name == command.service_name
        && process.role.to_proto() == command.role
        && process.owner_id == command.owner_id
        && paths_equal(&process.owner_root, Path::new(&command.owner_root))
        && project_service_process_endpoint_matches_expected_launch(process, command)
        && path_strings_equal(&process.program, &command.program)
        && Path::new(&process.program).is_file()
        && service_program_artifact_matches_current(
            &process.service_name,
            &process.program,
            process.program_artifact.as_ref(),
            process.state == ServiceProcessState::Planned && process.started_unix_ms.is_none(),
        )
        && (!program_freshness.verifies_program()
            || service_program_is_current(
                &process.service_name,
                &process.program,
                &process.cwd,
                &process.owner_root,
            ))
        && paths_equal(&process.cwd, &manifest.project_root)
        && service_launch_unmanaged_args(&process.args)
            == service_launch_unmanaged_args(&command.args)
        && manifest.services.iter().any(|descriptor| {
            descriptor.name == process.service_name
                && descriptor.role == process.role
                && descriptor.endpoint_kind == process.endpoint_kind
                && descriptor.endpoint_address == process.endpoint_address
        })
}

fn project_service_process_endpoint_matches_expected_launch(
    process: &ServiceProcessRecord,
    command: &ProjectServiceCommand,
) -> bool {
    let endpoint = Endpoint::new(process.endpoint_kind.to_proto(), &process.endpoint_address);
    endpoint == command.endpoint
}

fn service_process_matches_expected_launch_plan(
    manifest: &ProtoSessionManifest,
    process: &az_proto_session::ServiceProcessRecord,
    command: &ProjectServiceCommand,
    program_freshness: ServiceProgramFreshnessPolicy,
) -> bool {
    matches!(
        process.state,
        ProtoServiceProcessState::Planned
            | ProtoServiceProcessState::Starting
            | ProtoServiceProcessState::Running
            | ProtoServiceProcessState::Exited
    ) && process.service_name == command.service_name
        && process.role == command.role
        && process.owner_id == command.owner_id
        && path_strings_equal(&process.owner_root, &command.owner_root)
        && service_process_endpoint_matches_expected_launch(process, command)
        && path_strings_equal(&process.program, &command.program)
        && Path::new(&process.program).is_file()
        && session_service_program_artifact_matches_current(process)
        && (!program_freshness.verifies_program() || service_process_program_is_current(process))
        && path_strings_equal(&process.cwd, &manifest.workspace_root)
        && service_launch_unmanaged_args(&process.args)
            == service_launch_unmanaged_args(&command.args)
        && manifest.services.iter().any(|descriptor| {
            descriptor.id.name == process.service_name
                && descriptor.role == process.role
                && descriptor.endpoint == process.endpoint
        })
}

fn service_process_program_is_current(process: &az_proto_session::ServiceProcessRecord) -> bool {
    service_program_is_current(
        &process.service_name,
        &process.program,
        Path::new(&process.cwd),
        Path::new(&process.owner_root),
    )
}

fn session_service_program_artifact_matches_current(
    process: &az_proto_session::ServiceProcessRecord,
) -> bool {
    let recorded = process.program_artifact.as_ref().map(|artifact| {
        az_service_supervision::ServiceProgramArtifact {
            byte_length: artifact.byte_length,
            modified_unix_ns: artifact.modified_unix_ns,
            file_system_id: artifact.file_system_id,
            file_id: artifact.file_id,
        }
    });
    service_program_artifact_matches_current(
        &process.service_name,
        &process.program,
        recorded.as_ref(),
        process.state == ProtoServiceProcessState::Planned && process.started_unix_ms.is_none(),
    )
}

fn service_program_artifact_matches_current(
    service_name: &str,
    program: &str,
    recorded: Option<&az_service_supervision::ServiceProgramArtifact>,
    unlaunched_plan: bool,
) -> bool {
    let Some(recorded) = recorded else {
        return unlaunched_plan;
    };
    match az_service_supervision::ServiceProgramArtifact::capture(Path::new(program)) {
        Ok(current) if current == *recorded => true,
        Ok(_) => {
            info!(
                service = service_name,
                program, "project service executable artifact changed; a new launch is required"
            );
            false
        }
        Err(error) => {
            warn!(
                error = %error,
                service = service_name,
                program,
                "project service executable artifact could not be identified; a new launch is required"
            );
            false
        }
    }
}

fn service_program_is_current(
    service_name: &str,
    program: &str,
    cwd: &Path,
    owner_root: &Path,
) -> bool {
    let check_started = Instant::now();
    let program = Path::new(program);
    let program_modified = match fs::metadata(program).and_then(|metadata| metadata.modified()) {
        Ok(modified) => modified,
        Err(error) => {
            warn!(
                error = %error,
                service = service_name,
                program = %program.display(),
                build_check_ms = duration_millis_u64(check_started.elapsed()),
                "project service binary freshness could not be read; rebuild required"
            );
            return false;
        }
    };

    match newest_service_build_input_mtime(program, cwd, owner_root) {
        Ok(Some(newest_input)) if newest_input > program_modified => {
            info!(
                service = service_name,
                program = %program.display(),
                build_check_ms = duration_millis_u64(check_started.elapsed()),
                "project service binary is older than source inputs; rebuild required"
            );
            false
        }
        Ok(_) => {
            info!(
                service = service_name,
                program = %program.display(),
                build_check_ms = duration_millis_u64(check_started.elapsed()),
                "project service binary freshness verified"
            );
            true
        }
        Err(error) => {
            warn!(
                error = %error,
                service = service_name,
                program = %program.display(),
                build_check_ms = duration_millis_u64(check_started.elapsed()),
                "project service source freshness could not be verified; rebuild required"
            );
            false
        }
    }
}

fn newest_service_build_input_mtime(
    program: &Path,
    cwd: &Path,
    owner_root: &Path,
) -> Result<Option<SystemTime>, std::io::Error> {
    let mut newest = None;

    for root in [cwd, owner_root] {
        collect_file_mtime(&project_manifest_path(root), &mut newest)?;
        collect_file_mtime(&project_lock_path(root), &mut newest)?;
        collect_file_mtime(&root.join("Cargo.toml"), &mut newest)?;
        collect_file_mtime(&root.join("Cargo.lock"), &mut newest)?;
        collect_file_mtime(&root.join(".cargo").join("config"), &mut newest)?;
        collect_file_mtime(&root.join(".cargo").join("config.toml"), &mut newest)?;
    }

    let dep_info_path = program.with_extension("d");
    let dep_info = fs::read_to_string(&dep_info_path)?;
    let dependency_paths = parse_cargo_dep_info_paths(&dep_info);
    if dependency_paths.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Cargo dep-info `{}` has no inputs", dep_info_path.display()),
        ));
    }
    for path in dependency_paths {
        let modified = match fs::metadata(&path) {
            Ok(metadata) => metadata.modified()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut ancestor = path.parent();
                let mut ancestor_modified = None;
                while let Some(path) = ancestor {
                    match fs::metadata(path) {
                        Ok(metadata) => {
                            ancestor_modified = Some(metadata.modified()?);
                            break;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            ancestor = path.parent();
                        }
                        Err(error) => return Err(error),
                    }
                }
                ancestor_modified.unwrap_or(UNIX_EPOCH)
            }
            Err(error) => return Err(error),
        };
        if newest.is_none_or(|current| modified > current) {
            newest = Some(modified);
        }
    }

    Ok(newest)
}

fn parse_cargo_dep_info_paths(dep_info: &str) -> Vec<PathBuf> {
    let Some((_, dependencies)) = dep_info.split_once(": ") else {
        return Vec::new();
    };
    let dependencies = dependencies
        .split("\n\n")
        .next()
        .unwrap_or(dependencies)
        .replace("\\\r\n", " ")
        .replace("\\\n", " ");
    let mut paths = Vec::new();
    let mut token = String::new();
    let mut chars = dependencies.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            if !token.is_empty() {
                paths.push(PathBuf::from(std::mem::take(&mut token)));
            }
            continue;
        }
        if ch == '\\' && chars.peek().is_some_and(|next| next.is_whitespace()) {
            token.push(chars.next().expect("peeked dep-info escape"));
        } else {
            token.push(ch);
        }
    }
    if !token.is_empty() {
        paths.push(PathBuf::from(token));
    }
    paths
}

fn collect_file_mtime(path: &Path, newest: &mut Option<SystemTime>) -> Result<(), std::io::Error> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            let modified = metadata.modified()?;
            if newest.is_none_or(|current| modified > current) {
                *newest = Some(modified);
            }
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn service_process_endpoint_matches_expected_launch(
    process: &az_proto_session::ServiceProcessRecord,
    command: &ProjectServiceCommand,
) -> bool {
    process.endpoint == command.endpoint
}

fn path_strings_equal(left: &str, right: &str) -> bool {
    if Path::new(left) == Path::new(right) {
        return true;
    }

    cfg!(windows) && left.eq_ignore_ascii_case(right)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    left == right
        || (cfg!(windows)
            && left
                .to_string_lossy()
                .eq_ignore_ascii_case(&right.to_string_lossy()))
}

fn service_launch_unmanaged_args(args: &[String]) -> Vec<String> {
    const MANAGED_FLAGS: &[&str] = &[
        "--asset-db",
        "--asset-processor-endpoint",
        "--asset-processor-endpoint-kind",
        "--branch",
        "--cache-root",
        "--capability-grants",
        "--lifecycle-capability-grants",
        "--lifecycle-endpoint",
        "--lifecycle-endpoint-kind",
        "--endpoint",
        "--endpoint-kind",
        "--run",
        "--observability-capability-grants",
        "--observability-endpoint",
        "--observability-endpoint-kind",
        "--otlp-endpoint",
        "--owner-id",
        "--owner-root",
        "--project-id",
        "--project",
        "--ready-file",
        "--session",
        "--session-id",
        "--side-channel-root",
        "--staging-root",
        "--structured-log",
        "--workspace-root",
    ];

    let mut unmanaged = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if MANAGED_FLAGS
            .iter()
            .any(|flag| is_launch_flag(&args[index], flag))
        {
            index += if args[index].contains('=') || index + 1 >= args.len() {
                1
            } else {
                2
            };
        } else {
            unmanaged.push(args[index].clone());
            index += 1;
        }
    }

    unmanaged
}

fn is_launch_flag(arg: &str, flag: &str) -> bool {
    arg == flag
        || arg
            .strip_prefix(flag)
            .is_some_and(|rest| rest.starts_with('='))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionServiceReadinessBlocker {
    service: String,
    state: String,
    terminal: bool,
}

fn first_unready_session_service(
    manifest: &ProtoSessionManifest,
    service_names: &[String],
    fresh_after_unix_ms: Option<u64>,
) -> Option<SessionServiceReadinessBlocker> {
    service_names.iter().find_map(|service_name| {
        let Some(process) = current_process_for_service(manifest, service_name) else {
            return Some(SessionServiceReadinessBlocker {
                service: service_name.clone(),
                state: "missing process record".to_string(),
                terminal: true,
            });
        };
        service_process_ready_state(process, fresh_after_unix_ms).map(|state| {
            SessionServiceReadinessBlocker {
                service: service_name.clone(),
                state,
                terminal: service_process_failure_is_fresh(process, fresh_after_unix_ms),
            }
        })
    })
}

fn service_process_ready_state(
    process: &az_proto_session::ServiceProcessRecord,
    fresh_after_unix_ms: Option<u64>,
) -> Option<String> {
    if process.state != ProtoServiceProcessState::Running {
        return Some(service_process_state_detail(process));
    }

    match RecordedProcess::assess(process.pid, process.process_start_time) {
        Ok(RecordedProcess::Live { .. }) => {}
        Ok(RecordedProcess::Unrecorded) => {
            return Some(format!(
                "running for run {} without a process id",
                process.run
            ));
        }
        Ok(RecordedProcess::Unattributable { process_id }) => {
            return Some(format!(
                "running for run {} but process {process_id} was recorded without a start token",
                process.run
            ));
        }
        Ok(RecordedProcess::Exited { process_id }) => {
            return Some(format!(
                "running for run {} but process {process_id} is not alive",
                process.run
            ));
        }
        Ok(RecordedProcess::Reused {
            recorded,
            actual_start_time,
        }) => {
            return Some(format!(
                "running for run {} but pid {} now belongs to a process started at {actual_start_time}, not {}",
                process.run, recorded.process_id, recorded.process_start_time
            ));
        }
        Err(error) => {
            return Some(format!(
                "running for run {} but the identity of process {} could not be read: {error}",
                process.run,
                process.pid.unwrap_or_default()
            ));
        }
    }

    if let Some(fresh_after_unix_ms) = fresh_after_unix_ms {
        let started_unix_ms = process.started_unix_ms.unwrap_or(0);
        if started_unix_ms < fresh_after_unix_ms || process.updated_unix_ms < fresh_after_unix_ms {
            return Some(format!(
                "running for run {} from stale start record updated at {} before current start request {}",
                process.run, process.updated_unix_ms, fresh_after_unix_ms
            ));
        }
    }

    None
}

fn service_process_state_detail(process: &az_proto_session::ServiceProcessRecord) -> String {
    let mut detail = format!(
        "{} for run {}",
        format_proto_service_process_state(process.state),
        process.run
    );
    if let Some(failure) = process.failure.as_deref().and_then(first_non_empty_line) {
        detail.push_str(": ");
        detail.push_str(failure);
    }
    detail
}

fn service_process_failure_is_fresh(
    process: &az_proto_session::ServiceProcessRecord,
    fresh_after_unix_ms: Option<u64>,
) -> bool {
    if process.state != ProtoServiceProcessState::Failed {
        return false;
    }
    let Some(fresh_after_unix_ms) = fresh_after_unix_ms else {
        return true;
    };
    process.updated_unix_ms >= fresh_after_unix_ms
}

const fn format_proto_service_process_state(state: ProtoServiceProcessState) -> &'static str {
    match state {
        ProtoServiceProcessState::Planned => "planned",
        ProtoServiceProcessState::Starting => "starting",
        ProtoServiceProcessState::Running => "running",
        ProtoServiceProcessState::Exited => "exited",
        ProtoServiceProcessState::Failed => "failed",
    }
}

fn first_non_empty_line(reason: &str) -> Option<&str> {
    reason.lines().map(str::trim).find(|line| !line.is_empty())
}

fn current_process_for_service<'a>(
    manifest: &'a ProtoSessionManifest,
    service_name: &str,
) -> Option<&'a az_proto_session::ServiceProcessRecord> {
    manifest.processes.iter().find(|process| {
        process.service_name == service_name && process.role == ServiceRole::RuntimeHost
    })
}

fn running_session_service_names(
    manifest: &ProtoSessionManifest,
    requested: &[String],
) -> Vec<String> {
    requested
        .iter()
        .filter(|service_name| {
            current_process_for_service(manifest, service_name)
                .is_some_and(|process| service_process_ready_state(process, None).is_none())
        })
        .cloned()
        .collect()
}

/// How long the lease-recovery probe has taken so far, for the adoption log.
#[derive(Debug, Clone, Copy)]
struct LeaseRecoveryTiming {
    recovery_started: Instant,
    lease_read_ms: u64,
}

#[derive(Debug, Clone)]
struct ReachableSessionSupervisor {
    descriptor: ServiceDescriptor,
    manifest: ProtoSessionManifest,
    process: ProcessIdentity,
}

impl AzDaemon {
    #[instrument(
        skip_all,
        fields(project_id = %manifest.project_id, session = %manifest.slug)
    )]
    /// Challenge a live session-supervisor lease and adopt it, or clear it and
    /// return `None` so the caller starts a fresh `az-sessiond`.
    ///
    /// # Errors
    ///
    /// Returns [`AzDaemonError::ProcessIdentity`] when the leased process
    /// cannot be re-identified,
    /// [`AzDaemonError::SessionSupervisorLease`] when the lease record cannot be
    /// cleared, [`AzDaemonError::SessionServiceEndpointLayout`] when a stale
    /// unix endpoint cannot be removed, and any error
    /// [`Self::shutdown_session_supervisor_before_prepare`] or
    /// [`Self::publish_recovered_session_supervisor`] returns.
    fn adopt_live_session_supervisor_lease(
        &self,
        manifest: &az_session::SessionManifest,
        lease: &az_session::SessionSupervisorLeaseRecord,
        lease_store: &az_session::SessionSupervisorLeaseStore,
        heartbeat_expired: bool,
        timing: LeaseRecoveryTiming,
    ) -> Result<Option<ReachableSessionSupervisor>, AzDaemonError> {
        let descriptor = lease.descriptor();
        if heartbeat_expired {
            info!(
                session = %manifest.slug,
                run = %descriptor.run,
                process_id = lease.process.process_id,
                "re-challenging live session-supervisor with expired heartbeat"
            );
        }
        let challenge_started = Instant::now();
        match challenge_session_supervisor_lease(manifest, lease) {
            Ok(supervisor_manifest) => {
                let challenge_ms = duration_millis_u64(challenge_started.elapsed());
                let publish_started = Instant::now();
                self.publish_recovered_session_supervisor(manifest, &descriptor)?;
                let publish_ms = duration_millis_u64(publish_started.elapsed());
                info!(
                    session = %manifest.slug,
                    run = %descriptor.run,
                    endpoint_kind = ?descriptor.endpoint.kind,
                    endpoint = %descriptor.endpoint.address,
                    lease_read_ms = timing.lease_read_ms,
                    challenge_ms,
                    publish_ms,
                    total_ms = duration_millis_u64(timing.recovery_started.elapsed()),
                    "adopted verified session-supervisor lease"
                );
                Ok(Some(ReachableSessionSupervisor {
                    descriptor,
                    manifest: supervisor_manifest,
                    process: lease.process,
                }))
            }
            Err(error) => {
                match capture_process_identity(lease.process.process_id)? {
                    Some(actual) if actual == lease.process => {
                        info!(
                            session = %manifest.slug,
                            run = %descriptor.run,
                            endpoint_kind = ?descriptor.endpoint.kind,
                            endpoint = %descriptor.endpoint.address,
                            error = %error,
                            "live session-supervisor lease challenge failed; requesting graceful shutdown"
                        );
                        self.shutdown_session_supervisor_before_prepare(
                            manifest,
                            &descriptor,
                            lease.process,
                        )?;
                    }
                    _ => {
                        remove_stale_session_supervisor_unix_endpoint(&descriptor)?;
                    }
                }
                lease_store.clear_if_process(lease.process)?;
                Ok(None)
            }
        }
    }

    fn existing_reachable_session_supervisor_snapshot(
        &self,
        manifest: &az_session::SessionManifest,
    ) -> Result<Option<ReachableSessionSupervisor>, AzDaemonError> {
        let recovery_started = Instant::now();
        let lease_store = az_session::SessionSupervisorLeaseStore::new(&manifest.run_dir);
        let lease_read_started = Instant::now();
        let state = lease_store.load()?;
        let lease_read_ms = duration_millis_u64(lease_read_started.elapsed());
        let Some(lease) = state.record else {
            info!(
                session = %manifest.slug,
                lease_path = %lease_store.path().display(),
                "no session-supervisor lease was published; starting a new az-sessiond"
            );
            return Ok(None);
        };
        let now_unix_ms = current_unix_ms();
        let actual_process = capture_process_identity(lease.process.process_id)?;
        match assess_supervisor_lease_process(&lease, actual_process, now_unix_ms) {
            SupervisorLeaseProcessAssessment::Dead { heartbeat_expired } => {
                info!(
                    session = %manifest.slug,
                    process_id = lease.process.process_id,
                    heartbeat_expired,
                    "session-supervisor lease process is dead"
                );
                remove_stale_session_supervisor_unix_endpoint(&lease.descriptor())?;
                lease_store.clear_if_process(lease.process)?;
            }
            SupervisorLeaseProcessAssessment::PidReused { actual_start_time } => {
                info!(
                    session = %manifest.slug,
                    process_id = lease.process.process_id,
                    leased_start_time = lease.process.process_start_time,
                    actual_start_time,
                    "rejected session-supervisor lease after PID reuse"
                );
                remove_stale_session_supervisor_unix_endpoint(&lease.descriptor())?;
                lease_store.clear_if_process(lease.process)?;
            }
            SupervisorLeaseProcessAssessment::Live { heartbeat_expired } => {
                if let Some(reachable) = self.adopt_live_session_supervisor_lease(
                    manifest,
                    &lease,
                    &lease_store,
                    heartbeat_expired,
                    LeaseRecoveryTiming {
                        recovery_started,
                        lease_read_ms,
                    },
                )? {
                    return Ok(Some(reachable));
                }
            }
        }

        info!(
            session = %manifest.slug,
            lease_path = %lease_store.path().display(),
            "no verified session-supervisor lease was reachable; starting a new az-sessiond"
        );
        Ok(None)
    }

    #[instrument(
        skip_all,
        fields(
            project_id = %manifest.project_id,
            session = %manifest.slug,
            run = %descriptor.run,
            endpoint_kind = ?descriptor.endpoint.kind,
            endpoint = %descriptor.endpoint.address
        )
    )]
    fn publish_recovered_session_supervisor(
        &self,
        manifest: &az_session::SessionManifest,
        descriptor: &ServiceDescriptor,
    ) -> Result<(), AzDaemonError> {
        let manager = self.session_manager(&manifest.project_root)?;
        manager.register_service_descriptor(&manifest.slug, descriptor)?;
        self.register_session_supervisor(&manifest.project_id, &manifest.slug, descriptor)?;
        Ok(())
    }

    fn shutdown_session_supervisor_before_prepare(
        &self,
        manifest: &az_session::SessionManifest,
        descriptor: &ServiceDescriptor,
        process: ProcessIdentity,
    ) -> Result<(), AzDaemonError> {
        let reason = "azd rebuilding project session services";
        let _result = request_session_service_stop(manifest, descriptor, reason)?;
        info!(
            session = %manifest.slug,
            run = %descriptor.run,
            endpoint = %descriptor.endpoint.address,
            "waiting for inconsistent session-supervisor to stop before project-service rebuild"
        );
        if capture_process_identity(process.process_id)?.as_ref() == Some(&process) {
            let lifecycle = ServiceLifecycleEvents::new();
            if let Err(error) = lifecycle.add_identity(process) {
                if capture_process_identity(process.process_id)?.as_ref() == Some(&process) {
                    return Err(error.into());
                }
            } else {
                match lifecycle.wait_until(Instant::now() + SESSION_SUPERVISOR_SHUTDOWN_TIMEOUT)? {
                    Some(ServiceLifecycleEvent::ProcessExited(identity)) if identity == process => {
                        lifecycle.consume_exit(identity)?;
                    }
                    Some(ServiceLifecycleEvent::ProcessExitWaitFailed { identity, reason })
                        if identity == process =>
                    {
                        lifecycle.consume_exit(identity)?;
                        return Err(AzDaemonError::InvalidServicePlan {
                            service: manifest.slug.clone(),
                            reason: format!(
                                "session-supervisor process-exit wait failed for {identity:?}: {reason}"
                            ),
                        });
                    }
                    Some(event) => {
                        return Err(AzDaemonError::InvalidServicePlan {
                            service: manifest.slug.clone(),
                            reason: format!(
                                "session-supervisor shutdown observed an unrelated lifecycle event {event:?}"
                            ),
                        });
                    }
                    None => {
                        return Err(AzDaemonError::SessionSupervisorShutdownTimedOut {
                            session: manifest.slug.clone(),
                            run: descriptor.run,
                            timeout_ms: duration_millis_u64(SESSION_SUPERVISOR_SHUTDOWN_TIMEOUT),
                        });
                    }
                }
            }
        }
        remove_stale_session_supervisor_unix_endpoint(descriptor)?;
        let _ =
            self.unregister_session_supervisor(&manifest.project_id, &manifest.slug, descriptor)?;
        info!(
            session = %manifest.slug,
            run = %descriptor.run,
            "session-supervisor process exited; project-service rebuild may proceed"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupervisorLeaseProcessAssessment {
    Dead { heartbeat_expired: bool },
    PidReused { actual_start_time: u64 },
    Live { heartbeat_expired: bool },
}

/// Read a process identity, naming the pid in any failure.
fn capture_process_identity(process_id: u32) -> Result<Option<ProcessIdentity>, AzDaemonError> {
    ProcessIdentity::capture(process_id)
        .map_err(|source| AzDaemonError::ProcessIdentity { process_id, source })
}

fn assess_supervisor_lease_process(
    lease: &az_session::SessionSupervisorLeaseRecord,
    actual_process: Option<ProcessIdentity>,
    now_unix_ms: u64,
) -> SupervisorLeaseProcessAssessment {
    let heartbeat_expired = lease.heartbeat_expired(now_unix_ms);
    match actual_process {
        None => SupervisorLeaseProcessAssessment::Dead { heartbeat_expired },
        Some(actual) if actual != lease.process => SupervisorLeaseProcessAssessment::PidReused {
            actual_start_time: actual.process_start_time,
        },
        Some(_) => SupervisorLeaseProcessAssessment::Live { heartbeat_expired },
    }
}

fn challenge_session_supervisor_lease(
    manifest: &az_session::SessionManifest,
    lease: &az_session::SessionSupervisorLeaseRecord,
) -> Result<ProtoSessionManifest, AzDaemonError> {
    let descriptor = lease.descriptor();
    let snapshot = request_session_supervisor_challenge(manifest, &descriptor)?;
    let identity = snapshot.identity;
    if identity.process_id != lease.process.process_id
        || identity.process_start_time != lease.process.process_start_time
        || identity.descriptor.id != descriptor.id
        || identity.descriptor.role != descriptor.role
        || identity.descriptor.endpoint != descriptor.endpoint
    {
        return Err(AzDaemonError::InvalidSessionSupervisorDescriptor {
            reason: format!(
                "lease process {} start {} descriptor binding did not match RPC process {} start {} descriptor binding",
                lease.process.process_id,
                lease.process.process_start_time,
                identity.process_id,
                identity.process_start_time,
            ),
        });
    }
    if snapshot.manifest.id != manifest.id.0
        || snapshot.manifest.slug != manifest.slug
        || snapshot.manifest.project_id != manifest.project_id
    {
        return Err(AzDaemonError::InvalidSessionSupervisorDescriptor {
            reason: format!(
                "lease returned session `{}`/`{}` instead of `{}`/`{}`",
                snapshot.manifest.project_id,
                snapshot.manifest.slug,
                manifest.project_id,
                manifest.slug,
            ),
        });
    }
    Ok(snapshot.manifest)
}

#[derive(Debug)]
struct SessionSupervisorChallenge {
    manifest: ProtoSessionManifest,
    identity: az_proto_session::SessionSupervisorIdentity,
}

fn request_session_supervisor_challenge(
    manifest: &az_session::SessionManifest,
    descriptor: &ServiceDescriptor,
) -> Result<SessionSupervisorChallenge, AzDaemonError> {
    validate_session_supervisor_descriptor(descriptor, "azd session-supervisor challenge")?;
    let manifest = manifest.clone();
    let session = manifest.slug.clone();
    let descriptor = descriptor.clone();
    run_session_supervisor_rpc_with_timeout(
        session,
        "challenge",
        SESSION_SUPERVISOR_PROBE_RPC_TIMEOUT,
        move || async move {
            let client = connect_session_supervisor(&descriptor, &manifest, "challenge").await?;

            let health_promise = client.health_request().send().promise;
            let identity_promise = client.supervision_identity_request().send().promise;
            let mut list_request = client.list_request();
            SessionCapabilityRequest {
                capability: daemon_session_capability(
                    &descriptor,
                    manifest.id,
                    &[SESSION_READ_PERMISSION],
                )?,
            }
            .to_capnp(list_request.get())
            .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "challenge", error))?;
            let list_promise = list_request.send().promise;

            let (health_response, identity_response, list_response) = futures::try_join!(
                async {
                    health_promise.await.map_err(|error| {
                        session_supervisor_rpc_error(&manifest.slug, "challenge health", error)
                    })
                },
                async {
                    identity_promise.await.map_err(|error| {
                        session_supervisor_rpc_error(&manifest.slug, "challenge identity", error)
                    })
                },
                async {
                    list_promise.await.map_err(|error| {
                        session_supervisor_rpc_error(&manifest.slug, "challenge list", error)
                    })
                },
            )?;

            verify_challenge_health(&manifest, &descriptor, &health_response)?;
            let identity = decode_challenge_identity(&manifest, &identity_response)?;
            let matched_manifest = find_challenged_session_manifest(&manifest, &list_response)?;

            Ok(SessionSupervisorChallenge {
                manifest: matched_manifest,
                identity,
            })
        },
    )
}

/// Confirm the challenged supervisor speaks the current protocol and answers
/// for the descriptor the daemon dialled.
///
/// # Errors
///
/// Returns [`AzDaemonError::SessionSupervisorRpc`] when the health reply cannot
/// be decoded or reports an incompatible protocol version, and
/// [`AzDaemonError::InvalidSessionSupervisorDescriptor`] when it reports a
/// different service identity or role.
// `ServiceDescriptor` names its identity `id`, so clippy's suggested
// `health.service != descriptor.service` would not compile.
#[allow(clippy::suspicious_operation_groupings)]
fn verify_challenge_health(
    manifest: &az_session::SessionManifest,
    descriptor: &ServiceDescriptor,
    response: &capnp::capability::Response<
        session_capnp::session_supervisor::health_results::Owned,
    >,
) -> Result<(), AzDaemonError> {
    let health = ServiceHealth::from_capnp(
        response
            .get()
            .map_err(|error| {
                session_supervisor_rpc_error(&manifest.slug, "challenge health", error)
            })?
            .get_health()
            .map_err(|error| {
                session_supervisor_rpc_error(&manifest.slug, "challenge health", error)
            })?,
    )
    .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "challenge health", error))?;
    health
        .require_protocol_version(az_proto_core::ProtocolVersion::CURRENT)
        .map_err(|error| {
            session_supervisor_rpc_error(
                &manifest.slug,
                "challenge health",
                format!("unavailable until service restart: {error}"),
            )
        })?;
    if health.service != descriptor.id || health.role != descriptor.role {
        return Err(AzDaemonError::InvalidSessionSupervisorDescriptor {
            reason: format!(
                "challenge health identity `{}`/`{}` {:?} did not match candidate `{}`/`{}` {:?}",
                health.service.namespace,
                health.service.name,
                health.role,
                descriptor.id.namespace,
                descriptor.id.name,
                descriptor.role,
            ),
        });
    }
    Ok(())
}

/// Decode the supervision identity the challenged supervisor reported.
///
/// # Errors
///
/// Returns [`AzDaemonError::SessionSupervisorRpc`] when the reply cannot be
/// read or decoded.
fn decode_challenge_identity(
    manifest: &az_session::SessionManifest,
    response: &capnp::capability::Response<
        session_capnp::session_supervisor::supervision_identity_results::Owned,
    >,
) -> Result<SessionSupervisorIdentity, AzDaemonError> {
    SessionSupervisorIdentity::from_capnp(
        response
            .get()
            .map_err(|error| {
                session_supervisor_rpc_error(&manifest.slug, "challenge identity", error)
            })?
            .get_identity()
            .map_err(|error| {
                session_supervisor_rpc_error(&manifest.slug, "challenge identity", error)
            })?,
    )
    .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "challenge identity", error))
}

/// Find this session in the challenged supervisor's session list.
///
/// # Errors
///
/// Returns [`AzDaemonError::SessionSupervisorRpc`] when the list cannot be read
/// or decoded, or when the supervisor does not report this session at all.
fn find_challenged_session_manifest(
    manifest: &az_session::SessionManifest,
    response: &capnp::capability::Response<session_capnp::session_supervisor::list_results::Owned>,
) -> Result<ProtoSessionManifest, AzDaemonError> {
    let sessions = response
        .get()
        .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "challenge list", error))?
        .get_sessions()
        .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "challenge list", error))?;
    for index in 0..sessions.len() {
        let candidate = ProtoSessionManifest::from_capnp(sessions.get(index)).map_err(|error| {
            session_supervisor_rpc_error(&manifest.slug, "challenge list", error)
        })?;
        if candidate.id == manifest.id.0 && candidate.slug == manifest.slug {
            return Ok(candidate);
        }
    }
    Err(session_supervisor_rpc_error(
        &manifest.slug,
        "challenge list",
        format!("session `{}` was not returned", manifest.slug),
    ))
}

fn request_session_service_start(
    manifest: &az_session::SessionManifest,
    descriptor: &ServiceDescriptor,
    reason: &'static str,
    service_names: Vec<String>,
) -> Result<StartServicesResult, AzDaemonError> {
    validate_session_supervisor_descriptor(descriptor, "azd session-supervisor start-services")?;
    let manifest = manifest.clone();
    let session = manifest.slug.clone();
    let descriptor = descriptor.clone();
    run_session_supervisor_rpc(session, "startServices", move || async move {
        let client = connect_session_supervisor(&descriptor, &manifest, "startServices").await?;
        let mut request = client.start_services_request();
        StartServicesRequest {
            capability: daemon_session_capability(
                &descriptor,
                manifest.id,
                &[SESSION_MANAGE_PERMISSION],
            )?,
            slug: manifest.slug.clone(),
            reason: reason.to_string(),
            service_names,
        }
        .to_capnp(request.get().init_request())
        .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "startServices", error))?;
        let response = request.send().promise.await.map_err(|error| {
            session_supervisor_rpc_error(&manifest.slug, "startServices", error)
        })?;
        StartServicesResult::from_capnp(
            response
                .get()
                .map_err(|error| {
                    session_supervisor_rpc_error(&manifest.slug, "startServices", error)
                })?
                .get_result()
                .map_err(|error| {
                    session_supervisor_rpc_error(&manifest.slug, "startServices", error)
                })?,
        )
        .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "startServices", error))
    })
}

fn request_session_service_stop(
    manifest: &az_session::SessionManifest,
    descriptor: &ServiceDescriptor,
    reason: &str,
) -> Result<StopServicesResult, AzDaemonError> {
    validate_session_supervisor_descriptor(descriptor, "azd session-supervisor stop-services")?;
    let manifest = manifest.clone();
    let session = manifest.slug.clone();
    let descriptor = descriptor.clone();
    let reason = reason.to_string();
    run_session_supervisor_rpc_with_timeout(
        session,
        "stopServices",
        SESSION_SUPERVISOR_STOP_RPC_TIMEOUT,
        move || async move {
            let connection =
                connect_session_supervisor_scoped(&descriptor, &manifest, "stopServices").await?;
            let client = connection.client();
            let mut request = client.stop_services_request();
            StopServicesRequest {
                capability: daemon_session_capability(
                    &descriptor,
                    manifest.id,
                    &[SESSION_MANAGE_PERMISSION],
                )?,
                slug: manifest.slug.clone(),
                reason,
            }
            .to_capnp(request.get().init_request())
            .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "stopServices", error))?;
            let response = request.send().promise.await.map_err(|error| {
                session_supervisor_rpc_error(&manifest.slug, "stopServices", error)
            })?;
            let result = StopServicesResult::from_capnp(
                response
                    .get()
                    .map_err(|error| {
                        session_supervisor_rpc_error(&manifest.slug, "stopServices", error)
                    })?
                    .get_result()
                    .map_err(|error| {
                        session_supervisor_rpc_error(&manifest.slug, "stopServices", error)
                    })?,
            )
            .map_err(|error| session_supervisor_rpc_error(&manifest.slug, "stopServices", error))?;

            let mut shutdown = client.shutdown_supervisor_request();
            shutdown.get().set_slug(&manifest.slug);
            shutdown.get().set_reason("azd stopServices completed");
            daemon_session_capability(&descriptor, manifest.id, &[SESSION_MANAGE_PERMISSION])?
                .to_capnp(shutdown.get().init_capability())
                .map_err(|error| {
                    session_supervisor_rpc_error(&manifest.slug, "shutdownSupervisor", error)
                })?;
            // The stop result is already received. Queue the streaming
            // one-way call, then gracefully flush this short-lived RPC
            // connection without awaiting a supervisor result.
            drop(shutdown.send());
            connection.disconnect().await.map_err(|error| {
                session_supervisor_rpc_error(&manifest.slug, "shutdownSupervisor", error)
            })?;
            Ok(result)
        },
    )
}

fn run_session_supervisor_rpc<T, Fut>(
    session: String,
    operation: &'static str,
    rpc: impl FnOnce() -> Fut + Send + 'static,
) -> Result<T, AzDaemonError>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, AzDaemonError>> + 'static,
{
    run_session_supervisor_rpc_inner(session, operation, None, rpc)
}

fn run_session_supervisor_rpc_with_timeout<T, Fut>(
    session: String,
    operation: &'static str,
    deadline: Duration,
    rpc: impl FnOnce() -> Fut + Send + 'static,
) -> Result<T, AzDaemonError>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, AzDaemonError>> + 'static,
{
    run_session_supervisor_rpc_inner(session, operation, Some(deadline), rpc)
}

fn run_session_supervisor_rpc_inner<T, Fut>(
    session: String,
    operation: &'static str,
    deadline: Option<Duration>,
    rpc: impl FnOnce() -> Fut + Send + 'static,
) -> Result<T, AzDaemonError>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, AzDaemonError>> + 'static,
{
    let session_for_timeout = session.clone();
    let run = move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;
        let local = tokio::task::LocalSet::new();
        local.block_on(&runtime, async move {
            match deadline {
                Some(deadline) => tokio::time::timeout(deadline, rpc()).await.map_err(|_| {
                    AzDaemonError::SessionSupervisorRpc {
                        session: session_for_timeout.clone(),
                        operation,
                        reason: format!(
                            "timed out after {}ms",
                            deadline.as_millis().min(u128::from(u64::MAX))
                        ),
                    }
                })?,
                None => rpc().await,
            }
        })
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(run)
            .join()
            .map_err(|_| AzDaemonError::SessionSupervisorRpc {
                session,
                operation,
                reason: "session-supervisor RPC worker panicked".to_string(),
            })?
    } else {
        run()
    }
}

async fn connect_session_supervisor(
    descriptor: &ServiceDescriptor,
    manifest: &az_session::SessionManifest,
    operation: &'static str,
) -> Result<session_capnp::session_supervisor::Client, AzDaemonError> {
    az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint)
        .await
        .map_err(|error| session_supervisor_rpc_error(&manifest.slug, operation, error))
}

async fn connect_session_supervisor_scoped(
    descriptor: &ServiceDescriptor,
    manifest: &az_session::SessionManifest,
    operation: &'static str,
) -> Result<az_rpc::ScopedTwopartyClient<session_capnp::session_supervisor::Client>, AzDaemonError>
{
    az_rpc::connect_twoparty_bootstrap_scoped(&descriptor.endpoint)
        .await
        .map_err(|error| session_supervisor_rpc_error(&manifest.slug, operation, error))
}

fn daemon_session_capability(
    descriptor: &ServiceDescriptor,
    session_id: az_session::SessionId,
    permissions: &[&str],
) -> Result<Capability, AzDaemonError> {
    required_session_supervisor_capability(
        descriptor,
        &ServiceId::new(
            DAEMON_SESSION_SERVICE_NAMESPACE,
            DAEMON_SESSION_SERVICE_NAME,
        ),
        ServiceRole::Daemon,
        permissions,
        "daemon",
    )
    .map(|capability| capability.scoped_to(Some(session_id.0)))
}

fn validate_terminal_start_status_for_session(
    session: &str,
    result: &StartServicesResult,
    _reason: &str,
) -> Result<(), AzDaemonError> {
    if result.status.manifest.slug != session {
        return Err(AzDaemonError::SessionSupervisorRpc {
            session: session.to_string(),
            operation: "startServices",
            reason: "terminal status belongs to a different session".to_string(),
        });
    }
    Ok(())
}

fn session_supervisor_rpc_error(
    session: &str,
    operation: &'static str,
    error: impl std::fmt::Display,
) -> AzDaemonError {
    AzDaemonError::SessionSupervisorRpc {
        session: session.to_string(),
        operation,
        reason: error.to_string(),
    }
}

fn cargo_target_dir_or_default(root: &Path) -> PathBuf {
    load_cargo_metadata(root).map_or_else(
        |_| root.join("target"),
        |metadata| metadata.target_directory,
    )
}

fn service_binary_path(build_output_root: &Path, target: &ProjectServiceTarget) -> PathBuf {
    build_output_root
        .join("debug")
        .join(service_executable_name(&target.bin))
}

fn service_executable_name(bin: &str) -> String {
    if cfg!(windows) {
        format!("{bin}.exe")
    } else {
        bin.to_string()
    }
}

const fn service_role(role: ProjectServiceRole) -> ServiceRole {
    match role {
        ProjectServiceRole::ProjectHost => ServiceRole::ProjectHost,
        ProjectServiceRole::AssetProcessor => ServiceRole::AssetProcessor,
        ProjectServiceRole::AssetWorker => ServiceRole::Worker,
        ProjectServiceRole::RuntimeHost => ServiceRole::RuntimeHost,
    }
}

const fn service_plan_priority(role: ServiceRole) -> u8 {
    match role {
        ServiceRole::AssetProcessor => 0,
        ServiceRole::ProjectHost => 1,
        ServiceRole::RuntimeHost => 2,
        ServiceRole::Worker => 3,
        ServiceRole::SessionSupervisor => 4,
        ServiceRole::Daemon => 5,
        ServiceRole::Editor => 6,
        ServiceRole::Unknown => 7,
    }
}

fn service_endpoint(
    data_home: &AzothDataHome,
    root: &Path,
    project_id: &str,
    session_slug: &str,
    service_name: &str,
    role: ServiceRole,
    kind: EndpointKind,
) -> Result<Endpoint, AzDaemonError> {
    validate_public_endpoint_kind(kind, "azd project service planning")?;
    if role != ServiceRole::RuntimeHost {
        return Ok(project_service_endpoint_in(
            data_home,
            kind,
            root,
            service_name,
        )?);
    }

    let address = match kind {
        EndpointKind::WindowsNamedPipe => format!(
            r"\\.\pipe\azoth-{}-{}-{}",
            endpoint_token(project_id),
            endpoint_token(session_slug),
            endpoint_token(service_name)
        ),
        EndpointKind::UnixDomainSocket => {
            let data_paths = data_home.project(project_id, root);
            data_paths.prepare()?;
            data_paths
                .endpoints_dir()
                .join("sessions")
                .join(endpoint_token(session_slug))
                .join(format!("{}.sock", endpoint_token(service_name)))
                .to_string_lossy()
                .into_owned()
        }
        EndpointKind::Tcp => "127.0.0.1:0".to_string(),
        EndpointKind::InProcess => unreachable!("validated above"),
    };
    Ok(Endpoint::new(kind, address))
}

fn endpoint_kind_arg(kind: EndpointKind) -> Result<&'static str, AzDaemonError> {
    validate_public_endpoint_kind(kind, "azd project service planning")?;
    Ok(match kind {
        EndpointKind::WindowsNamedPipe => "windows-named-pipe",
        EndpointKind::UnixDomainSocket => "unix-domain-socket",
        EndpointKind::Tcp => "tcp",
        EndpointKind::InProcess => unreachable!("validated above"),
    })
}

const fn validate_public_endpoint_kind(
    kind: EndpointKind,
    operation: &'static str,
) -> Result<(), AzDaemonError> {
    if matches!(kind, EndpointKind::InProcess) {
        return Err(AzDaemonError::UnsupportedEndpointKind { operation, kind });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use az_architecture_guard::{
        production_source_without_cfg_test_modules, public_functions_are_cfg_test_or_test_support,
    };
    use az_project::{
        GemManifest, ProjectGem, ProjectManifest, ProjectPackageCompression,
        ProjectPackageContainer, ProjectPackageProfile, ProjectServiceRole, ProjectServiceTarget,
        refresh_project_lock, write_gem_manifest, write_project_manifest,
    };
    use az_proto_core::{Endpoint, ServiceId};
    use az_proto_daemon::{
        DAEMON_CONTROL_PERMISSION, EnsureProjectSessionRequest, ListProjectsRequest,
        ListProjectsResult, ListSessionSupervisorsRequest, ListSessionSupervisorsResult,
        PlanProjectBuildRequest, PlanProjectServicesRequest, PrepareProjectSessionServicesRequest,
        ProjectBuildPlan, ProjectRecord, ProjectResult, ProjectServicePlan, ProjectSessionResult,
        ProjectSessionServicesResult, RegisterProjectRequest, RegisterProjectRootRequest,
        RegisterSessionSupervisorRequest, ResolveProjectRequest, ResolveSessionSupervisorRequest,
        SessionSupervisorResult, ShutdownDaemonRequest, ShutdownDaemonResult,
        UnregisterSessionSupervisorRequest, UnregisterSessionSupervisorResult,
    };
    use az_proto_runtime::RUNTIME_HOST_SERVICE_NAME;
    use az_service_supervision::ServiceProcessExit;
    use futures::executor;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::path::Path;
    use std::process::Command;
    use std::rc::Rc;

    use super::*;

    fn test_daemon(root: &Path) -> AzDaemon {
        AzDaemon::with_data_home(AzothDataHome::new(root.join(".azoth-test"))).unwrap()
    }

    #[derive(Debug, Default)]
    struct ScriptedProjectServiceLauncher {
        exits: RefCell<BTreeMap<ServiceProcessKey, ServiceProcessExit>>,
        terminated: RefCell<Vec<ServiceProcessKey>>,
    }

    impl ServiceProcessLauncher for ScriptedProjectServiceLauncher {
        type Error = az_service_supervision::ServiceProcessError;

        fn spawn(
            &self,
            _process: &ServiceProcessRecord,
        ) -> Result<SpawnedServiceProcess, Self::Error> {
            unreachable!("readiness tests start from a captured spawned process")
        }

        fn is_tracking(&self, _process: &ServiceProcessKey) -> bool {
            true
        }

        fn try_wait(
            &self,
            process: &ServiceProcessKey,
        ) -> Result<Option<ServiceProcessExit>, Self::Error> {
            Ok(self.exits.borrow().get(process).cloned())
        }

        fn terminate(
            &self,
            process: &ServiceProcessKey,
        ) -> Result<Option<ServiceProcessExit>, Self::Error> {
            self.terminated.borrow_mut().push(process.clone());
            Ok(Some(ServiceProcessExit {
                service_name: process.service_name.clone(),
                exit_code: Some(1),
                success: false,
            }))
        }
    }

    #[derive(Debug)]
    enum ScriptedLifecycleWait {
        Event(ServiceLifecycleEvent),
        Failed,
        Timeout,
    }

    #[derive(Debug)]
    struct ScriptedReadySubscription {
        fail_finish: bool,
    }

    impl ProjectServiceReadySubscription for ScriptedReadySubscription {
        fn finish(&mut self) -> Result<(), az_service_supervision::ServiceProcessError> {
            if self.fail_finish {
                Err(az_service_supervision::ServiceProcessError::ReadyWatch {
                    path: PathBuf::from("ready"),
                    reason: "scripted finish failure".to_string(),
                })
            } else {
                Ok(())
            }
        }
    }

    #[derive(Debug, Default)]
    struct ScriptedProjectServiceLifecycle {
        bind_fails: bool,
        subscribe_fails: bool,
        finish_fails: bool,
        waits: RefCell<VecDeque<ScriptedLifecycleWait>>,
        consumed: RefCell<Vec<ProcessIdentity>>,
        retired: RefCell<Vec<ProcessIdentity>>,
    }

    impl ProjectServiceLifecycle for ScriptedProjectServiceLifecycle {
        type ReadySubscription = ScriptedReadySubscription;

        fn bind_exit(
            &self,
            identity: ProcessIdentity,
        ) -> Result<(), az_service_supervision::ServiceProcessError> {
            if self.bind_fails {
                Err(
                    az_service_supervision::ServiceProcessError::ProcessExitBindingUnavailable {
                        identity,
                        reason: "scripted bind failure".to_string(),
                    },
                )
            } else {
                Ok(())
            }
        }

        fn subscribe_ready(
            &self,
            _ready_paths: &[PathBuf],
        ) -> Result<Self::ReadySubscription, az_service_supervision::ServiceProcessError> {
            if self.subscribe_fails {
                Err(az_service_supervision::ServiceProcessError::ReadyWatch {
                    path: PathBuf::from("ready"),
                    reason: "scripted subscribe failure".to_string(),
                })
            } else {
                Ok(ScriptedReadySubscription {
                    fail_finish: self.finish_fails,
                })
            }
        }

        fn wait_until(
            &self,
            _deadline: Instant,
        ) -> Result<Option<ServiceLifecycleEvent>, az_service_supervision::ServiceProcessError>
        {
            match self
                .waits
                .borrow_mut()
                .pop_front()
                .unwrap_or(ScriptedLifecycleWait::Timeout)
            {
                ScriptedLifecycleWait::Event(event) => Ok(Some(event)),
                ScriptedLifecycleWait::Failed => {
                    Err(az_service_supervision::ServiceProcessError::LifecycleEventSourceClosed)
                }
                ScriptedLifecycleWait::Timeout => Ok(None),
            }
        }

        fn wait_for_exit_until(
            &self,
            _identity: ProcessIdentity,
            deadline: Instant,
        ) -> Result<Option<ServiceLifecycleEvent>, az_service_supervision::ServiceProcessError>
        {
            self.wait_until(deadline)
        }

        fn consume_exit(
            &self,
            identity: ProcessIdentity,
        ) -> Result<(), az_service_supervision::ServiceProcessError> {
            self.consumed.borrow_mut().push(identity);
            Ok(())
        }

        fn retire_exit(
            &self,
            identity: ProcessIdentity,
        ) -> Result<(), az_service_supervision::ServiceProcessError> {
            self.retired.borrow_mut().push(identity);
            Ok(())
        }
    }

    enum ScriptedReadyRead {
        Text(String),
        Failed,
    }

    struct ScriptedProjectServiceReadyReader(ScriptedReadyRead);

    impl ProjectServiceReadyReader for ScriptedProjectServiceReadyReader {
        fn read(&self, _path: &Path) -> Result<String, std::io::Error> {
            match &self.0 {
                ScriptedReadyRead::Text(text) => Ok(text.clone()),
                ScriptedReadyRead::Failed => Err(std::io::Error::other("scripted read failure")),
            }
        }
    }

    struct ProjectServiceReadinessFixture {
        temp: tempfile::TempDir,
        store: project_services::ProjectServiceStore,
        manifest: project_services::ProjectServiceManifest,
        pending: Vec<PendingProjectService>,
        started: Vec<SpawnedServiceProcess>,
        ready_file: PathBuf,
        endpoint: Endpoint,
    }

    fn project_service_readiness_fixture() -> ProjectServiceReadinessFixture {
        let temp = tempfile::tempdir().unwrap();
        let data_home = AzothDataHome::new(temp.path().join("data-home"));
        data_home.prepare().unwrap();
        let paths = data_home.project("Readiness Fixture", temp.path());
        paths.prepare().unwrap();
        let store = project_services::ProjectServiceStore::new(
            paths,
            "local.readiness_fixture",
            temp.path().to_path_buf(),
        )
        .unwrap();
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41000");
        let descriptor = az_service_catalog::asset_processor_service_descriptor(
            Uuid::now_v7(),
            endpoint.clone(),
        );
        let ready_file = store.ready_dir().join("asset-processor.toml");
        fs::create_dir_all(store.ready_dir()).unwrap();
        let mut process = ServiceProcessRecord::planned(
            "asset-processor",
            SupervisedServiceRole::AssetProcessor,
            descriptor.run,
            &endpoint,
            "asset-processor",
            temp.path().to_path_buf(),
            Vec::new(),
            store.logs_dir().join("asset-processor.stdout.log"),
            store.logs_dir().join("asset-processor.stderr.log"),
            store.logs_dir().join("asset-processor.capnp.log"),
            Some(ready_file.clone()),
            1,
        );
        process.mark_starting(2);
        let spawned = SpawnedServiceProcess {
            service_name: process.service_name.clone(),
            role: process.role,
            run: process.run,
            identity: ProcessIdentity {
                process_id: 41001,
                process_start_time: 7,
            },
        };
        let mut manifest = project_services::ProjectServiceManifest::new(
            "local.readiness_fixture",
            temp.path().to_path_buf(),
            1,
        );
        manifest.upsert_service_descriptor(&descriptor, 1).unwrap();
        manifest.upsert_process(process.clone(), 2);
        store.write(&manifest).unwrap();
        ProjectServiceReadinessFixture {
            temp,
            store,
            manifest,
            pending: vec![PendingProjectService {
                process,
                spawned: spawned.clone(),
            }],
            started: vec![spawned],
            ready_file,
            endpoint,
        }
    }

    fn valid_scripted_ready_record(fixture: &ProjectServiceReadinessFixture) -> ServiceReadyRecord {
        let pending = &fixture.pending[0];
        ServiceReadyRecord::new(
            &pending.process.service_name,
            pending.process.role,
            pending.process.run,
            &fixture.endpoint,
            Some(pending.spawned.identity.process_id),
            3,
        )
    }

    fn assert_readiness_rolled_back(fixture: &ProjectServiceReadinessFixture) {
        let key = ServiceProcessKey::from_process(&fixture.pending[0].process);
        let process = fixture.manifest.current_process(&key).unwrap();
        assert_eq!(process.state, ServiceProcessState::Failed);
        assert!(
            process
                .failure
                .as_deref()
                .is_some_and(|reason| !reason.is_empty())
        );
    }

    #[test]
    fn project_service_exit_bind_failure_rolls_back_before_returning() {
        let mut fixture = project_service_readiness_fixture();
        let launcher = ScriptedProjectServiceLauncher::default();
        let lifecycle = ScriptedProjectServiceLifecycle {
            bind_fails: true,
            ..Default::default()
        };

        let error = bind_project_service_exit_or_rollback(
            &fixture.store,
            &launcher,
            &lifecycle,
            &mut fixture.manifest,
            &fixture.started,
            &fixture.pending[0].process,
            &fixture.pending[0].spawned,
        )
        .unwrap_err();

        assert!(error.to_string().contains("scripted bind failure"));
        assert_readiness_rolled_back(&fixture);
        assert_eq!(launcher.terminated.borrow().len(), 1);
    }

    #[test]
    fn graceful_project_service_wait_reaps_only_the_exact_identity() {
        let fixture = project_service_readiness_fixture();
        let key = ServiceProcessKey::from_process(&fixture.pending[0].process);
        let launcher = ScriptedProjectServiceLauncher::default();
        launcher.exits.borrow_mut().insert(
            key.clone(),
            ServiceProcessExit {
                service_name: key.service_name.clone(),
                exit_code: Some(0),
                success: true,
            },
        );
        let lifecycle = ScriptedProjectServiceLifecycle {
            waits: RefCell::new(VecDeque::from([ScriptedLifecycleWait::Event(
                ServiceLifecycleEvent::ProcessExited(fixture.pending[0].spawned.identity),
            )])),
            ..Default::default()
        };

        let exit = wait_for_project_service_graceful_exit(
            &launcher,
            &lifecycle,
            &key,
            fixture.pending[0].spawned.identity,
        )
        .unwrap()
        .unwrap();

        assert!(exit.success);
        assert_eq!(
            lifecycle.consumed.borrow().as_slice(),
            &[fixture.pending[0].spawned.identity]
        );
        assert!(launcher.terminated.borrow().is_empty());
    }

    #[test]
    fn graceful_project_service_wait_timeout_leaves_force_fallback_to_owner() {
        let fixture = project_service_readiness_fixture();
        let key = ServiceProcessKey::from_process(&fixture.pending[0].process);
        let launcher = ScriptedProjectServiceLauncher::default();
        let lifecycle = ScriptedProjectServiceLifecycle::default();

        assert!(
            wait_for_project_service_graceful_exit(
                &launcher,
                &lifecycle,
                &key,
                fixture.pending[0].spawned.identity,
            )
            .unwrap()
            .is_none()
        );
        assert!(launcher.terminated.borrow().is_empty());
    }

    #[test]
    fn project_service_readiness_attributes_prior_service_exit_and_rolls_back_wave() {
        let mut fixture = project_service_readiness_fixture();
        let prior_identity = ProcessIdentity {
            process_id: 41002,
            process_start_time: 8,
        };
        let mut prior = ServiceProcessRecord::planned(
            "project-host",
            SupervisedServiceRole::ProjectHost,
            Uuid::now_v7(),
            &fixture.endpoint,
            "project-host",
            fixture.temp.path().to_path_buf(),
            Vec::new(),
            fixture.store.logs_dir().join("project-host.stdout.log"),
            fixture.store.logs_dir().join("project-host.stderr.log"),
            fixture.store.logs_dir().join("project-host.capnp.log"),
            None,
            1,
        );
        prior.mark_running(prior_identity, 2).unwrap();
        let prior_key = ServiceProcessKey::from_process(&prior);
        fixture.manifest.upsert_process(prior, 2);
        fixture.store.write(&fixture.manifest).unwrap();
        let launcher = ScriptedProjectServiceLauncher::default();
        launcher.exits.borrow_mut().insert(
            prior_key.clone(),
            ServiceProcessExit {
                service_name: "project-host".to_string(),
                exit_code: Some(23),
                success: false,
            },
        );
        let lifecycle = ScriptedProjectServiceLifecycle::default();
        lifecycle
            .waits
            .borrow_mut()
            .push_back(ScriptedLifecycleWait::Event(
                ServiceLifecycleEvent::ProcessExited(prior_identity),
            ));

        let error = wait_for_project_service_wave(
            &ProjectServiceWave {
                project_id: "local.readiness_fixture",
                store: &fixture.store,
                launcher: &launcher,
                lifecycle: &lifecycle,
                ready_reader: &ScriptedProjectServiceReadyReader(ScriptedReadyRead::Text(
                    String::new(),
                )),
                pending: &fixture.pending,
                started: &fixture.started,
                ready_timeout: Duration::from_secs(1),
            },
            &mut fixture.manifest,
        )
        .unwrap_err();

        assert!(error.to_string().contains("project-host"));
        assert_eq!(
            fixture.manifest.current_process(&prior_key).unwrap().state,
            ServiceProcessState::Failed
        );
        assert_readiness_rolled_back(&fixture);
        assert_eq!(&*lifecycle.consumed.borrow(), &[prior_identity]);
    }

    #[test]
    fn project_service_read_parse_validate_and_commit_failures_all_roll_back() {
        for failure in ["read", "parse", "validate", "commit"] {
            let mut fixture = project_service_readiness_fixture();
            fs::write(&fixture.ready_file, "ready").unwrap();
            let launcher = ScriptedProjectServiceLauncher::default();
            let lifecycle = ScriptedProjectServiceLifecycle::default();
            let reader = match failure {
                "read" => ScriptedProjectServiceReadyReader(ScriptedReadyRead::Failed),
                "parse" => ScriptedProjectServiceReadyReader(ScriptedReadyRead::Text(
                    "not = [valid".to_string(),
                )),
                "validate" => {
                    let mut ready = valid_scripted_ready_record(&fixture);
                    ready.service_name = "wrong-service".to_string();
                    ScriptedProjectServiceReadyReader(ScriptedReadyRead::Text(
                        toml::to_string(&ready).unwrap(),
                    ))
                }
                "commit" => {
                    let ready = valid_scripted_ready_record(&fixture);
                    fixture.manifest.services.clear();
                    ScriptedProjectServiceReadyReader(ScriptedReadyRead::Text(
                        toml::to_string(&ready).unwrap(),
                    ))
                }
                _ => unreachable!(),
            };

            let error = wait_for_project_service_wave(
                &ProjectServiceWave {
                    project_id: "local.readiness_fixture",
                    store: &fixture.store,
                    launcher: &launcher,
                    lifecycle: &lifecycle,
                    ready_reader: &reader,
                    pending: &fixture.pending,
                    started: &fixture.started,
                    ready_timeout: Duration::from_secs(1),
                },
                &mut fixture.manifest,
            )
            .unwrap_err();

            assert!(
                !error.to_string().is_empty(),
                "{failure} must return its error"
            );
            assert_readiness_rolled_back(&fixture);
            assert_eq!(launcher.terminated.borrow().len(), 1, "{failure}");
        }
    }

    #[test]
    fn project_service_readiness_wait_failure_and_timeout_roll_back() {
        for wait in [
            ScriptedLifecycleWait::Failed,
            ScriptedLifecycleWait::Timeout,
        ] {
            let mut fixture = project_service_readiness_fixture();
            let launcher = ScriptedProjectServiceLauncher::default();
            let lifecycle = ScriptedProjectServiceLifecycle::default();
            lifecycle.waits.borrow_mut().push_back(wait);

            let error = wait_for_project_service_wave(
                &ProjectServiceWave {
                    project_id: "local.readiness_fixture",
                    store: &fixture.store,
                    launcher: &launcher,
                    lifecycle: &lifecycle,
                    ready_reader: &ScriptedProjectServiceReadyReader(ScriptedReadyRead::Text(
                        String::new(),
                    )),
                    pending: &fixture.pending,
                    started: &fixture.started,
                    ready_timeout: Duration::from_millis(1),
                },
                &mut fixture.manifest,
            )
            .unwrap_err();

            assert!(!error.to_string().is_empty());
            assert_readiness_rolled_back(&fixture);
            assert_eq!(launcher.terminated.borrow().len(), 1);
        }
    }

    fn session_service_commands(
        daemon: &AzDaemon,
        project_id: &str,
        session: &az_session::SessionManifest,
        endpoint_kind: EndpointKind,
    ) -> Vec<ProjectServiceCommand> {
        let mut plan = daemon
            .plan_project_services(
                project_id,
                &session.slug,
                endpoint_kind,
                Some(&session.workspace_root),
            )
            .unwrap();
        apply_session_endpoint_layout(&mut plan.commands, session, endpoint_kind).unwrap();
        validate_planned_service_owner_context(&plan.commands).unwrap();
        plan.commands
    }

    #[test]
    fn cargo_dep_info_parser_handles_windows_paths_spaces_and_continuations() {
        let drive = "E:";
        let input = format!(
            "{drive}\\target\\service.exe: {drive}\\project\\src\\lib.rs \\\n{drive}\\project\\generated\\with\\ space.rs\n\n{drive}\\project\\src\\lib.rs:\n"
        );
        let paths = parse_cargo_dep_info_paths(&input);

        assert_eq!(
            paths,
            vec![
                PathBuf::from(format!(r"{drive}\project\src\lib.rs")),
                PathBuf::from(format!(r"{drive}\project\generated\with space.rs")),
            ]
        );
    }

    #[test]
    fn in_process_daemon_rpc_constructors_are_not_default_public_api() {
        let source = include_str!("lib.rs");
        let production_source = production_source_without_cfg_test_modules(source);

        for signature in [
            "pub(crate) fn with_shutdown(",
            "pub(crate) fn into_client(self)",
        ] {
            assert!(
                production_source.contains(signature),
                "`{signature}` must stay crate-private; production callers should start azd RPC instead of embedding it"
            );
        }
        assert!(
            !production_source.contains("pub fn daemon_client(")
                && !production_source.contains("pub(crate) fn daemon_client("),
            "az-daemon must not expose a default in-process client helper; use endpoint-hosted RPC or explicit test-support constructors"
        );

        for signature in [
            "pub fn new(daemon: AzDaemon)",
            "pub fn daemon(",
            "pub fn test_new(",
            "pub fn client_from_rc(",
        ] {
            assert!(
                public_functions_are_cfg_test_or_test_support(&production_source, signature),
                "`{signature}` must require the explicit test-support feature; in-process daemon clients are for tests/harnesses only"
            );
        }
    }

    #[test]
    fn progress_rpc_delivery_does_not_gate_result_write() {
        let source = include_str!("lib.rs");
        let open_rpc = source
            .split("fn ensure_project_session_services_with_progress(\n        self: capnp::capability::Rc<Self>,")
            .nth(1)
            .unwrap()
            .split("fn plan_project_build(")
            .next()
            .unwrap();
        let build_rpc = source
            .split("fn execute_project_build(\n        self: capnp::capability::Rc<Self>,")
            .nth(1)
            .unwrap()
            .split("fn plan_project_services(")
            .next()
            .unwrap();

        let result_writer = "(result).to_capnp(results.get().init_result())";
        for (body, result_label) in [
            (open_rpc, "project-session start result"),
            (build_rpc, "project-build execution result"),
        ] {
            let result = body
                .find(result_writer)
                .unwrap_or_else(|| panic!("{result_label} must use the to_capnp conversion idiom"));
            assert!(
                body.contains("let _progress_drain = tokio::task::spawn_local"),
                "progress drain must be detached for {result_label}"
            );
            assert!(
                // Indentation tracks the RPC method bodies, which sit one level
                // shallower now that they are `async fn` rather than a
                // hand-written `async move` block.
                !body[..result].contains(".await?;\n\n        match tokio::time::timeout")
                    && !body[..result].contains("PROGRESS_DRAIN_TIMEOUT"),
                "progress drain/timer must not gate {result_label}"
            );
            assert!(
                body.contains("let promise = update.send().promise;"),
                "progress RPC sends must detach their callback promises before {result_label}"
            );
            assert!(
                !body.contains("update.send().promise.await"),
                "progress callback acknowledgements must not gate {result_label}"
            );
        }
    }

    #[test]
    fn service_start_wait_ignores_failed_records_from_before_current_start_request() {
        let mut process = test_service_process_record();
        process.state = ProtoServiceProcessState::Failed;
        process.updated_unix_ms = 100;
        process.failure = Some("old worker failure".to_string());

        assert!(!service_process_failure_is_fresh(&process, Some(101)));
        assert!(service_process_failure_is_fresh(&process, Some(100)));
        assert!(service_process_failure_is_fresh(&process, None));
    }

    fn test_service_process_record() -> az_proto_session::ServiceProcessRecord {
        az_proto_session::ServiceProcessRecord {
            owner_id: "project:test".to_string(),
            owner_root: "project".to_string(),
            service_name: "asset-worker".to_string(),
            role: ServiceRole::Worker,
            run: uuid::Uuid::now_v7(),
            previous_run: None,
            endpoint: Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            program: "project/target/debug/asset-worker.exe".to_string(),
            program_artifact: None,
            cwd: "project".to_string(),
            args: Vec::new(),
            stdout_log: "worker.out.log".to_string(),
            stderr_log: "worker.err.log".to_string(),
            structured_log: "worker.capnp.log".to_string(),
            state: ProtoServiceProcessState::Planned,
            pid: None,
            process_start_time: None,
            exit_code: None,
            failure: None,
            planned_unix_ms: 1,
            updated_unix_ms: 1,
            started_unix_ms: None,
            exited_unix_ms: None,
        }
    }

    #[test]
    fn standalone_sessiond_args_can_opt_out_of_daemon_registration() {
        let args = sessiond_args(
            Path::new("project"),
            "main",
            EndpointKind::Tcp,
            None,
            true,
            1_800_000,
            &["project-host".to_string()],
        )
        .unwrap();

        assert!(!args.iter().any(|arg| arg == "--daemon-endpoint-kind"));
        assert!(!args.iter().any(|arg| arg == "--daemon-endpoint"));
        assert!(args.iter().any(|arg| arg == "--no-daemon-registration"));
        assert!(args.iter().any(|arg| arg == "--keep-alive"));
        assert_eq!(
            args.windows(2)
                .find(|window| window[0] == "--service-ready-timeout-ms")
                .map(|window| window[1].as_str()),
            Some("1800000")
        );
        assert_eq!(
            args.windows(2)
                .filter(|window| window[0] == "--start-service")
                .map(|window| window[1].as_str())
                .collect::<Vec<_>>(),
            vec!["project-host"]
        );
    }

    #[test]
    fn session_supervisor_lease_store_retains_only_the_current_owner() {
        let temp = tempfile::tempdir().unwrap();
        let store = az_session::SessionSupervisorLeaseStore::new(temp.path());
        for process_id in 1..=4 {
            store
                .acquire(
                    &session_supervisor_descriptor(Endpoint::new(
                        EndpointKind::WindowsNamedPipe,
                        r"\\.\pipe\azoth-session-session-supervisor",
                    )),
                    ProcessIdentity {
                        process_id,
                        process_start_time: u64::from(process_id),
                    },
                    u64::from(process_id),
                )
                .unwrap();
        }

        let current = store.load().unwrap().record.unwrap();
        assert_eq!(current.process.process_id, 4);
        let descriptor = current.descriptor();
        assert_eq!(descriptor.id, ServiceId::new("azoth", "session-supervisor"));
        assert_eq!(descriptor.role, ServiceRole::SessionSupervisor);
        assert_eq!(
            descriptor.endpoint.address,
            r"\\.\pipe\azoth-session-session-supervisor"
        );
    }

    #[test]
    fn stale_lease_with_dead_pid_is_reaped_after_heartbeat_expiry() {
        let temp = tempfile::tempdir().unwrap();
        let store = az_session::SessionSupervisorLeaseStore::new(temp.path());
        let lease = store
            .acquire(
                &session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41001")),
                ProcessIdentity {
                    process_id: 44,
                    process_start_time: 99,
                },
                1,
            )
            .unwrap();
        let now = 1 + az_session::SESSION_SUPERVISOR_LEASE_EXPIRY_MS + 1;

        assert_eq!(
            assess_supervisor_lease_process(&lease, None, now),
            SupervisorLeaseProcessAssessment::Dead {
                heartbeat_expired: true
            }
        );
        assert!(store.clear_if_process(lease.process).unwrap());
        assert!(store.load().unwrap().record.is_none());
    }

    #[test]
    fn pid_reuse_spoof_is_rejected_even_when_pid_matches() {
        let temp = tempfile::tempdir().unwrap();
        let store = az_session::SessionSupervisorLeaseStore::new(temp.path());
        let lease = store
            .acquire(
                &session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41002")),
                ProcessIdentity {
                    process_id: 77,
                    process_start_time: 100,
                },
                500,
            )
            .unwrap();

        assert_eq!(
            assess_supervisor_lease_process(
                &lease,
                Some(ProcessIdentity {
                    process_id: 77,
                    process_start_time: 101,
                }),
                501,
            ),
            SupervisorLeaseProcessAssessment::PidReused {
                actual_start_time: 101
            }
        );
    }

    #[test]
    fn heartbeat_expired_but_alive_lease_requires_rechallenge() {
        let temp = tempfile::tempdir().unwrap();
        let store = az_session::SessionSupervisorLeaseStore::new(temp.path());
        let process = ProcessIdentity {
            process_id: 88,
            process_start_time: 200,
        };
        let lease = store
            .acquire(
                &session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41003")),
                process,
                1,
            )
            .unwrap();
        let now = 1 + az_session::SESSION_SUPERVISOR_LEASE_EXPIRY_MS + 1;

        assert_eq!(
            assess_supervisor_lease_process(&lease, Some(process), now),
            SupervisorLeaseProcessAssessment::Live {
                heartbeat_expired: true
            }
        );
    }

    #[test]
    fn sessiond_restart_retains_exactly_current_and_previous_output_and_structured_logs() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("run");
        let manifest = az_session::SessionManifest::new(
            az_session::SessionId::new(),
            "local.sessiond_logs".to_string(),
            "main".to_string(),
            temp.path().to_path_buf(),
            temp.path().to_path_buf(),
            run_dir.clone(),
            1,
        );
        std::fs::create_dir_all(run_dir.join("logs")).unwrap();
        let output = az_session::sessiond_output_log_path(&manifest);
        let structured = az_session::sessiond_structured_log_path(&manifest);
        let output_previous = az_service_supervision::previous_log_path(&output);
        let structured_previous = az_service_supervision::previous_log_path(&structured);
        for (path, contents) in [
            (&output, "first output"),
            (&structured, "first structured"),
            (&output_previous, "obsolete output"),
            (&structured_previous, "obsolete structured"),
        ] {
            std::fs::write(path, contents).unwrap();
        }

        rotate_sessiond_launch_logs(&manifest).unwrap();
        std::fs::write(&output, "second output").unwrap();
        std::fs::write(&structured, "second structured").unwrap();
        rotate_sessiond_launch_logs(&manifest).unwrap();
        std::fs::write(&output, "third output").unwrap();
        std::fs::write(&structured, "third structured").unwrap();

        assert_eq!(std::fs::read_to_string(&output).unwrap(), "third output");
        assert_eq!(
            std::fs::read_to_string(&output_previous).unwrap(),
            "second output"
        );
        assert_eq!(
            std::fs::read_to_string(&structured).unwrap(),
            "third structured"
        );
        assert_eq!(
            std::fs::read_to_string(&structured_previous).unwrap(),
            "second structured"
        );
        assert_eq!(
            std::fs::read_dir(&run_dir)
                .unwrap()
                .filter(|entry| entry.as_ref().unwrap().path().is_file())
                .count(),
            2
        );
        assert_eq!(std::fs::read_dir(run_dir.join("logs")).unwrap().count(), 2);
    }

    #[test]
    fn repeated_tcp_lease_recovery_only_publishes_a_changed_descriptor_once() {
        let temp = tempfile::tempdir().unwrap();
        let project_manifest =
            ProjectManifest::new("local.tcp_lease_recovery", "TCP Lease Recovery", "0.1.0");
        write_project_manifest_with_lock(temp.path(), &project_manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        let manager = daemon.session_manager(temp.path()).unwrap();
        let initial = manager.session("main").unwrap();
        let server_manager = daemon.session_manager(temp.path()).unwrap();
        let server = az_session::start_session_supervisor_rpc_server_with_manager(
            server_manager,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            "main",
        )
        .unwrap();
        let leased_descriptor = az_service_catalog::session_supervisor_service_descriptor(
            initial.id.0,
            uuid::Uuid::now_v7(),
            server.endpoint().clone(),
        );
        manager
            .register_service_descriptor("main", &leased_descriptor)
            .unwrap();
        let process = ProcessIdentity::current().unwrap();
        let lease_store = az_session::SessionSupervisorLeaseStore::new(&initial.run_dir);
        lease_store
            .acquire(&leased_descriptor, process, current_unix_ms())
            .unwrap();
        server.set_run(leased_descriptor.run);
        server
            .set_supervision_identity(az_proto_session::SessionSupervisorIdentity {
                process_id: process.process_id,
                process_start_time: process.process_start_time,
                descriptor: leased_descriptor.clone(),
            })
            .unwrap();

        let stale_published_descriptor = az_service_catalog::session_supervisor_service_descriptor(
            initial.id.0,
            uuid::Uuid::now_v7(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:9"),
        );
        manager
            .register_service_descriptor("main", &stale_published_descriptor)
            .unwrap();
        let stale_manifest = manager.session("main").unwrap();

        let recovered = daemon
            .existing_reachable_session_supervisor_snapshot(&stale_manifest)
            .unwrap()
            .unwrap()
            .descriptor;

        assert_eq!(recovered, leased_descriptor);
        assert_eq!(recovered.run, leased_descriptor.run);
        assert_leased_descriptor_is_republished(
            &daemon,
            &manager,
            &project.project_id,
            &initial,
            &leased_descriptor,
        );

        assert_repeat_recovery_is_inert(&daemon, &manager, &initial, &leased_descriptor);

        server.stop();
    }

    /// The recovered lease must be republished into the session manifest and
    /// the daemon registry, atomically.
    fn assert_leased_descriptor_is_republished(
        daemon: &AzDaemon,
        manager: &az_session::SessionManager,
        project_id: &str,
        initial: &az_session::SessionManifest,
        leased_descriptor: &ServiceDescriptor,
    ) {
        let republished = manager
            .session("main")
            .unwrap()
            .service_descriptor(
                &ServiceId::new(
                    SESSION_SUPERVISOR_NAMESPACE,
                    SESSION_SUPERVISOR_SERVICE_NAME,
                ),
                ServiceRole::SessionSupervisor,
            )
            .unwrap();
        assert_eq!(&republished, leased_descriptor);
        assert_eq!(republished.run, leased_descriptor.run);
        let resolved = daemon
            .resolve_session_supervisor(project_id, "main")
            .unwrap();
        assert_eq!(&resolved, leased_descriptor);
        assert_eq!(resolved.run, leased_descriptor.run);
        assert_eq!(
            std::fs::read_dir(initial.run_dir.join("transactions"))
                .unwrap()
                .count(),
            0,
            "atomic descriptor republish must leave no pending manifest transaction"
        );
    }

    /// Recovering an already-published descriptor must touch nothing on disk:
    /// no manifest rewrite, no durable row bump, and no transaction entry.
    fn assert_repeat_recovery_is_inert(
        daemon: &AzDaemon,
        manager: &az_session::SessionManager,
        initial: &az_session::SessionManifest,
        leased_descriptor: &ServiceDescriptor,
    ) {
        let manifest_path = initial.manifest_path();
        let first_published_bytes = std::fs::read(&manifest_path).unwrap();
        let first_published_mtime = std::fs::metadata(&manifest_path)
            .unwrap()
            .modified()
            .unwrap();
        let transaction_root = initial.run_dir.join("transactions");
        let first_transaction_mtime = std::fs::metadata(&transaction_root)
            .unwrap()
            .modified()
            .unwrap();
        let first_published_manifest = manager.session("main").unwrap();
        std::thread::sleep(Duration::from_millis(25));

        let recovered_again = daemon
            .existing_reachable_session_supervisor_snapshot(&first_published_manifest)
            .unwrap()
            .unwrap()
            .descriptor;

        assert_eq!(&recovered_again, leased_descriptor);
        assert_eq!(recovered_again.run, leased_descriptor.run);
        assert_eq!(
            std::fs::read(&manifest_path).unwrap(),
            first_published_bytes
        );
        assert_eq!(
            std::fs::metadata(&manifest_path)
                .unwrap()
                .modified()
                .unwrap(),
            first_published_mtime,
            "an unchanged recovered descriptor must not rewrite the session manifest"
        );
        assert_eq!(
            manager.session("main").unwrap().updated_unix_ms,
            first_published_manifest.updated_unix_ms,
            "an unchanged recovery must not mutate the durable manifest row"
        );
        assert_eq!(
            std::fs::read_dir(&transaction_root).unwrap().count(),
            0,
            "an unchanged recovery must not create a manifest transaction"
        );
        assert_eq!(
            std::fs::metadata(&transaction_root)
                .unwrap()
                .modified()
                .unwrap(),
            first_transaction_mtime,
            "an unchanged recovery must not even create and remove a transaction entry"
        );
    }

    #[test]
    fn daemon_lifecycle_never_requests_workspace_status() {
        let source = include_str!("lib.rs");
        let production_source = production_source_without_cfg_test_modules(source);

        assert!(
            !production_source.contains(".status_request()"),
            "daemon lifecycle probes must not trigger a source-control workspace scan"
        );
        assert!(
            !production_source.contains("SessionWorkspaceStatus"),
            "daemon lifecycle code must use health, challenge, and durable manifests instead of workspace status"
        );
    }

    #[test]
    fn daemon_registered_sessiond_args_include_the_registration_endpoint() {
        let daemon_endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37123");
        let args = sessiond_args(
            Path::new("project"),
            "main",
            EndpointKind::Tcp,
            Some(&daemon_endpoint),
            true,
            30_000,
            &[],
        )
        .unwrap();

        assert!(
            args.windows(2)
                .any(|window| window[0] == "--daemon-endpoint-kind" && window[1] == "tcp")
        );
        assert!(
            args.windows(2).any(|window| {
                window[0] == "--daemon-endpoint" && window[1] == "127.0.0.1:37123"
            })
        );
    }

    #[test]
    fn editor_lease_tracks_live_owner_process() {
        let daemon = AzDaemon::new();
        let owner_process = ProcessIdentity::current().unwrap();
        let owner_process = ProtoProcessIdentity {
            process_id: owner_process.process_id,
            process_start_time: owner_process.process_start_time,
        };
        let request = TouchEditorLeaseRequest {
            capability: capability(DAEMON_LEASE_PERMISSION),
            lease_id: editor_process_lease_id(owner_process),
            owner_process,
            purpose: "editor test owner".to_string(),
        };

        let first = daemon.touch_editor_lease(&request).unwrap();
        let second = daemon.touch_editor_lease(&request).unwrap();

        assert!(first.accepted);
        assert_eq!(first.active_lease_count, 1);
        assert!(second.accepted);
        assert_eq!(second.active_lease_count, 1);
        assert_eq!(daemon.active_editor_lease_count(), 1);
        let shutdown = az_work::CancellationToken::new();
        shutdown.cancel();
        daemon.wait_for_shutdown(&shutdown, false).unwrap();
        assert!(matches!(
            daemon.touch_editor_lease(&request),
            Err(AzDaemonError::InvalidEditorLease { .. })
        ));
    }

    #[test]
    fn editor_sidecar_initial_empty_transition_closes_lease_admissions() {
        let daemon = AzDaemon::new();
        let shutdown = az_work::CancellationToken::new();

        daemon.wait_for_shutdown(&shutdown, true).unwrap();

        let owner = ProcessIdentity::current().unwrap();
        let owner = ProtoProcessIdentity {
            process_id: owner.process_id,
            process_start_time: owner.process_start_time,
        };
        let request = TouchEditorLeaseRequest {
            capability: capability(DAEMON_LEASE_PERMISSION),
            lease_id: editor_process_lease_id(owner),
            owner_process: owner,
            purpose: "late editor test owner".to_string(),
        };
        assert!(shutdown.is_cancelled());
        assert!(matches!(
            daemon.touch_editor_lease(&request),
            Err(AzDaemonError::InvalidEditorLease { .. })
        ));
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn editor_sidecar_shutdown_follows_exact_owner_exit_without_sampling() {
        #[cfg(windows)]
        let mut child = {
            let mut command = Command::new("ping.exe");
            command
                .args(["-n", "30", "127.0.0.1"])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command.spawn().unwrap()
        };
        #[cfg(unix)]
        let mut child = {
            let mut command = Command::new("sleep");
            command.arg("30");
            command.spawn().unwrap()
        };
        let owner = ProcessIdentity::capture(child.id()).unwrap().unwrap();
        let owner = ProtoProcessIdentity {
            process_id: owner.process_id,
            process_start_time: owner.process_start_time,
        };
        let daemon = AzDaemon::new();
        let request = TouchEditorLeaseRequest {
            capability: capability(DAEMON_LEASE_PERMISSION),
            lease_id: editor_process_lease_id(owner),
            owner_process: owner,
            purpose: "short-lived editor test owner".to_string(),
        };
        daemon.touch_editor_lease(&request).unwrap();
        let shutdown = az_work::CancellationToken::new();
        let waiter_daemon = daemon.clone();
        let waiter_shutdown = shutdown.clone();
        let waiter =
            std::thread::spawn(move || waiter_daemon.wait_for_shutdown(&waiter_shutdown, true));

        child.kill().unwrap();
        child.wait().unwrap();
        waiter.join().unwrap().unwrap();

        assert!(shutdown.is_cancelled());
        assert_eq!(daemon.active_editor_lease_count(), 0);
        assert!(matches!(
            daemon.touch_editor_lease(&request),
            Err(AzDaemonError::InvalidEditorLease { .. })
        ));
    }

    #[test]
    fn editor_lease_requires_lease_permission() {
        let daemon = AzDaemon::new();
        let owner_process = ProcessIdentity::current().unwrap();
        let owner_process = ProtoProcessIdentity {
            process_id: owner_process.process_id,
            process_start_time: owner_process.process_start_time,
        };
        let request = TouchEditorLeaseRequest {
            capability: capability(DAEMON_READ_PERMISSION),
            lease_id: editor_process_lease_id(owner_process),
            owner_process,
            purpose: "editor test owner".to_string(),
        };

        let error = daemon.touch_editor_lease(&request).unwrap_err();

        assert!(matches!(error, AzDaemonError::InvalidCapability { .. }));
    }

    #[test]
    fn editor_lease_does_not_transfer_to_a_reused_pid() {
        let daemon = AzDaemon::new();
        let live = ProcessIdentity::current().unwrap();
        let stale = ProcessIdentity {
            process_id: live.process_id,
            process_start_time: live.process_start_time.wrapping_sub(1),
        };
        let stale = ProtoProcessIdentity {
            process_id: stale.process_id,
            process_start_time: stale.process_start_time,
        };
        let request = TouchEditorLeaseRequest {
            capability: capability(DAEMON_LEASE_PERMISSION),
            lease_id: editor_process_lease_id(stale),
            owner_process: stale,
            purpose: "stale editor test owner".to_string(),
        };

        assert!(matches!(
            daemon.touch_editor_lease(&request),
            Err(AzDaemonError::InvalidEditorLease { .. })
        ));

        assert_eq!(daemon.active_editor_lease_count(), 0);
    }

    fn capability(permission: &str) -> Capability {
        Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience(DAEMON_AUDIENCE)
            .with_permissions([permission])
    }

    fn project_record() -> ProjectRecord {
        let root = std::env::temp_dir()
            .join("azoth-daemon-tests")
            .join("example");
        ProjectRecord {
            project_id: "local.example".to_string(),
            name: "Example".to_string(),
            root: root.to_string_lossy().into_owned(),
            manifest_path: project_manifest_path(&root).to_string_lossy().into_owned(),
            engine_version: "0.1.0".to_string(),
        }
    }

    #[test]
    fn refused_cleanup_blocks_project_service_supersession() {
        let project = project_record();
        let root = PathBuf::from(&project.root);
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0");
        let mut process = ServiceProcessRecord::planned(
            "asset-processor",
            SupervisedServiceRole::AssetProcessor,
            uuid::Uuid::now_v7(),
            &endpoint,
            "asset-processor".to_string(),
            root.clone(),
            Vec::new(),
            root.join("processor.out"),
            root.join("processor.err"),
            root.join("processor.log"),
            None,
            1,
        );
        process
            .mark_running(ProcessIdentity::current().unwrap(), 2)
            .unwrap();
        let cleanup = RecordedServiceProcessCleanup::Unattributable { pid: 73 };

        let error = require_project_process_gone(&project, &process, cleanup.clone())
            .expect_err("a refused cleanup must block project-service replacement");

        assert!(matches!(
            error,
            AzDaemonError::ProjectServiceCleanupRefused {
                project_id,
                service,
                cleanup: actual,
            } if project_id == project.project_id && service == "asset-processor" && actual == cleanup
        ));
        assert_eq!(process.state, ServiceProcessState::Running);
    }

    fn test_session_id(byte: u8) -> uuid::Uuid {
        uuid::Uuid::from_bytes([byte; 16])
    }

    fn session_supervisor_descriptor(endpoint: Endpoint) -> ServiceDescriptor {
        let session_id = test_session_id(0x5d);
        ServiceDescriptor::new(
            ServiceId::new(
                SESSION_SUPERVISOR_NAMESPACE,
                SESSION_SUPERVISOR_SERVICE_NAME,
            ),
            ServiceRole::SessionSupervisor,
            endpoint,
        )
        .with_run(uuid::Uuid::now_v7())
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_session(session_id)
            .with_audience(SESSION_SUPERVISOR_AUDIENCE)
            .with_permissions([
                SESSION_READ_PERMISSION,
                SESSION_SAVE_PERMISSION,
                SESSION_EXEC_PERMISSION,
                SESSION_MANAGE_PERMISSION,
            ])
            .with_token_hash([0xe1, 0x01]),
        )
        .with_capability(
            Capability::new(
                ServiceId::new(
                    DAEMON_SESSION_SERVICE_NAMESPACE,
                    DAEMON_SESSION_SERVICE_NAME,
                ),
                ServiceRole::Daemon,
            )
            .with_session(session_id)
            .with_audience(SESSION_SUPERVISOR_AUDIENCE)
            .with_permissions([SESSION_READ_PERMISSION, SESSION_MANAGE_PERMISSION])
            .with_token_hash([0xd1, 0x01]),
        )
    }

    fn write_project_manifest_with_lock(root: &Path, manifest: &ProjectManifest) {
        write_project_manifest(root, manifest).unwrap();
        refresh_project_lock(root).unwrap();
    }

    fn multiplayer_build_fixture() -> (
        tempfile::TempDir,
        ResolvedProjectGraph,
        GeneratedTargetsSyncReport,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let gem_root = temp.path().join("gems").join("game");
        fs::create_dir_all(&gem_root).unwrap();

        let mut manifest = ProjectManifest::new("local.sample", "Sample Game", "0.1.0");
        manifest.project.primary_gem = Some("local.sample.game".to_string());
        manifest.topology.kind = az_project::ProjectTopologyKind::MultiplayerClientServer;
        manifest.gems.push(ProjectGem {
            id: "local.sample.game".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            path: Some(PathBuf::from("gems/game")),
            linkage: None,
        });
        manifest.tools.build_targets.extend([
            ProjectBuildTarget::package("auth-server", "sample-auth-server"),
            ProjectBuildTarget::package("stdb", "sample-database"),
        ]);
        write_project_manifest(temp.path(), &manifest).unwrap();
        let mut gem_manifest = GemManifest::new("local.sample.game", "Game", "0.1.0");
        gem_manifest
            .tools
            .build_targets
            .push(ProjectBuildTarget::package("launcher", "sample-launcher"));
        write_gem_manifest(&gem_root, &gem_manifest).unwrap();
        refresh_project_lock(temp.path()).unwrap();

        let graph = load_resolved_project_graph(temp.path()).unwrap();
        let target_directory = temp.path().join("target");
        let generated = GeneratedTargetsSyncReport {
            status: az_project::GeneratedTargetsSyncStatus::Unchanged,
            target_directory,
            workspace_root: Some(temp.path().join(".azoth/targets")),
            old_fingerprint: None,
            fingerprint: Some("fixture".to_string()),
            targets: ["client", "server", "unified", "headless-server"]
                .into_iter()
                .map(|name| GeneratedTargetPackage {
                    name: name.to_string(),
                    package: generated_package_name(name),
                    roles: Vec::new(),
                    linked_packages: Vec::new(),
                })
                .collect(),
            manifests: Vec::new(),
        };
        (temp, graph, generated)
    }

    fn write_service_cargo_package_with_path_dependency(
        root: &Path,
        package: &str,
        bin: &str,
        dependency: &str,
    ) -> PathBuf {
        let bin_path = root.join("src").join("bin").join(format!("{bin}.rs"));
        fs::create_dir_all(bin_path.parent().unwrap()).unwrap();
        fs::write(&bin_path, "fn main() {}\n").unwrap();

        let dependency_root = root.join("engine").join(dependency);
        let dependency_src = dependency_root.join("src").join("lib.rs");
        fs::create_dir_all(dependency_src.parent().unwrap()).unwrap();
        fs::write(&dependency_src, "pub fn marker() {}\n").unwrap();

        let manifest = format!(
            r#"[package]
name = "{package}"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "{bin}"
path = "src/bin/{bin}.rs"

[dependencies]
"{dependency}" = {{ path = "engine/{dependency}" }}
"#
        );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();

        let dependency_manifest = format!(
            r#"[package]
name = "{dependency}"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#
        );
        fs::write(dependency_root.join("Cargo.toml"), dependency_manifest).unwrap();
        dependency_src
    }

    fn init_git_repo_with_commit(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["config", "user.email", "azoth@example.invalid"]);
        run_git(root, &["config", "user.name", "Azoth Test"]);
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "Initial project"]);
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
        assert!(status.success(), "git {args:?} failed with {status}");
    }

    fn add_asset_processor_target(manifest: &mut ProjectManifest, package: &str) {
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "asset-processor",
                ProjectServiceRole::AssetProcessor,
                package,
                "asset-processor",
            ));
    }

    #[test]
    fn daemon_capabilities_reject_expired_lifetime() {
        let expired = capability(DAEMON_READ_PERMISSION).with_expires_unix_ms(1);

        assert!(matches!(
            validate_capability(&expired, DAEMON_READ_PERMISSION),
            Err(AzDaemonError::InvalidCapability { .. })
        ));
    }

    #[test]
    fn daemon_registers_project_roots_from_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = ProjectManifest::new("local.daemon", "Daemon Project", "0.1.0");
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();

        let record = daemon.register_project_root(temp.path()).unwrap();

        assert_eq!(record.project_id, "local.daemon");
        assert_eq!(record.name, "Daemon Project");
        assert_eq!(record.engine_version, "0.1.0");
        assert_eq!(daemon.resolve_project("local.daemon"), Some(record));
    }

    #[test]
    fn daemon_register_project_root_canonicalizes_registry_paths() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = ProjectManifest::new("local.canonical", "Canonical Project", "0.1.0");
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        let noncanonical_root = temp.path().join(".");

        let record = daemon.register_project_root(&noncanonical_root).unwrap();

        let expected_root = normalize_existing_path(temp.path()).unwrap();
        assert_eq!(record.root, expected_root.to_string_lossy());
        assert_eq!(
            record.manifest_path,
            project_manifest_path(&expected_root).to_string_lossy()
        );
    }

    #[test]
    fn daemon_ensures_project_session_by_creating_then_reusing_supervision_scope() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = ProjectManifest::new("local.session", "Session Project", "0.1.0");
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();

        let created = daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        let reused = daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();

        assert!(created.created);
        assert!(!reused.created);
        assert_eq!(created.manifest.project_id, "local.session");
        assert_eq!(created.manifest.slug, "main");
        assert_eq!(
            created.manifest.workspace_root,
            reused.manifest.workspace_root
        );
        assert!(Path::new(&created.manifest.workspace_root).exists());
    }

    #[test]
    fn daemon_ensures_existing_failed_preserved_project_session_by_recovering_it() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest =
            ProjectManifest::new("local.session_recover", "Session Recover Project", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "session_recover_game",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "session_recover_game");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                RUNTIME_HOST_SERVICE_NAME,
                ProjectServiceRole::RuntimeHost,
                "session_recover_game",
                RUNTIME_HOST_SERVICE_NAME,
            ));
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();

        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        daemon
            .prepare_project_session_services(
                &project.project_id,
                "main",
                EndpointKind::WindowsNamedPipe,
                true,
            )
            .unwrap();
        let manager = daemon.session_manager(temp.path()).unwrap();
        let session = manager.session("main").unwrap();
        let commands = session_service_commands(
            &daemon,
            &project.project_id,
            &session,
            EndpointKind::WindowsNamedPipe,
        );
        manager
            .mark_service_exited(
                "main",
                &ServiceProcessKey::new(
                    RUNTIME_HOST_SERVICE_NAME,
                    SupervisedServiceRole::RuntimeHost,
                ),
                Some(1),
                Some("startup boundary probe failed".to_string()),
            )
            .unwrap();
        let failed_manifest = manager.session("main").unwrap();
        assert!(
            !session_services_have_persisted_reusable_launch_plan(
                &failed_manifest,
                &[RUNTIME_HOST_SERVICE_NAME.to_string()],
                &commands,
                ServiceProgramFreshnessPolicy::Verify,
            ),
            "failed process rows must not be considered reusable"
        );
        manager
            .mark_failed_preserved("main", "transient project-host failure")
            .unwrap();

        let recovered = daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();

        assert!(!recovered.created);
        assert_eq!(recovered.manifest.slug, "main");
        assert_eq!(
            recovered.manifest.state,
            az_proto_session::SessionState::Active
        );
        assert!(
            !Path::new(&recovered.manifest.run_dir)
                .join("failure.txt")
                .exists()
        );
    }

    #[test]
    fn daemon_register_project_root_rejects_stale_project_lock() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.stale_lock", "Stale Lock", "0.1.0");
        write_project_manifest_with_lock(temp.path(), &manifest);
        manifest.project.name = "Stale Lock Edited".to_string();
        write_project_manifest(temp.path(), &manifest).unwrap();
        let daemon = AzDaemon::new();

        let error = daemon.register_project_root(temp.path()).unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::ProjectManifest(ProjectManifestError::StaleProjectLock { .. })
        ));
        assert_eq!(daemon.resolve_project("local.stale_lock"), None);
    }

    #[test]
    fn daemon_rejects_relative_project_record_root() {
        let daemon = AzDaemon::new();
        let mut project = project_record();
        project.root = "relative/project".to_string();
        project.manifest_path = "relative/project/azoth.toml".to_string();

        let error = daemon.register_project(&project).unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidProjectRecord { project_id, reason }
                if project_id == "local.example" && reason.contains("not absolute")
        ));
    }

    #[test]
    fn daemon_rejects_project_record_manifest_outside_root() {
        let daemon = AzDaemon::new();
        let mut project = project_record();
        let other_root = std::env::temp_dir()
            .join("azoth-daemon-tests")
            .join("other");
        project.manifest_path = project_manifest_path(&other_root)
            .to_string_lossy()
            .into_owned();

        let error = daemon.register_project(&project).unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidProjectRecord { project_id, reason }
                if project_id == "local.example" && reason.contains("must match")
        ));
    }

    #[test]
    fn daemon_project_registry_persists_registered_projects() {
        let temp = tempfile::tempdir().unwrap();
        let registry_path = temp.path().join("azd.projects.toml");
        let daemon = AzDaemon::with_project_registry_path(&registry_path).unwrap();
        let project = project_record();

        daemon.register_project(&project).unwrap();

        let restored = AzDaemon::with_project_registry_path(&registry_path).unwrap();
        assert_eq!(
            restored.project_registry_path(),
            Some(registry_path.as_path())
        );
        assert_eq!(restored.resolve_project(&project.project_id), Some(project));
    }

    fn stage_pending_project_registry_write(
        path: &Path,
        previous: &BTreeMap<String, ProjectRecord>,
        pending: &BTreeMap<String, ProjectRecord>,
    ) -> PathBuf {
        write_project_registry(path, previous).unwrap();
        let previous_contents = fs::read(path).unwrap();

        fs::remove_file(path).unwrap();
        fs::create_dir(path).unwrap();
        let error = write_project_registry(path, pending).unwrap_err();
        assert!(matches!(
            error,
            AzDaemonError::ProjectRegistryTransaction { .. }
        ));

        fs::remove_dir(path).unwrap();
        fs::write(path, previous_contents).unwrap();
        project_registry_transaction_root(path)
    }

    fn project_registry_transaction_entry_count(root: &Path) -> usize {
        fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .count()
    }

    #[test]
    fn daemon_project_registry_read_recovers_pending_file_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let registry_path = temp.path().join("azd.projects.toml");
        let previous = BTreeMap::new();
        let mut pending_project = project_record();
        pending_project.name = "Pending project".to_string();
        let pending = BTreeMap::from([(pending_project.project_id.clone(), pending_project)]);
        let transaction_root =
            stage_pending_project_registry_write(&registry_path, &previous, &pending);

        let restored = read_project_registry(&registry_path).unwrap();

        assert_eq!(restored, pending);
        assert_eq!(
            project_registry_transaction_entry_count(&transaction_root),
            0
        );
    }

    #[test]
    fn daemon_project_registry_write_recovers_before_commit() {
        let temp = tempfile::tempdir().unwrap();
        let registry_path = temp.path().join("azd.projects.toml");
        let previous = BTreeMap::new();
        let mut pending_project = project_record();
        pending_project.name = "Pending project".to_string();
        let pending = BTreeMap::from([(pending_project.project_id.clone(), pending_project)]);
        let transaction_root =
            stage_pending_project_registry_write(&registry_path, &previous, &pending);

        let mut final_project = project_record();
        final_project.name = "Final project".to_string();
        let final_projects = BTreeMap::from([(final_project.project_id.clone(), final_project)]);
        write_project_registry(&registry_path, &final_projects).unwrap();

        let restored = read_project_registry(&registry_path).unwrap();
        assert_eq!(restored, final_projects);
        assert_eq!(
            project_registry_transaction_entry_count(&transaction_root),
            0
        );
    }

    #[test]
    fn daemon_project_registry_drops_stale_project_records() {
        let temp = tempfile::tempdir().unwrap();
        let registry_path = temp.path().join("azd.projects.toml");
        let mut stale = project_record();
        stale.project_id = "local.stale".to_string();
        stale.manifest_path = temp
            .path()
            .join("project.toml")
            .to_string_lossy()
            .into_owned();
        let valid = project_record();
        let file = ProjectRegistryFile {
            schema_version: PROJECT_REGISTRY_SCHEMA_VERSION,
            projects: vec![stale, valid.clone()],
        };
        std::fs::write(&registry_path, toml::to_string_pretty(&file).unwrap()).unwrap();

        let restored = AzDaemon::with_project_registry_path(&registry_path).unwrap();

        assert_eq!(restored.resolve_project("local.stale"), None);
        assert_eq!(restored.resolve_project(&valid.project_id), Some(valid));
    }

    #[test]
    fn daemon_project_registry_does_not_persist_live_session_supervisors() {
        let temp = tempfile::tempdir().unwrap();
        let registry_path = temp.path().join("azd.projects.toml");
        let daemon = AzDaemon::with_project_registry_path(&registry_path).unwrap();
        let project = project_record();
        let descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37670"));

        daemon.register_project(&project).unwrap();
        daemon
            .register_session_supervisor(&project.project_id, "lighting", &descriptor)
            .unwrap();

        let restored = AzDaemon::with_project_registry_path(&registry_path).unwrap();
        assert_eq!(restored.resolve_project(&project.project_id), Some(project));
        assert_eq!(
            restored.resolve_session_supervisor("local.example", "lighting"),
            None
        );
    }

    #[test]
    fn daemon_rejects_in_process_session_supervisor_registration() {
        let daemon = AzDaemon::new();
        let project = project_record();
        let descriptor = session_supervisor_descriptor(Endpoint::in_process("session:example"));

        daemon.register_project(&project).unwrap();
        let error = daemon
            .register_session_supervisor(&project.project_id, "lighting", &descriptor)
            .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::UnsupportedEndpointKind {
                operation: "azd session-supervisor registration",
                kind: EndpointKind::InProcess
            }
        ));
        assert_eq!(
            daemon.resolve_session_supervisor(&project.project_id, "lighting"),
            None
        );
    }

    #[test]
    fn daemon_rejects_outdated_session_supervisor_descriptor_before_rpc() {
        let daemon = AzDaemon::new();
        let project = project_record();
        let mut descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37669"));
        descriptor.protocol = ProtocolVersion {
            major: 0,
            minor: 1,
            patch: 0,
        };

        daemon.register_project(&project).unwrap();
        let error = daemon
            .register_session_supervisor(&project.project_id, "lighting", &descriptor)
            .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidSessionSupervisorDescriptor { reason }
                if reason.contains("unavailable until restarted")
                    && reason.contains("0.1.0")
                    && reason.contains("0.3.0")
        ));
        assert_eq!(
            daemon.resolve_session_supervisor(&project.project_id, "lighting"),
            None
        );
    }

    #[test]
    fn daemon_rejects_non_canonical_session_supervisor_descriptor_id() {
        let daemon = AzDaemon::new();
        let project = project_record();
        let mut descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37669"));
        descriptor.id = ServiceId::new("azoth", "not-session-supervisor");

        daemon.register_project(&project).unwrap();
        let error = daemon
            .register_session_supervisor(&project.project_id, "lighting", &descriptor)
            .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidSessionSupervisorDescriptor { reason }
                if reason.contains("canonical service id")
        ));
        assert_eq!(
            daemon.resolve_session_supervisor(&project.project_id, "lighting"),
            None
        );
    }

    #[test]
    fn daemon_rejects_unbrokered_session_supervisor_descriptor_capabilities() {
        let daemon = AzDaemon::new();
        let project = project_record();
        let bare_descriptor = ServiceDescriptor::new(
            ServiceId::new(
                SESSION_SUPERVISOR_NAMESPACE,
                SESSION_SUPERVISOR_SERVICE_NAME,
            ),
            ServiceRole::SessionSupervisor,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37668"),
        );
        let mut tokenless_descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37667"));
        tokenless_descriptor.capabilities[0].token_hash.clear();

        daemon.register_project(&project).unwrap();
        let bare_error = daemon
            .register_session_supervisor(&project.project_id, "bare", &bare_descriptor)
            .unwrap_err();
        let tokenless_error = daemon
            .register_session_supervisor(&project.project_id, "tokenless", &tokenless_descriptor)
            .unwrap_err();

        assert!(matches!(
            bare_error,
            AzDaemonError::InvalidSessionSupervisorDescriptor { reason }
                if reason.contains("no brokered capability templates")
        ));
        assert!(matches!(
            tokenless_error,
            AzDaemonError::InvalidSessionSupervisorDescriptor { reason }
                if reason.contains("brokered token hash")
        ));
        assert!(
            daemon
                .list_session_supervisors(&project.project_id)
                .is_empty()
        );
    }

    #[test]
    fn daemon_rejects_expired_session_supervisor_descriptor_capabilities() {
        let daemon = AzDaemon::new();
        let project = project_record();
        let mut descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37665"));
        descriptor.capabilities[0].expires_unix_ms = 1;

        daemon.register_project(&project).unwrap();
        let error = daemon
            .register_session_supervisor(&project.project_id, "expired", &descriptor)
            .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidSessionSupervisorDescriptor { reason }
                if reason.contains("lifetime is invalid")
        ));
        assert!(
            daemon
                .list_session_supervisors(&project.project_id)
                .is_empty()
        );
    }

    #[test]
    fn daemon_rejects_unexpected_session_supervisor_descriptor_capabilities() {
        let daemon = AzDaemon::new();
        let project = project_record();
        let mut descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37666"));
        descriptor.capabilities.push(
            Capability::new(ServiceId::new("azoth", "asset-worker"), ServiceRole::Worker)
                .with_session(test_session_id(0x5d))
                .with_audience(SESSION_SUPERVISOR_AUDIENCE)
                .with_permissions([SESSION_READ_PERMISSION])
                .with_token_hash([0xaa, 0x55]),
        );

        daemon.register_project(&project).unwrap();
        let error = daemon
            .register_session_supervisor(&project.project_id, "worker", &descriptor)
            .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidSessionSupervisorDescriptor { reason }
                if reason.contains("not a valid session-supervisor caller")
        ));
        assert!(
            daemon
                .list_session_supervisors(&project.project_id)
                .is_empty()
        );
    }

    #[test]
    fn daemon_rpc_registers_and_resolves_project_and_session_supervisor() {
        let rpc = Rc::new(AzDaemonRpc::new(AzDaemon::new()));
        let client = AzDaemonRpc::client_from_rc(&rpc);
        let project = project_record();

        let mut register_project = client.register_project_request();
        (RegisterProjectRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project: project.clone(),
        })
        .to_capnp(register_project.get().init_request())
        .unwrap();
        let register_project_response =
            executor::block_on(register_project.send().promise).unwrap();
        let registered = ProjectRecord::from_capnp(
            register_project_response
                .get()
                .unwrap()
                .get_project()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(registered, project);

        let mut list_projects = client.list_projects_request();
        (ListProjectsRequest {
            capability: capability(DAEMON_READ_PERMISSION),
        })
        .to_capnp(list_projects.get().init_request())
        .unwrap();
        let list_response = executor::block_on(list_projects.send().promise).unwrap();
        let listed =
            ListProjectsResult::from_capnp(list_response.get().unwrap().get_result().unwrap())
                .unwrap();
        assert_eq!(listed.projects, vec![project.clone()]);

        let mut resolve_project = client.resolve_project_request();
        (ResolveProjectRequest {
            capability: capability(DAEMON_READ_PERMISSION),
            project_id: project.project_id.clone(),
        })
        .to_capnp(resolve_project.get().init_request())
        .unwrap();
        let project_response = executor::block_on(resolve_project.send().promise).unwrap();
        let lookup =
            ProjectResult::from_capnp(project_response.get().unwrap().get_result().unwrap())
                .unwrap();
        assert_eq!(lookup.project, Some(project.clone()));

        exercise_session_supervisor_rpc_round_trip(&client, &project.project_id);
    }

    /// Register, resolve, list, and unregister one session supervisor over the
    /// daemon RPC surface, asserting each reply round-trips its descriptor.
    fn exercise_session_supervisor_rpc_round_trip(
        client: &daemon_capnp::az_daemon::Client,
        project_id: &str,
    ) {
        let descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37671"));
        let mut register_session = client.register_session_supervisor_request();
        (RegisterSessionSupervisorRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: project_id.to_string(),
            session_slug: "lighting".to_string(),
            descriptor: descriptor.clone(),
        })
        .to_capnp(register_session.get().init_request())
        .unwrap();
        let session_response = executor::block_on(register_session.send().promise).unwrap();
        let registered_descriptor = az_proto_core::ServiceDescriptor::from_capnp(
            session_response.get().unwrap().get_descriptor().unwrap(),
        )
        .unwrap();
        assert_eq!(registered_descriptor, descriptor);

        assert_eq!(
            resolve_session_supervisor_over_rpc(client, project_id),
            Some(descriptor.clone())
        );

        let mut list_sessions = client.list_session_supervisors_request();
        (ListSessionSupervisorsRequest {
            capability: capability(DAEMON_READ_PERMISSION),
            project_id: project_id.to_string(),
        })
        .to_capnp(list_sessions.get().init_request())
        .unwrap();
        let list_sessions_response = executor::block_on(list_sessions.send().promise).unwrap();
        let supervisors = ListSessionSupervisorsResult::from_capnp(
            list_sessions_response.get().unwrap().get_result().unwrap(),
        )
        .unwrap();
        assert_eq!(supervisors.supervisors.len(), 1);
        assert_eq!(supervisors.supervisors[0].session_slug, "lighting");
        assert_eq!(supervisors.supervisors[0].descriptor, descriptor);

        let mut unregister_session = client.unregister_session_supervisor_request();
        (UnregisterSessionSupervisorRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: project_id.to_string(),
            session_slug: "lighting".to_string(),
            descriptor,
        })
        .to_capnp(unregister_session.get().init_request())
        .unwrap();
        let unregister_response = executor::block_on(unregister_session.send().promise).unwrap();
        let unregister_result = UnregisterSessionSupervisorResult::from_capnp(
            unregister_response.get().unwrap().get_result().unwrap(),
        )
        .unwrap();
        assert!(unregister_result.removed);

        assert_eq!(
            resolve_session_supervisor_over_rpc(client, project_id),
            None
        );
    }

    /// Resolve the `lighting` session supervisor over RPC.
    fn resolve_session_supervisor_over_rpc(
        client: &daemon_capnp::az_daemon::Client,
        project_id: &str,
    ) -> Option<az_proto_core::ServiceDescriptor> {
        let mut resolve_session = client.resolve_session_supervisor_request();
        (ResolveSessionSupervisorRequest {
            capability: capability(DAEMON_READ_PERMISSION),
            project_id: project_id.to_string(),
            session_slug: "lighting".to_string(),
        })
        .to_capnp(resolve_session.get().init_request())
        .unwrap();
        let resolve_response = executor::block_on(resolve_session.send().promise).unwrap();
        SessionSupervisorResult::from_capnp(resolve_response.get().unwrap().get_result().unwrap())
            .unwrap()
            .descriptor
    }

    #[test]
    fn daemon_rpc_ensures_project_session() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = ProjectManifest::new("local.rpc_session", "RPC Session", "0.1.0");
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let rpc = Rc::new(AzDaemonRpc::new(test_daemon(temp.path())));
        let client = AzDaemonRpc::client_from_rc(&rpc);

        let mut register_project = client.register_project_root_request();
        (RegisterProjectRootRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            root: temp.path().to_string_lossy().into_owned(),
        })
        .to_capnp(register_project.get().init_request())
        .unwrap();
        let register_response = executor::block_on(register_project.send().promise).unwrap();
        let project =
            ProjectRecord::from_capnp(register_response.get().unwrap().get_project().unwrap())
                .unwrap();

        let mut create_session = client.ensure_project_session_request();
        (EnsureProjectSessionRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: project.project_id.clone(),
            session_name: "main".to_string(),
        })
        .to_capnp(create_session.get().init_request())
        .unwrap();
        let create_response = executor::block_on(create_session.send().promise).unwrap();
        let created =
            ProjectSessionResult::from_capnp(create_response.get().unwrap().get_result().unwrap())
                .unwrap();

        let mut reuse_session = client.ensure_project_session_request();
        (EnsureProjectSessionRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: project.project_id,
            session_name: "main".to_string(),
        })
        .to_capnp(reuse_session.get().init_request())
        .unwrap();
        let reuse_response = executor::block_on(reuse_session.send().promise).unwrap();
        let reused =
            ProjectSessionResult::from_capnp(reuse_response.get().unwrap().get_result().unwrap())
                .unwrap();

        assert!(created.created);
        assert!(!reused.created);
        assert_eq!(created.manifest.project_id, "local.rpc_session");
        assert_eq!(created.manifest.slug, "main");
        assert_eq!(
            created.manifest.workspace_root,
            reused.manifest.workspace_root
        );
    }

    #[test]
    fn daemon_rpc_prepares_project_session_services() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.rpc_prepare", "RPC Prepare", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "rpc_prepare_game",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "rpc_prepare_game");
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let rpc = Rc::new(AzDaemonRpc::new(test_daemon(temp.path())));
        let client = AzDaemonRpc::client_from_rc(&rpc);

        let mut register_project = client.register_project_root_request();
        (RegisterProjectRootRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            root: temp.path().to_string_lossy().into_owned(),
        })
        .to_capnp(register_project.get().init_request())
        .unwrap();
        let register_response = executor::block_on(register_project.send().promise).unwrap();
        let project =
            ProjectRecord::from_capnp(register_response.get().unwrap().get_project().unwrap())
                .unwrap();

        let mut create_session = client.ensure_project_session_request();
        (EnsureProjectSessionRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: project.project_id.clone(),
            session_name: "main".to_string(),
        })
        .to_capnp(create_session.get().init_request())
        .unwrap();
        executor::block_on(create_session.send().promise).unwrap();

        let mut prepare = client.prepare_project_session_services_request();
        (PrepareProjectSessionServicesRequest {
            capability: capability(DAEMON_SESSIONS_PERMISSION),
            project_id: project.project_id,
            session_slug: "main".to_string(),
            endpoint_kind: EndpointKind::WindowsNamedPipe,
            skip_build: true,
            service_names: vec!["asset-processor".to_string()],
            otlp_endpoint: Some("http://127.0.0.1:4317".to_string()),
            recover: false,
        })
        .to_capnp(prepare.get().init_request())
        .unwrap();
        let prepare_response = executor::block_on(prepare.send().promise).unwrap();
        let prepared = ProjectSessionServicesResult::from_capnp(
            prepare_response.get().unwrap().get_result().unwrap(),
        )
        .unwrap();

        assert_eq!(prepared.manifest.project_id, "local.rpc_prepare");
        assert_eq!(prepared.manifest.slug, "main");
        assert_eq!(prepared.prepared_process_count, 1);
        assert_eq!(prepared.service_names, ["asset-processor"]);
    }

    #[test]
    fn daemon_rpc_rejects_in_process_session_supervisor_registration() {
        let rpc = Rc::new(AzDaemonRpc::new(AzDaemon::new()));
        let client = AzDaemonRpc::client_from_rc(&rpc);
        let project = project_record();

        let mut register_project = client.register_project_request();
        (RegisterProjectRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project: project.clone(),
        })
        .to_capnp(register_project.get().init_request())
        .unwrap();
        executor::block_on(register_project.send().promise).unwrap();

        let descriptor = session_supervisor_descriptor(Endpoint::in_process("session:example"));
        let mut register_session = client.register_session_supervisor_request();
        {
            let mut request = register_session.get().init_request();
            az_proto_core::Capability::to_capnp(
                &capability(DAEMON_SESSIONS_PERMISSION),
                request.reborrow().init_capability(),
            )
            .unwrap();
            request.set_project_id(&project.project_id);
            request.set_session_slug("lighting");
            az_proto_core::ServiceDescriptor::to_capnp(&descriptor, request.init_descriptor())
                .unwrap();
        }

        let Err(error) = executor::block_on(register_session.send().promise) else {
            panic!("azd RPC accepted an in-process session-supervisor descriptor")
        };

        assert!(
            error.to_string().contains(
                "invalid session supervisor descriptor: in-process endpoints are test-only"
            ),
            "{error}"
        );

        let mut resolve_session = client.resolve_session_supervisor_request();
        (ResolveSessionSupervisorRequest {
            capability: capability(DAEMON_READ_PERMISSION),
            project_id: project.project_id,
            session_slug: "lighting".to_string(),
        })
        .to_capnp(resolve_session.get().init_request())
        .unwrap();
        let resolve_response = executor::block_on(resolve_session.send().promise).unwrap();
        let supervisor = SessionSupervisorResult::from_capnp(
            resolve_response.get().unwrap().get_result().unwrap(),
        )
        .unwrap();
        assert_eq!(supervisor.descriptor, None);
    }

    #[test]
    fn daemon_rpc_shutdown_requires_control_permission_and_cancels_token() {
        let shutdown = az_work::CancellationToken::new();
        let rpc = Rc::new(AzDaemonRpc::with_shutdown(
            AzDaemon::new(),
            shutdown.clone(),
            uuid::Uuid::now_v7(),
        ));
        let client = AzDaemonRpc::client_from_rc(&rpc);

        let mut unauthorized = client.shutdown_request();
        let mut raw = unauthorized.get().init_request();
        az_proto_core::Capability::to_capnp(
            &capability(DAEMON_READ_PERMISSION),
            raw.reborrow().init_capability(),
        )
        .unwrap();
        raw.set_reason("not enough authority");
        assert!(executor::block_on(unauthorized.send().promise).is_err());
        assert!(!shutdown.is_cancelled());
        assert!(!rpc.shutdown_requested());

        let mut request = client.shutdown_request();
        (ShutdownDaemonRequest {
            capability: capability(DAEMON_CONTROL_PERMISSION),
            reason: "operator requested stop".to_string(),
        })
        .to_capnp(request.get().init_request())
        .unwrap();
        let response = executor::block_on(request.send().promise).unwrap();
        let result =
            ShutdownDaemonResult::from_capnp(response.get().unwrap().get_result().unwrap())
                .unwrap();

        assert!(result.accepted);
        assert_eq!(result.reason, "operator requested stop");
        assert!(shutdown.is_cancelled());
        assert!(rpc.shutdown_requested());
    }

    #[test]
    fn daemon_build_command_rejects_package_target_without_package() {
        let temp = tempfile::tempdir().unwrap();
        let target = ProjectBuildTarget {
            name: "game".to_string(),
            role: az_project::ProjectBuildTargetRole::Generic,
            settings: None,
            package: None,
            kind: ProjectBuildTargetKind::Package,
            default: true,
            features: Vec::new(),
            runtime_files: Vec::new(),
        };

        let err = build_command(
            temp.path(),
            "local.build",
            &target,
            BuildProfile::Debug,
            None,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            AzDaemonError::ProjectManifest(ProjectManifestError::MissingBuildPackage { target })
                if target == "game"
        ));
    }

    #[test]
    fn primary_gem_selector_catalog_orders_runtime_before_authored_support_targets() {
        let (_temp, graph, _generated) = multiplayer_build_fixture();
        let candidates = project_build_selector_candidates(&graph);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| (
                    candidate.target_name.as_str(),
                    candidate.package_name.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                ("client", "azoth-target-client"),
                ("server", "azoth-target-server"),
                ("unified", "azoth-target-unified"),
                ("headless-server", "azoth-target-headless-server"),
                ("auth-server", "sample-auth-server"),
                ("stdb", "sample-database"),
                ("launcher", "sample-launcher"),
            ]
        );
        assert_eq!(
            resolve_project_build_selector_indices(
                &candidates,
                &["auth-server".to_string(), "sample-auth-server".to_string()]
            )
            .unwrap(),
            [4]
        );
    }

    #[test]
    fn primary_gem_mixed_selection_plans_runtime_then_authored_target() {
        let (temp, graph, generated) = multiplayer_build_fixture();
        let plan = project_build_plan_from_graph(
            temp.path(),
            "local.sample",
            "pc-dev",
            None,
            &["auth-server".to_string(), "server".to_string()],
            &graph,
            &generated,
        )
        .unwrap();

        assert_eq!(plan.commands.len(), 3);
        let generated = plan
            .commands
            .iter()
            .find(|command| command.target_name == "server")
            .unwrap();
        let role_root = temp.path().join(".azoth/targets/server");
        assert_eq!(Path::new(&generated.cwd), role_root);
        assert_eq!(&generated.args[..2], ["build", "--manifest-path"]);
        assert_eq!(Path::new(&generated.args[2]), role_root.join("Cargo.toml"));
        assert_eq!(
            generated.cargo_target_dir.as_deref(),
            Some(temp.path().join("target").to_string_lossy().as_ref())
        );
        let asset_worker = plan
            .commands
            .iter()
            .find(|command| command.target_name == "asset-worker")
            .unwrap();
        let asset_worker_root = temp.path().join(".azoth/targets/asset-worker");
        assert_eq!(Path::new(&asset_worker.cwd), asset_worker_root);
        assert!(asset_worker.args.windows(2).any(|pair| {
            pair[0] == "--manifest-path"
                && Path::new(&pair[1]) == asset_worker_root.join("Cargo.toml")
        }));
        let authored = plan
            .commands
            .iter()
            .find(|command| command.target_name == "auth-server")
            .unwrap();
        assert!(
            authored
                .args
                .windows(2)
                .any(|pair| pair == ["-p", "sample-auth-server"])
        );
        assert_eq!(authored.cargo_target_dir, None);
        assert!(plan.package_profile.is_some());
    }

    #[test]
    fn direct_daemon_planning_rejects_a_missing_generated_role_workspace() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nresolver = \"3\"\nmembers = []\n",
        )
        .unwrap();
        fs::write(temp.path().join("Cargo.lock"), "version = 4\n").unwrap();
        let report = GeneratedTargetsSyncReport {
            status: az_project::GeneratedTargetsSyncStatus::Unchanged,
            target_directory: temp.path().join("target"),
            workspace_root: Some(temp.path().join(".azoth/targets")),
            old_fingerprint: None,
            fingerprint: Some("fixture".to_string()),
            targets: vec![GeneratedTargetPackage {
                name: "server".to_string(),
                package: "azoth-target-server".to_string(),
                roles: Vec::new(),
                linked_packages: Vec::new(),
            }],
            manifests: Vec::new(),
        };

        let error = validate_daemon_generated_targets(temp.path(), &report).unwrap_err();

        let AzDaemonError::ProjectManifest(ProjectManifestError::InvalidGeneratedTargets {
            reason,
            ..
        }) = &error
        else {
            panic!("expected InvalidGeneratedTargets, got {error:?}");
        };
        assert!(
            reason.contains("server") && reason.contains("azoth engine sync"),
            "{reason}"
        );
    }

    #[test]
    fn primary_gem_authored_only_selection_omits_runtime_package_profile() {
        let (temp, graph, generated) = multiplayer_build_fixture();

        for (selector, expected_target) in [
            ("auth-server", "auth-server"),
            ("sample-auth-server", "auth-server"),
            ("stdb", "stdb"),
        ] {
            let plan = project_build_plan_from_graph(
                temp.path(),
                "local.sample",
                "pc-dev",
                None,
                &[selector.to_string()],
                &graph,
                &generated,
            )
            .unwrap();

            assert_eq!(plan.commands.len(), 1);
            assert_eq!(plan.commands[0].target_name, expected_target);
            assert_eq!(plan.package_profile, None);
        }
    }

    #[test]
    fn empty_primary_gem_selectors_preserve_generated_runtime_matrix_only() {
        let (temp, graph, generated) = multiplayer_build_fixture();
        let plan = project_build_plan_from_graph(
            temp.path(),
            "local.sample",
            "pc-dev",
            None,
            &[],
            &graph,
            &generated,
        )
        .unwrap();

        assert_eq!(plan.commands.len(), 5);
        for role in ["client", "server", "unified", "headless-server"] {
            let command = plan
                .commands
                .iter()
                .find(|command| command.target_name == role)
                .unwrap();
            assert_eq!(
                Path::new(&command.cwd),
                temp.path().join(".azoth/targets").join(role)
            );
            assert!(command.args.iter().any(|arg| arg == "--manifest-path"));
            assert_eq!(
                command.cargo_target_dir.as_deref(),
                Some(temp.path().join("target").to_string_lossy().as_ref())
            );
        }
        let asset_worker = plan
            .commands
            .iter()
            .find(|command| command.target_name == "asset-worker")
            .unwrap();
        assert!(asset_worker.args.windows(2).any(|pair| {
            pair[0] == "--manifest-path" && Path::new(&pair[1]).ends_with("asset-worker/Cargo.toml")
        }));
        assert!(plan.package_profile.is_some());
    }

    #[test]
    fn selected_asset_worker_service_plan_includes_only_asset_processing_dependencies() {
        let prepared = vec![
            "asset-processor".to_string(),
            "project-host".to_string(),
            "runtime-host".to_string(),
            "asset-worker".to_string(),
        ];

        let selected = requested_service_names(&["asset-worker".to_string()], &prepared).unwrap();

        assert_eq!(selected, ["asset-processor", "asset-worker"]);
    }

    #[test]
    fn selected_services_expand_transitive_dependencies_in_prepared_order() {
        fn dependencies(service_name: &str) -> &'static [&'static str] {
            match service_name {
                "leaf" => &["middle"],
                "middle" => &["root"],
                _ => &[],
            }
        }

        let selected = requested_service_names_with_dependencies(
            &["leaf".to_string()],
            &[
                "root".to_string(),
                "unrelated".to_string(),
                "middle".to_string(),
                "leaf".to_string(),
            ],
            dependencies,
        )
        .unwrap();

        assert_eq!(selected, ["root", "middle", "leaf"]);
    }

    #[test]
    fn selected_services_reject_dependency_cycles() {
        fn dependencies(service_name: &str) -> &'static [&'static str] {
            match service_name {
                "alpha" => &["beta"],
                "beta" => &["alpha"],
                _ => &[],
            }
        }

        let error = requested_service_names_with_dependencies(
            &["alpha".to_string()],
            &["alpha".to_string(), "beta".to_string()],
            dependencies,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidServicePlan { reason, .. }
                if reason.contains("alpha -> beta -> alpha")
        ));
    }

    #[test]
    fn selected_services_reject_missing_transitive_dependencies() {
        fn dependencies(service_name: &str) -> &'static [&'static str] {
            match service_name {
                "leaf" => &["middle"],
                "middle" => &["missing-root"],
                _ => &[],
            }
        }

        let error = requested_service_names_with_dependencies(
            &["leaf".to_string()],
            &["middle".to_string(), "leaf".to_string()],
            dependencies,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidServicePlan { service, reason }
                if service == "middle" && reason.contains("missing-root")
        ));
    }

    #[test]
    fn selected_service_plan_filters_build_and_launch_commands_together() {
        let mut plan = ProjectServicePlan {
            build_commands: [
                "project-host",
                "asset-processor",
                "runtime-host",
                "asset-worker",
            ]
            .into_iter()
            .map(|name| ProjectBuildCommand {
                owner_id: "local.example".to_string(),
                owner_root: "example".to_string(),
                target_name: name.to_string(),
                program: "cargo".to_string(),
                cwd: "example".to_string(),
                args: Vec::new(),
                cargo_target_dir: None,
            })
            .collect(),
            commands: [
                "project-host",
                "asset-processor",
                "runtime-host",
                "asset-worker",
            ]
            .into_iter()
            .map(|name| ProjectServiceCommand {
                owner_id: "local.example".to_string(),
                owner_root: "example".to_string(),
                build_output_root: "example/target".to_string(),
                service_name: name.to_string(),
                role: ServiceRole::Unknown,
                endpoint: Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
                program: name.to_string(),
                cwd: "example".to_string(),
                args: Vec::new(),
            })
            .collect(),
        };

        retain_project_service_plan_services(
            &mut plan,
            &["asset-processor".to_string(), "asset-worker".to_string()],
        )
        .unwrap();

        assert_eq!(
            plan.build_commands
                .iter()
                .map(|command| command.target_name.as_str())
                .collect::<Vec<_>>(),
            ["asset-processor", "asset-worker"]
        );
        assert_eq!(
            plan.commands
                .iter()
                .map(|command| command.service_name.as_str())
                .collect::<Vec<_>>(),
            ["asset-processor", "asset-worker"]
        );

        let error = retain_project_service_plan_services(&mut plan, &[]).unwrap_err();
        assert!(matches!(
            error,
            AzDaemonError::InvalidServicePlan { reason, .. }
                if reason.contains("selection cannot be empty")
        ));
    }

    #[test]
    fn selected_engine_host_tool_service_plan_needs_no_project_build_command() {
        let temp = tempfile::tempdir().unwrap();
        let bundle_dir = temp.path().join("host-tools");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        let azoth_executable = bundle_dir.join(if cfg!(windows) { "azoth.exe" } else { "azoth" });
        let asset_processor_executable =
            bundle_dir.join(HostTool::AssetProcessor.executable_name());
        std::fs::write(&asset_processor_executable, []).unwrap();
        let bundle = HostToolBundle::adjacent_to(azoth_executable).unwrap();
        let target = engine_asset_processor_service_target();
        let command = engine_asset_processor_service_command_from_bundle(
            &AzothDataHome::new(temp.path().join("azoth-home")),
            temp.path(),
            "local.example",
            "local.example",
            "editor",
            EndpointKind::Tcp,
            &bundle,
        )
        .unwrap();

        assert_eq!(target.role, ProjectServiceRole::AssetProcessor);
        assert_eq!(target.package, HostTool::AssetProcessor.cargo_package());
        assert_eq!(target.bin, HostTool::AssetProcessor.cargo_binary());
        assert_eq!(command.role, ServiceRole::AssetProcessor);
        assert_eq!(PathBuf::from(&command.program), asset_processor_executable);

        let mut plan = ProjectServicePlan {
            build_commands: Vec::new(),
            commands: vec![command],
        };

        retain_project_service_plan_services(&mut plan, &["asset-processor".to_string()]).unwrap();

        assert!(plan.build_commands.is_empty());
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].service_name, "asset-processor");
    }

    #[test]
    fn selected_runtime_host_service_plan_includes_declared_dependencies_in_standard_order() {
        let prepared = vec![
            "asset-processor".to_string(),
            "project-host".to_string(),
            "runtime-host".to_string(),
            "asset-worker".to_string(),
        ];

        let selected = requested_service_names(&["runtime-host".to_string()], &prepared).unwrap();

        assert_eq!(
            selected,
            ["asset-processor", "project-host", "runtime-host"]
        );
    }

    #[test]
    fn selected_services_reject_unknown_names_before_execution() {
        let error = requested_service_names(
            &["not-a-service".to_string()],
            &["asset-processor".to_string(), "asset-worker".to_string()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidServicePlan { service, reason }
                if service == "not-a-service" && reason.contains("not part")
        ));
    }

    #[test]
    fn daemon_plans_manifest_declared_cargo_build_targets() {
        let temp = tempfile::tempdir().unwrap();
        let root = normalize(temp.path());
        let mut manifest = ProjectManifest::new("local.build", "Build", "0.1.0");
        let mut target = ProjectBuildTarget::package("game", "build_game");
        target.features.push("editor-tools".to_string());
        manifest.tools.build_targets.push(target);
        write_project_manifest_with_lock(&root, &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(&root).unwrap();

        let plan = daemon
            .plan_project_build("local.build", "release", Some("x86_64-pc-windows-msvc"))
            .unwrap();

        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].owner_id, "local.build");
        assert_eq!(plan.commands[0].owner_root, root.to_string_lossy());
        assert_eq!(plan.commands[0].target_name, "game");
        assert_eq!(plan.commands[0].cwd, root.to_string_lossy());
        assert_eq!(plan.commands[0].program, "cargo");
        let mut expected_args = if cfg!(target_os = "windows") {
            vec!["build"]
        } else {
            vec!["xwin", "build"]
        };
        expected_args.extend([
            "-p",
            "build_game",
            "--release",
            "--target",
            "x86_64-pc-windows-msvc",
            "--features",
            "editor-tools",
        ]);
        assert_eq!(plan.commands[0].args, expected_args);
        assert_eq!(plan.package_profile, None);
    }

    #[test]
    fn daemon_selects_authored_build_target_by_package_name() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.select_build", "Build", "0.1.0");
        manifest.tools.build_targets.extend([
            ProjectBuildTarget::package("client", "select_client"),
            ProjectBuildTarget::package("server", "select_server"),
        ]);
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();

        let plan = daemon
            .plan_project_build_selected(
                "local.select_build",
                "debug",
                None,
                &["select_server".to_string()],
            )
            .unwrap();

        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].target_name, "server");
        assert_eq!(plan.commands[0].args, ["build", "-p", "select_server"]);
    }

    #[test]
    fn daemon_rejects_ambiguous_authored_package_selector() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.ambiguous_build", "Build", "0.1.0");
        manifest.tools.build_targets.extend([
            ProjectBuildTarget::package("client", "shared_package"),
            ProjectBuildTarget::package("server", "shared_package"),
        ]);
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();

        let error = daemon
            .plan_project_build_selected(
                "local.ambiguous_build",
                "debug",
                None,
                &["shared_package".to_string()],
            )
            .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidBuildPackageSelector {
                selector,
                reason,
                ..
            } if selector == "shared_package" && reason == "ambiguous"
        ));
    }

    #[test]
    fn daemon_plans_package_profile_build_targets() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.package_build", "Package Build", "0.1.0");
        manifest
            .tools
            .build_targets
            .push(ProjectBuildTarget::package("game", "package_build_game"));
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();

        let plan = daemon
            .plan_project_build(
                "local.package_build",
                "pc-release",
                Some("x86_64-pc-windows-msvc"),
            )
            .unwrap();

        assert_eq!(plan.commands.len(), 1);
        let mut expected_args = if cfg!(target_os = "windows") {
            vec!["build"]
        } else {
            vec!["xwin", "build"]
        };
        expected_args.extend([
            "-p",
            "package_build_game",
            "--release",
            "--target",
            "x86_64-pc-windows-msvc",
        ]);
        assert_eq!(plan.commands[0].args, expected_args);
        let profile = plan.package_profile.unwrap();
        assert_eq!(profile.name, "pc-release");
        assert_eq!(profile.asset_platform, "pc");
        assert_eq!(profile.cargo_profile, "release");
        assert_eq!(profile.container, "azpack");
        assert_eq!(profile.compression, "oodle");
        assert_eq!(profile.oodle_compressor.as_deref(), Some("kraken"));
        assert_eq!(profile.oodle_effort.as_deref(), Some("normal"));
    }

    #[test]
    fn daemon_uses_package_profile_cargo_profile_for_commands() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new(
            "local.custom_package_build",
            "Custom Package Build",
            "0.1.0",
        );
        manifest.packaging.profiles.push(ProjectPackageProfile {
            name: "pc-editor".to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "editor-dev".to_string(),
            container: ProjectPackageContainer::Loose,
            compression: ProjectPackageCompression::None,
            oodle: None,
        });
        manifest
            .tools
            .build_targets
            .push(ProjectBuildTarget::package("game", "custom_package_game"));
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();

        let plan = daemon
            .plan_project_build("local.custom_package_build", "pc-editor", None)
            .unwrap();

        assert_eq!(
            plan.commands[0].args,
            vec![
                "build",
                "-p",
                "custom_package_game",
                "--profile",
                "editor-dev",
            ]
        );
        let profile = plan.package_profile.unwrap();
        assert_eq!(profile.name, "pc-editor");
        assert_eq!(profile.cargo_profile, "editor-dev");
        assert_eq!(profile.container, "loose");
        assert_eq!(profile.compression, "none");
        assert_eq!(profile.oodle_compressor, None);
        assert_eq!(profile.oodle_effort, None);
    }

    #[test]
    fn daemon_plans_enabled_gem_build_targets() {
        let temp = tempfile::tempdir().unwrap();
        let root = normalize(temp.path());
        let gem_root = root.join("gems").join("physics");
        std::fs::create_dir_all(&gem_root).unwrap();
        let mut manifest = ProjectManifest::new("local.gem_build", "Gem Build", "0.1.0");
        manifest.gems.push(ProjectGem {
            id: "azoth.physics".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            path: Some(std::path::PathBuf::from("gems").join("physics")),
            linkage: None,
        });
        write_project_manifest(&root, &manifest).unwrap();
        let mut gem_manifest = GemManifest::new("azoth.physics", "Physics", "0.1.0");
        gem_manifest
            .tools
            .build_targets
            .push(ProjectBuildTarget::package("physics", "azoth_physics"));
        write_gem_manifest(&gem_root, &gem_manifest).unwrap();
        refresh_project_lock(&root).unwrap();
        let daemon = AzDaemon::new();
        daemon.register_project_root(&root).unwrap();

        let plan = daemon
            .plan_project_build("local.gem_build", "debug", None)
            .unwrap();

        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].owner_id, "azoth.physics");
        assert_eq!(plan.commands[0].owner_root, gem_root.to_string_lossy());
        assert_eq!(plan.commands[0].target_name, "physics");
        assert_eq!(plan.commands[0].cwd, gem_root.to_string_lossy());
        assert_eq!(plan.commands[0].args, vec!["build", "-p", "azoth_physics"]);
    }

    #[test]
    fn daemon_rpc_plans_project_build() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.rpc_build", "RPC Build", "0.1.0");
        manifest
            .tools
            .build_targets
            .push(ProjectBuildTarget::package("game", "rpc_build"));
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();
        let rpc = Rc::new(AzDaemonRpc::new(daemon));
        let client = AzDaemonRpc::client_from_rc(&rpc);

        let mut request = client.plan_project_build_request();
        (PlanProjectBuildRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project_id: "local.rpc_build".to_string(),
            profile: "debug".to_string(),
            target_triple: None,
            package_selectors: Vec::new(),
        })
        .to_capnp(request.get().init_request())
        .unwrap();

        let response = executor::block_on(request.send().promise).unwrap();
        let plan =
            ProjectBuildPlan::from_capnp(response.get().unwrap().get_plan().unwrap()).unwrap();
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].owner_id, "local.rpc_build");
        assert_eq!(plan.commands[0].args, vec!["build", "-p", "rpc_build"]);
    }

    #[test]
    fn legacy_layout_daemon_plans_manifest_declared_project_services() {
        let temp = tempfile::tempdir().unwrap();
        let root = normalize(temp.path());
        let mut manifest = ProjectManifest::new("local.services", "Services", "0.1.0");
        let mut target = ProjectServiceTarget::cargo_bin(
            "project-host",
            ProjectServiceRole::ProjectHost,
            "services_game",
            "project-host",
        );
        target.args.push("--trace".to_string());
        target.features.push("project-host-service".to_string());
        manifest.tools.service_targets.push(target);
        add_asset_processor_target(&mut manifest, "services_game");
        write_project_manifest_with_lock(&root, &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(&root).unwrap();

        let plan = daemon
            .plan_project_services(
                "local.services",
                "editor-work",
                EndpointKind::WindowsNamedPipe,
                None,
            )
            .unwrap();

        assert_eq!(plan.commands.len(), 2);
        assert_eq!(plan.build_commands.len(), 2);
        assert_eq!(
            plan.commands
                .iter()
                .map(|command| command.service_name.as_str())
                .collect::<Vec<_>>(),
            ["asset-processor", "project-host"]
        );
        assert_declared_project_host_build_command(&plan.build_commands[1], &root);
        let command = &plan.commands[1];
        assert_eq!(command.owner_id, "local.services");
        assert_eq!(command.owner_root, root.to_string_lossy());
        assert_eq!(
            command.build_output_root,
            root.join("target").to_string_lossy()
        );
        assert_eq!(command.service_name, "project-host");
        assert_eq!(command.role, ServiceRole::ProjectHost);
        assert_eq!(command.endpoint.kind, EndpointKind::WindowsNamedPipe);
        let expected_endpoint = project_service_endpoint_in(
            &AzothDataHome::resolve(),
            EndpointKind::WindowsNamedPipe,
            &az_filesystem::canonical(&root).unwrap(),
            "project-host",
        )
        .unwrap()
        .address;
        assert_eq!(command.endpoint.address, expected_endpoint);
        assert_eq!(
            command.program,
            service_binary_path(&root.join("target"), &manifest.tools.service_targets[0])
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(
            command.args,
            vec![
                "--endpoint-kind",
                "windows-named-pipe",
                "--endpoint",
                &expected_endpoint,
                "--project",
                &root.to_string_lossy(),
                "--project-id",
                "local.services",
                "--owner-root",
                &root.to_string_lossy(),
                "--service",
                "project-host",
                "--trace",
            ]
        );
        assert!(
            !command.args.iter().any(|arg| arg == "--journal-root"),
            "project-host service plans must not preserve the legacy source-root/session-journal authority path"
        );

        assert_feature_specific_builds_do_not_coalesce(&plan, &root);
        // Legacy layout still plans/builds a project-declared asset-processor
        // package. Primary-gem projects use the engine host tool instead.
    }

    /// A manifest-declared project-host target builds through cargo with its
    /// own package, bin, and feature selectors, owned by the project root.
    fn assert_declared_project_host_build_command(build: &ProjectBuildCommand, root: &Path) {
        assert_eq!(build.owner_id, "local.services");
        assert_eq!(build.owner_root, root.to_string_lossy());
        assert_eq!(build.target_name, "project-host");
        assert_eq!(build.program, "cargo");
        assert_eq!(
            build.args,
            vec![
                "build",
                "-p",
                "services_game",
                "--bin",
                "project-host",
                "--features",
                "project-host-service"
            ]
        );
    }

    /// Only service build commands with the same feature set coalesce. The
    /// project-host command has a role-specific feature, so it must stay
    /// separate from the default asset-processor build.
    fn assert_feature_specific_builds_do_not_coalesce(plan: &ProjectServicePlan, root: &Path) {
        let coalesced = coalesce_build_commands(&plan.build_commands);
        assert_eq!(
            coalesced.len(),
            2,
            "service bins with different feature sets must not be merged"
        );
        let feature_coalesced = coalesced
            .iter()
            .find(|command| command.args.iter().any(|arg| arg == "project-host-service"))
            .expect("feature-specific project-host build");
        assert_eq!(feature_coalesced.program, "cargo");
        assert_eq!(feature_coalesced.cwd, root.to_string_lossy());
        assert!(
            feature_coalesced
                .args
                .windows(2)
                .any(|pair| pair == ["--features", "project-host-service"])
        );
        assert!(
            feature_coalesced
                .args
                .windows(2)
                .any(|pair| pair == ["--bin", "project-host"])
        );
        let default_coalesced = coalesced
            .iter()
            .find(|command| !command.args.iter().any(|arg| arg == "--features"))
            .expect("default-feature asset-processor build");
        let bin_flags = default_coalesced
            .args
            .iter()
            .filter(|arg| arg.as_str() == "--bin")
            .count();
        assert_eq!(
            bin_flags, 1,
            "default-feature AP build stays in its own cargo command"
        );
        assert!(
            default_coalesced
                .args
                .windows(2)
                .any(|pair| pair == ["--bin", "asset-processor"])
        );
        let target_dir_flags = default_coalesced
            .args
            .iter()
            .filter(|arg| arg.as_str() == "--target-dir")
            .count();
        assert_eq!(
            target_dir_flags, 0,
            "normal service builds must use Cargo's resolved target directory"
        );
    }

    #[test]
    fn generated_service_builds_its_independent_role_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path();
        let target_directory = project_root.join("custom-cargo-target");
        let context = GeneratedBuildContext {
            target_directory: target_directory.clone(),
            workspace_root: project_root.join(".azoth/targets"),
        };
        let target = generated_service_targets()
            .into_iter()
            .find(|target| target.name == "project-host")
            .unwrap();

        let build =
            generated_service_build_command(project_root, "local.generated", &context, &target);
        let role_root = context.workspace_root.join("project-host");
        assert_eq!(
            build.cargo_target_dir.as_deref(),
            Some(target_directory.to_string_lossy().as_ref())
        );
        assert_eq!(build.cwd, role_root.to_string_lossy());
        assert_eq!(
            build.args,
            vec![
                "build",
                "--manifest-path",
                role_root.join("Cargo.toml").to_string_lossy().as_ref(),
            ]
        );

        let launch = generated_service_command(
            &ServiceSite {
                data_home: &AzothDataHome::new(temp.path().join("azoth-home")),
                project_root,
                binary_root: project_root,
                owner_id: "local.generated",
                project_id: "local.generated",
                session_slug: "editor",
            },
            &context,
            &target,
            EndpointKind::Tcp,
        )
        .unwrap();
        assert_eq!(launch.build_output_root, target_directory.to_string_lossy());
        assert_eq!(
            PathBuf::from(launch.program),
            service_binary_path(&target_directory, &target)
        );
        assert_eq!(launch.owner_root, project_root.to_string_lossy());
    }

    #[test]
    fn post_sync_build_waves_lock_each_isolated_role_workspace() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("Cargo.lock"), "version = 4\n").unwrap();

        let authored = build_command(
            temp.path(),
            "local.locked",
            &ProjectBuildTarget::package("server", "example-server"),
            BuildProfile::Debug,
            None,
        )
        .unwrap();
        assert!(authored.args.iter().any(|arg| arg == "--locked"));

        let report = GeneratedTargetsSyncReport {
            status: az_project::GeneratedTargetsSyncStatus::Unchanged,
            target_directory: temp.path().join("target"),
            workspace_root: Some(temp.path().join(".azoth/targets")),
            old_fingerprint: None,
            fingerprint: Some("test".to_string()),
            targets: Vec::new(),
            manifests: Vec::new(),
        };
        let generated_root = temp.path().join(".azoth/targets");
        for role in ["server", "headless-server"] {
            let role_root = generated_root.join(role);
            std::fs::create_dir_all(&role_root).unwrap();
            std::fs::write(role_root.join("Cargo.lock"), "version = 4\n").unwrap();
        }
        let commands = [
            GeneratedTargetPackage {
                name: "server".to_string(),
                package: "azoth-target-server".to_string(),
                roles: Vec::new(),
                linked_packages: Vec::new(),
            },
            GeneratedTargetPackage {
                name: "headless-server".to_string(),
                package: "azoth-target-headless-server".to_string(),
                roles: Vec::new(),
                linked_packages: Vec::new(),
            },
        ]
        .into_iter()
        .map(|target| {
            generated_build_command(
                temp.path(),
                "local.locked",
                &report,
                &target,
                BuildProfile::Debug,
                None,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
        let coalesced = coalesce_build_commands(&commands);

        assert_eq!(coalesced.len(), 2);
        for command in &coalesced {
            assert_eq!(
                command
                    .args
                    .iter()
                    .filter(|arg| arg.as_str() == "--locked")
                    .count(),
                1
            );
            assert!(command.args.iter().any(|arg| arg == "--manifest-path"));
            assert_eq!(
                command.cargo_target_dir.as_deref(),
                Some(temp.path().join("target").to_string_lossy().as_ref())
            );
        }
    }

    #[test]
    fn xwin_build_commands_coalesce_without_losing_the_cross_driver() {
        let temp = tempfile::tempdir().unwrap();
        let commands = ["client", "headless-server"].map(|package| ProjectBuildCommand {
            owner_id: "local.cross".to_string(),
            owner_root: temp.path().to_string_lossy().into_owned(),
            target_name: package.to_string(),
            program: "cargo".to_string(),
            cwd: temp.path().to_string_lossy().into_owned(),
            args: vec![
                "xwin".to_string(),
                "build".to_string(),
                "-p".to_string(),
                package.to_string(),
                "--target".to_string(),
                "x86_64-pc-windows-msvc".to_string(),
                "--locked".to_string(),
            ],
            cargo_target_dir: None,
        });

        let coalesced = coalesce_build_commands(&commands);

        assert_eq!(coalesced.len(), 1);
        assert_eq!(
            coalesced[0].args,
            [
                "xwin",
                "build",
                "-p",
                "client",
                "-p",
                "headless-server",
                "--target",
                "x86_64-pc-windows-msvc",
                "--locked",
            ]
        );
        assert_eq!(
            build_command_target_dir(&coalesced[0]),
            Some(temp.path().join("target"))
        );
    }

    #[test]
    fn daemon_prepares_project_services_in_project_manifest_and_attaches_descriptors() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest =
            ProjectManifest::new("local.prepare_services", "Prepare Services", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "prepare_services_game",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "prepare_services_game");
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();

        let result = daemon
            .prepare_project_session_services(
                &project.project_id,
                "main",
                EndpointKind::WindowsNamedPipe,
                true,
            )
            .unwrap();

        assert!(!result.built);
        assert_eq!(result.build_command_count, 2);
        assert_eq!(result.prepared_process_count, 2);
        assert_eq!(
            result.service_names,
            vec!["asset-processor".to_string(), "project-host".to_string()]
        );
        assert_eq!(result.manifest.project_id, "local.prepare_services");
        assert_eq!(result.manifest.slug, "main");
        assert_eq!(result.manifest.services.len(), 2);
        assert!(result.manifest.processes.is_empty());

        let store = daemon.project_service_store(&project).unwrap();
        let project_manifest = store.load_or_create(u128::from(current_unix_ms())).unwrap();
        assert_eq!(project_manifest.services.len(), 2);
        assert_eq!(project_manifest.processes.len(), 2);
        let project_host = project_manifest
            .processes
            .iter()
            .find(|process| process.service_name == "project-host")
            .unwrap();
        assert_eq!(project_host.state, ServiceProcessState::Planned);
        assert_eq!(
            project_host.endpoint_kind.to_proto(),
            EndpointKind::WindowsNamedPipe
        );
        assert!(
            !project_host
                .endpoint_address
                .contains(&result.manifest.id.to_string()),
            "project endpoint must not be session-id scoped: {}",
            project_host.endpoint_address
        );
        let expected_endpoint = project_service_endpoint_in(
            &AzothDataHome::resolve(),
            EndpointKind::WindowsNamedPipe,
            &az_filesystem::canonical(temp.path()).unwrap(),
            "project-host",
        )
        .unwrap()
        .address;
        assert_eq!(project_host.endpoint_address, expected_endpoint);
        assert!(project_host.stdout_log.starts_with(store.logs_dir()));

        assert_asset_processor_grants_are_exact(&project_manifest);
    }

    /// The asset processor's planned grant files must carry exactly the
    /// brokered role and lifecycle capabilities the catalog requires.
    fn assert_asset_processor_grants_are_exact(
        project_manifest: &project_services::ProjectServiceManifest,
    ) {
        let asset_processor = project_manifest
            .processes
            .iter()
            .find(|process| process.service_name == ASSET_PROCESSOR_SERVICE_NAME)
            .unwrap();
        let arg = |flag: &str| {
            asset_processor
                .args
                .windows(2)
                .find(|pair| pair[0] == flag)
                .map(|pair| pair[1].as_str())
        };
        let role_grants_path = PathBuf::from(arg("--capability-grants").unwrap());
        let lifecycle_grants_path = PathBuf::from(arg("--lifecycle-capability-grants").unwrap());
        let role_grants =
            az_proto_core::decode_capability_grant_set(&std::fs::read(role_grants_path).unwrap())
                .unwrap();
        let lifecycle_grants = az_proto_core::decode_capability_grant_set(
            &std::fs::read(lifecycle_grants_path).unwrap(),
        )
        .unwrap();

        role_grants
            .validate_exact_brokered_for_project(
                az_service_catalog::ASSET_PROCESSOR_CAPABILITY_REQUIREMENTS,
            )
            .unwrap();
        lifecycle_grants
            .validate_exact_brokered_for_project(
                az_service_catalog::PROJECT_SERVICE_LIFECYCLE_CAPABILITY_REQUIREMENTS,
            )
            .unwrap();
    }

    #[test]
    fn daemon_prepares_only_selected_project_session_services() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new(
            "local.prepare_selected_services",
            "Prepare Selected Services",
            "0.1.0",
        );
        manifest.tools.service_targets.extend([
            ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "prepare_selected_services_game",
                "project-host",
            ),
            ProjectServiceTarget::cargo_bin(
                "asset-processor",
                ProjectServiceRole::AssetProcessor,
                "prepare_selected_services_game",
                "asset-processor",
            ),
            ProjectServiceTarget::cargo_bin(
                "runtime-host",
                ProjectServiceRole::RuntimeHost,
                "prepare_selected_services_game",
                "runtime-host",
            ),
            ProjectServiceTarget::cargo_bin(
                "asset-worker",
                ProjectServiceRole::AssetWorker,
                "prepare_selected_services_game",
                "asset-worker",
            ),
        ]);
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();

        let selected = vec!["project-host".to_string(), "asset-processor".to_string()];
        let result = daemon
            .prepare_project_session_services_inner(
                &ProjectSessionServicesRequest {
                    project_id: &project.project_id,
                    session_slug: "main",
                    endpoint_kind: EndpointKind::WindowsNamedPipe,
                    skip_build: true,
                    service_names: &selected,
                    otlp_endpoint: None,
                    recover: false,
                },
                None,
            )
            .unwrap();

        assert!(!result.built);
        assert_eq!(result.build_command_count, 2);
        assert_eq!(result.prepared_process_count, 2);
        assert_eq!(
            result.service_names,
            ["asset-processor".to_string(), "project-host".to_string()]
        );
        assert_eq!(result.manifest.services.len(), 2);
        assert!(result.manifest.processes.is_empty());
        assert!(result.manifest.services.iter().all(|service| {
            service.id.name == "project-host" || service.id.name == "asset-processor"
        }));
        let store = daemon.project_service_store(&project).unwrap();
        let project_manifest = store.load_or_create(u128::from(current_unix_ms())).unwrap();
        assert_eq!(project_manifest.processes.len(), 2);
        assert!(project_manifest.processes.iter().all(|process| {
            process.service_name == "project-host" || process.service_name == "asset-processor"
        }));
    }

    #[test]
    fn persisted_project_service_plan_is_reusable_when_current_binaries_exist() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.reuse_services", "Reuse Services", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "reuse_services_game",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "reuse_services_game");
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        daemon
            .prepare_project_session_services(
                &project.project_id,
                "main",
                EndpointKind::WindowsNamedPipe,
                true,
            )
            .unwrap();
        let manager = daemon.session_manager(temp.path()).unwrap();
        let session = manager.session("main").unwrap();
        let commands = session_service_commands(
            &daemon,
            &project.project_id,
            &session,
            EndpointKind::WindowsNamedPipe,
        );
        let store = daemon.project_service_store(&project).unwrap();
        let project_manifest = store.load_or_create(u128::from(current_unix_ms())).unwrap();

        assert!(
            !project_services_have_reusable_launch_plan(
                &project_manifest,
                &commands,
                ServiceProgramFreshnessPolicy::Verify,
            ),
            "a persisted service plan must not be reusable after target cleanup removed binaries"
        );

        for process in &project_manifest.processes {
            let program = Path::new(&process.program);
            fs::create_dir_all(program.parent().unwrap()).unwrap();
            fs::write(program, []).unwrap();
            fs::write(
                program.with_extension("d"),
                format!(
                    "{}: {}\n",
                    program.display(),
                    temp.path().join("azoth.toml").display()
                ),
            )
            .unwrap();
        }

        assert!(
            project_services_have_reusable_launch_plan(
                &project_manifest,
                &commands,
                ServiceProgramFreshnessPolicy::Verify,
            ),
            "a cold supervisor start should reuse a valid persisted launch plan instead of rebuilding"
        );

        let mut running_manifest = project_manifest;
        let identity = ProcessIdentity::current().unwrap();
        for process in &mut running_manifest.processes {
            process.capture_program_artifact().unwrap();
            process
                .mark_running(identity, u128::from(current_unix_ms()))
                .unwrap();
        }
        assert!(project_services_have_reusable_launch_plan(
            &running_manifest,
            &commands,
            ServiceProgramFreshnessPolicy::TrustPrebuilt,
        ));

        std::thread::sleep(Duration::from_millis(20));
        fs::write(&running_manifest.processes[0].program, b"rebuilt").unwrap();
        assert!(
            !project_services_have_reusable_launch_plan(
                &running_manifest,
                &commands,
                ServiceProgramFreshnessPolicy::TrustPrebuilt,
            ),
            "prebuilt service reuse must still reject a process mapped from a replaced executable"
        );
    }

    #[test]
    fn persisted_project_service_plan_is_not_reusable_when_path_dependency_source_is_newer() {
        let temp = tempfile::tempdir().unwrap();
        let package = "reuse_services_game";
        let bin = "asset-processor";
        let dependency_src = write_service_cargo_package_with_path_dependency(
            temp.path(),
            package,
            bin,
            "stale-engine",
        );
        let mut manifest = ProjectManifest::new(
            "local.reuse_stale_path_dep",
            "Reuse Stale Path Dependency",
            "0.1.0",
        );
        add_asset_processor_target(&mut manifest, package);
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        daemon
            .prepare_project_session_services(
                &project.project_id,
                "main",
                EndpointKind::WindowsNamedPipe,
                true,
            )
            .unwrap();
        let manager = daemon.session_manager(temp.path()).unwrap();
        let session = manager.session("main").unwrap();
        let commands = session_service_commands(
            &daemon,
            &project.project_id,
            &session,
            EndpointKind::WindowsNamedPipe,
        );
        let store = daemon.project_service_store(&project).unwrap();
        let project_manifest = store.load_or_create(u128::from(current_unix_ms())).unwrap();

        for process in &project_manifest.processes {
            let program = Path::new(&process.program);
            fs::create_dir_all(program.parent().unwrap()).unwrap();
            fs::write(program, []).unwrap();
            fs::write(
                program.with_extension("d"),
                format!("{}: {}\n", program.display(), dependency_src.display()),
            )
            .unwrap();
        }
        std::thread::sleep(Duration::from_millis(20));
        fs::write(&dependency_src, "pub fn marker() {}\npub fn changed() {}\n").unwrap();

        assert!(
            !project_services_have_reusable_launch_plan(
                &project_manifest,
                &commands,
                ServiceProgramFreshnessPolicy::Verify,
            ),
            "a persisted service plan must rebuild when a local path dependency source changed after the binary"
        );
        assert!(
            project_services_have_reusable_launch_plan(
                &project_manifest,
                &commands,
                ServiceProgramFreshnessPolicy::TrustPrebuilt,
            ),
            "an explicitly prebuilt service plan must trust the caller's completed Cargo build"
        );
    }

    #[test]
    fn service_running_checks_use_only_the_current_process_record() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest =
            ProjectManifest::new("local.stale_running", "Stale Running Services", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "stale_running_game",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "stale_running_game");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                RUNTIME_HOST_SERVICE_NAME,
                ProjectServiceRole::RuntimeHost,
                "stale_running_game",
                RUNTIME_HOST_SERVICE_NAME,
            ));
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        daemon
            .prepare_project_session_services(
                &project.project_id,
                "main",
                EndpointKind::WindowsNamedPipe,
                true,
            )
            .unwrap();
        let manager = daemon.session_manager(temp.path()).unwrap();
        let session = manager.session("main").unwrap();
        let mut manifest = az_session::session_manifest_to_proto(&session);
        let current_index = manifest
            .processes
            .iter()
            .position(|process| process.service_name == RUNTIME_HOST_SERVICE_NAME)
            .unwrap();
        manifest.processes[current_index].previous_run = Some(uuid::Uuid::now_v7());
        manifest.processes[current_index].planned_unix_ms = 2;
        manifest.processes[current_index].updated_unix_ms = 2;
        manifest.processes[current_index].state = ProtoServiceProcessState::Planned;

        let requested = [RUNTIME_HOST_SERVICE_NAME.to_string()];
        assert!(
            !session_services_are_running(&manifest, &requested),
            "a previous run label must not make the current planned service look ready"
        );
        assert!(running_session_service_names(&manifest, &requested).is_empty());

        manifest.processes[current_index].state = ProtoServiceProcessState::Running;
        manifest.processes[current_index].started_unix_ms = Some(2);
        manifest.processes[current_index].pid = Some(std::process::id());
        manifest.processes[current_index].process_start_time =
            Some(ProcessIdentity::current().unwrap().process_start_time);
        assert!(session_services_are_running(&manifest, &requested));
        assert_eq!(
            running_session_service_names(&manifest, &requested),
            requested
        );
    }

    #[test]
    fn service_start_wait_rejects_stale_running_records() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest =
            ProjectManifest::new("local.stale_start_wait", "Stale Start Wait", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "stale_start_wait_game",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "stale_start_wait_game");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                RUNTIME_HOST_SERVICE_NAME,
                ProjectServiceRole::RuntimeHost,
                "stale_start_wait_game",
                RUNTIME_HOST_SERVICE_NAME,
            ));
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        daemon
            .prepare_project_session_services(
                &project.project_id,
                "main",
                EndpointKind::WindowsNamedPipe,
                true,
            )
            .unwrap();

        let manager = daemon.session_manager(temp.path()).unwrap();
        let session = manager.session("main").unwrap();
        let mut manifest = az_session::session_manifest_to_proto(&session);
        let process = manifest
            .processes
            .iter_mut()
            .find(|process| process.service_name == RUNTIME_HOST_SERVICE_NAME)
            .unwrap();
        process.state = ProtoServiceProcessState::Running;
        process.pid = Some(std::process::id());
        process.process_start_time = Some(ProcessIdentity::current().unwrap().process_start_time);
        process.started_unix_ms = Some(10);
        process.updated_unix_ms = 10;

        let requested = [RUNTIME_HOST_SERVICE_NAME.to_string()];
        let blocker = first_unready_session_service(&manifest, &requested, Some(20)).unwrap();
        assert_eq!(blocker.service, RUNTIME_HOST_SERVICE_NAME);
        assert!(
            blocker.state.contains("stale start record"),
            "expected stale running record blocker, got `{}`",
            blocker.state
        );

        let process = manifest
            .processes
            .iter_mut()
            .find(|process| process.service_name == RUNTIME_HOST_SERVICE_NAME)
            .unwrap();
        process.started_unix_ms = Some(21);
        process.updated_unix_ms = 21;
        assert!(first_unready_session_service(&manifest, &requested, Some(20)).is_none());
    }

    #[test]
    fn service_start_wait_uses_transport_subscription_without_manifest_polling() {
        let source = include_str!("lib.rs");
        let wait_body = source
            .split("fn wait_for_session_services_running(\n")
            .nth(1)
            .unwrap()
            .split("fn session_services_are_running(")
            .next()
            .unwrap();

        assert!(
            wait_body.contains("subscribe_events_request"),
            "service readiness must subscribe to the session-supervisor status broker"
        );
        assert!(
            !wait_body.contains("std::thread::sleep"),
            "service readiness must not poll after subscribing"
        );
        assert!(
            source.contains("SESSION_SUPERVISOR_PROBE_RPC_TIMEOUT"),
            "session-supervisor health and challenge probes need a deadline"
        );
    }

    #[test]
    fn supervisor_start_wait_uses_registration_and_exact_process_exit_events() {
        let source = include_str!("lib.rs");
        let wait_body = source
            .split("fn wait_for_session_supervisor_start(")
            .nth(1)
            .unwrap()
            .split("fn wait_for_session_services_running(")
            .next()
            .unwrap();

        assert!(wait_body.contains("capture_process_identity(child.id())"));
        assert!(wait_body.contains("SessionSupervisorLeaseStore::new"));
        assert!(wait_body.contains("lease.process != expected_process"));
        assert!(wait_body.contains("challenge_session_supervisor_lease"));
        assert!(wait_body.contains("subscribe_session_supervisor_registration"));
        assert!(wait_body.contains("lifecycle.add_identity(expected_process)"));
        assert!(!wait_body.contains("std::thread::sleep"));
        assert!(
            !wait_body.contains("session_supervisor_descriptor_from_manifest"),
            "a predeclared descriptor is not evidence that the spawned supervisor is ready"
        );
    }

    #[test]
    fn service_readiness_reports_current_failed_process_detail() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new(
            "local.failed_readiness",
            "Failed Readiness Services",
            "0.1.0",
        );
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "failed_readiness_game",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "failed_readiness_game");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                RUNTIME_HOST_SERVICE_NAME,
                ProjectServiceRole::RuntimeHost,
                "failed_readiness_game",
                RUNTIME_HOST_SERVICE_NAME,
            ));
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        daemon
            .prepare_project_session_services(
                &project.project_id,
                "main",
                EndpointKind::WindowsNamedPipe,
                true,
            )
            .unwrap();

        let manager = daemon.session_manager(temp.path()).unwrap();
        let session = manager.session("main").unwrap();
        let mut manifest = az_session::session_manifest_to_proto(&session);
        let process = manifest
            .processes
            .iter_mut()
            .find(|process| process.service_name == RUNTIME_HOST_SERVICE_NAME)
            .unwrap();
        process.state = ProtoServiceProcessState::Failed;
        process.failure = Some("service exited before readiness\nsecond line".to_string());
        let failed_run = process.run;

        let requested = [RUNTIME_HOST_SERVICE_NAME.to_string()];
        let blocker = first_unready_session_service(&manifest, &requested, None).unwrap();

        assert_eq!(blocker.service, RUNTIME_HOST_SERVICE_NAME);
        assert!(blocker.terminal);
        assert_eq!(
            blocker.state,
            format!("failed for run {failed_run}: service exited before readiness")
        );
    }

    #[test]
    fn persisted_project_service_plan_is_not_reusable_when_target_args_change() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest =
            ProjectManifest::new("local.reuse_changed_args", "Reuse Changed Args", "0.1.0");
        let mut project_host = ProjectServiceTarget::cargo_bin(
            "project-host",
            ProjectServiceRole::ProjectHost,
            "reuse_changed_args_game",
            "project-host",
        );
        project_host.args.push("--old-arg".to_string());
        manifest.tools.service_targets.push(project_host);
        add_asset_processor_target(&mut manifest, "reuse_changed_args_game");
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        daemon
            .prepare_project_session_services(
                &project.project_id,
                "main",
                EndpointKind::WindowsNamedPipe,
                true,
            )
            .unwrap();
        let manager = daemon.session_manager(temp.path()).unwrap();
        let session = manager.session("main").unwrap();
        let store = daemon.project_service_store(&project).unwrap();
        let project_manifest = store.load_or_create(u128::from(current_unix_ms())).unwrap();
        for process in &project_manifest.processes {
            let program = Path::new(&process.program);
            fs::create_dir_all(program.parent().unwrap()).unwrap();
            fs::write(program, []).unwrap();
        }

        let mut changed_commands = session_service_commands(
            &daemon,
            &project.project_id,
            &session,
            EndpointKind::WindowsNamedPipe,
        );
        let project_host = changed_commands
            .iter_mut()
            .find(|command| command.service_name == "project-host")
            .unwrap();
        let old_arg = project_host
            .args
            .iter_mut()
            .find(|arg| **arg == "--old-arg")
            .unwrap();
        *old_arg = "--new-arg".to_string();

        assert!(
            !project_services_have_reusable_launch_plan(
                &project_manifest,
                &changed_commands,
                ServiceProgramFreshnessPolicy::Verify,
            ),
            "a persisted service plan must be rebuilt when project service target args change"
        );
    }

    #[test]
    fn persisted_project_service_plan_is_not_reusable_when_current_row_failed() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest =
            ProjectManifest::new("local.failed_services", "Failed Services", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "failed_services_game",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "failed_services_game");
        write_project_manifest_with_lock(temp.path(), &manifest);
        init_git_repo_with_commit(temp.path());
        let daemon = test_daemon(temp.path());
        let project = daemon.register_project_root(temp.path()).unwrap();
        daemon
            .ensure_project_session(&project.project_id, "main")
            .unwrap();
        daemon
            .prepare_project_session_services(
                &project.project_id,
                "main",
                EndpointKind::WindowsNamedPipe,
                true,
            )
            .unwrap();
        let manager = daemon.session_manager(temp.path()).unwrap();
        let session = manager.session("main").unwrap();
        let commands = session_service_commands(
            &daemon,
            &project.project_id,
            &session,
            EndpointKind::WindowsNamedPipe,
        );
        let store = daemon.project_service_store(&project).unwrap();
        let mut project_manifest = store.load_or_create(u128::from(current_unix_ms())).unwrap();
        for process in &project_manifest.processes {
            let program = Path::new(&process.program);
            fs::create_dir_all(program.parent().unwrap()).unwrap();
            fs::write(program, []).unwrap();
        }
        project_manifest
            .current_process_mut(&ServiceProcessKey::new(
                "project-host",
                SupervisedServiceRole::ProjectHost,
            ))
            .unwrap()
            .state = ServiceProcessState::Failed;

        assert!(
            !project_services_have_reusable_launch_plan(
                &project_manifest,
                &commands,
                ServiceProgramFreshnessPolicy::Verify,
            ),
            "failed current service rows require prepare/build rather than silent reuse"
        );
    }

    #[test]
    fn daemon_rejects_in_process_project_service_endpoint_kind() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.in_process_services", "Services", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "services_game",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "services_game");
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();

        let error = daemon
            .plan_project_services(
                "local.in_process_services",
                "editor-work",
                EndpointKind::InProcess,
                None,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::UnsupportedEndpointKind {
                operation: "azd project service planning",
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn daemon_rejects_service_targets_that_override_launch_context() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            project_manifest_path(temp.path()),
            r#"
[manifest]
kind = "project"
schema = "azoth.project/v1"

[project]
id = "local.reserved_service_args"
name = "Reserved Service Args"
version = "0.1.0"
engine_version = "0.1.0"

[paths]
assets = "assets"
scripts = "scripts"

[[tools.service_targets]]
name = "project-host"
role = "project-host"
package = "reserved_service_args"
bin = "project-host"
args = ["--workspace-root", "stale/workspace"]
"#,
        )
        .unwrap();
        let daemon = AzDaemon::new();
        daemon
            .register_project(&ProjectRecord {
                project_id: "local.reserved_service_args".to_string(),
                name: "Reserved Service Args".to_string(),
                root: temp.path().to_string_lossy().into_owned(),
                manifest_path: project_manifest_path(temp.path())
                    .to_string_lossy()
                    .into_owned(),
                engine_version: "0.1.0".to_string(),
            })
            .unwrap();

        let error = daemon
            .plan_project_services(
                "local.reserved_service_args",
                "editor-work",
                EndpointKind::Tcp,
                None,
            )
            .unwrap_err();

        assert!(
            matches!(
                &error,
                AzDaemonError::ProjectManifest(ProjectManifestError::ReservedServiceTargetArg {
                    target,
                    flag,
                }) if target == "project-host" && flag == "--workspace-root"
            ),
            "{error:?}"
        );
    }

    #[test]
    fn daemon_rejects_equals_style_reserved_service_target_args() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            project_manifest_path(temp.path()),
            r#"
[manifest]
kind = "project"
schema = "azoth.project/v1"

[project]
id = "local.reserved_equals_service_args"
name = "Reserved Equals Service Args"
version = "0.1.0"
engine_version = "0.1.0"

[paths]
assets = "assets"
scripts = "scripts"

[[tools.service_targets]]
name = "runtime-host"
role = "runtime-host"
package = "reserved_equals_service_args"
bin = "runtime-host"
args = ["--project-id=local.other"]
"#,
        )
        .unwrap();
        let daemon = AzDaemon::new();
        daemon
            .register_project(&ProjectRecord {
                project_id: "local.reserved_equals_service_args".to_string(),
                name: "Reserved Equals Service Args".to_string(),
                root: temp.path().to_string_lossy().into_owned(),
                manifest_path: project_manifest_path(temp.path())
                    .to_string_lossy()
                    .into_owned(),
                engine_version: "0.1.0".to_string(),
            })
            .unwrap();

        let error = daemon
            .plan_project_services(
                "local.reserved_equals_service_args",
                "editor-work",
                EndpointKind::Tcp,
                None,
            )
            .unwrap_err();

        assert!(
            matches!(
                &error,
                AzDaemonError::ProjectManifest(ProjectManifestError::ReservedServiceTargetArg {
                    target,
                    flag,
                }) if target == "runtime-host" && flag == "--project-id"
            ),
            "{error:?}"
        );
    }

    #[test]
    fn project_service_startup_visits_each_requested_dependency_tier() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0");
        let process = |service_name: &str, role| {
            ServiceProcessRecord::planned(
                service_name,
                role,
                Uuid::now_v7(),
                &endpoint,
                service_name,
                temp.path().to_path_buf(),
                Vec::new(),
                temp.path().join(format!("{service_name}.stdout.log")),
                temp.path().join(format!("{service_name}.stderr.log")),
                temp.path().join(format!("{service_name}.capnp.log")),
                None,
                1,
            )
        };
        let processes = [
            process("asset-worker", SupervisedServiceRole::Worker),
            process("idle-worker", SupervisedServiceRole::Worker),
            process("project-host", SupervisedServiceRole::ProjectHost),
            process("asset-processor", SupervisedServiceRole::AssetProcessor),
        ];
        let requested = [
            "asset-worker".to_string(),
            "asset-processor".to_string(),
            "project-host".to_string(),
        ];

        assert_eq!(
            project_service_start_waves(&processes, &requested)
                .iter()
                .map(|wave| {
                    wave.iter()
                        .map(|process| process.service_name.as_str())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
            [
                vec!["asset-processor"],
                vec!["project-host"],
                vec!["asset-worker"],
            ]
        );
    }

    #[test]
    fn daemon_orders_service_plan_by_runtime_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.service_order", "Service Order", "0.1.0");
        manifest.tools.service_targets.extend([
            ProjectServiceTarget::cargo_bin(
                "asset-worker",
                ProjectServiceRole::AssetWorker,
                "service_order",
                "asset-worker",
            ),
            ProjectServiceTarget::cargo_bin(
                "runtime-host",
                ProjectServiceRole::RuntimeHost,
                "service_order",
                "runtime-host",
            ),
            ProjectServiceTarget::cargo_bin(
                "asset-processor",
                ProjectServiceRole::AssetProcessor,
                "service_order",
                "asset-processor",
            ),
            ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "service_order",
                "project-host",
            ),
        ]);
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();

        let plan = daemon
            .plan_project_services(
                "local.service_order",
                "editor-work",
                EndpointKind::Tcp,
                None,
            )
            .unwrap();

        assert_eq!(
            plan.commands
                .iter()
                .map(|command| command.service_name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "asset-processor",
                "project-host",
                "runtime-host",
                "asset-worker"
            ]
        );

        let asset_plan = daemon
            .plan_project_services_selected(
                "local.service_order",
                "editor-work",
                EndpointKind::Tcp,
                None,
                &["asset-worker".to_string()],
            )
            .unwrap();
        assert_selected_worker_plan_pulls_in_its_asset_processor(&asset_plan);
        assert_generated_services_exclude_asset_processor(temp.path());
        assert_eq!(
            plan.commands
                .iter()
                .map(|command| command.role)
                .collect::<Vec<_>>(),
            vec![
                ServiceRole::AssetProcessor,
                ServiceRole::ProjectHost,
                ServiceRole::RuntimeHost,
                ServiceRole::Worker
            ]
        );
    }

    /// Selecting only the asset worker still plans the asset processor it
    /// depends on, and both bins coalesce into one cargo invocation.
    ///
    /// Legacy layout still builds a project-declared asset-processor package.
    fn assert_selected_worker_plan_pulls_in_its_asset_processor(asset_plan: &ProjectServicePlan) {
        assert_eq!(
            asset_plan
                .build_commands
                .iter()
                .map(|command| command.target_name.as_str())
                .collect::<Vec<_>>(),
            ["asset-processor", "asset-worker"]
        );
        let coalesced = coalesce_build_commands(&asset_plan.build_commands);
        assert_eq!(coalesced.len(), 1);
        assert!(
            coalesced[0]
                .args
                .windows(2)
                .any(|pair| pair == ["--bin", "asset-processor"])
        );
        assert!(
            coalesced[0]
                .args
                .windows(2)
                .any(|pair| pair == ["--bin", "asset-worker"])
        );
        assert_eq!(
            asset_plan
                .commands
                .iter()
                .map(|command| command.service_name.as_str())
                .collect::<Vec<_>>(),
            ["asset-processor", "asset-worker"]
        );
    }

    /// Generated primary-gem services no longer include an asset processor:
    /// that host is engine-owned, so only the worker package is generated.
    fn assert_generated_services_exclude_asset_processor(root: &Path) {
        let generated_context = GeneratedBuildContext {
            target_directory: root.join("target"),
            workspace_root: root.join(".azoth/targets"),
        };
        let generated_commands = generated_service_targets()
            .into_iter()
            .filter(|target| matches!(target.name.as_str(), "asset-processor" | "asset-worker"))
            .map(|target| {
                generated_service_build_command(
                    root,
                    "local.service_order",
                    &generated_context,
                    &target,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            generated_commands
                .iter()
                .map(|command| command.target_name.as_str())
                .collect::<Vec<_>>(),
            ["asset-worker"]
        );
        let generated_coalesced = coalesce_build_commands(&generated_commands);
        assert_eq!(generated_coalesced.len(), 1);
        assert!(
            !generated_coalesced[0]
                .args
                .windows(2)
                .any(|pair| pair == ["-p", "azoth-target-asset-processor"])
        );
        assert!(generated_coalesced[0].args.windows(2).any(|pair| {
            pair[0] == "--manifest-path" && Path::new(&pair[1]).ends_with("asset-worker/Cargo.toml")
        }));
    }

    #[test]
    fn daemon_plans_enabled_gem_services_with_project_launch_context() {
        let temp = tempfile::tempdir().unwrap();
        let root = normalize(temp.path());
        let gem_root = root.join("gems").join("physics");
        std::fs::create_dir_all(&gem_root).unwrap();
        let mut manifest = ProjectManifest::new("local.gem_services", "Gem Services", "0.1.0");
        manifest.gems.push(ProjectGem {
            id: "azoth.physics".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            path: Some(std::path::PathBuf::from("gems").join("physics")),
            linkage: None,
        });
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "gem_services",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "gem_services");
        write_project_manifest(&root, &manifest).unwrap();
        let mut gem_manifest = GemManifest::new("azoth.physics", "Physics", "0.1.0");
        gem_manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "runtime-host",
                ProjectServiceRole::RuntimeHost,
                "azoth_physics",
                "physics-runtime",
            ));
        write_gem_manifest(&gem_root, &gem_manifest).unwrap();
        refresh_project_lock(&root).unwrap();
        let daemon = AzDaemon::new();
        daemon.register_project_root(&root).unwrap();

        let plan = daemon
            .plan_project_services("local.gem_services", "editor-work", EndpointKind::Tcp, None)
            .unwrap();

        assert_eq!(plan.build_commands.len(), 3);
        assert_eq!(plan.commands.len(), 3);
        let build = plan
            .build_commands
            .iter()
            .find(|command| command.target_name == "runtime-host")
            .unwrap();
        assert_eq!(build.owner_id, "azoth.physics");
        assert_eq!(build.owner_root, gem_root.to_string_lossy());
        assert_eq!(build.target_name, "runtime-host");
        assert_eq!(build.cwd, gem_root.to_string_lossy());
        assert_eq!(
            build.args,
            vec!["build", "-p", "azoth_physics", "--bin", "physics-runtime"]
        );

        let command = plan
            .commands
            .iter()
            .find(|command| command.service_name == "runtime-host")
            .unwrap();
        assert_eq!(command.owner_id, "azoth.physics");
        assert_eq!(command.owner_root, gem_root.to_string_lossy());
        assert_eq!(
            command.build_output_root,
            gem_root.join("target").to_string_lossy()
        );
        assert_eq!(command.service_name, "runtime-host");
        assert_eq!(command.role, ServiceRole::RuntimeHost);
        assert_eq!(command.endpoint.kind, EndpointKind::Tcp);
        assert_eq!(command.endpoint.address, "127.0.0.1:0");
        assert_eq!(
            command.program,
            service_binary_path(
                &gem_root.join("target"),
                &gem_manifest.tools.service_targets[0]
            )
            .to_string_lossy()
            .into_owned()
        );
        assert_eq!(command.cwd, root.to_string_lossy());
        assert!(
            command
                .args
                .windows(2)
                .any(|arg| arg == ["--project", root.to_str().unwrap()])
        );
        assert!(
            command
                .args
                .windows(2)
                .any(|arg| arg == ["--owner-root", gem_root.to_str().unwrap()])
        );
    }

    #[test]
    fn daemon_service_planning_rejects_duplicate_names_at_project_lock_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let gem_root = temp.path().join("gems").join("physics");
        std::fs::create_dir_all(&gem_root).unwrap();
        let mut manifest =
            ProjectManifest::new("local.duplicate_services", "Duplicate Services", "0.1.0");
        manifest.gems.push(ProjectGem {
            id: "azoth.physics".to_string(),
            enabled: true,
            capabilities: Vec::new(),
            path: Some(std::path::PathBuf::from("gems").join("physics")),
            linkage: None,
        });
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "runtime-host",
                ProjectServiceRole::RuntimeHost,
                "project_runtime",
                "runtime",
            ));
        write_project_manifest(temp.path(), &manifest).unwrap();
        let mut gem_manifest = GemManifest::new("azoth.physics", "Physics", "0.1.0");
        gem_manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "runtime-host",
                ProjectServiceRole::RuntimeHost,
                "azoth_physics",
                "physics-runtime",
            ));
        write_gem_manifest(&gem_root, &gem_manifest).unwrap();
        let error = refresh_project_lock(temp.path()).unwrap_err();

        assert!(matches!(
            error,
            ProjectManifestError::DuplicateServiceTarget { name } if name == "runtime-host"
        ));
    }

    #[test]
    fn daemon_plans_session_services_from_workspace_without_replacing_project_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = normalize(temp.path());
        let project_root = root.join("project");
        let workspace_root = root.join("workspace");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&workspace_root).unwrap();
        let mut manifest =
            ProjectManifest::new("local.workspace_services", "Workspace Services", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "workspace_services",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "workspace_services");
        write_project_manifest_with_lock(&project_root, &manifest);
        write_project_manifest_with_lock(&workspace_root, &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(&project_root).unwrap();

        let plan = daemon
            .plan_project_services(
                "local.workspace_services",
                "editor-work",
                EndpointKind::Tcp,
                Some(&workspace_root),
            )
            .unwrap();

        assert_eq!(
            daemon
                .resolve_project("local.workspace_services")
                .unwrap()
                .root,
            project_root.to_string_lossy()
        );
        assert_eq!(plan.build_commands[0].cwd, workspace_root.to_string_lossy());
        assert_eq!(
            plan.build_commands[0].owner_root,
            workspace_root.to_string_lossy()
        );
        assert_eq!(plan.commands[0].cwd, workspace_root.to_string_lossy());
        assert_eq!(
            plan.commands[0].owner_root,
            workspace_root.to_string_lossy()
        );
        assert!(
            plan.commands[0]
                .args
                .windows(2)
                .any(|arg| arg == ["--project", workspace_root.to_str().unwrap()])
        );
        assert!(
            plan.commands[0]
                .args
                .windows(2)
                .any(|arg| arg == ["--owner-root", workspace_root.to_str().unwrap()])
        );
    }

    #[test]
    fn daemon_rejects_service_workspace_for_different_project() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let other_root = temp.path().join("other-workspace");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&other_root).unwrap();
        let mut manifest = ProjectManifest::new("local.services", "Services", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "services",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "services");
        write_project_manifest_with_lock(&project_root, &manifest);
        write_project_manifest_with_lock(
            &other_root,
            &ProjectManifest::new("local.other", "Other", "0.1.0"),
        );
        let daemon = AzDaemon::new();
        daemon.register_project_root(&project_root).unwrap();

        let error = daemon
            .plan_project_services(
                "local.services",
                "editor-work",
                EndpointKind::Tcp,
                Some(&other_root),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::ProjectServiceWorkspaceMismatch { .. }
        ));
    }

    #[test]
    fn daemon_rejects_relative_service_workspace_root() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest =
            ProjectManifest::new("local.relative_workspace", "Relative Workspace", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "services",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "services");
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();

        let error = daemon
            .plan_project_services(
                "local.relative_workspace",
                "editor-work",
                EndpointKind::Tcp,
                Some(Path::new("relative-workspace")),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            AzDaemonError::InvalidProjectServiceWorkspaceRoot { workspace_root, reason }
                if workspace_root == "relative-workspace" && reason.contains("absolute")
        ));
    }

    #[test]
    fn daemon_rpc_plans_project_services() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = ProjectManifest::new("local.rpc_services", "RPC Services", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "rpc_services",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "rpc_services");
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();
        let rpc = Rc::new(AzDaemonRpc::new(daemon));
        let client = AzDaemonRpc::client_from_rc(&rpc);

        let mut request = client.plan_project_services_request();
        (PlanProjectServicesRequest {
            capability: capability(DAEMON_PROJECTS_PERMISSION),
            project_id: "local.rpc_services".to_string(),
            session_slug: "editor".to_string(),
            endpoint_kind: EndpointKind::Tcp,
            workspace_root: None,
            service_names: Vec::new(),
        })
        .to_capnp(request.get().init_request())
        .unwrap();

        let response = executor::block_on(request.send().promise).unwrap();
        let plan =
            ProjectServicePlan::from_capnp(response.get().unwrap().get_plan().unwrap()).unwrap();
        assert_eq!(plan.build_commands.len(), 2);
        assert_eq!(plan.commands.len(), 2);
        assert_eq!(
            plan.commands
                .iter()
                .map(|command| command.service_name.as_str())
                .collect::<Vec<_>>(),
            ["asset-processor", "project-host"]
        );
        assert_eq!(plan.commands[1].owner_id, "local.rpc_services");
        assert_eq!(plan.commands[1].role, ServiceRole::ProjectHost);
        assert_eq!(plan.commands[1].endpoint.kind, EndpointKind::Tcp);
        assert_eq!(plan.commands[1].endpoint.address, "127.0.0.1:0");
    }

    #[test]
    fn daemon_rpc_rejects_in_process_project_service_endpoints() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest =
            ProjectManifest::new("local.rpc_in_process_services", "RPC Services", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(ProjectServiceTarget::cargo_bin(
                "project-host",
                ProjectServiceRole::ProjectHost,
                "rpc_services",
                "project-host",
            ));
        add_asset_processor_target(&mut manifest, "rpc_services");
        write_project_manifest_with_lock(temp.path(), &manifest);
        let daemon = AzDaemon::new();
        daemon.register_project_root(temp.path()).unwrap();
        let rpc = Rc::new(AzDaemonRpc::new(daemon));
        let client = AzDaemonRpc::client_from_rc(&rpc);

        let mut request = client.plan_project_services_request();
        {
            let mut plan_request = request.get().init_request();
            az_proto_core::Capability::to_capnp(
                &capability(DAEMON_PROJECTS_PERMISSION),
                plan_request.reborrow().init_capability(),
            )
            .unwrap();
            plan_request.set_project_id("local.rpc_in_process_services");
            plan_request.set_session_slug("editor");
            plan_request.set_endpoint_kind(EndpointKind::InProcess.to_capnp());
        }

        let Err(error) = executor::block_on(request.send().promise) else {
            panic!("azd RPC accepted an in-process project service endpoint kind")
        };

        assert!(
            error
                .to_string()
                .contains("invalid plan project services request endpoint kind: in-process endpoints are test-only"),
            "{error}"
        );
    }

    #[test]
    fn daemon_rejects_session_supervisor_for_unknown_project() {
        let daemon = AzDaemon::new();
        let descriptor = session_supervisor_descriptor(Endpoint::in_process("session:missing"));

        let error = daemon
            .register_session_supervisor("local.missing", "session", &descriptor)
            .unwrap_err();

        assert!(matches!(error, AzDaemonError::UnknownProject { .. }));
    }

    #[test]
    fn daemon_unregisters_session_supervisor_only_on_descriptor_match() {
        let daemon = AzDaemon::new();
        let project = project_record();
        daemon.register_project(&project).unwrap();
        let descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37672"));
        let stale_descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37673"));
        daemon
            .register_session_supervisor(&project.project_id, "editor", &descriptor)
            .unwrap();

        let removed_stale = daemon
            .unregister_session_supervisor(&project.project_id, "editor", &stale_descriptor)
            .unwrap();
        assert!(!removed_stale);
        assert_eq!(
            daemon.resolve_session_supervisor(&project.project_id, "editor"),
            Some(descriptor.clone())
        );

        let removed_current = daemon
            .unregister_session_supervisor(&project.project_id, "editor", &descriptor)
            .unwrap();
        assert!(removed_current);
        assert_eq!(
            daemon.resolve_session_supervisor(&project.project_id, "editor"),
            None
        );
    }

    #[test]
    fn daemon_unregister_does_not_use_observational_run_as_a_control_key() {
        let daemon = AzDaemon::new();
        let project = project_record();
        daemon.register_project(&project).unwrap();
        let descriptor =
            session_supervisor_descriptor(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37672"));
        daemon
            .register_session_supervisor(&project.project_id, "editor", &descriptor)
            .unwrap();

        let mut same_contract = descriptor;
        same_contract.run = uuid::Uuid::now_v7();
        let removed = daemon
            .unregister_session_supervisor(&project.project_id, "editor", &same_contract)
            .unwrap();

        assert!(removed);
        assert_eq!(
            daemon.resolve_session_supervisor(&project.project_id, "editor"),
            None
        );
    }

    #[test]
    fn daemon_launches_registered_sessiond_as_a_direct_host_tool() {
        let temp = tempfile::tempdir().unwrap();
        let bundle_dir = temp.path().join("bundle");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        let azoth_executable = bundle_dir.join(if cfg!(windows) { "azoth.exe" } else { "azoth" });
        let sessiond = bundle_dir.join(HostTool::SessionSupervisor.executable_name());
        std::fs::write(&sessiond, []).unwrap();
        let bundle = HostToolBundle::adjacent_to(azoth_executable).unwrap();
        let daemon_endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37123");

        let command = daemon_registered_sessiond_launch_command_from_bundle(
            &bundle,
            temp.path(),
            "editor",
            EndpointKind::Tcp,
            &daemon_endpoint,
            true,
            30_000,
            &[],
        )
        .unwrap();

        assert_eq!(command.program, sessiond.to_string_lossy());
        assert_eq!(command.cwd, temp.path());
        assert!(!command.args.iter().any(|arg| arg == "run"));
        assert!(command.args.iter().any(|arg| arg == "--keep-alive"));
        assert!(
            !command
                .args
                .iter()
                .any(|arg| arg == "--no-daemon-registration")
        );
        assert!(command.args.windows(2).any(|window| {
            window[0] == "--daemon-endpoint" && window[1] == daemon_endpoint.address
        }));
    }
}
