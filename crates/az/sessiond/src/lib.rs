use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use az_endpoint_discovery::endpoint_token;
use az_filesystem::AzothDataHome;
use az_observability::{OTEL_EXPORTER_OTLP_ENDPOINT_ENV, ObservedLogContext};
use az_proto_core::{
    Capability, Endpoint, EndpointKind, ProtocolVersion, ServiceDescriptor, ServiceId, ServiceRole,
};
use az_proto_daemon::{
    DAEMON_AUDIENCE, DAEMON_PROJECTS_PERMISSION, DAEMON_READ_PERMISSION,
    DAEMON_SESSIONS_PERMISSION, ListProjectsRequest, ListProjectsResult, ProjectRecord,
    RegisterProjectRootRequest, RegisterSessionSupervisorRequest,
    UnregisterSessionSupervisorRequest, UnregisterSessionSupervisorResult, daemon_capnp,
};
use az_proto_session::SessionSupervisorIdentity;
use az_rpc::AzRpcTransportError;
use az_service_supervision::{
    ProcessIdentity, ServiceLifecycleEvent, ServiceProcessLauncher, StdServiceProcessLauncher,
};
use az_session::{
    ProtocolServiceBoundaryProbe, SESSION_SUPERVISOR_LEASE_HEARTBEAT_INTERVAL_MS,
    ServiceBoundaryProbe, SessionError, SessionManager, SessionManifest, SessionRpcTransportError,
    SessionServiceSupervisor, SessionStatus, SessionStatusPublisher, SessionSupervisorCommand,
    SessionSupervisorCommandResult, SessionSupervisorCommandSender, SessionSupervisorLeaseError,
    SessionSupervisorLeaseRecord, SessionSupervisorLeaseStore, SessionSupervisorRpcServer,
    StartServicesFilter, StartServicesReport, session_supervisor_command_channel,
    session_workspace_status_to_proto, start_session_supervisor_rpc_server_with_command_sender,
};
use crossbeam_channel as channel;
use thiserror::Error;
use tokio::runtime::Builder;
use tokio::task::LocalSet;
use tracing::{info, instrument, warn};
use uuid::Uuid;

/// What az-sessiond does with the session's planned services at startup.
///
/// Replaces a `start_services` flag paired with a name list, which could
/// express the meaningless "do not start, but only these".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStartup {
    /// Leave the planned services alone; only publish current status.
    None,
    /// Start every planned service.
    All,
    /// Start only the planned services with these names.
    Named(Vec<String>),
}

impl ServiceStartup {
    /// `All` and `Named([])` select the same set; an empty name list is how
    /// [`StartServicesFilter`] itself spells "everything".
    fn filter(&self) -> StartServicesFilter {
        match self {
            Self::None | Self::All => StartServicesFilter::all(),
            Self::Named(names) => StartServicesFilter::named(names.clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionDaemonConfig {
    pub project_root: PathBuf,
    pub data_home: AzothDataHome,
    pub session: String,
    pub service_ready_timeout: Duration,
    pub startup: ServiceStartup,
    pub exit_when_services_exit: bool,
    pub stop_services_on_shutdown: bool,
    pub publish_session_supervisor: bool,
    pub daemon_endpoint: Option<Endpoint>,
    pub child_otlp_endpoint: Option<String>,
    pub session_supervisor_endpoint_kind: EndpointKind,
    pub session_supervisor_endpoint: Option<Endpoint>,
    /// Causal shutdown input selected alongside control commands and lifecycle
    /// exits; it is never sampled on a cadence.
    pub shutdown_events: Option<channel::Receiver<String>>,
    /// `UUIDv7` label minted once for this planned session-supervisor launch.
    pub run: Uuid,
}

impl SessionDaemonConfig {
    #[must_use]
    pub fn new(project_root: impl Into<PathBuf>, session: impl Into<String>) -> Self {
        Self {
            project_root: project_root.into(),
            data_home: AzothDataHome::resolve(),
            session: session.into(),
            service_ready_timeout: Duration::from_mins(5),
            startup: ServiceStartup::All,
            exit_when_services_exit: true,
            stop_services_on_shutdown: true,
            publish_session_supervisor: true,
            daemon_endpoint: None,
            child_otlp_endpoint: None,
            session_supervisor_endpoint_kind: default_session_supervisor_endpoint_kind(),
            session_supervisor_endpoint: None,
            shutdown_events: None,
            run: Uuid::now_v7(),
        }
    }
}

#[derive(Debug, Error)]
pub enum SessionDaemonError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Session(#[from] SessionError),

    #[error(transparent)]
    RpcTransport(#[from] SessionRpcTransportError),

    #[error(transparent)]
    SupervisorLease(#[from] SessionSupervisorLeaseError),

    #[error(transparent)]
    DaemonRpcTransport(#[from] AzRpcTransportError),

    #[error("azd protocol error: {0}")]
    DaemonProtocol(#[from] capnp::Error),

    #[error("azd unavailable until restarted: protocol preflight failed: {reason}")]
    DaemonUnavailableUntilRestart { reason: String },

    #[error("invalid child OTLP endpoint: {reason}")]
    InvalidChildOtlpEndpoint { reason: String },

    #[error("service readiness timeout must be positive")]
    InvalidServiceReadyTimeout,

    #[error(
        "{operation} cannot use endpoint kind {kind:?}; use platform IPC or explicit TCP debug endpoints"
    )]
    UnsupportedEndpointKind {
        operation: &'static str,
        kind: EndpointKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDaemonReport {
    pub started: usize,
    pub skipped: usize,
    pub exited: usize,
    pub stopped: usize,
    pub shutdown_requested: bool,
    pub session_supervisor_endpoint: Option<Endpoint>,
}

#[must_use]
pub fn sessiond_observed_log_context(manifest: &SessionManifest, run: Uuid) -> ObservedLogContext {
    ObservedLogContext::new(
        ServiceId::new(
            az_proto_session::SESSION_SUPERVISOR_NAMESPACE,
            az_proto_session::SESSION_SUPERVISOR_SERVICE_NAME,
        ),
        ServiceRole::SessionSupervisor,
        run,
    )
    .with_session_slug(manifest.slug.clone())
}

#[derive(Debug, Clone)]
struct DaemonSessionRegistration {
    project_id: String,
    session_slug: String,
    descriptor: ServiceDescriptor,
    daemon_endpoint: Endpoint,
}

/// The concrete supervisor az-sessiond drives.
type DaemonSupervisor =
    SessionServiceSupervisor<StdServiceProcessLauncher, ProtocolServiceBoundaryProbe>;

/// What one turn of the daemon's event loop decided.
enum DaemonStep {
    /// Keep selecting.
    Continue,
    /// Stop; this is the daemon's answer.
    Finished(SessionDaemonReport),
}

/// Everything publishing the session-supervisor endpoint leaves behind.
struct PublishedSessionSupervisor {
    endpoint: Endpoint,
    server: SessionSupervisorRpcServer,
    lease: ActiveSupervisorLease,
    registration: Option<DaemonSessionRegistration>,
}

/// The daemon's mutable run state: what it has published and what it has
/// counted so far.
#[derive(Default)]
struct SessionDaemonState {
    started: usize,
    skipped: usize,
    exited: usize,
    stopped: usize,
    rpc_server: Option<SessionSupervisorRpcServer>,
    lease: Option<ActiveSupervisorLease>,
    registration: Option<DaemonSessionRegistration>,
    status_publisher: Option<SessionStatusPublisher>,
    supervisor_endpoint: Option<Endpoint>,
}

impl SessionDaemonState {
    fn adopt(&mut self, published: PublishedSessionSupervisor) {
        self.status_publisher = Some(published.server.status_publisher());
        self.supervisor_endpoint = Some(published.endpoint);
        self.lease = Some(published.lease);
        self.registration = published.registration;
        self.rpc_server = Some(published.server);
    }

    /// Retire the published session-supervisor endpoint and, when this daemon
    /// registered with an azd, withdraw that registration.
    fn stop_publication(&mut self, session: &str) {
        stop_session_supervisor_publication(
            session,
            &mut self.rpc_server,
            self.registration.as_ref(),
        );
    }

    /// Retire the published endpoint and answer with what this run did.
    fn finish(&mut self, session: &str, shutdown_requested: bool) -> SessionDaemonReport {
        self.stop_publication(session);
        SessionDaemonReport {
            started: self.started,
            skipped: self.skipped,
            exited: self.exited,
            stopped: self.stopped,
            shutdown_requested,
            session_supervisor_endpoint: self.supervisor_endpoint.clone(),
        }
    }
}

/// Publish the session-supervisor endpoint, take its durable lease, and — when
/// the operator named an azd — register the descriptor there.
fn publish_session_supervisor(
    config: &SessionDaemonConfig,
    supervisor: &DaemonSupervisor,
    command_sender: SessionSupervisorCommandSender,
) -> Result<PublishedSessionSupervisor, SessionDaemonError> {
    let session_manifest = supervisor.manager().session(&config.session)?;
    let endpoint_template = match &config.session_supervisor_endpoint {
        Some(endpoint) => endpoint.clone(),
        None => default_session_supervisor_endpoint(
            &session_manifest,
            config.session_supervisor_endpoint_kind,
        )?,
    };
    validate_public_endpoint_kind(
        endpoint_template.kind,
        "az-sessiond session-supervisor endpoint",
    )?;
    let (descriptor, endpoint, server) = if endpoint_template.kind == EndpointKind::Tcp {
        // A `:0` request only learns its port once the listener is bound, so
        // the descriptor has to be registered after the server starts.
        let server = start_session_supervisor_rpc_server_with_command_sender(
            &config.project_root,
            config.data_home.clone(),
            endpoint_template,
            config.session.clone(),
            command_sender,
        )?;
        let endpoint = server.endpoint().clone();
        let registration = supervisor
            .manager()
            .register_session_supervisor_descriptor(
                &config.session,
                config.run,
                endpoint.clone(),
            )?;
        (registration.descriptor, endpoint, server)
    } else {
        let registration = supervisor
            .manager()
            .register_session_supervisor_descriptor(
                &config.session,
                config.run,
                endpoint_template,
            )?;
        let endpoint = registration.descriptor.endpoint.clone();
        let server = start_session_supervisor_rpc_server_with_command_sender(
            &config.project_root,
            config.data_home.clone(),
            endpoint.clone(),
            config.session.clone(),
            command_sender,
        )?;
        (registration.descriptor, endpoint, server)
    };
    server.set_run(descriptor.run);
    let process = ProcessIdentity::current()?;
    let lease_store = SessionSupervisorLeaseStore::new(&session_manifest.run_dir);
    let lease = lease_store.acquire(&descriptor, process, current_unix_ms())?;
    server.set_supervision_identity(SessionSupervisorIdentity {
        process_id: process.process_id,
        process_start_time: process.process_start_time,
        descriptor: descriptor.clone(),
    })?;
    let lease = ActiveSupervisorLease {
        store: lease_store.clone(),
        record: lease,
        next_heartbeat: Instant::now()
            + Duration::from_millis(SESSION_SUPERVISOR_LEASE_HEARTBEAT_INTERVAL_MS),
    };
    info!(
        session = %config.session,
        run = %descriptor.run,
        process_id = process.process_id,
        process_start_time = process.process_start_time,
        endpoint_kind = ?endpoint.kind,
        endpoint = %endpoint.address,
        lease_path = %lease_store.path().display(),
        "az-sessiond published session-supervisor endpoint and acquired lease"
    );
    let registration = register_session_supervisor_with_azd(config, &descriptor)?;
    Ok(PublishedSessionSupervisor {
        endpoint,
        server,
        lease,
        registration,
    })
}

/// Register `descriptor` with the azd the operator named, if any.
fn register_session_supervisor_with_azd(
    config: &SessionDaemonConfig,
    descriptor: &ServiceDescriptor,
) -> Result<Option<DaemonSessionRegistration>, SessionDaemonError> {
    let Some(daemon_endpoint) = &config.daemon_endpoint else {
        return Ok(None);
    };
    let project_id = register_session_supervisor_with_daemon(
        &config.project_root,
        &config.session,
        descriptor,
        daemon_endpoint,
    )?;
    info!(
        session = %config.session,
        daemon_endpoint_kind = ?daemon_endpoint.kind,
        daemon_endpoint = %daemon_endpoint.address,
        "az-sessiond registered session-supervisor with azd"
    );
    Ok(Some(DaemonSessionRegistration {
        project_id,
        session_slug: config.session.clone(),
        descriptor: descriptor.clone(),
        daemon_endpoint: daemon_endpoint.clone(),
    }))
}

/// Apply the configured startup policy and publish the resulting status.
fn start_configured_services(
    config: &SessionDaemonConfig,
    supervisor: &DaemonSupervisor,
    state: &mut SessionDaemonState,
) -> Result<(), SessionDaemonError> {
    if config.startup == ServiceStartup::None {
        publish_current_session_status(
            supervisor,
            state.status_publisher.as_ref(),
            &config.session,
        )?;
        return Ok(());
    }
    let startup_filter = config.startup.filter();
    let (report, _) = publish_session_mutation_outcome(
        supervisor,
        state.status_publisher.as_ref(),
        &config.session,
        supervisor.start_planned_services_matching(&config.session, &startup_filter),
    )?;
    let counts = log_start_services_report(&config.session, report);
    state.started = counts.started;
    state.skipped = counts.skipped;
    Ok(())
}

/// Stop the session's owned children (when policy asks for it) and answer with
/// the shutdown report.
fn shut_down_session(
    config: &SessionDaemonConfig,
    supervisor: &DaemonSupervisor,
    state: &mut SessionDaemonState,
    reason: &str,
) -> Result<SessionDaemonReport, SessionDaemonError> {
    if config.stop_services_on_shutdown {
        let (report, _) = publish_session_mutation_outcome(
            supervisor,
            state.status_publisher.as_ref(),
            &config.session,
            supervisor.stop_owned_services(&config.session, reason),
        )?;
        state.stopped += report.stopped.len();
    }
    Ok(state.finish(&config.session, true))
}

/// Apply one control command from the session-supervisor RPC surface.
fn handle_supervisor_command(
    config: &SessionDaemonConfig,
    supervisor: &DaemonSupervisor,
    state: &mut SessionDaemonState,
    command: SessionSupervisorCommand,
) -> Result<DaemonStep, SessionDaemonError> {
    match command {
        SessionSupervisorCommand::Start { filter, completion } => {
            let outcome = publish_session_mutation_outcome(
                supervisor,
                state.status_publisher.as_ref(),
                &config.session,
                supervisor.start_planned_services_matching(&config.session, &filter),
            )
            .map(|(report, status)| SessionSupervisorCommandResult::Started { report, status });
            if let Ok(SessionSupervisorCommandResult::Started { report, .. }) = &outcome {
                state.started += report.started.len();
                state.skipped += report.skipped.len();
            }
            let _ = completion.send(outcome);
            Ok(DaemonStep::Continue)
        }
        SessionSupervisorCommand::Stop { reason, completion } => {
            let outcome = publish_session_mutation_outcome(
                supervisor,
                state.status_publisher.as_ref(),
                &config.session,
                supervisor.stop_owned_services(&config.session, &reason),
            )
            .map(|(report, status)| SessionSupervisorCommandResult::Stopped { report, status });
            if let Ok(SessionSupervisorCommandResult::Stopped { report, .. }) = &outcome {
                state.stopped += report.stopped.len();
            }
            let _ = completion.send(outcome);
            Ok(DaemonStep::Continue)
        }
        SessionSupervisorCommand::Shutdown { reason } => {
            info!(session = %config.session, %reason, "az-sessiond shutdown command received");
            shut_down_session(config, supervisor, state, &reason).map(DaemonStep::Finished)
        }
    }
}

/// Apply one observed service lifecycle event.
fn handle_service_lifecycle_event(
    config: &SessionDaemonConfig,
    supervisor: &DaemonSupervisor,
    state: &mut SessionDaemonState,
    event: ServiceLifecycleEvent,
) -> Result<DaemonStep, SessionDaemonError> {
    let service = match supervisor.handle_lifecycle_event(&config.session, event) {
        Ok(service) => service,
        Err(error) => {
            let reason = format!("sessiond lifecycle supervision failed: {error}");
            // A supervision failure is not a requested shutdown. It cannot
            // leave owned rows Running/Starting merely because the caller
            // opted out of ordinary shutdown cleanup.
            if let Err(cleanup_error) = supervisor.stop_owned_services(&config.session, &reason) {
                warn!(session = %config.session, error = %cleanup_error, "sessiond lifecycle failure cleanup also failed");
            }
            if let Err(publication_error) = publish_current_session_status(
                supervisor,
                state.status_publisher.as_ref(),
                &config.session,
            ) {
                warn!(session = %config.session, error = %publication_error, "sessiond could not publish final lifecycle-failure status");
            }
            state.stop_publication(&config.session);
            return Err(error.into());
        }
    };
    let Some(service) = service else {
        return Ok(DaemonStep::Continue);
    };
    state.exited += 1;
    publish_current_session_status(supervisor, state.status_publisher.as_ref(), &config.session)?;
    info!(session = %config.session, service = %service.service_name, exit_code = ?service.exit_code, success = service.success, "az-sessiond observed service exit");
    if config.exit_when_services_exit
        && supervisor
            .running_service_names(&config.session)?
            .is_empty()
    {
        return Ok(DaemonStep::Finished(state.finish(&config.session, false)));
    }
    Ok(DaemonStep::Continue)
}

/// Select over control commands, lifecycle exits, and the shutdown signal until
/// one of them ends the run. Nothing here is sampled on a cadence; the only
/// deadline is the lease heartbeat.
fn supervise_session_events(
    config: &SessionDaemonConfig,
    supervisor: &DaemonSupervisor,
    command_receiver: &channel::Receiver<SessionSupervisorCommand>,
    state: &mut SessionDaemonState,
) -> Result<SessionDaemonReport, SessionDaemonError> {
    let lifecycle_events = supervisor.lifecycle_events();
    let shutdown_events = config.shutdown_events.as_ref();
    loop {
        let mut select = channel::Select::new();
        let command_index = select.recv(command_receiver);
        let lifecycle_index = select.recv(&lifecycle_events);
        let shutdown_index = shutdown_events.map(|events| select.recv(events));
        let operation = match state.lease.as_ref() {
            Some(lease) => {
                if let Ok(operation) = select.select_deadline(lease.next_heartbeat()) {
                    operation
                } else {
                    if let Some(lease) = state.lease.as_mut() {
                        lease.renew()?;
                    }
                    continue;
                }
            }
            None => select.select(),
        };
        let shutdown_source = shutdown_events.filter(|_| shutdown_index == Some(operation.index()));
        let step = if let Some(events) = shutdown_source {
            let reason = operation.recv(events).map_err(|_| {
                SessionDaemonError::Session(SessionError::InvalidSessionCommand {
                    message: "sessiond shutdown signal source disconnected".to_string(),
                })
            })?;
            info!(session = %config.session, %reason, "az-sessiond shutdown signal received");
            DaemonStep::Finished(shut_down_session(config, supervisor, state, &reason)?)
        } else if operation.index() == command_index {
            let command = operation.recv(command_receiver).map_err(|_| {
                SessionDaemonError::Session(SessionError::InvalidSessionCommand {
                    message: "session-supervisor command senders disconnected".to_string(),
                })
            })?;
            handle_supervisor_command(config, supervisor, state, command)?
        } else if operation.index() == lifecycle_index {
            let event = operation.recv(&lifecycle_events).map_err(|_| {
                SessionDaemonError::Session(SessionError::InvalidSessionCommand {
                    message: "service lifecycle hub disconnected".to_string(),
                })
            })?;
            handle_service_lifecycle_event(config, supervisor, state, event)?
        } else {
            DaemonStep::Continue
        };
        if let DaemonStep::Finished(report) = step {
            return Ok(report);
        }
    }
}

/// Run one session's supervisor to completion.
///
/// # Errors
///
/// Returns [`SessionDaemonError::InvalidServiceReadyTimeout`] or
/// [`SessionDaemonError::InvalidChildOtlpEndpoint`] if `config` is
/// self-inconsistent; [`SessionDaemonError::Session`] if the session is not an
/// active workspace, a planned service cannot be started or stopped, or one of
/// the daemon's channels disconnects;
/// [`SessionDaemonError::UnsupportedEndpointKind`] if the requested
/// session-supervisor endpoint is in-process;
/// [`SessionDaemonError::RpcTransport`] if that endpoint cannot be served;
/// [`SessionDaemonError::SupervisorLease`] if the durable supervisor lease
/// cannot be taken or renewed; [`SessionDaemonError::Io`] if this process's own
/// identity cannot be captured; and
/// [`SessionDaemonError::DaemonRpcTransport`],
/// [`SessionDaemonError::DaemonProtocol`], or
/// [`SessionDaemonError::DaemonUnavailableUntilRestart`] if registering with a
/// named azd fails.
#[instrument(
    skip_all,
    fields(session = %config.session, project_root = %config.project_root.display())
)]
pub fn run_session_daemon(
    config: &SessionDaemonConfig,
) -> Result<SessionDaemonReport, SessionDaemonError> {
    let daemon_started = Instant::now();
    let setup_started = Instant::now();
    let supervisor = session_supervisor_for_config(config)?;
    let setup_ms = duration_millis(setup_started.elapsed());
    let mut state = SessionDaemonState::default();
    let (command_sender, command_receiver) = session_supervisor_command_channel();

    info!(
        session = %config.session,
        project_root = %config.project_root.display(),
        service_ready_timeout_ms = duration_millis(config.service_ready_timeout),
        "az-sessiond starting"
    );

    supervisor
        .manager()
        .require_active_session(&config.session, "supervise services")?;

    let publication_started = Instant::now();
    if config.publish_session_supervisor {
        state.adopt(publish_session_supervisor(
            config,
            &supervisor,
            command_sender,
        )?);
    }
    let publication_ms = duration_millis(publication_started.elapsed());

    let service_start_started = Instant::now();
    start_configured_services(config, &supervisor, &mut state)?;
    let service_start_ms = duration_millis(service_start_started.elapsed());
    let total_ms = duration_millis(daemon_started.elapsed());
    info!(
        session = %config.session,
        total_ms,
        setup_ms,
        publication_ms,
        service_start_ms,
        timing_table = %format!(
            "stage                 ms\nsetup+plan        {setup_ms:>6}\nsupervisor publish {publication_ms:>6}\nspawn+ready       {service_start_ms:>6}\ntotal             {total_ms:>6}"
        ),
        "sessiond startup timing summary"
    );

    if config.exit_when_services_exit
        && supervisor
            .running_service_names(&config.session)?
            .is_empty()
    {
        info!(session = %config.session, "az-sessiond exiting; startup completed with no running children");
        return Ok(state.finish(&config.session, false));
    }

    supervise_session_events(config, &supervisor, &command_receiver, &mut state)
}

fn publish_current_session_status<L, P>(
    supervisor: &SessionServiceSupervisor<L, P>,
    publisher: Option<&SessionStatusPublisher>,
    session: &str,
) -> Result<(), SessionError>
where
    L: ServiceProcessLauncher,
    SessionError: From<L::Error>,
    P: ServiceBoundaryProbe,
{
    let Some(publisher) = publisher else {
        return Ok(());
    };
    let status = supervisor.manager().status(session)?;
    publisher.publish(session, session_workspace_status_to_proto(&status))
}

fn publish_session_mutation_outcome<T, L, P>(
    supervisor: &SessionServiceSupervisor<L, P>,
    publisher: Option<&SessionStatusPublisher>,
    session: &str,
    outcome: Result<T, SessionError>,
) -> Result<(T, SessionStatus), SessionError>
where
    L: ServiceProcessLauncher,
    SessionError: From<L::Error>,
    P: ServiceBoundaryProbe,
{
    let status = supervisor.manager().status(session);
    match outcome {
        Ok(value) => {
            let status = status?;
            if let Some(publisher) = publisher {
                publisher.publish(session, session_workspace_status_to_proto(&status))?;
            }
            Ok((value, status))
        }
        Err(primary) => {
            match status {
                Ok(status) => {
                    if let Some(publisher) = publisher
                        && let Err(publication_error) =
                            publisher.publish(session, session_workspace_status_to_proto(&status))
                    {
                        warn!(%session, error = %publication_error, primary = %primary, "session mutation failed and its canonical rollback status could not be published");
                    }
                }
                Err(status_error) => {
                    warn!(%session, error = %status_error, primary = %primary, "session mutation failed and its canonical rollback status could not be read");
                }
            }
            Err(primary)
        }
    }
}

#[derive(Debug)]
struct ActiveSupervisorLease {
    store: SessionSupervisorLeaseStore,
    record: SessionSupervisorLeaseRecord,
    next_heartbeat: Instant,
}

impl ActiveSupervisorLease {
    fn renew(&mut self) -> Result<(), SessionSupervisorLeaseError> {
        self.store.renew(self.record.process, current_unix_ms())?;
        self.next_heartbeat =
            Instant::now() + Duration::from_millis(SESSION_SUPERVISOR_LEASE_HEARTBEAT_INTERVAL_MS);
        Ok(())
    }

    #[must_use]
    const fn next_heartbeat(&self) -> Instant {
        self.next_heartbeat
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, duration_millis)
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn session_supervisor_for_config(
    config: &SessionDaemonConfig,
) -> Result<
    SessionServiceSupervisor<StdServiceProcessLauncher, ProtocolServiceBoundaryProbe>,
    SessionDaemonError,
> {
    if config.service_ready_timeout.is_zero() {
        return Err(SessionDaemonError::InvalidServiceReadyTimeout);
    }
    let launcher = session_service_process_launcher(config)?;
    Ok(SessionServiceSupervisor::with_manager_and_probe(
        SessionManager::with_data_home(&config.project_root, config.data_home.clone())?,
        launcher,
        ProtocolServiceBoundaryProbe::new(),
    )
    .with_ready_timeout(config.service_ready_timeout))
}

fn session_service_process_launcher(
    config: &SessionDaemonConfig,
) -> Result<StdServiceProcessLauncher, SessionDaemonError> {
    let mut launcher = StdServiceProcessLauncher::new();
    if let Some(endpoint) = normalized_child_otlp_endpoint(config)? {
        launcher = launcher.with_environment_var(OTEL_EXPORTER_OTLP_ENDPOINT_ENV, endpoint);
    }
    Ok(launcher)
}

fn normalized_child_otlp_endpoint(
    config: &SessionDaemonConfig,
) -> Result<Option<String>, SessionDaemonError> {
    let Some(endpoint) = config.child_otlp_endpoint.as_deref() else {
        return Ok(None);
    };
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(SessionDaemonError::InvalidChildOtlpEndpoint {
            reason: "endpoint must not be empty".to_string(),
        });
    }
    if endpoint.starts_with('-') {
        return Err(SessionDaemonError::InvalidChildOtlpEndpoint {
            reason: "endpoint must be a value, not another option".to_string(),
        });
    }
    Ok(Some(endpoint.to_string()))
}

#[derive(Debug, Clone, Copy)]
struct StartServiceCounts {
    started: usize,
    skipped: usize,
}

fn log_start_services_report(session: &str, report: StartServicesReport) -> StartServiceCounts {
    let started = report.started.len();
    let skipped = report.skipped.len();
    for service in report.started {
        info!(
            session,
            service = %service.service_name,
            pid = service.identity.process_id,
            "az-sessiond spawned service"
        );
    }
    for service in report.skipped {
        info!(
            session,
            service = %service,
            "az-sessiond skipped service"
        );
    }
    StartServiceCounts { started, skipped }
}

/// Owns the OS signal iterator and its worker thread.
///
/// Dropping the subscription closes the iterator and joins its worker, so a
/// non-signal sessiond shutdown cannot leave a forever-blocked helper behind.
pub struct ShutdownSignalSubscription {
    receiver: channel::Receiver<String>,
    #[cfg(unix)]
    handle: signal_hook::iterator::Handle,
    #[cfg(windows)]
    close: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ShutdownSignalSubscription {
    #[must_use]
    pub fn receiver(&self) -> channel::Receiver<String> {
        self.receiver.clone()
    }

    pub fn close(&mut self) {
        #[cfg(unix)]
        self.handle.close();
        #[cfg(windows)]
        if let Some(close) = self.close.take() {
            let _ = close.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for ShutdownSignalSubscription {
    fn drop(&mut self) {
        self.close();
    }
}

/// Subscribes to the platform's shutdown signals and hands back a channel that
/// carries the first one observed.
///
/// # Errors
///
/// Returns [`std::io::Error`] if the process cannot register a handler for
/// `SIGINT`/`SIGTERM` (Unix) or for the console control events (Windows).
pub fn install_shutdown_signal_channel() -> Result<ShutdownSignalSubscription, std::io::Error> {
    let (sender, receiver) = channel::unbounded();

    #[cfg(unix)]
    {
        use signal_hook::consts::signal::{SIGINT, SIGTERM};
        let mut signals = signal_hook::iterator::Signals::new([SIGINT, SIGTERM])?;
        let handle = signals.handle();
        let worker = thread::spawn(move || {
            if let Some(signal) = signals.forever().next() {
                let _ = sender.send(format!("signal {signal}"));
            }
        });
        return Ok(ShutdownSignalSubscription {
            receiver,
            handle,
            worker: Some(worker),
        });
    }

    #[cfg(windows)]
    {
        let runtime = Builder::new_current_thread().enable_io().build()?;
        let (close, closed) = tokio::sync::oneshot::channel();
        let worker = thread::spawn(move || {
            runtime.block_on(async move {
                tokio::select! {
                    signal = tokio::signal::ctrl_c() => {
                        if signal.is_ok() {
                            let _ = sender.send("signal Ctrl-C".to_string());
                        }
                    }
                    _ = closed => {}
                }
            });
        });
        Ok(ShutdownSignalSubscription {
            receiver,
            close: Some(close),
            worker: Some(worker),
        })
    }

    #[cfg(not(any(unix, windows)))]
    {
        drop(sender);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "sessiond signal subscriptions are unsupported on this platform",
        ))
    }
}

fn register_session_supervisor_with_daemon(
    project_root: &std::path::Path,
    session_slug: &str,
    descriptor: &ServiceDescriptor,
    daemon_endpoint: &Endpoint,
) -> Result<String, SessionDaemonError> {
    let runtime = Builder::new_current_thread().enable_io().build()?;
    let local = LocalSet::new();
    let project_root = project_root.to_string_lossy().into_owned();
    let session_slug = session_slug.to_string();
    let descriptor = descriptor.clone();
    let daemon_endpoint = daemon_endpoint.clone();

    local.block_on(&runtime, async move {
        let client = connect_daemon_rpc_client(&daemon_endpoint, &descriptor).await?;

        let mut register_project = client.register_project_root_request();
        (RegisterProjectRootRequest {
            capability: daemon_capability(&descriptor, DAEMON_PROJECTS_PERMISSION),
            root: project_root,
        })
        .to_capnp(register_project.get().init_request())?;
        let project_response = register_project.send().promise.await?;
        let project = ProjectRecord::from_capnp(project_response.get()?.get_project()?)?;

        let mut register_session = client.register_session_supervisor_request();
        (RegisterSessionSupervisorRequest {
            capability: daemon_capability(&descriptor, DAEMON_SESSIONS_PERMISSION),
            project_id: project.project_id.clone(),
            session_slug,
            descriptor,
        })
        .to_capnp(register_session.get().init_request())?;
        register_session.send().promise.await?;

        Ok::<_, SessionDaemonError>(project.project_id)
    })
}

fn unregister_session_supervisor_with_daemon(
    registration: &DaemonSessionRegistration,
) -> Result<bool, SessionDaemonError> {
    let runtime = Builder::new_current_thread().enable_io().build()?;
    let local = LocalSet::new();
    let registration = registration.clone();

    local.block_on(&runtime, async move {
        let client =
            connect_daemon_rpc_client(&registration.daemon_endpoint, &registration.descriptor)
                .await?;

        let mut request = client.unregister_session_supervisor_request();
        (UnregisterSessionSupervisorRequest {
            capability: daemon_capability(&registration.descriptor, DAEMON_SESSIONS_PERMISSION),
            project_id: registration.project_id,
            session_slug: registration.session_slug,
            descriptor: registration.descriptor,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let result = UnregisterSessionSupervisorResult::from_capnp(response.get()?.get_result()?)?;

        Ok::<_, SessionDaemonError>(result.removed)
    })
}

fn stop_session_supervisor_publication(
    session: &str,
    server: &mut Option<SessionSupervisorRpcServer>,
    daemon_registration: Option<&DaemonSessionRegistration>,
) {
    if let Some(registration) = daemon_registration {
        match unregister_session_supervisor_with_daemon(registration) {
            Ok(true) => {
                info!(session = %session, "az-sessiond unregistered session-supervisor from azd");
            }
            Ok(false) => warn!(
                session = %session,
                "az-sessiond skipped azd session-supervisor unregister; descriptor changed or missing"
            ),
            Err(error) => warn!(
                session = %session,
                error = %error,
                "az-sessiond could not unregister session-supervisor from azd"
            ),
        }
    }

    if let Some(server) = server.take() {
        server.stop();
    }
}

// Cannot be made `Send`: the azd bootstrap capability is a capnp-rpc `Client`,
// which is `!Send` by construction; this future is awaited on a private
// current-thread runtime.
#[allow(clippy::future_not_send)]
async fn connect_daemon_rpc_client(
    endpoint: &Endpoint,
    descriptor: &ServiceDescriptor,
) -> Result<daemon_capnp::az_daemon::Client, SessionDaemonError> {
    let client: daemon_capnp::az_daemon::Client =
        az_rpc::connect_twoparty_bootstrap(endpoint).await?;
    let mut request = client.list_projects_request();
    (ListProjectsRequest {
        capability: daemon_capability(descriptor, DAEMON_READ_PERMISSION),
    })
    .to_capnp(request.get().init_request())
    .map_err(daemon_protocol_preflight_failed)?;
    let response = request
        .send()
        .promise
        .await
        .map_err(daemon_protocol_preflight_failed)?;
    let result = ListProjectsResult::from_capnp(
        response
            .get()
            .map_err(daemon_protocol_preflight_failed)?
            .get_result()
            .map_err(daemon_protocol_preflight_failed)?,
    )
    .map_err(daemon_protocol_preflight_failed)?;
    result
        .protocol_version
        .require(ProtocolVersion::CURRENT)
        .map_err(daemon_protocol_preflight_failed)?;
    Ok(client)
}

fn daemon_protocol_preflight_failed(error: impl std::fmt::Display) -> SessionDaemonError {
    SessionDaemonError::DaemonUnavailableUntilRestart {
        reason: error.to_string(),
    }
}

fn daemon_capability(descriptor: &ServiceDescriptor, permission: &str) -> Capability {
    Capability::new(descriptor.id.clone(), ServiceRole::SessionSupervisor)
        .with_audience(DAEMON_AUDIENCE)
        .with_permissions([permission])
}

#[must_use]
pub const fn default_session_supervisor_endpoint_kind() -> EndpointKind {
    if cfg!(windows) {
        EndpointKind::WindowsNamedPipe
    } else {
        EndpointKind::UnixDomainSocket
    }
}

/// The endpoint az-sessiond publishes for `manifest` when the operator did not
/// name one.
///
/// # Errors
///
/// Returns [`SessionDaemonError::UnsupportedEndpointKind`] if `kind` is
/// [`EndpointKind::InProcess`], and [`SessionDaemonError::Session`] if the
/// unix-socket path cannot be created under the session's short-lived IPC
/// directory.
pub fn default_session_supervisor_endpoint(
    manifest: &SessionManifest,
    kind: EndpointKind,
) -> Result<Endpoint, SessionDaemonError> {
    validate_public_endpoint_kind(kind, "az-sessiond session-supervisor endpoint")?;
    let address = match kind {
        EndpointKind::WindowsNamedPipe => {
            format!(
                r"\\.\pipe\azoth-{}-session-supervisor",
                endpoint_token(&manifest.id.to_string())
            )
        }
        EndpointKind::UnixDomainSocket => az_session::session_ipc_dir(manifest.id)?
            .join("session-supervisor.sock")
            .to_string_lossy()
            .into_owned(),
        EndpointKind::Tcp => "127.0.0.1:0".to_string(),
        EndpointKind::InProcess => unreachable!("validated above"),
    };
    Ok(Endpoint::new(kind, address))
}

const fn validate_public_endpoint_kind(
    kind: EndpointKind,
    operation: &'static str,
) -> Result<(), SessionDaemonError> {
    if matches!(kind, EndpointKind::InProcess) {
        return Err(SessionDaemonError::UnsupportedEndpointKind { operation, kind });
    }

    Ok(())
}

#[cfg(test)]
mod architecture_tests {
    use az_architecture_guard::{
        COMPATIBILITY_FORMAT_DEPENDENCIES, DependencyBoundary, PROJECT_OR_FORMAT_PATH_SEGMENTS,
        PROJECT_OR_GAME_DEPENDENCY_PREFIXES, forbidden_production_dependencies,
    };
    use toml::Value;

    const FORBIDDEN_PRODUCTION_DEPENDENCIES: &[&str] = &[
        "az-asset-processor",
        "az-assetdb",
        "az-daemon",
        "az-editor",
        "az-editor-inspector",
        "az-editor-ui",
        "az-engine",
        "az-framework",
        "az-project",
        "az-project-host",
        "az-runtime-host",
        "bevy",
        "gridmate",
        "lmbr-central",
        "lyshine",
        "sample-plugin",
    ];
    #[test]
    fn production_sessiond_dependencies_stay_protocol_and_session_core_only() {
        let manifest: Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
        let workspace_manifest: Value =
            toml::from_str(include_str!("../../../../Cargo.toml")).unwrap();
        let violations = forbidden_sessiond_dependencies(&manifest, &workspace_manifest);

        assert!(
            violations.is_empty(),
            "az-sessiond production dependencies must stay protocol/session-core only; forbidden direct deps: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn production_sessiond_rejects_forbidden_workspace_aliases() {
        let manifest: Value = toml::from_str(
            r"
            [dependencies]
            daemon-core = { workspace = true }
            project-api = { workspace = true }
            ",
        )
        .unwrap();
        let workspace_manifest: Value = toml::from_str(
            r#"
            [workspace.dependencies]
            daemon-core = { package = "az-daemon", path = "crates/az/daemon" }
            project-api = { package = "sample-plugin", path = "projects/sample/crates/plugin" }
            "#,
        )
        .unwrap();

        assert_eq!(
            forbidden_sessiond_dependencies(&manifest, &workspace_manifest),
            vec![
                "dependencies.daemon-core(workspace package az-daemon)",
                "dependencies.project-api(workspace package sample-plugin)",
            ]
        );
    }

    #[test]
    fn production_sessiond_rejects_forbidden_target_paths_and_path_only_aliases() {
        let manifest: Value = toml::from_str(
            r#"
            [dependencies]
            terrain = { path = "../gems/legacy-terrain" }

            [target.'cfg(windows)'.dependencies]
            legacy-format = { path = "../formats/legacy/cry-chunk-assets" }
            net = { package = "gridmate", path = "../gridmate" }
            "#,
        )
        .unwrap();
        let workspace_manifest: Value = toml::from_str("[workspace.dependencies]").unwrap();

        assert_eq!(
            forbidden_sessiond_dependencies(&manifest, &workspace_manifest),
            vec![
                "dependencies.terrain(path ../gems/legacy-terrain)",
                "target.cfg(windows).dependencies.legacy-format(path ../formats/legacy/cry-chunk-assets)",
                "target.cfg(windows).dependencies.net(package gridmate)",
            ]
        );
    }

    fn forbidden_sessiond_dependencies(
        manifest: &Value,
        workspace_manifest: &Value,
    ) -> Vec<String> {
        let mut exact_names = FORBIDDEN_PRODUCTION_DEPENDENCIES.to_vec();
        exact_names.extend_from_slice(COMPATIBILITY_FORMAT_DEPENDENCIES);
        forbidden_production_dependencies(
            manifest,
            workspace_manifest,
            DependencyBoundary::new(
                &exact_names,
                PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
                PROJECT_OR_FORMAT_PATH_SEGMENTS,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use az_project::{ProjectManifest, write_project_manifest};
    use az_proto_core::{Capability, ServiceId, ServiceRole};
    use az_proto_daemon::{
        DAEMON_AUDIENCE, DAEMON_READ_PERMISSION, ResolveSessionSupervisorRequest,
        SessionSupervisorResult,
    };
    use az_proto_session::{
        SESSION_MANAGE_PERMISSION, SESSION_SUPERVISOR_AUDIENCE, SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME, StartServicesRequest, StartServicesResult,
        StopServicesRequest, StopServicesResult,
    };
    use az_session::{SESSION_MANIFEST_FILE, SessionId, SessionManager};

    use super::*;

    static SESSIOND_RUN_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn sessiond_run_test_guard() -> MutexGuard<'static, ()> {
        SESSIOND_RUN_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn test_manifest(temp: &tempfile::TempDir) -> SessionManifest {
        test_manifest_with_project_id(temp, "local.test")
    }

    fn test_manifest_with_project_id(
        temp: &tempfile::TempDir,
        project_id: &str,
    ) -> SessionManifest {
        let id = SessionId::new();
        let run_dir = AzothDataHome::new(temp.path())
            .project("Sessiond Test", temp.path())
            .sessions_dir()
            .join(id.to_string());
        SessionManifest::new(
            id,
            project_id.to_string(),
            "editor".to_string(),
            temp.path().to_path_buf(),
            temp.path().to_path_buf(),
            run_dir,
            0,
        )
    }

    fn write_test_project_manifest(temp: &tempfile::TempDir, project_id: &str) {
        write_project_manifest(
            temp.path(),
            &ProjectManifest::new(project_id, "Sessiond Test", "0.1.0"),
        )
        .unwrap();
        az_project::refresh_project_lock(temp.path()).unwrap();
    }

    #[test]
    fn config_defaults_to_start_and_exit_when_services_exit() {
        let temp = tempfile::tempdir().unwrap();
        let config = SessionDaemonConfig::new(temp.path(), "editor");

        assert_eq!(config.project_root, temp.path());
        assert_eq!(config.session, "editor");
        assert_eq!(config.service_ready_timeout, Duration::from_mins(5));
        assert_eq!(config.startup, ServiceStartup::All);
        assert!(config.exit_when_services_exit);
        assert!(config.stop_services_on_shutdown);
        assert!(config.publish_session_supervisor);
        assert_eq!(config.daemon_endpoint, None);
        assert_eq!(config.child_otlp_endpoint, None);
        assert_eq!(
            config.session_supervisor_endpoint_kind,
            default_session_supervisor_endpoint_kind()
        );
        assert_eq!(config.session_supervisor_endpoint, None);
        assert!(config.shutdown_events.is_none());
    }

    #[test]
    fn config_child_otlp_endpoint_becomes_supervised_service_environment() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = SessionDaemonConfig::new(temp.path(), "editor");
        config.child_otlp_endpoint = Some(" http://127.0.0.1:4317 ".to_string());

        let launcher = session_service_process_launcher(&config).unwrap();

        assert_eq!(
            launcher
                .environment()
                .get(OTEL_EXPORTER_OTLP_ENDPOINT_ENV)
                .map(String::as_str),
            Some("http://127.0.0.1:4317")
        );
    }

    #[test]
    fn config_child_otlp_endpoint_rejects_empty_or_option_like_values() {
        for endpoint in [" ", "--daemon-endpoint"] {
            let temp = tempfile::tempdir().unwrap();
            let mut config = SessionDaemonConfig::new(temp.path(), "editor");
            config.child_otlp_endpoint = Some(endpoint.to_string());

            let error = session_service_process_launcher(&config).unwrap_err();

            assert!(matches!(
                error,
                SessionDaemonError::InvalidChildOtlpEndpoint { .. }
            ));
        }
    }

    #[test]
    fn default_endpoint_uses_short_session_runtime_directory_for_unix_sockets() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = test_manifest(&temp);

        let endpoint =
            default_session_supervisor_endpoint(&manifest, EndpointKind::UnixDomainSocket).unwrap();
        let expected = az_session::session_ipc_dir(manifest.id)
            .unwrap()
            .join("session-supervisor.sock");

        assert_eq!(endpoint.kind, EndpointKind::UnixDomainSocket);
        assert_eq!(std::path::Path::new(&endpoint.address), expected);
        assert!(!std::path::Path::new(&endpoint.address).starts_with(&manifest.run_dir));
    }

    #[test]
    fn default_endpoint_rejects_in_process_kind() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = test_manifest(&temp);

        let error =
            default_session_supervisor_endpoint(&manifest, EndpointKind::InProcess).unwrap_err();

        assert!(matches!(
            error,
            SessionDaemonError::UnsupportedEndpointKind {
                operation: "az-sessiond session-supervisor endpoint",
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn structured_log_path_lives_under_session_logs_dir() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = test_manifest(&temp);

        assert_eq!(
            az_session::sessiond_structured_log_path(&manifest),
            manifest.run_dir.join("logs").join("az-sessiond.capnp.log")
        );
    }

    #[test]
    fn observed_log_context_uses_the_planned_session_supervisor_run() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = test_manifest(&temp);
        let run = Uuid::now_v7();

        let context = sessiond_observed_log_context(&manifest, run);

        assert_eq!(
            context.service,
            ServiceId::new(
                az_proto_session::SESSION_SUPERVISOR_NAMESPACE,
                az_proto_session::SESSION_SUPERVISOR_SERVICE_NAME,
            )
        );
        assert_eq!(context.role, ServiceRole::SessionSupervisor);
        assert_eq!(context.run, run);
        assert_eq!(context.session_slug, "editor");
    }

    #[test]
    fn run_publishes_session_supervisor_descriptor() {
        let _guard = sessiond_run_test_guard();
        let temp = tempfile::tempdir().unwrap();
        write_test_project_manifest(&temp, "local.test");
        let mut manifest = test_manifest(&temp);
        let run_dir = manifest.run_dir.clone();
        std::fs::create_dir_all(&run_dir).unwrap();

        manifest.activate(0);
        std::fs::write(
            run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let mut config = SessionDaemonConfig::new(temp.path(), "editor");
        config.data_home = AzothDataHome::new(temp.path());
        config.startup = ServiceStartup::None;
        config.session_supervisor_endpoint_kind = EndpointKind::Tcp;

        let report = run_session_daemon(&config).unwrap();

        let endpoint = report.session_supervisor_endpoint.unwrap();
        assert_eq!(report.stopped, 0);
        assert!(!report.shutdown_requested);
        assert_eq!(endpoint.kind, EndpointKind::Tcp);
        assert!(endpoint.address.starts_with("127.0.0.1:"));
        assert_ne!(endpoint.address, "127.0.0.1:0");
        let manager =
            SessionManager::with_data_home(temp.path(), AzothDataHome::new(temp.path())).unwrap();
        let descriptor = manager
            .service_descriptor(
                "editor",
                &ServiceId::new(
                    SESSION_SUPERVISOR_NAMESPACE,
                    SESSION_SUPERVISOR_SERVICE_NAME,
                ),
                ServiceRole::SessionSupervisor,
            )
            .unwrap()
            .unwrap();
        assert_eq!(descriptor.endpoint, endpoint);
        assert_eq!(descriptor.run, config.run);
        assert_eq!(descriptor.run.get_version_num(), 7);
        assert_eq!(descriptor.capabilities.len(), 2);
    }

    #[test]
    fn run_rejects_in_process_session_supervisor_endpoint() {
        let _guard = sessiond_run_test_guard();
        let temp = tempfile::tempdir().unwrap();
        write_test_project_manifest(&temp, "local.test");
        let mut manifest = test_manifest(&temp);
        let run_dir = manifest.run_dir.clone();
        std::fs::create_dir_all(&run_dir).unwrap();

        manifest.activate(0);
        std::fs::write(
            run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let mut config = SessionDaemonConfig::new(temp.path(), "editor");
        config.data_home = AzothDataHome::new(temp.path());
        config.startup = ServiceStartup::None;
        config.session_supervisor_endpoint =
            Some(Endpoint::in_process("session-supervisor:editor"));

        let error = run_session_daemon(&config).unwrap_err();

        assert!(matches!(
            error,
            SessionDaemonError::UnsupportedEndpointKind {
                operation: "az-sessiond session-supervisor endpoint",
                kind: EndpointKind::InProcess
            }
        ));

        let manager =
            SessionManager::with_data_home(temp.path(), AzothDataHome::new(temp.path())).unwrap();
        assert!(
            manager
                .service_descriptor(
                    "editor",
                    &ServiceId::new(
                        SESSION_SUPERVISOR_NAMESPACE,
                        SESSION_SUPERVISOR_SERVICE_NAME,
                    ),
                    ServiceRole::SessionSupervisor,
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn run_registers_and_unregisters_session_supervisor_with_azd() {
        let _guard = sessiond_run_test_guard();

        let temp = tempfile::tempdir().unwrap();
        write_test_project_manifest(&temp, "local.sessiond_daemon");
        let mut manifest = test_manifest_with_project_id(&temp, "local.sessiond_daemon");
        let run_dir = manifest.run_dir.clone();
        std::fs::create_dir_all(&run_dir).unwrap();

        manifest.activate(0);
        std::fs::write(
            run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let daemon_server =
            az_daemon::start_az_daemon_rpc_server(Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"))
                .unwrap();
        let mut config = SessionDaemonConfig::new(temp.path(), "editor");
        config.data_home = AzothDataHome::new(temp.path());
        config.startup = ServiceStartup::None;
        config.exit_when_services_exit = false;
        config.session_supervisor_endpoint_kind = EndpointKind::Tcp;
        config.daemon_endpoint = Some(daemon_server.endpoint().clone());
        let (shutdown_sender, shutdown_events) = channel::bounded(1);
        config.shutdown_events = Some(shutdown_events);

        let daemon_config = config;
        let handle = std::thread::spawn(move || run_session_daemon(&daemon_config).unwrap());
        let runtime = Builder::new_current_thread().enable_io().build().unwrap();
        let local = LocalSet::new();
        let endpoint = {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let descriptor = local.block_on(&runtime, async {
                    let client = az_daemon::connect_az_daemon_rpc_client(daemon_server.endpoint())
                        .await
                        .unwrap();
                    let mut request = client.resolve_session_supervisor_request();
                    (ResolveSessionSupervisorRequest {
                        capability: daemon_read_capability(),
                        project_id: "local.sessiond_daemon".to_string(),
                        session_slug: "editor".to_string(),
                    })
                    .to_capnp(request.get().init_request())
                    .unwrap();
                    let response = request.send().promise.await.unwrap();
                    SessionSupervisorResult::from_capnp(
                        response.get().unwrap().get_result().unwrap(),
                    )
                    .unwrap()
                    .descriptor
                });
                if let Some(descriptor) = descriptor {
                    assert_eq!(descriptor.role, ServiceRole::SessionSupervisor);
                    break descriptor.endpoint;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for azd session-supervisor registration"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };

        shutdown_sender.send("test completed".to_string()).unwrap();
        let report = handle.join().unwrap();
        assert!(report.shutdown_requested);
        assert_eq!(report.session_supervisor_endpoint, Some(endpoint));

        local.block_on(&runtime, async {
            let client = az_daemon::connect_az_daemon_rpc_client(daemon_server.endpoint())
                .await
                .unwrap();
            let mut request = client.resolve_session_supervisor_request();
            (ResolveSessionSupervisorRequest {
                capability: daemon_read_capability(),
                project_id: "local.sessiond_daemon".to_string(),
                session_slug: "editor".to_string(),
            })
            .to_capnp(request.get().init_request())
            .unwrap();
            let response = request.send().promise.await.unwrap();
            eprintln!("stop response received");
            let result =
                SessionSupervisorResult::from_capnp(response.get().unwrap().get_result().unwrap())
                    .unwrap();
            assert_eq!(result.descriptor, None);
        });

        daemon_server.stop();
    }

    #[test]
    fn run_exits_on_causal_shutdown_event_even_when_kept_alive() {
        let _guard = sessiond_run_test_guard();

        let temp = tempfile::tempdir().unwrap();
        write_test_project_manifest(&temp, "local.test");
        let mut manifest = test_manifest(&temp);
        let run_dir = manifest.run_dir.clone();
        std::fs::create_dir_all(&run_dir).unwrap();

        manifest.activate(0);
        std::fs::write(
            run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let (shutdown_sender, shutdown_events) = channel::bounded(1);
        shutdown_sender
            .send("test requested shutdown".to_string())
            .unwrap();
        let mut config = SessionDaemonConfig::new(temp.path(), "editor");
        config.data_home = AzothDataHome::new(temp.path());
        config.startup = ServiceStartup::None;
        config.exit_when_services_exit = false;
        config.publish_session_supervisor = false;
        config.shutdown_events = Some(shutdown_events);

        let report = run_session_daemon(&config).unwrap();

        assert!(report.shutdown_requested);
        assert_eq!(report.started, 0);
        assert_eq!(report.stopped, 0);
        assert_eq!(report.session_supervisor_endpoint, None);
    }

    #[test]
    fn stop_services_rpc_returns_terminal_status_then_shutdown_is_one_way() {
        use std::time::Instant;

        let _guard = sessiond_run_test_guard();

        let temp = tempfile::tempdir().unwrap();
        write_test_project_manifest(&temp, "local.test");
        let mut manifest = test_manifest(&temp);
        let run_dir = manifest.run_dir.clone();
        std::fs::create_dir_all(&run_dir).unwrap();

        manifest.activate(0);
        std::fs::write(
            run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let mut config = SessionDaemonConfig::new(temp.path(), "editor");
        config.data_home = AzothDataHome::new(temp.path());
        config.startup = ServiceStartup::None;
        config.exit_when_services_exit = false;
        config.session_supervisor_endpoint_kind = EndpointKind::Tcp;

        let manager =
            SessionManager::with_data_home(temp.path(), AzothDataHome::new(temp.path())).unwrap();
        manager.session("editor").unwrap();

        let daemon_config = config;
        let handle = std::thread::spawn(move || run_session_daemon(&daemon_config).unwrap());

        let descriptor = {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(descriptor) = manager
                    .service_descriptor(
                        "editor",
                        &ServiceId::new(
                            SESSION_SUPERVISOR_NAMESPACE,
                            SESSION_SUPERVISOR_SERVICE_NAME,
                        ),
                        ServiceRole::SessionSupervisor,
                    )
                    .unwrap()
                {
                    break descriptor;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for session-supervisor descriptor"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };

        let runtime = Builder::new_current_thread().enable_io().build().unwrap();
        let local = LocalSet::new();
        local.block_on(&runtime, async {
            let connection: az_rpc::ScopedTwopartyClient<
                az_proto_session::session_capnp::session_supervisor::Client,
            > = az_rpc::connect_twoparty_bootstrap_scoped(&descriptor.endpoint)
                .await
                .unwrap();
            let client = connection.client();
            let mut request = client.stop_services_request();
            (StopServicesRequest {
                capability: session_manage_capability(&descriptor, manifest.id),
                slug: "editor".to_string(),
                reason: "test requested shutdown".to_string(),
            })
            .to_capnp(request.get().init_request())
            .unwrap();
            let response = request.send().promise.await.unwrap();
            let result =
                StopServicesResult::from_capnp(response.get().unwrap().get_result().unwrap())
                    .unwrap();
            assert_eq!(result.status.manifest.slug, "editor");
            assert!(result.stopped.is_empty());

            let mut shutdown = client.shutdown_supervisor_request();
            let mut params = shutdown.get();
            params.set_slug("editor");
            params.set_reason("test requested shutdown");
            session_manage_capability(&descriptor, manifest.id)
                .to_capnp(params.init_capability())
                .unwrap();
            // Fire and forget: the test only needs the call delivered.
            drop(shutdown.send());
            connection.disconnect().await.unwrap();
        });

        let report = handle.join().unwrap();
        assert!(report.shutdown_requested);
        assert_eq!(report.stopped, 0);
    }

    #[test]
    fn start_services_rpc_requests_daemon_start_loop() {
        use std::time::Instant;

        let _guard = sessiond_run_test_guard();

        let temp = tempfile::tempdir().unwrap();
        write_test_project_manifest(&temp, "local.test");
        let mut manifest = test_manifest(&temp);
        let run_dir = manifest.run_dir.clone();
        std::fs::create_dir_all(&run_dir).unwrap();

        manifest.activate(0);
        std::fs::write(
            run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let mut config = SessionDaemonConfig::new(temp.path(), "editor");
        config.data_home = AzothDataHome::new(temp.path());
        config.startup = ServiceStartup::None;
        config.exit_when_services_exit = false;
        config.session_supervisor_endpoint_kind = EndpointKind::Tcp;

        let manager =
            SessionManager::with_data_home(temp.path(), AzothDataHome::new(temp.path())).unwrap();
        manager.session("editor").unwrap();

        let daemon_config = config;
        let handle = std::thread::spawn(move || run_session_daemon(&daemon_config).unwrap());

        let descriptor = {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(descriptor) = manager
                    .service_descriptor(
                        "editor",
                        &ServiceId::new(
                            SESSION_SUPERVISOR_NAMESPACE,
                            SESSION_SUPERVISOR_SERVICE_NAME,
                        ),
                        ServiceRole::SessionSupervisor,
                    )
                    .unwrap()
                {
                    break descriptor;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for session-supervisor descriptor"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };

        let runtime = Builder::new_current_thread().enable_io().build().unwrap();
        let local = LocalSet::new();
        local.block_on(&runtime, async {
            let connection: az_rpc::ScopedTwopartyClient<
                az_proto_session::session_capnp::session_supervisor::Client,
            > = az_rpc::connect_twoparty_bootstrap_scoped(&descriptor.endpoint)
                .await
                .unwrap();
            let client = connection.client();
            let mut request = client.start_services_request();
            (StartServicesRequest {
                capability: session_manage_capability(&descriptor, manifest.id),
                slug: "editor".to_string(),
                reason: "test requested start".to_string(),
                service_names: Vec::new(),
            })
            .to_capnp(request.get().init_request())
            .unwrap();
            let response = request.send().promise.await.unwrap();
            let result =
                StartServicesResult::from_capnp(response.get().unwrap().get_result().unwrap())
                    .unwrap();
            assert_eq!(result.status.manifest.slug, "editor");
            assert!(result.started.is_empty());

            let mut shutdown = client.shutdown_supervisor_request();
            let mut params = shutdown.get();
            params.set_slug("editor");
            params.set_reason("test completed");
            session_manage_capability(&descriptor, manifest.id)
                .to_capnp(params.init_capability())
                .unwrap();
            // Fire and forget: the test only needs the call delivered.
            drop(shutdown.send());
            connection.disconnect().await.unwrap();
        });

        let report = handle.join().unwrap();
        assert!(report.shutdown_requested);
    }

    fn daemon_read_capability() -> Capability {
        Capability::new(
            ServiceId::new("azoth", "sessiond-test"),
            ServiceRole::Editor,
        )
        .with_audience(DAEMON_AUDIENCE)
        .with_permissions([DAEMON_READ_PERMISSION])
    }

    fn session_manage_capability(
        descriptor: &ServiceDescriptor,
        session: az_session::SessionId,
    ) -> Capability {
        descriptor
            .brokered_capability_template(
                ServiceRole::Editor,
                SESSION_SUPERVISOR_AUDIENCE,
                &[SESSION_MANAGE_PERMISSION],
                Some(session.0),
            )
            .unwrap()
    }
}
