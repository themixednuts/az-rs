use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

use az_proto_core::{
    Capability, ServiceDescriptor, ServiceId, ServiceRole, decode_capability_grant_set,
};
use az_proto_project::{PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME};
use az_proto_runtime::{
    RUNTIME_CONTROL_PERMISSION, RUNTIME_HOST_AUDIENCE, RUNTIME_HOST_NAMESPACE,
    RUNTIME_HOST_SERVICE_NAME, RUNTIME_READ_PERMISSION, RuntimeProjectionCatalogRequest,
    RuntimeProjectionCatalogResult, runtime_capnp,
};
use az_proto_session::{SESSION_SUPERVISOR_NAMESPACE, SESSION_SUPERVISOR_SERVICE_NAME};
use az_service_supervision::{
    ProcessIdentity, RecordedServiceProcessCleanup, ServiceLifecycleController,
    ServiceLifecycleEvent, ServiceLifecycleEvents, ServiceProcessExit, ServiceProcessKey,
    ServiceProcessLauncher, ServiceProcessRecord, ServiceProcessState, SpawnedServiceProcess,
    StdServiceProcessLauncher, SupervisedServiceRole, request_service_lifecycle_shutdown,
    terminate_recorded_service_process,
};
use crossbeam_channel as channel;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::LocalSet;
use tracing::{info, instrument, warn};

use crate::{SessionError, SessionManager, SessionManifest, SessionStatus};

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn service_descriptor_for_process(
    manifest: &SessionManifest,
    process: &ServiceProcessRecord,
) -> Option<ServiceDescriptor> {
    if process.role != SupervisedServiceRole::RuntimeHost {
        return None;
    }
    let id = ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME);
    manifest.service_descriptor(&id, process.role.to_proto())
}

const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartServicesReport {
    pub started: Vec<SpawnedServiceProcess>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartServicesFilter {
    service_names: BTreeSet<String>,
}

impl StartServicesFilter {
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn named(service_names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            service_names: service_names
                .into_iter()
                .map(Into::into)
                .filter(|service_name: &String| !service_name.trim().is_empty())
                .collect(),
        }
    }

    #[must_use]
    pub fn is_all(&self) -> bool {
        self.service_names.is_empty()
    }

    #[must_use]
    pub fn service_names(&self) -> Vec<String> {
        self.service_names.iter().cloned().collect()
    }

    #[must_use]
    pub fn includes(&self, service_name: &str) -> bool {
        self.is_all() || self.service_names.contains(service_name)
    }
}

/// One request from an RPC or signal bridge to the single sessiond owner loop.
///
/// The loop performs every durable mutation before resolving `completion`, so
/// callers never observe an accepted-but-not-yet-applied control request.
#[derive(Debug)]
pub enum SessionSupervisorCommand {
    Start {
        filter: StartServicesFilter,
        completion: oneshot::Sender<Result<SessionSupervisorCommandResult, SessionError>>,
    },
    Stop {
        reason: String,
        completion: oneshot::Sender<Result<SessionSupervisorCommandResult, SessionError>>,
    },
    Shutdown {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub enum SessionSupervisorCommandResult {
    Started {
        report: StartServicesReport,
        status: SessionStatus,
    },
    Stopped {
        report: StopServicesReport,
        status: SessionStatus,
    },
}

#[derive(Debug, Clone)]
pub struct SessionSupervisorCommandSender(channel::Sender<SessionSupervisorCommand>);

impl SessionSupervisorCommandSender {
    #[must_use]
    pub fn has_pending_operation(&self) -> bool {
        self.0.is_full()
    }

    /// Queues a start command for the owner loop and hands back the channel it
    /// will resolve.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::SupervisorCommandBusy`] if a command is already
    /// queued (the channel holds one), and
    /// [`SessionError::InvalidSessionCommand`] if the owner loop has hung up.
    pub fn request_start(
        &self,
        filter: StartServicesFilter,
    ) -> Result<oneshot::Receiver<Result<SessionSupervisorCommandResult, SessionError>>, SessionError>
    {
        let (completion, response) = oneshot::channel();
        self.0
            .try_send(SessionSupervisorCommand::Start { filter, completion })
            .map_err(|error| match error {
                channel::TrySendError::Full(_) => SessionError::SupervisorCommandBusy,
                channel::TrySendError::Disconnected(_) => SessionError::InvalidSessionCommand {
                    message: "session-supervisor command loop is unavailable".to_string(),
                },
            })?;
        Ok(response)
    }

    /// Queues a stop command for the owner loop and hands back the channel it
    /// will resolve.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::SupervisorCommandBusy`] if a command is already
    /// queued (the channel holds one), and
    /// [`SessionError::InvalidSessionCommand`] if the owner loop has hung up.
    pub fn request_stop(
        &self,
        reason: impl Into<String>,
    ) -> Result<oneshot::Receiver<Result<SessionSupervisorCommandResult, SessionError>>, SessionError>
    {
        let (completion, response) = oneshot::channel();
        self.0
            .try_send(SessionSupervisorCommand::Stop {
                reason: reason.into(),
                completion,
            })
            .map_err(|error| match error {
                channel::TrySendError::Full(_) => SessionError::SupervisorCommandBusy,
                channel::TrySendError::Disconnected(_) => SessionError::InvalidSessionCommand {
                    message: "session-supervisor command loop is unavailable".to_string(),
                },
            })?;
        Ok(response)
    }

    /// Queues a shutdown command for the owner loop. Unlike start and stop this
    /// carries no completion channel; the loop exits instead of replying.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::SupervisorCommandBusy`] if a command is already
    /// queued (the channel holds one), and
    /// [`SessionError::InvalidSessionCommand`] if the owner loop has hung up.
    pub fn request_shutdown(&self, reason: impl Into<String>) -> Result<(), SessionError> {
        self.0
            .try_send(SessionSupervisorCommand::Shutdown {
                reason: reason.into(),
            })
            .map_err(|error| match error {
                channel::TrySendError::Full(_) => SessionError::SupervisorCommandBusy,
                channel::TrySendError::Disconnected(_) => SessionError::InvalidSessionCommand {
                    message: "session-supervisor command loop is unavailable".to_string(),
                },
            })
    }
}

#[must_use]
pub fn session_supervisor_command_channel() -> (
    SessionSupervisorCommandSender,
    channel::Receiver<SessionSupervisorCommand>,
) {
    let (sender, receiver) = channel::bounded(1);
    (SessionSupervisorCommandSender(sender), receiver)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollServicesReport {
    pub exited: Vec<ServiceProcessExit>,
    pub still_running: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopServicesReport {
    pub stopped: Vec<ServiceProcessExit>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceBoundaryProbeReport {
    pub checks: Vec<String>,
}

impl ServiceBoundaryProbeReport {
    #[must_use]
    pub const fn none() -> Self {
        Self { checks: Vec::new() }
    }

    #[must_use]
    pub fn from_checks(checks: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            checks: checks.into_iter().map(Into::into).collect(),
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.checks.is_empty()
    }
}

pub trait ServiceBoundaryProbe: std::fmt::Debug {
    /// Checks that `process` really speaks the protocol `descriptor` advertises.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::ServiceBoundaryProbeFailed`] when the probe
    /// reaches the service but its protocol surface does not match the
    /// descriptor's role, and whatever transport-shaped [`SessionError`] the
    /// implementation's own dial path produces when the service cannot be
    /// reached at all.
    fn verify_service_boundary(
        &self,
        session: &str,
        manifest: &SessionManifest,
        process: &ServiceProcessRecord,
        descriptor: &ServiceDescriptor,
    ) -> Result<ServiceBoundaryProbeReport, SessionError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopServiceBoundaryProbe;

impl ServiceBoundaryProbe for NoopServiceBoundaryProbe {
    fn verify_service_boundary(
        &self,
        _session: &str,
        _manifest: &SessionManifest,
        _process: &ServiceProcessRecord,
        _descriptor: &ServiceDescriptor,
    ) -> Result<ServiceBoundaryProbeReport, SessionError> {
        Ok(ServiceBoundaryProbeReport::none())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProtocolServiceBoundaryProbe {
    rpc_timeout: Duration,
}

impl Default for ProtocolServiceBoundaryProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolServiceBoundaryProbe {
    pub const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(30);

    #[must_use]
    pub const fn new() -> Self {
        Self {
            rpc_timeout: Self::DEFAULT_RPC_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_rpc_timeout(mut self, timeout: Duration) -> Self {
        self.rpc_timeout = timeout;
        self
    }
}

impl ServiceBoundaryProbe for ProtocolServiceBoundaryProbe {
    fn verify_service_boundary(
        &self,
        session: &str,
        manifest: &SessionManifest,
        process: &ServiceProcessRecord,
        descriptor: &ServiceDescriptor,
    ) -> Result<ServiceBoundaryProbeReport, SessionError> {
        if descriptor.role != ServiceRole::RuntimeHost {
            return Err(service_boundary_probe_failed(
                session,
                process,
                descriptor,
                format!(
                    "role {:?} is project-scoped and cannot be probed by a session supervisor",
                    descriptor.role
                ),
            ));
        }
        self.run_probe(session, process, descriptor, async move {
            verify_runtime_host_boundary(manifest, process, descriptor).await
        })?;
        Ok(ServiceBoundaryProbeReport::from_checks([
            "runtime-host.projectionCatalog",
            "runtime-host.projectHostRuntimeControlGrant",
        ]))
    }
}

impl ProtocolServiceBoundaryProbe {
    fn run_probe<F>(
        &self,
        session: &str,
        process: &ServiceProcessRecord,
        descriptor: &ServiceDescriptor,
        probe: F,
    ) -> Result<(), SessionError>
    where
        F: Future<Output = Result<(), String>>,
    {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                service_boundary_probe_failed(
                    session,
                    process,
                    descriptor,
                    format!("failed to start probe runtime: {error}"),
                )
            })?;
        let local = LocalSet::new();
        let timeout = self.rpc_timeout;

        local.block_on(&runtime, async move {
            match tokio::time::timeout(timeout, probe).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(reason)) => Err(service_boundary_probe_failed(
                    session, process, descriptor, reason,
                )),
                Err(_) => Err(service_boundary_probe_failed(
                    session,
                    process,
                    descriptor,
                    format!(
                        "protocol probe timed out after {}ms",
                        duration_millis(timeout)
                    ),
                )),
            }
        })
    }
}

// Cannot be made `Send`: the runtime-host capability this dials is a capnp-rpc
// `Client`, which is `!Send` by construction; the probe drives it on its own
// current-thread runtime.
#[allow(clippy::future_not_send)]
async fn verify_runtime_host_boundary(
    manifest: &SessionManifest,
    process: &ServiceProcessRecord,
    descriptor: &ServiceDescriptor,
) -> Result<(), String> {
    let capability = editor_capability(
        manifest,
        descriptor,
        RUNTIME_HOST_AUDIENCE,
        &[RUNTIME_READ_PERMISSION],
    )?;
    let client: runtime_capnp::runtime_host::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint)
            .await
            .map_err(|error| format!("connect runtime-host failed: {error}"))?;
    let mut request = client.projection_catalog_request();
    (RuntimeProjectionCatalogRequest { capability })
        .to_capnp(request.get().init_request())
        .map_err(|error| format!("runtime-host projectionCatalog request failed: {error}"))?;
    let response = request
        .send()
        .promise
        .await
        .map_err(|error| format!("runtime-host projectionCatalog RPC failed: {error}"))?;
    RuntimeProjectionCatalogResult::from_capnp(
        response
            .get()
            .map_err(|error| format!("runtime-host projectionCatalog response failed: {error}"))?
            .get_result()
            .map_err(|error| format!("runtime-host projectionCatalog result failed: {error}"))?,
    )
    .map_err(|error| format!("runtime-host projectionCatalog decode failed: {error}"))?;
    verify_runtime_host_project_grant(manifest, process, descriptor)?;
    Ok(())
}

fn verify_runtime_host_project_grant(
    manifest: &SessionManifest,
    process: &ServiceProcessRecord,
    descriptor: &ServiceDescriptor,
) -> Result<(), String> {
    let project_host_capability = descriptor
        .brokered_capability_template(
            ServiceRole::ProjectHost,
            RUNTIME_HOST_AUDIENCE,
            &[RUNTIME_CONTROL_PERMISSION],
            Some(manifest.id.0),
        )
        .ok_or_else(|| {
            format!(
                "runtime-host `{}`/`{}` does not grant `{}` to project-host for session `{}`",
                descriptor.id.namespace,
                descriptor.id.name,
                RUNTIME_CONTROL_PERMISSION,
                manifest.id
            )
        })?;

    let expected_project_host = ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME);
    if project_host_capability.service != expected_project_host {
        return Err(format!(
            "runtime-host project-host grant targets `{}`/`{}` instead of `{}`/`{}`",
            project_host_capability.service.namespace,
            project_host_capability.service.name,
            expected_project_host.namespace,
            expected_project_host.name
        ));
    }
    if project_host_capability.token_hash.is_empty() {
        return Err("runtime-host project-host grant has no brokered token hash".to_string());
    }

    let grants_path = process_capability_grants_path("runtime-host", process)?;
    let bytes = fs::read(grants_path).map_err(|error| {
        format!(
            "runtime-host capability grant file `{}` could not be read: {error}",
            grants_path.display()
        )
    })?;
    let grant_set = decode_capability_grant_set(&bytes).map_err(|error| {
        format!(
            "runtime-host capability grant file `{}` could not be decoded: {error}",
            grants_path.display()
        )
    })?;
    grant_set
        .validate(&project_host_capability, RUNTIME_CONTROL_PERMISSION)
        .map_err(|error| {
            format!(
                "runtime-host capability grant file `{}` does not contain the brokered project-host runtime-control grant: {error}",
                grants_path.display()
            )
        })?;

    Ok(())
}

fn process_capability_grants_path<'a>(
    service_label: &str,
    process: &'a ServiceProcessRecord,
) -> Result<&'a Path, String> {
    process
        .args
        .windows(2)
        .find(|pair| pair[0] == "--capability-grants")
        .map(|pair| Path::new(&pair[1]))
        .ok_or_else(|| {
            format!(
                "{service_label} `{}` was not launched with --capability-grants",
                process.service_name
            )
        })
}

fn editor_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    audience: &str,
    permissions: &[&str],
) -> Result<Capability, String> {
    descriptor
        .brokered_capability_template(
            ServiceRole::Editor,
            audience,
            permissions,
            Some(manifest.id.0),
        )
        .ok_or_else(|| {
            format!(
                "service `{}`/`{}` does not grant `{}` to editor for audience `{audience}`",
                descriptor.id.namespace,
                descriptor.id.name,
                permissions.join(", ")
            )
        })
}

fn service_boundary_probe_failed(
    session: &str,
    process: &ServiceProcessRecord,
    descriptor: &ServiceDescriptor,
    reason: impl Into<String>,
) -> SessionError {
    SessionError::ServiceBoundaryProbeFailed {
        session: session.to_string(),
        service: process.service_name.clone(),
        role: descriptor.role,
        reason: reason.into(),
    }
}

#[derive(Debug)]
pub struct SessionServiceSupervisor<L = StdServiceProcessLauncher, P = NoopServiceBoundaryProbe> {
    manager: SessionManager,
    launcher: L,
    boundary_probe: P,
    lifecycle: RefCell<ServiceLifecycleEvents>,
    ready_timeout: Duration,
}

impl SessionServiceSupervisor<StdServiceProcessLauncher, ProtocolServiceBoundaryProbe> {
    /// Builds a supervisor over the project at `project_root` with the real
    /// process launcher and the protocol boundary probe.
    ///
    /// # Errors
    ///
    /// Returns any error [`SessionManager::new`] returns while reading the
    /// project manifest and resolving the machine-local data home.
    pub fn new(project_root: impl AsRef<Path>) -> Result<Self, SessionError> {
        Ok(Self::with_manager_and_probe(
            SessionManager::new(project_root)?,
            StdServiceProcessLauncher::new(),
            ProtocolServiceBoundaryProbe::new(),
        ))
    }
}

impl<L> SessionServiceSupervisor<L, NoopServiceBoundaryProbe>
where
    L: ServiceProcessLauncher,
    SessionError: From<L::Error>,
{
    #[must_use]
    pub fn with_manager(manager: SessionManager, launcher: L) -> Self {
        Self::with_manager_and_probe(manager, launcher, NoopServiceBoundaryProbe)
    }
}

impl<L, P> SessionServiceSupervisor<L, P>
where
    L: ServiceProcessLauncher,
    SessionError: From<L::Error>,
    P: ServiceBoundaryProbe,
{
    #[must_use]
    pub fn with_manager_and_probe(manager: SessionManager, launcher: L, boundary_probe: P) -> Self {
        Self {
            manager,
            launcher,
            boundary_probe,
            lifecycle: RefCell::new(ServiceLifecycleEvents::new()),
            ready_timeout: Duration::from_mins(5),
        }
    }

    #[must_use]
    pub const fn with_ready_timeout(mut self, timeout: Duration) -> Self {
        self.ready_timeout = timeout;
        self
    }

    #[must_use]
    pub const fn manager(&self) -> &SessionManager {
        &self.manager
    }

    /// Starts every startable service planned for `session`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::start_planned_services_matching`] returns.
    #[instrument(skip(self))]
    pub fn start_planned_services(
        &self,
        session: &str,
    ) -> Result<StartServicesReport, SessionError> {
        self.start_planned_services_matching(session, &StartServicesFilter::all())
    }

    /// Starts the planned services `filter` selects, waits for each to publish
    /// readiness, and rolls the whole batch back if any step fails.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::SessionNotFound`] or
    /// [`SessionError::SessionNotActive`] if `session` is not an active
    /// workspace; [`SessionError::ServiceProcessCleanupRefused`] if a stale
    /// recorded process cannot be proven gone;
    /// [`SessionError::UnsupportedServiceRole`] or
    /// [`SessionError::MissingServiceDescriptor`] if a planned service is not a
    /// runtime host or its project-host/asset-processor attachments are absent;
    /// [`SessionError::ServiceProcess`] if a child cannot be launched or the
    /// lifecycle event source rejects its identity; and the readiness errors
    /// [`SessionError::ServiceProcessExitedBeforeReady`],
    /// [`SessionError::ServiceProcessReadyTimeout`],
    /// [`SessionError::InvalidServiceReadyRecord`], and
    /// [`SessionError::ServiceBoundaryProbeFailed`]. Manifest writes along the
    /// way surface [`SessionError::Io`], [`SessionError::TomlSerialize`], and
    /// [`SessionError::FileTransaction`].
    #[instrument(skip(self, filter))]
    pub fn start_planned_services_matching(
        &self,
        session: &str,
        filter: &StartServicesFilter,
    ) -> Result<StartServicesReport, SessionError> {
        let start_services_started = Instant::now();
        let setup_started = Instant::now();
        let manifest = self
            .manager
            .active_session_workspace(session, "start services")?;

        let setup_ms = duration_millis(setup_started.elapsed());
        let mut started = Vec::new();
        let mut skipped = Vec::new();
        let mut pending = Vec::new();

        for process in current_process_records(&manifest) {
            if !filter.includes(&process.service_name) {
                continue;
            }
            let key = ServiceProcessKey::from_process(&process);
            let stale_active_record = matches!(
                process.state,
                ServiceProcessState::Starting | ServiceProcessState::Running
            ) && !self.launcher.is_tracking(&key);
            if stale_active_record {
                let cleanup = require_recorded_process_gone(
                    session,
                    &process,
                    terminate_recorded_service_process(&process)?,
                )?;
                info!(
                    session,
                    service = %process.service_name,
                    state = ?process.state,
                    cleanup = ?cleanup,
                    "recovering stale unowned session service record before startup"
                );
                self.mark_process_exited(session, &process, None, None)?;
            }

            if session_process_state_is_startable(process.state) || stale_active_record {
                match self.spawn_service_process(session, &process) {
                    Ok(spawned) => {
                        started.push(spawned.clone());
                        pending.push((process, spawned, Instant::now()));
                    }
                    Err(error) => {
                        let message = error.to_string();
                        self.rollback_started_services_after_startup_failure(
                            session, &started, &message,
                        )?;
                        return Err(error);
                    }
                }
            } else {
                skipped.push(process.service_name);
            }
        }

        for (_, spawned, _) in &pending {
            if let Err(error) = self.lifecycle.borrow().add_identity(spawned.identity) {
                let message = error.to_string();
                self.rollback_started_services_after_startup_failure(session, &started, &message)?;
                return Err(error.into());
            }
        }
        match self.wait_for_services_ready(session, &pending) {
            Ok(()) => {}
            Err(error) => {
                let message = error.to_string();
                self.rollback_started_services_after_startup_failure(session, &started, &message)?;
                return Err(error);
            }
        }

        let total_ms = duration_millis(start_services_started.elapsed());
        info!(
            session,
            total_ms,
            setup_ms,
            started = started.len(),
            skipped = skipped.len(),
            timing_table = %format!(
                "stage                 ms\nsetup+manifest     {setup_ms:>6}\nspawn+ready        {:>6}\ntotal              {total_ms:>6}",
                total_ms.saturating_sub(setup_ms)
            ),
            "session service startup timing summary"
        );
        Ok(StartServicesReport { started, skipped })
    }

    /// Launch the session-owned runtime host and mark it `Starting`.
    fn spawn_service_process(
        &self,
        session: &str,
        process: &ServiceProcessRecord,
    ) -> Result<SpawnedServiceProcess, SessionError> {
        let spawn_started = Instant::now();
        self.ensure_project_service_attachments_available(session, process)?;
        let key = ServiceProcessKey::from_process(process);
        self.manager.mark_service_starting(session, &key)?;
        let spawned = match self.launcher.spawn(process) {
            Ok(spawned) => spawned,
            Err(error) => {
                let error = SessionError::from(error);
                self.mark_process_exited(
                    session,
                    process,
                    None,
                    Some(format!("spawn failed: {error}")),
                )?;
                return Err(error);
            }
        };
        info!(
            session,
            service = %spawned.service_name,
            pid = spawned.identity.process_id,
            spawn_ms = duration_millis(spawn_started.elapsed()),
            "started session service"
        );
        Ok(spawned)
    }

    fn ensure_project_service_attachments_available(
        &self,
        session: &str,
        process: &ServiceProcessRecord,
    ) -> Result<(), SessionError> {
        if process.role != SupervisedServiceRole::RuntimeHost {
            return Err(SessionError::UnsupportedServiceRole {
                role: process.role.to_proto(),
            });
        }
        let manifest = self.manager.session(session)?;
        for (role, label) in [
            (SupervisedServiceRole::ProjectHost, "project-host"),
            (SupervisedServiceRole::AssetProcessor, "asset-processor"),
        ] {
            if !manifest.services.iter().any(|service| service.role == role) {
                return Err(SessionError::MissingServiceDescriptor {
                    session: session.to_string(),
                    service: label.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Observe readiness after the lifecycle event source has woken the
    /// supervisor. Returns
    /// `Ok(true)` once the service is verified Running, `Ok(false)` while it is
    /// still starting, and `Err` if the ready record or boundary probe is
    /// invalid. Exit and deadline handling belong to the lifecycle wait below.
    fn observe_service_ready(
        &self,
        session: &str,
        process: &ServiceProcessRecord,
        spawned: &SpawnedServiceProcess,
        started: Instant,
    ) -> Result<bool, SessionError> {
        let Some(ready_file) = &process.ready_file else {
            let key = ServiceProcessKey::from_spawned(spawned);
            self.manager
                .mark_service_running(session, &key, spawned.identity)?;
            return Ok(true);
        };

        if ready_file.exists() {
            let ready_observed = Instant::now();
            let key = ServiceProcessKey::from_spawned(spawned);
            match self
                .manager
                .mark_service_ready(session, &key, spawned.identity)
            {
                Ok(manifest) => {
                    let process =
                        spawned_service_process_record(&manifest, spawned).ok_or_else(|| {
                            SessionError::MissingServiceProcess {
                                session: session.to_string(),
                                service: spawned.service_name.clone(),
                            }
                        })?;
                    let descriptor =
                        service_descriptor_for_process(&manifest, process).ok_or_else(|| {
                            SessionError::MissingServiceDescriptor {
                                session: session.to_string(),
                                service: process.service_name.clone(),
                            }
                        });
                    let boundary_started = Instant::now();
                    match descriptor.and_then(|descriptor| {
                        self.boundary_probe.verify_service_boundary(
                            session,
                            &manifest,
                            process,
                            &descriptor,
                        )
                    }) {
                        Ok(report) => {
                            if !report.is_empty() {
                                info!(
                                    session,
                                    service = %spawned.service_name,
                                    pid = spawned.identity.process_id,
                                    checks = ?report.checks,
                                    "session service protocol boundary verified"
                                );
                            }
                        }
                        Err(error) => {
                            let message = error.to_string();
                            let exit_code = self
                                .terminate_startup_child(spawned)?
                                .and_then(|exit| exit.exit_code);
                            self.mark_spawned_service_exited(
                                session,
                                spawned,
                                exit_code,
                                Some(format!("boundary probe failed: {message}")),
                            )?;
                            return Err(error);
                        }
                    }
                    let boundary_ms = duration_millis(boundary_started.elapsed());
                    let spawn_to_ready_ms = duration_millis(started.elapsed());
                    info!(
                            session,
                            service = %spawned.service_name,
                            pid = spawned.identity.process_id,
                            endpoint_kind = ?process.endpoint_kind,
                            endpoint = %process.endpoint_address,
                            spawn_to_ready_ms,
                            ready_commit_ms = duration_millis(ready_observed.elapsed()),
                            boundary_ms,
                            "session service published readiness"
                    );
                    return Ok(true);
                }
                Err(error) => {
                    let message = error.to_string();
                    let exit_code = self
                        .terminate_startup_child(spawned)?
                        .and_then(|exit| exit.exit_code);
                    self.mark_spawned_service_exited(
                        session,
                        spawned,
                        exit_code,
                        Some(format!("readiness failed: {message}")),
                    )?;
                    return Err(error);
                }
            }
        }

        Ok(false)
    }

    /// Wait for the session-owned runtime service to publish readiness.
    fn wait_for_services_ready(
        &self,
        session: &str,
        pending: &[(ServiceProcessRecord, SpawnedServiceProcess, Instant)],
    ) -> Result<(), SessionError> {
        let mut ready_subscription = self.lifecycle.borrow().subscribe_ready(
            pending
                .iter()
                .filter_map(|(process, _, _)| process.ready_file.as_deref()),
        )?;
        let deadline = pending
            .iter()
            .map(|(_, _, started)| *started + self.ready_timeout)
            .min()
            .unwrap_or_else(Instant::now);
        let mut ready = vec![false; pending.len()];
        loop {
            let mut all_ready = true;
            for (index, (process, spawned, started)) in pending.iter().enumerate() {
                if ready[index] {
                    continue;
                }
                if self.observe_service_ready(session, process, spawned, *started)? {
                    ready[index] = true;
                } else {
                    all_ready = false;
                }
            }
            if all_ready {
                ready_subscription.finish()?;
                return Ok(());
            }
            match self.lifecycle.borrow().wait_until(deadline)? {
                Some(ServiceLifecycleEvent::ReadyFileChanged) => {}
                Some(ServiceLifecycleEvent::ProcessExited(identity)) => {
                    let (process, spawned, _) = pending
                        .iter()
                        .find(|(_, spawned, _)| spawned.identity == identity)
                        .ok_or_else(|| SessionError::InvalidSessionCommand {
                            message: format!(
                                "received an exit event for untracked process identity {identity:?}"
                            ),
                        })?;
                    let key = ServiceProcessKey::from_spawned(spawned);
                    let exit = self.launcher.try_wait(&key)?.ok_or_else(|| {
                        SessionError::InvalidSessionCommand {
                            message: format!(
                                "exit event for `{}` could not reap its owned child",
                                spawned.service_name
                            ),
                        }
                    })?;
                    let failure = format!(
                        "service `{}` exited before publishing readiness; status={:?}",
                        exit.service_name, exit.exit_code
                    );
                    self.mark_process_exited(session, process, exit.exit_code, Some(failure))?;
                    self.lifecycle.borrow().consume_exit(identity)?;
                    return Err(SessionError::ServiceProcessExitedBeforeReady {
                        service: exit.service_name,
                        exit_code: exit.exit_code,
                    });
                }
                Some(ServiceLifecycleEvent::ProcessExitWaitFailed { identity, reason }) => {
                    self.lifecycle.borrow().consume_exit(identity)?;
                    return Err(SessionError::InvalidSessionCommand {
                        message: format!(
                            "identity-bound exit wait for {identity:?} failed: {reason}"
                        ),
                    });
                }
                None => {
                    let (process, spawned, _) = pending
                        .iter()
                        .zip(&ready)
                        .find(|(_, ready)| !**ready)
                        .map(|((process, spawned, started), _)| (process, spawned, started))
                        .expect("unready service exists before the startup deadline");
                    let timeout_ms = duration_millis(self.ready_timeout);
                    let exit_code = self
                        .terminate_startup_child(spawned)?
                        .and_then(|exit| exit.exit_code);
                    self.mark_process_exited(
                        session,
                        process,
                        exit_code,
                        Some(format!(
                            "service `{}` did not publish readiness within {timeout_ms}ms",
                            spawned.service_name
                        )),
                    )?;
                    return Err(SessionError::ServiceProcessReadyTimeout {
                        service: spawned.service_name.clone(),
                        timeout_ms,
                    });
                }
            }
        }
    }

    fn mark_spawned_service_exited(
        &self,
        session: &str,
        spawned: &SpawnedServiceProcess,
        exit_code: Option<i32>,
        failure: Option<String>,
    ) -> Result<SessionManifest, SessionError> {
        let key = ServiceProcessKey::from_spawned(spawned);
        self.manager
            .mark_service_exited(session, &key, exit_code, failure)
    }

    fn mark_process_exited(
        &self,
        session: &str,
        process: &ServiceProcessRecord,
        exit_code: Option<i32>,
        failure: Option<String>,
    ) -> Result<SessionManifest, SessionError> {
        let key = ServiceProcessKey::from_process(process);
        self.manager
            .mark_service_exited(session, &key, exit_code, failure)
    }

    fn terminate_startup_child(
        &self,
        spawned: &SpawnedServiceProcess,
    ) -> Result<Option<ServiceProcessExit>, SessionError> {
        let key = ServiceProcessKey::from_spawned(spawned);
        let exit = self.launcher.terminate(&key)?;
        if exit.is_some() {
            self.lifecycle.borrow().retire_exit(spawned.identity)?;
            info!(
                service = %spawned.service_name,
                run = %spawned.run,
                "terminated session service after startup failure"
            );
        }
        Ok(exit)
    }

    fn rollback_started_services_after_startup_failure(
        &self,
        session: &str,
        started: &[SpawnedServiceProcess],
        reason: &str,
    ) -> Result<Vec<ServiceProcessExit>, SessionError> {
        let mut terminated = Vec::new();
        for spawned in started.iter().rev() {
            let key = ServiceProcessKey::from_spawned(spawned);
            let exit = if self.launcher.is_tracking(&key) {
                self.launcher
                    .terminate(&key)?
                    .unwrap_or_else(|| ServiceProcessExit {
                        service_name: spawned.service_name.clone(),
                        exit_code: None,
                        success: false,
                    })
            } else {
                ServiceProcessExit {
                    service_name: spawned.service_name.clone(),
                    exit_code: None,
                    success: false,
                }
            };
            self.lifecycle.borrow().retire_exit(spawned.identity)?;
            self.mark_spawned_service_exited(
                session,
                spawned,
                exit.exit_code,
                Some(format!(
                    "service `{}` terminated because startup failed: {reason}",
                    spawned.service_name
                )),
            )?;
            info!(
                session,
                service = %spawned.service_name,
                "terminated already-started session service after startup failure"
            );
            terminated.push(exit);
        }
        Ok(terminated)
    }

    /// Wait for the next exact owned-child exit after successful startup.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::ServiceProcess`] if the lifecycle event source
    /// fails while waiting, and any error [`Self::handle_lifecycle_event`]
    /// returns for the event it delivers.
    pub fn wait_for_service_exit(&self, session: &str) -> Result<ServiceProcessExit, SessionError> {
        loop {
            if let Some(exit) =
                self.handle_lifecycle_event(session, self.lifecycle.borrow().wait()?)?
            {
                return Ok(exit);
            }
        }
    }

    #[must_use]
    pub fn lifecycle_events(&self) -> crossbeam_channel::Receiver<ServiceLifecycleEvent> {
        self.lifecycle.borrow().receiver_clone()
    }

    /// Apply one selected lifecycle event to the durable session state.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidSessionCommand`] if the event names a
    /// process identity this session does not track, if the owned child cannot
    /// be reaped, or if the event itself reports a failed identity-bound exit
    /// wait; [`SessionError::ServiceProcess`] if the launcher or the lifecycle
    /// event source fails; and the manifest-write errors [`SessionError::Io`],
    /// [`SessionError::TomlSerialize`], and [`SessionError::FileTransaction`]
    /// raised while recording the exit.
    pub fn handle_lifecycle_event(
        &self,
        session: &str,
        event: ServiceLifecycleEvent,
    ) -> Result<Option<ServiceProcessExit>, SessionError> {
        match event {
            ServiceLifecycleEvent::ReadyFileChanged => Ok(None),
            ServiceLifecycleEvent::ProcessExited(identity) => {
                let manifest = self.manager.snapshot(session)?;
                let process = active_process_records_in_stop_order(&manifest)
                    .into_iter()
                    .find(|process| {
                        process.pid == Some(identity.process_id)
                            && process.process_start_time == Some(identity.process_start_time)
                    })
                    .ok_or_else(|| SessionError::InvalidSessionCommand {
                        message: format!("exit event for untracked process identity {identity:?}"),
                    })?;
                let key = ServiceProcessKey::from_process(&process);
                let exit = self.launcher.try_wait(&key)?.ok_or_else(|| {
                    SessionError::InvalidSessionCommand {
                        message: format!(
                            "exit event for `{}` could not reap its owned child",
                            process.service_name
                        ),
                    }
                })?;
                let failure = (!exit.success).then(|| {
                    format!(
                        "service `{}` exited with status {:?}",
                        exit.service_name, exit.exit_code
                    )
                });
                self.mark_process_exited(session, &process, exit.exit_code, failure)?;
                self.lifecycle.borrow().consume_exit(identity)?;
                Ok(Some(exit))
            }
            ServiceLifecycleEvent::ProcessExitWaitFailed { identity, reason } => {
                let manifest = self.manager.snapshot(session)?;
                let process = active_process_records_in_stop_order(&manifest)
                    .into_iter()
                    .find(|process| {
                        process.pid == Some(identity.process_id)
                            && process.process_start_time == Some(identity.process_start_time)
                    })
                    .ok_or_else(|| SessionError::InvalidSessionCommand {
                        message: format!(
                            "exit wait failed for untracked process identity {identity:?}: {reason}"
                        ),
                    })?;
                let failure = format!("identity-bound exit wait failed for {identity:?}: {reason}");
                self.mark_process_exited(session, &process, None, Some(failure))?;
                self.lifecycle.borrow().consume_exit(identity)?;
                Err(SessionError::InvalidSessionCommand {
                    message: format!("identity-bound exit wait for {identity:?} failed: {reason}"),
                })
            }
        }
    }

    /// Names the services `session` currently has in an active process record,
    /// in stop order.
    ///
    /// # Errors
    ///
    /// Returns any error [`SessionManager::snapshot`] returns while reading the
    /// session manifest.
    pub fn running_service_names(&self, session: &str) -> Result<Vec<String>, SessionError> {
        let manifest = self.manager.snapshot(session)?;
        Ok(active_process_records_in_stop_order(&manifest)
            .into_iter()
            .map(|process| process.service_name)
            .collect())
    }

    fn wait_for_graceful_shutdown_exit(
        &self,
        session: &str,
        identity: ProcessIdentity,
    ) -> Result<Option<ServiceProcessExit>, SessionError> {
        let deadline = Instant::now() + GRACEFUL_SHUTDOWN_TIMEOUT;
        let Some(event) = self
            .lifecycle
            .borrow()
            .wait_for_exit_until(identity, deadline)?
        else {
            return Ok(None);
        };
        self.handle_lifecycle_event(session, event)
    }

    /// Stops every owned child of `session`, gracefully first and then by
    /// termination, recording each exit.
    ///
    /// # Errors
    ///
    /// Returns any error [`SessionManager::session`] returns while reading the
    /// manifest, [`SessionError::ServiceProcess`] if a shutdown request,
    /// lifecycle wait, or termination fails,
    /// [`SessionError::ServiceProcessCleanupRefused`] if a terminated process
    /// cannot be proven gone, and the manifest-write errors
    /// [`SessionError::Io`], [`SessionError::TomlSerialize`], and
    /// [`SessionError::FileTransaction`].
    pub fn stop_owned_services(
        &self,
        session: &str,
        reason: &str,
    ) -> Result<StopServicesReport, SessionError> {
        let manifest = self.manager.session(session)?;
        let mut stopped = Vec::new();
        let mut skipped = Vec::new();

        for process in active_process_records_in_stop_order(&manifest) {
            let key = ServiceProcessKey::from_process(&process);
            if !self.launcher.is_tracking(&key) {
                skipped.push(process.service_name.clone());
                continue;
            }

            let graceful_exit = if let Some(descriptor) =
                service_descriptor_for_process(&manifest, &process)
            {
                let controller = ServiceLifecycleController::new(
                    ServiceId::new(
                        SESSION_SUPERVISOR_NAMESPACE,
                        SESSION_SUPERVISOR_SERVICE_NAME,
                    ),
                    ServiceRole::SessionSupervisor,
                );
                match request_service_lifecycle_shutdown(&descriptor, &controller) {
                    Ok(()) => match self.wait_for_graceful_shutdown_exit(
                        session,
                        recorded_process_identity(&process)?,
                    ) {
                        Ok(Some(exit)) => Some(exit),
                        Ok(None) => {
                            warn!(
                                session,
                                service = %process.service_name,
                                "service lifecycle control did not exit before the shutdown deadline; forcing exact process termination"
                            );
                            None
                        }
                        Err(error) => {
                            warn!(
                                session,
                                service = %process.service_name,
                                error = %error,
                                "service lifecycle exit wait failed; forcing exact process termination"
                            );
                            None
                        }
                    },
                    Err(error) => {
                        warn!(
                            session,
                            service = %process.service_name,
                            error = %error,
                            "service lifecycle control request failed; forcing exact process termination"
                        );
                        None
                    }
                }
            } else {
                None
            };

            let exit = if let Some(exit) = graceful_exit {
                exit
            } else {
                let termination = self.launcher.terminate(&key)?;
                let terminated = termination.is_some();
                let exit = termination.unwrap_or_else(|| ServiceProcessExit {
                    service_name: process.service_name.clone(),
                    exit_code: None,
                    success: false,
                });
                if terminated {
                    self.lifecycle
                        .borrow()
                        .retire_exit(recorded_process_identity(&process)?)?;
                }
                self.mark_process_exited(session, &process, None, None)?;
                exit
            };
            info!(
                session,
                service = %process.service_name,
                exit_code = ?exit.exit_code,
                reason,
                "stopped owned session service"
            );
            stopped.push(exit);
        }

        Ok(StopServicesReport { stopped, skipped })
    }
}

fn spawned_service_process_record<'a>(
    manifest: &'a SessionManifest,
    spawned: &SpawnedServiceProcess,
) -> Option<&'a ServiceProcessRecord> {
    manifest.processes.iter().find(|process| {
        process.service_name == spawned.service_name && process.role == spawned.role
    })
}

fn current_process_records(manifest: &SessionManifest) -> Vec<ServiceProcessRecord> {
    manifest.processes.clone()
}

fn recorded_process_identity(
    process: &ServiceProcessRecord,
) -> Result<ProcessIdentity, SessionError> {
    match (process.pid, process.process_start_time) {
        (Some(process_id), Some(process_start_time)) => Ok(ProcessIdentity {
            process_id,
            process_start_time,
        }),
        _ => Err(SessionError::InvalidSessionCommand {
            message: format!(
                "running service `{}` has no complete process identity",
                process.service_name
            ),
        }),
    }
}

fn active_process_records_in_stop_order(manifest: &SessionManifest) -> Vec<ServiceProcessRecord> {
    let mut processes = manifest
        .processes
        .iter()
        .enumerate()
        .filter(|(_, process)| {
            matches!(
                process.state,
                ServiceProcessState::Starting | ServiceProcessState::Running
            )
        })
        .collect::<Vec<_>>();
    processes.sort_by_key(|(index, _)| *index);
    processes.reverse();
    processes
        .into_iter()
        .map(|(_, process)| process.clone())
        .collect()
}

const fn session_process_state_is_startable(state: ServiceProcessState) -> bool {
    matches!(
        state,
        ServiceProcessState::Planned | ServiceProcessState::Exited | ServiceProcessState::Failed
    )
}

fn require_recorded_process_gone(
    session: &str,
    process: &ServiceProcessRecord,
    cleanup: RecordedServiceProcessCleanup,
) -> Result<RecordedServiceProcessCleanup, SessionError> {
    if cleanup.proves_recorded_process_gone() {
        Ok(cleanup)
    } else {
        Err(SessionError::ServiceProcessCleanupRefused {
            session: session.to_string(),
            service: process.service_name.clone(),
            cleanup,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::SessionId;
    use az_proto_runtime::RUNTIME_HOST_SERVICE_NAME;

    #[test]
    fn unowned_cleanup_refusal_blocks_session_record_retirement() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint =
            az_proto_core::Endpoint::new(az_proto_core::EndpointKind::Tcp, "127.0.0.1:0");
        let mut process = ServiceProcessRecord::planned(
            RUNTIME_HOST_SERVICE_NAME,
            SupervisedServiceRole::RuntimeHost,
            uuid::Uuid::now_v7(),
            &endpoint,
            "runtime-host".to_string(),
            temp.path().to_path_buf(),
            Vec::new(),
            temp.path().join("runtime.out"),
            temp.path().join("runtime.err"),
            temp.path().join("runtime.log"),
            None,
            1,
        );
        process
            .mark_running(
                az_service_supervision::ProcessIdentity::current().unwrap(),
                2,
            )
            .unwrap();
        let cleanup = RecordedServiceProcessCleanup::IdentityBindingUnavailable {
            pid: 41,
            reason: "pidfd unavailable".to_string(),
        };

        let error = require_recorded_process_gone("active", &process, cleanup.clone())
            .expect_err("a refused cleanup must block session recovery");

        assert!(matches!(
            error,
            SessionError::ServiceProcessCleanupRefused {
                session,
                service,
                cleanup: actual,
            } if session == "active" && service == RUNTIME_HOST_SERVICE_NAME && actual == cleanup
        ));
        assert_eq!(process.state, ServiceProcessState::Running);
    }

    #[test]
    fn explicit_start_retries_failed_service_records() {
        assert!(session_process_state_is_startable(
            ServiceProcessState::Planned
        ));
        assert!(session_process_state_is_startable(
            ServiceProcessState::Exited
        ));
        assert!(session_process_state_is_startable(
            ServiceProcessState::Failed
        ));
        assert!(!session_process_state_is_startable(
            ServiceProcessState::Starting
        ));
        assert!(!session_process_state_is_startable(
            ServiceProcessState::Running
        ));
    }

    #[test]
    fn readiness_probe_uses_the_single_current_service_record() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = az_proto_core::Endpoint::new(
            az_proto_core::EndpointKind::WindowsNamedPipe,
            r"\\.\pipe\worker",
        );
        let record_run = uuid::Uuid::now_v7();
        let process = ServiceProcessRecord::planned(
            RUNTIME_HOST_SERVICE_NAME,
            SupervisedServiceRole::RuntimeHost,
            record_run,
            &endpoint,
            "asset-worker".to_string(),
            temp.path().to_path_buf(),
            Vec::new(),
            temp.path().join("worker.out"),
            temp.path().join("worker.err"),
            temp.path().join("worker.capnp.log"),
            Some(temp.path().join("worker.ready")),
            1,
        );
        let mut manifest = SessionManifest::new(
            SessionId::new(),
            "project".to_string(),
            "session".to_string(),
            temp.path().to_path_buf(),
            temp.path().to_path_buf(),
            temp.path().to_path_buf(),
            1,
        );
        manifest.processes.push(process);
        let spawned = SpawnedServiceProcess {
            service_name: RUNTIME_HOST_SERVICE_NAME.to_string(),
            role: SupervisedServiceRole::RuntimeHost,
            run: uuid::Uuid::now_v7(),
            identity: az_service_supervision::ProcessIdentity {
                process_id: 42,
                process_start_time: 9_001,
            },
        };

        let selected = spawned_service_process_record(&manifest, &spawned).unwrap();

        assert_eq!(selected.run, record_run);
        assert_eq!(selected.endpoint_address, r"\\.\pipe\worker");
    }

    #[test]
    fn std_launcher_drop_terminates_spawned_service_process() {
        let temp = tempfile::tempdir().unwrap();
        let (program, args) = sleeping_child_command();
        let endpoint =
            az_proto_core::Endpoint::new(az_proto_core::EndpointKind::Tcp, "127.0.0.1:0");
        let process = ServiceProcessRecord::planned(
            RUNTIME_HOST_SERVICE_NAME,
            SupervisedServiceRole::RuntimeHost,
            uuid::Uuid::now_v7(),
            &endpoint,
            program,
            temp.path().to_path_buf(),
            args,
            temp.path().join("dummy.out"),
            temp.path().join("dummy.err"),
            temp.path().join("dummy.capnp.log"),
            None,
            1,
        );

        let launcher = StdServiceProcessLauncher::new();
        let spawned = launcher.spawn(&process).unwrap();
        assert!(test_process_is_alive(spawned.identity.process_id));

        drop(launcher);

        let deadline = Instant::now() + Duration::from_secs(5);
        while test_process_is_alive(spawned.identity.process_id) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(
            !test_process_is_alive(spawned.identity.process_id),
            "spawned service pid {} survived launcher drop",
            spawned.identity.process_id
        );
    }

    #[cfg(windows)]
    fn sleeping_child_command() -> (String, Vec<String>) {
        (
            "powershell.exe".to_string(),
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "Start-Sleep -Seconds 60".to_string(),
            ],
        )
    }

    #[cfg(unix)]
    fn sleeping_child_command() -> (String, Vec<String>) {
        (
            "sh".to_string(),
            vec!["-c".to_string(), "sleep 60".to_string()],
        )
    }

    #[cfg(not(any(windows, unix)))]
    fn sleeping_child_command() -> (String, Vec<String>) {
        ("cargo".to_string(), vec!["--version".to_string()])
    }

    #[cfg(windows)]
    fn test_process_is_alive(pid: u32) -> bool {
        use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return false;
        };
        let mut exit_code = 0;
        let alive = unsafe { GetExitCodeProcess(handle, &raw mut exit_code).is_ok() }
            && exit_code == STILL_ACTIVE.0 as u32;
        let _ = unsafe { CloseHandle(handle) };
        alive
    }

    #[cfg(unix)]
    fn test_process_is_alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    #[cfg(not(any(windows, unix)))]
    fn test_process_is_alive(_pid: u32) -> bool {
        false
    }
}
