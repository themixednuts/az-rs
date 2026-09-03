use crate::ServiceLogStreamArg;
use crate::commands::log_follow::FileChangeEvents;
use crate::error::{
    CliError, CliResult, CommandFailedDetails, InvalidServiceLogPathDetails,
    MissingServiceCapabilityDetails, MissingServiceProcessDetails, SessionServiceNotRunningDetails,
};
use az_observability::{
    ObservedLogFileError, format_log_record_for_console, read_observed_log_file_from_offset,
};
use az_proto_asset::{
    ASSET_PROCESSOR_AUDIENCE, ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME,
    ASSET_READ_PERMISSION, ASSET_WRITE_PERMISSION, AssetBuilderCatalogRequest,
    AssetBuilderCatalogResult, AssetBuilderDescriptor, AssetBuilderPatternDescriptor,
    AssetBuilderPatternKind, AssetProcessingStatusRequest, AssetProcessingStatusResult,
    AssetRootScope, AttemptStatus, CatalogProductEntry, CatalogProductsRequest,
    CatalogProductsResult, ForceReprocessAssetRequest, ForceReprocessAssetResult,
    InspectJobRequest, InspectJobResult, InspectJobSelector, JobActivity, JobDependencyKind,
    JobInspection, JobOwner, JobStatus, PROJECT_SOURCE_ROOT, ProductFormatDescriptor,
    PublishAssetCatalogRequest, PublishAssetCatalogResult, ReconcileAssetSourcesRequest,
    ReconcileAssetSourcesResult, SourceFileCreateContent, SourceFileCreateRequest,
    SourceFileCreateResult, SourceFileWorkflowDescriptor, SourceSchemaAuthoring,
    SourceSchemaDescriptor, WorkspaceEntry, WorkspaceEntryDiff as ProtoWorkspaceEntryDiff,
    WorkspaceEntryPageRequest, WorkspaceEntryPageResult, WorkspaceRoot, WorkspaceSnapshot,
    WorkspaceSnapshotRequest, WorkspaceSnapshotResult, asset_capnp,
};
use az_proto_core::{
    Capability, Endpoint, EndpointKind, ProtocolVersion, ServiceDescriptor, ServiceHealth,
    ServiceId, ServiceRole, SideChannelHandle, SideChannelKind,
    validate_side_channel_capability_matches, validated_mmap_file_path,
    validated_staging_file_path, write_content_addressed_staging_file,
};
use az_proto_daemon::{
    DAEMON_PROJECTS_PERMISSION, DAEMON_READ_PERMISSION, DAEMON_SESSIONS_PERMISSION,
    EnsureProjectSessionServicesRequest, ListSessionSupervisorsRequest,
    ListSessionSupervisorsResult, PrepareProjectSessionServicesRequest, ProjectRecord,
    ProjectSessionServicesResult, ProjectSessionServicesStartResult, RegisterProjectRootRequest,
    ResolveSessionSupervisorRequest, SessionSupervisorResult, daemon_capnp,
};
#[cfg(test)]
use az_proto_daemon::{PlanProjectServicesRequest, ProjectServicePlan};
use az_proto_project::{
    FromCapnp as _, PROJECT_DOCUMENT_READ_PERMISSION, PROJECT_DOCUMENT_WRITE_PERMISSION,
    PROJECT_HOST_AUDIENCE, PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME,
    PROJECT_RUNTIME_LAUNCH_PERMISSION, ProjectSideChannelResult, RuntimeLaunchSnapshotRequest,
    ToCapnp as _, project_capnp,
};
use az_proto_runtime::{
    LaunchRuntimeRequest, RUNTIME_CONTROL_PERMISSION, RUNTIME_HOST_AUDIENCE,
    RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME, RUNTIME_READ_PERMISSION,
    RuntimeAssetPackageRoot, RuntimeAssetSourceRoot, RuntimeProjectionCatalogRequest,
    RuntimeProjectionCatalogResult, RuntimeProjectionDescriptor, RuntimeRole, RuntimeState,
    RuntimeStatus, RuntimeStatusRequest, RuntimeStatusResult, RuntimeViewportFrame,
    RuntimeViewportRequest, RuntimeViewportResult, StopRuntimeRequest, ViewportPixelFormat,
    runtime_capnp,
};
use az_proto_session::{
    ExecCommandRequest as ProtoExecCommandRequest, ExecCommandResult as ProtoExecCommandResult,
    RecoverSessionRequest as ProtoRecoverSessionRequest,
    RegisterServiceRequest as ProtoRegisterServiceRequest,
    ResolveServiceRequest as ProtoResolveServiceRequest, RuntimeAssetPackageRootsRequest,
    RuntimeAssetPackageRootsResult, SESSION_EXEC_PERMISSION, SESSION_MANAGE_PERMISSION,
    SESSION_READ_PERMISSION, SESSION_SUPERVISOR_AUDIENCE, SESSION_SUPERVISOR_NAMESPACE,
    SESSION_SUPERVISOR_SERVICE_NAME, ServiceLogRequest as ProtoServiceLogRequest,
    ServiceLogResult as ProtoServiceLogResult, ServiceLogStream as ProtoServiceLogStream,
    ServiceProcessRecord as ProtoServiceProcessRecord,
    ServiceProcessState as ProtoServiceProcessState,
    SessionCapabilityRequest as ProtoSessionCapabilityRequest,
    SessionManifest as ProtoSessionManifest, SessionSlugRequest as ProtoSessionSlugRequest,
    SessionState as ProtoSessionState, SessionWorkspaceStatus as ProtoSessionWorkspaceStatus,
    StartServicesRequest, StartServicesResult, StopServicesRequest, StopServicesResult,
    session_capnp,
};
use az_service_catalog::runtime_host_service_descriptor;
#[cfg(test)]
use az_service_supervision::ProcessIdentity;
use az_service_supervision::{
    RecordedServiceProcessCleanup, ServiceProcessKey, ServiceProcessRecord, ServiceProcessState,
    ServiceRecord, SupervisedServiceRole, previous_log_path, terminate_recorded_service_process,
};
use az_session::{
    CreateSessionRequest, SessionError, SessionManager, SessionManifest, SessionState,
};
use std::collections::{BTreeSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio::runtime::Builder;
use tokio::runtime::Runtime;
use tokio::task::LocalSet;
use tracing::{info, instrument};
use uuid::Uuid;

const SESSION_EXEC_OUTPUT_LIMIT_BYTES: u32 = 1024 * 1024;
const RUNTIME_HOST_START_TIMEOUT_MS: u64 = 30_000;
const SESSION_LIST_OPERATION: &str = "session list";
const BUILD_ASSET_SESSION: &str = "build";
const BUILD_ASSET_PROCESSING_TIMEOUT: Duration = Duration::from_mins(30);
const SWEEP_REASON: &str = "azoth session sweep";
const SWEEP_POLL_INTERVAL: Duration = Duration::from_millis(250);

struct NoopProjectOpenProgressSink;

impl daemon_capnp::project_open_progress_sink::Server for NoopProjectOpenProgressSink {
    fn update(
        self: capnp::capability::Rc<Self>,
        _params: daemon_capnp::project_open_progress_sink::UpdateParams,
        _results: daemon_capnp::project_open_progress_sink::UpdateResults,
    ) -> impl std::future::Future<Output = Result<(), capnp::Error>> + 'static {
        std::future::ready(Ok(()))
    }
}

fn session_cli_rpc_runtime() -> CliResult<Runtime> {
    Ok(Builder::new_current_thread().enable_all().build()?)
}

pub fn create(name: String, path: Option<PathBuf>) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!("Creating session: {} in {}", name, project_path.display());

    let manager = SessionManager::new(project_path)?;
    let manifest = manager.create_session(CreateSessionRequest { name })?;

    println!("Session '{}' created", manifest.slug);
    print_manifest_summary(&manifest);

    Ok(())
}

pub fn list(
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!("Listing sessions for {}", project_path.display());

    if let Some(daemon_endpoint) = daemon_endpoint.as_ref() {
        match request_session_list_through_daemon(&project_path, daemon_endpoint) {
            Ok(Some(sessions)) => {
                print_proto_session_list(&sessions);
                return Ok(());
            }
            Ok(None) if can_fallback_to_local_session(Some(daemon_endpoint)) => {
                info!(
                    project_root = %project_path.display(),
                    "no live session supervisors registered with azd; falling back to local session metadata"
                );
            }
            Ok(None) => {
                println!("No sessions found");
                return Ok(());
            }
            Err(error) if can_fallback_after_supervisor_error(Some(daemon_endpoint), &error) => {
                info!(
                    error = %error,
                    project_root = %project_path.display(),
                    "daemon-backed session list failed; falling back to local session metadata"
                );
            }
            Err(error) => return Err(error),
        }
    }

    let manager = SessionManager::new(&project_path)?;
    let sessions = manager.list_sessions()?;

    if sessions.is_empty() {
        println!("No sessions found");
        return Ok(());
    }

    print_local_session_list(&sessions);
    Ok(())
}

fn print_local_session_list(sessions: &[SessionManifest]) {
    println!("Sessions:");
    for session in sessions {
        println!(
            "  {:<20} {:<16} {}",
            session.slug,
            format_state(session.state),
            session.workspace_root.display()
        );
    }
}

fn print_proto_session_list(sessions: &[ProtoSessionManifest]) {
    if sessions.is_empty() {
        println!("No sessions found");
        return;
    }

    println!("Sessions:");
    for session in sessions {
        println!(
            "  {:<20} {:<16} {}",
            session.slug,
            format_proto_state(session.state),
            session.workspace_root
        );
    }
}

pub fn open(
    name: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!("Opening session: {} from {}", name, project_path.display());

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    match session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref()) {
        Ok(descriptor) => match request_session_status(&manifest, &descriptor) {
            Ok(status) => {
                println!("Session '{}'", status.manifest.slug);
                print_proto_manifest_summary(&status.manifest);
                println!("Use this workspace path when attaching az-editor or project tools.");
                return Ok(());
            }
            Err(error) if can_fallback_after_supervisor_error(daemon_endpoint.as_ref(), &error) => {
                info!(
                    error = %error,
                    session = %manifest.slug,
                    "session-supervisor open status failed; falling back to local session metadata"
                );
            }
            Err(error) => return Err(error),
        },
        Err(CliError::MissingSessionService { .. })
            if can_fallback_to_local_session(daemon_endpoint.as_ref()) => {}
        Err(error) => return Err(error),
    }

    println!("Session '{}'", manifest.slug);
    print_manifest_summary(&manifest);
    println!("Use this workspace path when attaching az-editor or project tools.");

    Ok(())
}

pub fn exec(
    name: &str,
    command: Vec<String>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let (program, args) = split_exec_command(command)?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Running session command '{}' in '{}' at {}",
        program,
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;
    let result = request_session_exec_command(&manifest, &descriptor, &program, &args)?;
    print_session_exec_result(&result);
    if result.success {
        Ok(())
    } else {
        Err(CliError::CommandFailed(Box::new(CommandFailedDetails {
            program,
            args,
            cwd: manifest.workspace_root,
            status: result.exited.then_some(result.exit_code),
        })))
    }
}

pub fn status(
    name: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!("Session status: {} in {}", name, project_path.display());

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    match session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref()) {
        Ok(descriptor) => match request_session_status(&manifest, &descriptor) {
            Ok(status) => {
                print_proto_session_workspace_status(&status);
                return Ok(());
            }
            Err(error) if can_fallback_after_supervisor_error(daemon_endpoint.as_ref(), &error) => {
                info!(
                    error = %error,
                    session = %manifest.slug,
                    "session-supervisor status failed; falling back to local session metadata"
                );
            }
            Err(error) => return Err(error),
        },
        Err(CliError::MissingSessionService { .. })
            if can_fallback_to_local_session(daemon_endpoint.as_ref()) => {}
        Err(error) => return Err(error),
    }

    let status = manager.status(name)?;
    print_local_session_workspace_status(&status);

    Ok(())
}

pub fn validate(name: &str, path: Option<PathBuf>) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!(
        "Validating workspace referenced by session: {} in {}",
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.validate_session_workspace(name)?;

    println!("Session '{}' workspace is valid", manifest.slug);
    print_manifest_summary(&manifest);
    Ok(())
}

fn print_local_session_workspace_status(status: &az_session::SessionStatus) {
    println!("Session '{}' status", status.manifest.slug);
    print_manifest_summary(&status.manifest);
    if let Some(reason) = &status.failure_reason {
        println!("Failure: {reason}");
    }
}

pub fn retire(
    name: &str,
    stop_services: bool,
    shutdown_timeout_ms: u64,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!("Retiring session: {} from {}", name, project_path.display());

    let manifest = run_session_lifecycle_operation(
        &project_path,
        name,
        "retire",
        stop_services,
        shutdown_timeout_ms,
        daemon_endpoint_kind,
        daemon_endpoint,
        |manager| manager.retire_session(name),
    )?;

    println!("Session '{}' retired", manifest.slug);
    print_manifest_summary(&manifest);

    Ok(())
}

/// Everything one `azoth session services prepare` was asked for: which session in which
/// project directory, what endpoint kind and OTLP endpoint its services should be planned
/// with, whether a failed session may be recovered first, how to reach azd, and which
/// services to prepare (empty selects every service). The set is parsed as a unit and
/// forwarded unchanged into azd's `prepareProjectSessionServices` call, so it travels as one
/// request rather than as eight positional arguments.
#[derive(Debug)]
pub struct PrepareServicesOptions {
    pub name: String,
    pub kind: Option<EndpointKind>,
    pub recover: bool,
    pub otlp_endpoint: Option<String>,
    pub daemon_endpoint_kind: Option<EndpointKind>,
    pub daemon_endpoint: Option<String>,
    pub path: Option<PathBuf>,
    pub service_names: Vec<String>,
}

pub fn prepare_services(options: PrepareServicesOptions) -> CliResult<()> {
    prepare_selected_services_with_build_policy(options, false)
}

fn prepare_selected_services_with_build_policy(
    options: PrepareServicesOptions,
    skip_build: bool,
) -> CliResult<()> {
    let PrepareServicesOptions {
        name,
        kind,
        recover,
        otlp_endpoint,
        daemon_endpoint_kind,
        daemon_endpoint,
        path,
        service_names,
    } = options;
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let endpoint_kind = kind.unwrap_or_else(default_service_endpoint_kind);
    validate_public_endpoint_kind(endpoint_kind, "session service planning")?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Preparing services for session '{}' at {} with {:?} endpoints",
        name,
        project_path.display(),
        endpoint_kind
    );
    println!(
        "Preparing project services for session '{}' at {}",
        name,
        project_path.display()
    );
    println!("  endpoint kind: {endpoint_kind:?}");
    std::io::stdout().flush()?;

    let manager = SessionManager::new(&project_path)?;
    let session = manager.session(&name)?;
    ensure_active_or_recovery(&session, recover)?;
    let resolved = daemon_endpoint
        .as_ref()
        .ok_or(CliError::MissingDaemonEndpoint {
            operation: "session service preparation",
        })?;
    let result = match prepare_project_session_services_through_daemon(
        manager.project_root(),
        session.slug.clone(),
        endpoint_kind,
        service_names,
        otlp_endpoint,
        recover,
        skip_build,
        &resolved.endpoint,
    ) {
        Ok(result) => result,
        Err(error)
            if resolved.source == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                && crate::commands::daemon::is_daemon_connection_failure(&error) =>
        {
            crate::commands::daemon::handle_stale_runtime_record(&error)?;
            return Err(CliError::MissingDaemonEndpoint {
                operation: "session service preparation",
            });
        }
        Err(error) => return Err(error),
    };
    println!(
        "Service plan received: {} build command(s), {} service command(s)",
        result.build_command_count, result.prepared_process_count
    );
    let manifest = manager.session(&session.slug)?;

    println!("Project services prepared for session '{}'", manifest.slug);
    print_manifest_summary(&manifest);
    println!("Service commands:");
    for process in &manifest.processes {
        print_service_process(process);
    }

    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "these are the fields of azd's `prepareProjectSessionServices` request plus the \
              project root and endpoint needed to reach azd; grouping them again on the CLI \
              side would only mirror `PrepareProjectSessionServicesRequest`"
)]
fn prepare_project_session_services_through_daemon(
    project_root: &Path,
    session_slug: String,
    endpoint_kind: EndpointKind,
    service_names: Vec<String>,
    otlp_endpoint: Option<String>,
    recover: bool,
    skip_build: bool,
    endpoint: &Endpoint,
) -> CliResult<ProjectSessionServicesResult> {
    let requested_root = project_root.to_path_buf();
    let root = project_root.to_string_lossy().into_owned();
    crate::commands::daemon::with_daemon_progress(
        endpoint,
        "session service preparation",
        crate::commands::daemon::DAEMON_RPC_PROGRESS_INTERVAL,
        async move |client| {
            let mut register = client.register_project_root_request();
            (RegisterProjectRootRequest {
                capability: crate::commands::daemon::daemon_capability(DAEMON_PROJECTS_PERMISSION),
                root,
            })
            .to_capnp(register.get().init_request())?;
            let response = register.send().promise.await?;
            let project = ProjectRecord::from_capnp(response.get()?.get_project()?)?;
            crate::commands::daemon::ensure_daemon_project_record_matches_request(
                &project,
                None,
                Some(&requested_root),
                "registerProjectRoot",
            )?;

            let mut prepare = client.prepare_project_session_services_request();
            (PrepareProjectSessionServicesRequest {
                capability: crate::commands::daemon::daemon_capability(DAEMON_SESSIONS_PERMISSION),
                project_id: project.project_id.clone(),
                session_slug: session_slug.clone(),
                endpoint_kind,
                skip_build,
                service_names,
                otlp_endpoint,
                recover,
            })
            .to_capnp(prepare.get().init_request())?;
            let response = prepare.send().promise.await?;
            let result = ProjectSessionServicesResult::from_capnp(response.get()?.get_result()?)?;
            if result.manifest.project_id != project.project_id
                || result.manifest.slug != session_slug
            {
                return Err(CliError::InvalidServicePlan {
                    message: format!(
                        "azd prepared session `{}` for project `{}`, expected session `{}` for project `{}`",
                        result.manifest.slug,
                        result.manifest.project_id,
                        session_slug,
                        project.project_id
                    ),
                });
            }
            Ok(result)
        },
    )
}

#[cfg(test)]
fn plan_project_services_for_session(
    project_root: &Path,
    service_root: &Path,
    session_slug: String,
    endpoint_kind: EndpointKind,
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
    service_names: Vec<String>,
) -> CliResult<ProjectServicePlan> {
    let Some(resolved) = daemon_endpoint else {
        return Err(CliError::MissingDaemonEndpoint {
            operation: "session service planning",
        });
    };

    let endpoint = &resolved.endpoint;
    info!(
        endpoint = %endpoint.address,
        endpoint_kind = ?endpoint.kind,
        session = %session_slug,
        "planning session services through azd"
    );
    println!(
        "Planning project services through azd at {} ({:?})",
        endpoint.address, endpoint.kind
    );
    println!(
        "  session: {session_slug}, project root: {}, workspace: {}",
        project_root.display(),
        service_root.display()
    );
    std::io::stdout().flush()?;
    match plan_project_services_through_daemon(
        project_root,
        service_root,
        session_slug,
        endpoint_kind,
        endpoint,
        service_names,
    ) {
        Ok(plan) => Ok(plan),
        Err(error)
            if resolved.source == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                && crate::commands::daemon::is_daemon_connection_failure(&error) =>
        {
            crate::commands::daemon::handle_stale_runtime_record(&error)?;
            Err(CliError::MissingDaemonEndpoint {
                operation: "session service planning",
            })
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn plan_project_services_through_daemon(
    project_root: &Path,
    service_root: &Path,
    session_slug: String,
    endpoint_kind: EndpointKind,
    endpoint: &Endpoint,
    service_names: Vec<String>,
) -> CliResult<ProjectServicePlan> {
    let requested_root = project_root.to_path_buf();
    let project_root = project_root.to_string_lossy().into_owned();
    let workspace_root = service_root.to_string_lossy().into_owned();
    crate::commands::daemon::with_daemon_progress(
        endpoint,
        "session service planning",
        crate::commands::daemon::DAEMON_RPC_PROGRESS_INTERVAL,
        async move |client| {
            println!(
                "  azd: registering project root {}",
                requested_root.display()
            );
            std::io::stdout().flush()?;
            let mut register = client.register_project_root_request();
            (RegisterProjectRootRequest {
                capability: crate::commands::daemon::daemon_capability(DAEMON_PROJECTS_PERMISSION),
                root: project_root,
            })
            .to_capnp(register.get().init_request())?;
            let register_response = register.send().promise.await?;
            let project = ProjectRecord::from_capnp(register_response.get()?.get_project()?)?;
            crate::commands::daemon::ensure_daemon_project_record_matches_request(
                &project,
                None,
                Some(&requested_root),
                "registerProjectRoot",
            )?;
            let project_id = project.project_id.clone();
            println!("  azd: project resolved as {project_id}; requesting service plan");
            std::io::stdout().flush()?;

            let mut plan_request = client.plan_project_services_request();
            (PlanProjectServicesRequest {
                capability: crate::commands::daemon::daemon_capability(DAEMON_PROJECTS_PERMISSION),
                project_id: project_id.clone(),
                session_slug: session_slug.clone(),
                endpoint_kind,
                workspace_root: Some(workspace_root),
                service_names,
            })
            .to_capnp(plan_request.get().init_request())?;
            let plan_response = plan_request.send().promise.await?;
            let plan = ProjectServicePlan::from_capnp(plan_response.get()?.get_plan()?)?;
            crate::commands::daemon::ensure_daemon_project_service_plan_matches_request(
                &plan,
                &project_id,
                &session_slug,
            )?;
            println!(
                "  azd: service plan ready ({} build command(s), {} service command(s))",
                plan.build_commands.len(),
                plan.commands.len()
            );
            std::io::stdout().flush()?;
            Ok(plan)
        },
    )
}

pub fn supervise_services(
    name: &str,
    session_supervisor_kind: Option<EndpointKind>,
    session_supervisor_endpoint: Option<&str>,
    otlp_endpoint: Option<&str>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;
    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let forwarded_daemon_endpoint = forwarded_sessiond_daemon_endpoint(daemon_endpoint.as_ref());
    let sessiond_executable = crate::commands::host_tools::ensure_session_supervisor()?;
    let command = sessiond_launch_command(
        &sessiond_executable,
        &project_path,
        &SessiondLaunch {
            session: name,
            session_supervisor_kind,
            session_supervisor_endpoint,
            daemon_endpoint: forwarded_daemon_endpoint,
            otlp_endpoint,
            keep_alive: false,
            start_service_names: &[],
        },
    )?;

    info!(
        session = %name,
        root = %project_path.display(),
        program = %command.program,
        args = ?command.args,
        "launching az-sessiond"
    );

    let status = Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.cwd)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(CliError::CommandFailed(Box::new(CommandFailedDetails {
            program: command.program,
            args: command.args,
            cwd: command.cwd,
            status: status.code(),
        })))
    }
}

/// Everything one `azoth session services start` was asked for: which session in which
/// project directory, the supervisor endpoint kind and the rejected explicit
/// supervisor/OTLP endpoints, how long to wait, how to reach azd, and which services to
/// start (empty starts the whole prepared plan). Callers that start services on a session's
/// behalf — the editor and the runtime host — pass the same set, so it travels as one
/// request rather than as nine positional arguments.
#[derive(Debug)]
pub struct StartServicesOptions {
    pub name: String,
    pub session_supervisor_kind: Option<EndpointKind>,
    pub session_supervisor_endpoint: Option<String>,
    pub otlp_endpoint: Option<String>,
    pub timeout_ms: u64,
    pub daemon_endpoint_kind: Option<EndpointKind>,
    pub daemon_endpoint: Option<String>,
    pub path: Option<PathBuf>,
    pub service_names: Vec<String>,
}

pub fn start_services(options: StartServicesOptions) -> CliResult<()> {
    let StartServicesOptions {
        name,
        session_supervisor_kind,
        session_supervisor_endpoint,
        otlp_endpoint,
        timeout_ms,
        daemon_endpoint_kind,
        daemon_endpoint,
        path,
        service_names,
    } = options;
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    if session_supervisor_endpoint.is_some() {
        return Err(CliError::InvalidServicePlan {
            message: "daemon-owned session startup does not accept an explicit session-supervisor endpoint"
                .to_string(),
        });
    }
    if otlp_endpoint.is_some() {
        return Err(CliError::InvalidServicePlan {
            message:
                "configure OTLP while preparing services; startup reuses the prepared service plan"
                    .to_string(),
        });
    }
    let endpoint_kind = session_supervisor_kind.unwrap_or_else(default_service_endpoint_kind);
    validate_public_endpoint_kind(endpoint_kind, "session service startup")?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?
    .ok_or(CliError::MissingDaemonEndpoint {
        operation: "session service startup",
    })?;

    info!(
        session = %name,
        root = %project_path.display(),
        services = ?service_names,
        "ensuring project-instance and session services through azd"
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(&name)?;
    ensure_active(&manifest)?;
    let result = ensure_project_session_services_through_daemon(
        &manifest,
        true,
        timeout_ms,
        endpoint_kind,
        &daemon_endpoint,
        service_names,
    )?;

    println!("session services started for '{}'", manifest.slug);
    println!(
        "session-supervisor: {:?} {}",
        result.supervisor.endpoint.kind, result.supervisor.endpoint.address
    );
    if result.sessiond_pid != 0 {
        println!("session-supervisor process_id: {}", result.sessiond_pid);
    }
    println!("running services:");
    for service in result.running_service_names {
        println!("  {service}");
    }
    Ok(())
}

pub fn stop_services(
    name: &str,
    reason: Option<String>,
    wait: bool,
    _shutdown_timeout_ms: u64,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Stopping session services for '{}' at {}",
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    let live_services = live_service_names(&manifest);
    let descriptor =
        match session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref()) {
            Ok(descriptor) => Some(descriptor),
            Err(CliError::MissingSessionService { .. }) if live_services.is_empty() => {
                println!("Session '{}' has no live service records", manifest.slug);
                return Ok(());
            }
            Err(CliError::MissingSessionService { .. }) => None,
            Err(error) => return Err(error),
        };
    let reason = reason.unwrap_or_else(|| "azoth session services stop".to_string());

    let Some(descriptor) = descriptor else {
        cleanup_orphaned_session_services(&manager, &manifest, "missing session-supervisor")?;
        println!("Session '{}' orphaned services cleaned up", manifest.slug);
        return Ok(());
    };

    let result = match request_session_service_shutdown(&manifest, &descriptor, &reason) {
        Ok(result) => result,
        Err(error)
            if supervisor_shutdown_request_unreachable(&error) && !live_services.is_empty() =>
        {
            cleanup_orphaned_session_services(&manager, &manifest, &error.to_string())?;
            println!("Session '{}' orphaned services cleaned up", manifest.slug);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    request_session_supervisor_shutdown(&manifest, &descriptor, &reason)?;

    println!("Session services stopped for '{}'", manifest.slug);
    for service in &result.stopped {
        println!("  stopped: {service}");
    }
    for service in &result.skipped {
        println!("  skipped: {service}");
    }
    if wait {
        println!(
            "Session '{}' terminal stop result received; supervisor shutdown requested",
            manifest.slug
        );
    }
    Ok(())
}

/// Stop every session supervisor running the `az-sessiond` image that sits
/// beside this CLI.
///
/// A Windows process holds its own executable open, so Cargo cannot relink
/// `target/<profile>/az-sessiond.exe` while a supervisor from an earlier editor
/// run is still alive. Those supervisors belong to whatever projects they were
/// launched for, so there is no single session to address: the sweep discovers
/// them from the process table and stops each through the same daemon-backed
/// path as `azoth session services stop`.
///
/// # Errors
///
/// Returns an error when a discovered supervisor cannot be addressed, when a
/// stop request fails, or when a supervisor is still running once
/// `service_shutdown_timeout_ms` has elapsed.
pub fn sweep(reason: Option<String>, service_shutdown_timeout_ms: u64) -> CliResult<()> {
    let sessiond_image = workspace_sessiond_image()?;
    let reason = reason.unwrap_or_else(|| SWEEP_REASON.to_string());
    info!(
        image = %sessiond_image.display(),
        "sweeping workspace session supervisors"
    );

    let supervisors = workspace_supervisors(&running_processes(), &sessiond_image)?;
    if supervisors.is_empty() {
        println!(
            "No session supervisors are running {}",
            sessiond_image.display()
        );
        return Ok(());
    }

    for supervisor in &supervisors {
        println!(
            "Stopping session supervisor pid {}: {} [{}]",
            supervisor.pid,
            supervisor.project.display(),
            supervisor.session
        );
        stop_services(
            &supervisor.session,
            Some(reason.clone()),
            true,
            service_shutdown_timeout_ms,
            None,
            None,
            Some(supervisor.project.clone()),
        )?;
    }

    await_swept_supervisors(&sessiond_image, service_shutdown_timeout_ms)?;
    println!(
        "Swept {} session supervisor(s) running {}",
        supervisors.len(),
        sessiond_image.display()
    );
    Ok(())
}

/// The `az-sessiond` image in this CLI's own directory.
///
/// `just editor build` links `azoth` and `az-sessiond` into one target
/// directory, so the supervisors that block that directory are exactly the ones
/// running this path.
fn workspace_sessiond_image() -> CliResult<PathBuf> {
    let azoth_image = std::env::current_exe()?;
    let Some(directory) = azoth_image.parent() else {
        return Err(CliError::InvalidArgument {
            message: format!(
                "azoth executable `{}` has no parent directory to sweep",
                azoth_image.display()
            ),
        });
    };
    Ok(az_filesystem::normalize(&directory.join(format!(
        "az-sessiond{}",
        std::env::consts::EXE_SUFFIX
    ))))
}

/// Poll until nothing is running `sessiond_image` any more.
fn await_swept_supervisors(sessiond_image: &Path, timeout_ms: u64) -> CliResult<()> {
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    loop {
        let remaining: Vec<String> = running_processes()
            .iter()
            .filter(|process| process.runs_image(sessiond_image))
            .map(|process| format!("{}:{}", process.pid, process.executable.display()))
            .collect();
        if remaining.is_empty() {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            // Waiting on the OS to reap a process is an environment condition,
            // not a bad argument or a bad service plan.
            return Err(CliError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "session supervisors still hold {} after {timeout_ms}ms: {}",
                    sessiond_image.display(),
                    remaining.join(", ")
                ),
            )));
        }
        std::thread::sleep(SWEEP_POLL_INTERVAL);
    }
}

/// A running process reduced to what the sweep matches on.
///
/// Enumeration is kept separate from selection so the matching rules are
/// testable without live processes.
#[derive(Debug, Clone)]
struct SweptProcess {
    pid: u32,
    executable: PathBuf,
    argv: Vec<String>,
}

impl SweptProcess {
    /// Whether this process runs `image`. The file-name test is a cheap
    /// prefilter over the whole process table; the normalized comparison is
    /// what makes short, long, and differently cased spellings of one file
    /// compare equal. `image` must already be normalized.
    fn runs_image(&self, image: &Path) -> bool {
        let (Some(name), Some(expected)) = (self.executable.file_name(), image.file_name()) else {
            return false;
        };
        name.eq_ignore_ascii_case(expected)
            && az_filesystem::normalize(&self.executable).as_path() == image
    }
}

/// One session supervisor the sweep will stop.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SweptSupervisor {
    pid: u32,
    project: PathBuf,
    session: String,
}

/// Snapshot every running process the sweep can match, dropping those whose
/// image path the OS will not disclose: a process we cannot name by path can
/// never be the supervisor we are about to address by path.
fn running_processes() -> Vec<SweptProcess> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            Some(SweptProcess {
                pid: pid.as_u32(),
                executable: process.exe()?.to_path_buf(),
                argv: process
                    .cmd()
                    .iter()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect(),
            })
        })
        .collect()
}

/// Select the supervisors running `sessiond_image`, one per project and session.
///
/// azd launches a supervisor per (project, session) pair, so two live processes
/// carrying the same pair name one target: stopping it twice would report the
/// second stop as having no live services.
fn workspace_supervisors(
    processes: &[SweptProcess],
    sessiond_image: &Path,
) -> CliResult<Vec<SweptSupervisor>> {
    let mut addressed = BTreeSet::new();
    let mut supervisors = Vec::new();
    for process in processes
        .iter()
        .filter(|process| process.runs_image(sessiond_image))
    {
        let Some((project, session)) = sessiond_target(&process.argv) else {
            return Err(CliError::InvalidArgument {
                message: format!(
                    "session supervisor pid {} names no --project/--session to stop: {:?}",
                    process.pid, process.argv
                ),
            });
        };
        if !addressed.insert((project.clone(), session.clone())) {
            continue;
        }
        supervisors.push(SweptSupervisor {
            pid: process.pid,
            project,
            session,
        });
    }
    Ok(supervisors)
}

/// Read back the project root and session name a supervisor was launched with:
/// the inverse of the argv [`sessiond_args`] builds. Both are required, because
/// [`stop_services`] addresses a supervisor by exactly that pair.
fn sessiond_target(argv: &[String]) -> Option<(PathBuf, String)> {
    let project = argv_value(argv, "--project")?;
    let session = argv_value(argv, "--session")?;
    Some((PathBuf::from(project), session.to_string()))
}

/// The value following `flag` in an OS-split argv, ignoring a flag that trails
/// the argv or carries a blank value.
fn argv_value<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    argv.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
        .filter(|value| !value.trim().is_empty())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the trailing `operation` closure is the lifecycle action itself and the leading \
              arguments are `retire`'s own flags forwarded unchanged; a struct would rename \
              the flags while the closure stayed positional, gaining nothing at the one call \
              site"
)]
fn run_session_lifecycle_operation(
    project_path: &Path,
    session_name: &str,
    operation_name: &str,
    stop_services: bool,
    _shutdown_timeout_ms: u64,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    operation: impl Fn(&SessionManager) -> Result<SessionManifest, SessionError>,
) -> CliResult<SessionManifest> {
    let manager = SessionManager::new(project_path)?;
    match operation(&manager) {
        Ok(manifest) => Ok(manifest),
        Err(SessionError::SessionServicesRunning { services, .. }) if stop_services => {
            println!(
                "Session '{session_name}' has live services; requesting shutdown before {operation_name}"
            );
            for service in &services {
                println!("  {service}");
            }

            let manifest = manager.session(session_name)?;
            let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
                daemon_endpoint_kind,
                daemon_endpoint,
            )?;
            let descriptor =
                session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;
            let reason = format!("azoth session {operation_name}");
            let result = request_session_service_shutdown(&manifest, &descriptor, &reason)?;
            request_session_supervisor_shutdown(&manifest, &descriptor, &reason)?;
            println!(
                "Session services stopped for '{}' (stopped={}, skipped={})",
                manifest.slug,
                result.stopped.len(),
                result.skipped.len()
            );

            operation(&manager).map_err(Into::into)
        }
        Err(error) => Err(error.into()),
    }
}

fn request_session_service_shutdown(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    reason: &str,
) -> CliResult<StopServicesResult> {
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let manifest = manifest.clone();
    let descriptor = descriptor.clone();
    let reason = reason.to_string();
    local.block_on(&runtime, async move {
        let client = connect_session_supervisor(&descriptor).await?;
        let mut request = client.stop_services_request();
        (StopServicesRequest {
            capability: session_manage_capability(&manifest, &descriptor)?,
            slug: manifest.slug.clone(),
            reason: reason.clone(),
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let result = StopServicesResult::from_capnp(response.get()?.get_result()?)?;
        ensure_cli_session_workspace_status_matches_manifest(
            &result.status,
            &manifest,
            "stopServices",
        )?;
        Ok(result)
    })
}

fn request_session_supervisor_shutdown(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    reason: &str,
) -> CliResult<()> {
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let manifest = manifest.clone();
    let descriptor = descriptor.clone();
    let reason = reason.to_string();
    local.block_on(&runtime, async move {
        validate_session_supervisor_descriptor(&descriptor, "connect session-supervisor")?;
        let connection: az_rpc::ScopedTwopartyClient<session_capnp::session_supervisor::Client> =
            az_rpc::connect_twoparty_bootstrap_scoped(&descriptor.endpoint).await?;
        let client = connection.client();
        let mut request = client.shutdown_supervisor_request();
        let mut params = request.get();
        params.set_slug(&manifest.slug);
        params.set_reason(&reason);
        session_manage_capability(&manifest, &descriptor)?.to_capnp(params.init_capability())?;
        // The terminal stop result is received before this function is
        // called. Queue the streaming call, then gracefully flush this
        // short-lived connection without awaiting a supervisor result.
        drop(request.send());
        connection.disconnect().await?;
        Ok(())
    })
}

fn request_session_service_start_for_proto_manifest(
    manifest: &ProtoSessionManifest,
    descriptor: &ServiceDescriptor,
    reason: &str,
    service_names: Vec<String>,
) -> CliResult<StartServicesResult> {
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let manifest = manifest.clone();
    let descriptor = descriptor.clone();
    let reason = reason.to_string();
    local.block_on(&runtime, async move {
        let client = connect_session_supervisor(&descriptor).await?;
        let mut request = client.start_services_request();
        (StartServicesRequest {
            capability: proto_session_manage_capability(&manifest, &descriptor)?,
            slug: manifest.slug.clone(),
            reason: reason.clone(),
            service_names,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let result = StartServicesResult::from_capnp(response.get()?.get_result()?)?;
        ensure_cli_session_workspace_status_matches_proto_manifest(
            &result.status,
            &manifest,
            "startServices",
        )?;
        Ok(result)
    })
}

pub fn start_services_for_proto_manifest_through_daemon(
    manifest: &ProtoSessionManifest,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
    reason: &str,
    service_names: Vec<String>,
) -> CliResult<StartServicesResult> {
    let descriptor =
        live_proto_session_supervisor_descriptor_through_daemon(manifest, daemon_endpoint)?;
    request_session_service_start_for_proto_manifest(manifest, &descriptor, reason, service_names)
}

fn request_session_status(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<ProtoSessionWorkspaceStatus> {
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let manifest = manifest.clone();
    let descriptor = descriptor.clone();
    local.block_on(&runtime, async move {
        let client = connect_session_supervisor(&descriptor).await?;
        request_session_status_from_supervisor(&client, &descriptor, &manifest).await
    })
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_session_status_from_supervisor(
    supervisor: &session_capnp::session_supervisor::Client,
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
) -> CliResult<ProtoSessionWorkspaceStatus> {
    let mut request = supervisor.status_request();
    ProtoSessionSlugRequest {
        capability: session_read_capability(manifest, descriptor)?,
        slug: manifest.slug.clone(),
    }
    .to_capnp(request.get())?;
    let response = request.send().promise.await?;
    let status = ProtoSessionWorkspaceStatus::from_capnp(response.get()?.get_status()?)?;
    ensure_cli_session_workspace_status_matches_manifest(
        &status,
        manifest,
        "session-supervisor status",
    )?;
    Ok(status)
}

fn request_session_status_for_proto_manifest(
    manifest: &ProtoSessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<ProtoSessionWorkspaceStatus> {
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let manifest = manifest.clone();
    let descriptor = descriptor.clone();
    local.block_on(&runtime, async move {
        let client = connect_session_supervisor(&descriptor).await?;
        let mut request = client.status_request();
        ProtoSessionSlugRequest {
            capability: proto_session_read_capability(&manifest, &descriptor)?,
            slug: manifest.slug.clone(),
        }
        .to_capnp(request.get())?;
        let response = request.send().promise.await?;
        let status = ProtoSessionWorkspaceStatus::from_capnp(response.get()?.get_status()?)?;
        ensure_cli_session_workspace_status_matches_proto_manifest(
            &status,
            &manifest,
            "session-supervisor status",
        )?;
        Ok(status)
    })
}

fn ensure_cli_session_workspace_status_matches_manifest(
    status: &ProtoSessionWorkspaceStatus,
    manifest: &SessionManifest,
    operation: &'static str,
) -> CliResult<()> {
    ensure_proto_session_response_matches_manifest(&status.manifest, manifest, operation)?;
    ensure_cli_session_workspace_status_is_consistent(status, operation)
}

fn ensure_cli_session_workspace_status_matches_proto_manifest(
    status: &ProtoSessionWorkspaceStatus,
    manifest: &ProtoSessionManifest,
    operation: &'static str,
) -> CliResult<()> {
    ensure_proto_session_response_matches_proto_manifest(&status.manifest, manifest, operation)?;
    ensure_cli_session_workspace_status_is_consistent(status, operation)
}

fn ensure_cli_session_workspace_status_is_consistent(
    status: &ProtoSessionWorkspaceStatus,
    operation: &'static str,
) -> CliResult<()> {
    if status.manifest.state == ProtoSessionState::FailedPreserved
        && status
            .failure_reason
            .as_deref()
            .is_none_or(|reason| reason.trim().is_empty())
    {
        return Err(session_supervisor_authority_mismatch(
            operation,
            format!(
                "failed-preserved session `{}` must include a failure reason",
                status.manifest.slug
            ),
        ));
    }
    Ok(())
}

pub fn live_session_status_through_daemon(
    manifest: &SessionManifest,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<ProtoSessionWorkspaceStatus> {
    let descriptor = live_session_supervisor_descriptor_through_daemon(manifest, daemon_endpoint)?;
    request_session_status(manifest, &descriptor)
}

pub fn live_session_status_for_proto_manifest_through_daemon(
    manifest: &ProtoSessionManifest,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<ProtoSessionWorkspaceStatus> {
    let descriptor =
        live_proto_session_supervisor_descriptor_through_daemon(manifest, daemon_endpoint)?;
    request_session_status_for_proto_manifest(manifest, &descriptor)
}

fn request_session_exec_command(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    program: &str,
    args: &[String],
) -> CliResult<ProtoExecCommandResult> {
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let manifest = manifest.clone();
    let descriptor = descriptor.clone();
    let program = program.to_string();
    let args = args.to_vec();
    local.block_on(&runtime, async move {
        let client = connect_session_supervisor(&descriptor).await?;
        let mut request = client.exec_command_request();
        (ProtoExecCommandRequest {
            capability: session_exec_capability(&manifest, &descriptor)?,
            slug: manifest.slug.clone(),
            program,
            args,
            max_output_bytes: SESSION_EXEC_OUTPUT_LIMIT_BYTES,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let result = ProtoExecCommandResult::from_capnp(response.get()?.get_result()?)?;
        ensure_cli_exec_command_result_matches_request(&result, SESSION_EXEC_OUTPUT_LIMIT_BYTES)?;
        Ok(result)
    })
}

fn ensure_cli_exec_command_result_matches_request(
    result: &ProtoExecCommandResult,
    max_output_bytes: u32,
) -> CliResult<()> {
    if result.success && !result.exited {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            "successful command result must report an exited process".to_string(),
        ));
    }
    if result.success && result.exit_code != 0 {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            format!(
                "successful command result must report exit code 0, got {}",
                result.exit_code
            ),
        ));
    }
    if !result.exited && result.exit_code != 0 {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            format!(
                "non-exited command result cannot carry exit code {}",
                result.exit_code
            ),
        ));
    }

    let max_lossy_utf8_len = (max_output_bytes as usize).saturating_mul(3);
    if result.stdout.len() > max_lossy_utf8_len {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            format!(
                "stdout length {} exceeds bounded output limit {}",
                result.stdout.len(),
                max_lossy_utf8_len
            ),
        ));
    }
    if result.stderr.len() > max_lossy_utf8_len {
        return Err(session_supervisor_authority_mismatch(
            "execCommand",
            format!(
                "stderr length {} exceeds bounded output limit {}",
                result.stderr.len(),
                max_lossy_utf8_len
            ),
        ));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn request_session_service_log(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    service: &str,
    run: Option<Uuid>,
    stream: ServiceLogStreamArg,
    tail: usize,
    all: bool,
    offset: Option<u64>,
) -> CliResult<ProtoServiceLogResult> {
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let manifest = manifest.clone();
    let descriptor = descriptor.clone();
    let service = service.to_string();
    local.block_on(&runtime, async move {
        let client = connect_session_supervisor(&descriptor).await?;
        request_service_log_chunk(
            &client,
            &descriptor,
            &manifest,
            &service,
            run,
            stream,
            tail,
            all,
            offset,
        )
        .await
    })
}

fn request_session_list_through_daemon(
    project_path: &Path,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<Option<Vec<ProtoSessionManifest>>> {
    let Some(project) = register_session_discovery_project_root(project_path, daemon_endpoint)?
    else {
        return Ok(None);
    };
    let supervisors = match session_supervisor_descriptors_through_daemon(
        &project.project_id,
        &daemon_endpoint.endpoint,
    ) {
        Ok(supervisors) => supervisors,
        Err(error)
            if daemon_endpoint.source
                == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                && is_daemon_connection_failure(&error) =>
        {
            crate::commands::daemon::handle_stale_runtime_record(&error)?;
            return Ok(None);
        }
        Err(error) => return Err(error),
    };

    if supervisors.is_empty() {
        return Ok(None);
    }

    let descriptors = supervisors
        .into_iter()
        .map(|supervisor| supervisor.descriptor)
        .collect::<Vec<_>>();
    request_project_session_list_from_supervisors(&project, descriptors, SESSION_LIST_OPERATION)
}

pub fn active_session_slug_through_daemon(
    project_path: &Path,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
    operation: &'static str,
) -> CliResult<Option<String>> {
    active_session_manifest_through_daemon(project_path, daemon_endpoint, operation)
        .map(|manifest| manifest.map(|manifest| manifest.slug))
}

pub fn active_session_manifest_through_daemon(
    project_path: &Path,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
    operation: &'static str,
) -> CliResult<Option<ProtoSessionManifest>> {
    let Some(project) = register_session_discovery_project_root(project_path, daemon_endpoint)?
    else {
        return Ok(None);
    };
    let supervisors = match session_supervisor_descriptors_through_daemon(
        &project.project_id,
        &daemon_endpoint.endpoint,
    ) {
        Ok(supervisors) => supervisors,
        Err(error)
            if daemon_endpoint.source
                == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                && is_daemon_connection_failure(&error) =>
        {
            crate::commands::daemon::handle_stale_runtime_record(&error)?;
            return Ok(None);
        }
        Err(error) => return Err(error),
    };

    if supervisors.is_empty() {
        return Err(CliError::NoActiveSession { operation });
    }

    let descriptors = supervisors
        .into_iter()
        .map(|supervisor| supervisor.descriptor)
        .collect::<Vec<_>>();
    let Some(sessions) =
        request_project_session_list_from_supervisors(&project, descriptors, operation)?
    else {
        return Err(CliError::NoActiveSession { operation });
    };
    active_session_manifest_from_proto_sessions(sessions, operation).map(Some)
}

pub fn requested_session_manifest_through_daemon(
    project_path: &Path,
    requested_session: &str,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
    operation: &'static str,
) -> CliResult<Option<ProtoSessionManifest>> {
    let requested_slug = normalize_requested_session_slug(requested_session)?;
    let Some(project) = register_session_discovery_project_root(project_path, daemon_endpoint)?
    else {
        return Ok(None);
    };
    let descriptor = match session_supervisor_descriptor_through_daemon(
        &project.project_id,
        &requested_slug,
        &daemon_endpoint.endpoint,
    ) {
        Ok(Some(descriptor)) => descriptor,
        Ok(None) => return Ok(None),
        Err(error)
            if daemon_endpoint.source
                == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                && is_daemon_connection_failure(&error) =>
        {
            crate::commands::daemon::handle_stale_runtime_record(&error)?;
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let Some(sessions) = request_session_list_from_supervisors(vec![descriptor])? else {
        return Ok(None);
    };
    let sessions = validate_proto_sessions_match_project(&project, sessions, operation)?;
    let session = sessions
        .into_iter()
        .find(|session| session.slug == requested_slug)
        .ok_or_else(|| CliError::SessionDiscoveryMismatch {
            operation,
            session: requested_slug.clone(),
            reason: "daemon-registered session-supervisor did not list the requested session"
                .to_string(),
        })?;

    if session.state != ProtoSessionState::Active {
        return Err(CliError::SessionNotActive {
            session: session.slug,
            state: format_proto_state(session.state).to_string(),
        });
    }

    Ok(Some(session))
}

fn normalize_requested_session_slug(name: &str) -> CliResult<String> {
    let slug = name.trim().to_ascii_lowercase().replace([' ', '_'], "-");

    if slug.is_empty()
        || slug.starts_with('-')
        || slug.ends_with('-')
        || !slug
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(SessionError::InvalidSessionName(name.to_string()).into());
    }

    Ok(slug)
}

fn register_session_discovery_project_root(
    project_path: &Path,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<Option<ProjectRecord>> {
    match register_project_root_for_session_discovery(project_path, &daemon_endpoint.endpoint) {
        Ok(project) => Ok(Some(project)),
        Err(error)
            if daemon_endpoint.source
                == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                && is_daemon_connection_failure(&error) =>
        {
            crate::commands::daemon::handle_stale_runtime_record(&error)?;
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn register_project_root_for_session_discovery(
    project_path: &Path,
    endpoint: &Endpoint,
) -> CliResult<ProjectRecord> {
    let requested_root = project_path.to_path_buf();
    let project_root = project_path.to_string_lossy().into_owned();
    crate::commands::daemon::with_daemon(endpoint, async move |client| {
        let mut request = client.register_project_root_request();
        (RegisterProjectRootRequest {
            capability: crate::commands::daemon::daemon_capability(DAEMON_PROJECTS_PERMISSION),
            root: project_root,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let project = ProjectRecord::from_capnp(response.get()?.get_project()?)?;
        crate::commands::daemon::ensure_daemon_project_record_matches_request(
            &project,
            None,
            Some(&requested_root),
            "registerProjectRoot",
        )?;
        Ok(project)
    })
}

fn active_session_manifest_from_proto_sessions(
    sessions: Vec<ProtoSessionManifest>,
    operation: &'static str,
) -> CliResult<ProtoSessionManifest> {
    let active = sessions
        .into_iter()
        .filter(|manifest| manifest.state == ProtoSessionState::Active)
        .collect::<Vec<_>>();

    match active.len() {
        0 => Err(CliError::NoActiveSession { operation }),
        1 => Ok(active
            .into_iter()
            .next()
            .expect("length checked to contain exactly one active session")),
        _ => Err(CliError::AmbiguousActiveSessions {
            operation,
            sessions: active.into_iter().map(|manifest| manifest.slug).collect(),
        }),
    }
}

fn request_project_session_list_from_supervisors(
    project: &ProjectRecord,
    descriptors: Vec<ServiceDescriptor>,
    operation: &'static str,
) -> CliResult<Option<Vec<ProtoSessionManifest>>> {
    request_session_list_from_supervisors(descriptors)?
        .map(|sessions| validate_proto_sessions_match_project(project, sessions, operation))
        .transpose()
}

fn validate_proto_sessions_match_project(
    project: &ProjectRecord,
    sessions: Vec<ProtoSessionManifest>,
    operation: &'static str,
) -> CliResult<Vec<ProtoSessionManifest>> {
    for session in &sessions {
        ensure_proto_session_matches_project(session, project, operation)?;
    }
    Ok(sessions)
}

fn ensure_proto_session_matches_project(
    session: &ProtoSessionManifest,
    project: &ProjectRecord,
    operation: &'static str,
) -> CliResult<()> {
    if session.project_id != project.project_id {
        return Err(CliError::SessionDiscoveryMismatch {
            operation,
            session: session.slug.clone(),
            reason: format!(
                "session project `{}` does not match daemon project `{}`",
                session.project_id, project.project_id
            ),
        });
    }
    if !same_protocol_path(Path::new(&session.project_root), Path::new(&project.root)) {
        return Err(CliError::SessionDiscoveryMismatch {
            operation,
            session: session.slug.clone(),
            reason: format!(
                "session project root `{}` does not match daemon project root `{}`",
                session.project_root, project.root
            ),
        });
    }
    Ok(())
}

fn ensure_proto_session_response_matches_manifest(
    response: &ProtoSessionManifest,
    expected: &SessionManifest,
    operation: &'static str,
) -> CliResult<()> {
    if response.id != expected.id.0 {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response session id `{}` does not match requested session id `{}`",
                response.id, expected.id.0
            ),
        ));
    }
    if response.slug != expected.slug {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response session slug `{}` does not match requested session slug `{}`",
                response.slug, expected.slug
            ),
        ));
    }
    if response.project_id != expected.project_id {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response project `{}` does not match requested project `{}`",
                response.project_id, expected.project_id
            ),
        ));
    }
    if !same_protocol_path(Path::new(&response.project_root), &expected.project_root) {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response project root `{}` does not match requested project root `{}`",
                response.project_root,
                expected.project_root.display()
            ),
        ));
    }
    if !same_protocol_path(
        Path::new(&response.workspace_root),
        &expected.workspace_root,
    ) {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response workspace `{}` does not match requested workspace `{}`",
                response.workspace_root,
                expected.workspace_root.display()
            ),
        ));
    }
    if !same_protocol_path(Path::new(&response.run_dir), &expected.run_dir) {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response run directory `{}` does not match requested run directory `{}`",
                response.run_dir,
                expected.run_dir.display()
            ),
        ));
    }
    Ok(())
}

fn ensure_proto_session_response_matches_proto_manifest(
    response: &ProtoSessionManifest,
    expected: &ProtoSessionManifest,
    operation: &'static str,
) -> CliResult<()> {
    if response.id != expected.id {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response session id `{}` does not match requested session id `{}`",
                response.id, expected.id
            ),
        ));
    }
    if response.slug != expected.slug {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response session slug `{}` does not match requested session slug `{}`",
                response.slug, expected.slug
            ),
        ));
    }
    if response.project_id != expected.project_id {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response project `{}` does not match requested project `{}`",
                response.project_id, expected.project_id
            ),
        ));
    }
    if !same_protocol_path(
        Path::new(&response.project_root),
        Path::new(&expected.project_root),
    ) {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response project root `{}` does not match requested project root `{}`",
                response.project_root, expected.project_root
            ),
        ));
    }
    if !same_protocol_path(
        Path::new(&response.workspace_root),
        Path::new(&expected.workspace_root),
    ) {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response workspace `{}` does not match requested workspace `{}`",
                response.workspace_root, expected.workspace_root
            ),
        ));
    }
    if !same_protocol_path(Path::new(&response.run_dir), Path::new(&expected.run_dir)) {
        return Err(session_response_mismatch(
            response,
            operation,
            format!(
                "response run directory `{}` does not match requested run directory `{}`",
                response.run_dir, expected.run_dir
            ),
        ));
    }
    Ok(())
}

fn session_response_mismatch(
    response: &ProtoSessionManifest,
    operation: &'static str,
    reason: String,
) -> CliError {
    CliError::SessionDiscoveryMismatch {
        operation,
        session: response.slug.clone(),
        reason,
    }
}

fn request_session_list_from_supervisors(
    descriptors: Vec<ServiceDescriptor>,
) -> CliResult<Option<Vec<ProtoSessionManifest>>> {
    if descriptors.is_empty() {
        return Ok(None);
    }

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    local.block_on(&runtime, async move {
        let mut last_error = None;
        for descriptor in descriptors {
            match request_session_list_from_supervisor(&descriptor).await {
                Ok(sessions) => return Ok(Some(sessions)),
                Err(error) => last_error = Some(error),
            }
        }

        last_error.map_or(Ok(None), Err)
    })
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_session_list_from_supervisor(
    descriptor: &ServiceDescriptor,
) -> CliResult<Vec<ProtoSessionManifest>> {
    let client = connect_session_supervisor(descriptor).await?;
    let mut request = client.list_request();
    (ProtoSessionCapabilityRequest {
        capability: unscoped_session_read_capability(descriptor)?,
    })
    .to_capnp(request.get())?;
    let response = request.send().promise.await?;
    response
        .get()?
        .get_sessions()?
        .iter()
        .map(ProtoSessionManifest::from_capnp)
        .collect::<Result<Vec<_>, capnp::Error>>()
        .map_err(Into::into)
}

fn request_register_service(
    manifest: &SessionManifest,
    supervisor: &ServiceDescriptor,
    descriptor: &ServiceDescriptor,
) -> CliResult<ProtoSessionManifest> {
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let manifest = manifest.clone();
    let supervisor = supervisor.clone();
    let descriptor = descriptor.clone();
    local.block_on(&runtime, async move {
        let client = connect_session_supervisor(&supervisor).await?;
        let mut request = client.register_service_request();
        (ProtoRegisterServiceRequest {
            capability: session_manage_capability(&manifest, &supervisor)?,
            slug: manifest.slug.clone(),
            descriptor: descriptor.clone(),
        })
        .to_capnp(request.get())?;
        let response = request.send().promise.await?;
        let updated = ProtoSessionManifest::from_capnp(response.get()?.get_manifest()?)?;
        ensure_proto_session_response_matches_manifest(
            &updated,
            &manifest,
            "session service registration",
        )?;
        ensure_registered_service_response_contains_descriptor(&updated, &descriptor)?;
        Ok(updated)
    })
}

fn ensure_registered_service_response_contains_descriptor(
    response: &ProtoSessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<()> {
    match response
        .services
        .iter()
        .find(|service| service.id == descriptor.id && service.role == descriptor.role)
    {
        Some(registered) if registered.has_same_connection_contract(descriptor) => Ok(()),
        Some(registered) => Err(session_response_mismatch(
            response,
            "session service registration",
            format!(
                "{} response descriptor endpoint {:?} `{}` does not match requested endpoint {:?} `{}`",
                service_descriptor_label(&descriptor.id, descriptor.role),
                registered.endpoint.kind,
                registered.endpoint.address,
                descriptor.endpoint.kind,
                descriptor.endpoint.address,
            ),
        )),
        None => Err(session_response_mismatch(
            response,
            "session service registration",
            format!(
                "{} response did not include the registered service descriptor",
                service_descriptor_label(&descriptor.id, descriptor.role),
            ),
        )),
    }
}

fn request_recover_session(
    manifest: &SessionManifest,
    supervisor: &ServiceDescriptor,
    force: bool,
) -> CliResult<ProtoSessionManifest> {
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let manifest = manifest.clone();
    let supervisor = supervisor.clone();
    local.block_on(&runtime, async move {
        let client = connect_session_supervisor(&supervisor).await?;
        let mut request = client.recover_request();
        (ProtoRecoverSessionRequest {
            capability: session_manage_capability(&manifest, &supervisor)?,
            slug: manifest.slug.clone(),
            force,
        })
        .to_capnp(request.get())?;
        let response = request.send().promise.await?;
        let recovered = ProtoSessionManifest::from_capnp(response.get()?.get_manifest()?)?;
        ensure_proto_session_response_matches_manifest(&recovered, &manifest, "session recovery")?;
        Ok(recovered)
    })
}

fn cleanup_orphaned_session_services(
    manager: &SessionManager,
    manifest: &SessionManifest,
    cause: &str,
) -> CliResult<()> {
    let processes = live_service_processes(manifest);
    if processes.is_empty() {
        println!("Session '{}' has no live service records", manifest.slug);
        return Ok(());
    }

    println!(
        "Session supervisor for '{}' is unavailable ({cause}); cleaning up recorded service PIDs",
        manifest.slug
    );

    let mut failures = Vec::new();
    for process in processes {
        match terminate_recorded_service_process(&process).map_err(SessionError::from)? {
            RecordedServiceProcessCleanup::Terminated { pid } => {
                let key = ServiceProcessKey::from_process(&process);
                manager.mark_service_exited(&manifest.slug, &key, None, None)?;
                println!("  stopped {} pid {}", process.service_name, pid);
            }
            RecordedServiceProcessCleanup::NotRunning { pid } => {
                let key = ServiceProcessKey::from_process(&process);
                manager.mark_service_exited(&manifest.slug, &key, None, None)?;
                println!("  cleared stale {} pid {}", process.service_name, pid);
            }
            RecordedServiceProcessCleanup::NoPid => {
                let key = ServiceProcessKey::from_process(&process);
                manager.mark_service_exited(&manifest.slug, &key, None, None)?;
                println!(
                    "  cleared stale {} record with no pid",
                    process.service_name
                );
            }
            RecordedServiceProcessCleanup::ImageMismatch {
                pid,
                expected,
                actual,
            } => {
                failures.push(format!(
                    "{} pid {} image mismatch: expected `{}`, actual `{}`",
                    process.service_name, pid, expected, actual
                ));
            }
            RecordedServiceProcessCleanup::Reused {
                pid,
                recorded_start_time,
                actual_start_time,
            } => {
                failures.push(format!(
                    "{} pid {} was reused: recorded start {}, actual start {}",
                    process.service_name, pid, recorded_start_time, actual_start_time
                ));
            }
            RecordedServiceProcessCleanup::Unattributable { pid } => {
                failures.push(format!(
                    "{} pid {} has no recorded process-start identity; refusing to terminate it",
                    process.service_name, pid
                ));
            }
            RecordedServiceProcessCleanup::IdentityBindingUnavailable { pid, reason } => {
                failures.push(format!(
                    "{} pid {} cannot be terminated through an identity-bound platform handle: {}",
                    process.service_name, pid, reason
                ));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(CliError::SessionOrphanCleanupFailed {
            session: manifest.slug.clone(),
            failures,
        })
    }
}

fn live_service_names(manifest: &SessionManifest) -> Vec<String> {
    live_service_processes(manifest)
        .into_iter()
        .map(|process| process.service_name)
        .collect()
}

fn live_service_processes(manifest: &SessionManifest) -> Vec<ServiceProcessRecord> {
    manifest
        .processes
        .iter()
        .filter(|process| {
            matches!(
                process.state,
                ServiceProcessState::Starting | ServiceProcessState::Running
            )
        })
        .cloned()
        .collect()
}

const fn supervisor_shutdown_request_unreachable(error: &CliError) -> bool {
    matches!(error, CliError::RpcTransport(_))
}

fn can_fallback_to_local_session(
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
) -> bool {
    !matches!(
        daemon_endpoint.map(|resolved| resolved.source),
        Some(crate::commands::daemon::DaemonEndpointSource::Explicit)
    )
}

fn can_fallback_after_supervisor_error(
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
    error: &CliError,
) -> bool {
    can_fallback_to_local_session(daemon_endpoint)
        && !matches!(
            error,
            CliError::MissingServiceCapability(_) | CliError::InvalidServiceDescriptor { .. }
        )
}

fn forwarded_sessiond_daemon_endpoint(
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
) -> Option<&Endpoint> {
    daemon_endpoint
        .filter(|resolved| {
            resolved.source == crate::commands::daemon::DaemonEndpointSource::Explicit
        })
        .map(|resolved| &resolved.endpoint)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessiondLaunchCommand {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
}

/// How the `az-sessiond` child process is invoked for one session: the session it supervises,
/// the endpoints it should serve and dial, and the startup scope. Every field becomes a
/// command-line flag of the child process, and rendering those flags and building the launch
/// command need the identical set, so the invocation is described once instead of repeated as
/// eight positional arguments in both places.
#[derive(Debug)]
struct SessiondLaunch<'a> {
    session: &'a str,
    session_supervisor_kind: Option<EndpointKind>,
    session_supervisor_endpoint: Option<&'a str>,
    daemon_endpoint: Option<&'a Endpoint>,
    otlp_endpoint: Option<&'a str>,
    keep_alive: bool,
    start_service_names: &'a [String],
}

fn sessiond_launch_command(
    sessiond_executable: &Path,
    project_path: &Path,
    launch: &SessiondLaunch<'_>,
) -> CliResult<SessiondLaunchCommand> {
    let project_path = child_project_path(project_path)?;
    let sessiond_args = sessiond_args(&project_path, launch)?;
    Ok(SessiondLaunchCommand {
        program: sessiond_executable.to_string_lossy().into_owned(),
        args: sessiond_args,
        cwd: project_path,
    })
}

fn sessiond_args(project_path: &Path, launch: &SessiondLaunch<'_>) -> CliResult<Vec<String>> {
    let mut args = vec![
        "--project".to_string(),
        project_path.to_string_lossy().into_owned(),
        "--session".to_string(),
        launch.session.to_string(),
    ];
    if launch.keep_alive {
        args.push("--keep-alive".to_string());
    }
    for service_name in launch.start_service_names {
        args.extend(["--start-service".to_string(), service_name.clone()]);
    }
    if let Some(kind) = launch.session_supervisor_kind {
        args.extend([
            "--endpoint-kind".to_string(),
            endpoint_kind_arg(kind, "session-supervisor launch")?.to_string(),
        ]);
    }
    if let Some(endpoint) = launch.session_supervisor_endpoint {
        args.extend(["--endpoint".to_string(), endpoint.to_string()]);
    }
    if let Some(endpoint) = launch.daemon_endpoint {
        args.extend([
            "--daemon-endpoint-kind".to_string(),
            endpoint_kind_arg(endpoint.kind, "session-supervisor daemon endpoint")?.to_string(),
            "--daemon-endpoint".to_string(),
            endpoint.address.clone(),
        ]);
    }
    if let Some(endpoint) = launch.otlp_endpoint {
        args.extend(["--otlp-endpoint".to_string(), endpoint.to_string()]);
    }
    Ok(args)
}

fn child_project_path(project_path: &Path) -> CliResult<PathBuf> {
    if project_path.is_absolute() {
        Ok(project_path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(project_path))
    }
}

pub fn services(
    name: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!(
        "Inspecting session services for '{}' at {}",
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    match session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref()) {
        Ok(descriptor) => match request_session_status(&manifest, &descriptor) {
            Ok(status) => {
                print_proto_services_status(&status);
                return Ok(());
            }
            Err(error) if can_fallback_after_supervisor_error(daemon_endpoint.as_ref(), &error) => {
                info!(
                    error = %error,
                    session = %manifest.slug,
                    "session-supervisor services status failed; falling back to local session metadata"
                );
            }
            Err(error) => return Err(error),
        },
        Err(CliError::MissingSessionService { .. })
            if can_fallback_to_local_session(daemon_endpoint.as_ref()) => {}
        Err(error) => return Err(error),
    }

    println!("Session '{}' services", manifest.slug);
    print_manifest_summary(&manifest);

    if manifest.services.is_empty() {
        println!("Services: none");
    } else {
        println!("Services:");
        for service in &manifest.services {
            print_service_record(service);
        }
    }

    if manifest.processes.is_empty() {
        println!("Service processes: none");
    } else {
        println!("Service processes:");
        for process in &manifest.processes {
            print_service_process(process);
        }
    }

    Ok(())
}

/// Everything one `azoth session services report log` was asked for: which service log of
/// which session in which project directory, which retained run and stream, how much of it to
/// print, whether to keep following it, and how to reach azd. The same selection is answered
/// first by the supervisor and then, on fallback, from local session metadata, so it travels
/// as one request rather than as ten positional arguments.
#[derive(Debug)]
pub struct ServiceLogOptions {
    pub name: String,
    pub service: String,
    pub run: Option<Uuid>,
    pub stream: ServiceLogStreamArg,
    pub tail: usize,
    pub all: bool,
    pub follow: bool,
    pub daemon_endpoint_kind: Option<EndpointKind>,
    pub daemon_endpoint: Option<String>,
    pub path: Option<PathBuf>,
}

pub fn service_log(options: ServiceLogOptions) -> CliResult<()> {
    let ServiceLogOptions {
        name,
        service,
        run,
        stream,
        tail,
        all,
        follow,
        daemon_endpoint_kind,
        daemon_endpoint,
        path,
    } = options;
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!(
        "Reading {stream:?} log for service '{}' in session '{}' at {}",
        service,
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(&name)?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    match session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref()) {
        Ok(descriptor) => match request_session_service_log(
            &manifest,
            &descriptor,
            &service,
            run,
            stream,
            tail,
            all,
            None,
        ) {
            Ok(result) => {
                print_proto_service_log(&result)?;
                if follow {
                    follow_session_service_log(&result.path, stream, result.next_offset)?;
                }
                return Ok(());
            }
            Err(error) if can_fallback_after_supervisor_error(daemon_endpoint.as_ref(), &error) => {
                info!(
                    error = %error,
                    session = %manifest.slug,
                    service = %service,
                    "session-supervisor service log status failed; falling back to local session metadata"
                );
            }
            Err(error) => return Err(error),
        },
        Err(CliError::MissingSessionService { .. })
            if can_fallback_to_local_session(daemon_endpoint.as_ref()) => {}
        Err(error) => return Err(error),
    }

    let selection = select_local_service_log(&manifest, &service, run, stream)?;
    print_service_log(&selection, tail, all, follow)
}

pub fn recover(
    name: &str,
    force: bool,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));

    info!(
        "Recovering failed-preserved session '{}' at {}",
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let existing = manager.session(name)?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    let manifest =
        match session_supervisor_descriptor_for_command(&existing, daemon_endpoint.as_ref()) {
            Ok(supervisor) => match request_recover_session(&existing, &supervisor, force) {
                Ok(manifest) => {
                    println!("Session '{}' recovered", manifest.slug);
                    print_proto_manifest_summary(&manifest);
                    return Ok(());
                }
                Err(error)
                    if can_fallback_after_supervisor_error(daemon_endpoint.as_ref(), &error) =>
                {
                    info!(
                        error = %error,
                        session = %existing.slug,
                        "session-supervisor recover failed; falling back to local session metadata"
                    );
                    manager.recover_session(name, force)?
                }
                Err(error) => return Err(error),
            },
            Err(CliError::MissingSessionService { .. })
                if can_fallback_to_local_session(daemon_endpoint.as_ref()) =>
            {
                manager.recover_session(name, force)?
            }
            Err(error) => return Err(error),
        };

    println!("Session '{}' recovered", manifest.slug);
    print_manifest_summary(&manifest);
    Ok(())
}

pub fn register_runtime_host(
    name: &str,
    endpoint: String,
    kind: EndpointKind,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let manager = SessionManager::new(&project_path)?;
    validate_public_endpoint_kind(kind, "manual session service registration")?;
    let endpoint = Endpoint::new(kind, endpoint);
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    let supervisor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;
    let descriptor =
        runtime_host_service_descriptor(manifest.id.0, Uuid::now_v7(), endpoint.clone());
    let updated = request_register_service(&manifest, &supervisor, &descriptor)?;

    println!("Runtime-host registered for session '{}'", updated.slug);
    print_service_registration(RUNTIME_HOST_SERVICE_NAME, &endpoint);
    print_proto_manifest_summary(&updated);
    Ok(())
}

pub fn save_document(
    name: &str,
    document: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let (manifest, result) = execute_source_session_lifecycle(
        name,
        document,
        az_proto_project::vnext::SourceSessionCommand::Save,
        None,
        daemon_endpoint_kind,
        daemon_endpoint,
        path,
    )?;
    print_source_session_result(&manifest, document, "saved", &result);
    Ok(())
}

fn execute_source_session_lifecycle(
    name: &str,
    source_path: &str,
    command: az_proto_project::vnext::SourceSessionCommand,
    expected_revision: Option<u64>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<(
    SessionManifest,
    az_proto_project::vnext::SourceSessionResult,
)> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;
    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let supervisor_descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let result = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&supervisor_descriptor).await?;
        let project_host = resolve_session_service(
            &supervisor,
            &supervisor_descriptor,
            &manifest,
            project_host_service_id(),
            ServiceRole::ProjectHost,
        )
        .await?;
        let expected_revision = match command {
            az_proto_project::vnext::SourceSessionCommand::Open
            | az_proto_project::vnext::SourceSessionCommand::Status => 0,
            _ => match expected_revision {
                Some(revision) => revision,
                None => {
                    request_source_session_lifecycle_on_host(
                        &project_host,
                        &manifest,
                        source_path,
                        az_proto_project::vnext::SourceSessionCommand::Status,
                        0,
                    )
                    .await?
                    .status
                    .revision
                }
            },
        };
        request_source_session_lifecycle_on_host(
            &project_host,
            &manifest,
            source_path,
            command,
            expected_revision,
        )
        .await
    })?;
    ensure_source_session_result(source_path, command, &result)?;
    Ok((manifest, result))
}
pub fn workspace_entries(
    name: &str,
    after_entry_id: Option<i64>,
    page_size: u32,
    all_pages: bool,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Reading asset status for session '{}' at {}",
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;
    let page_size = validate_workspace_entry_paging(after_entry_id, page_size)?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let result = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        if all_pages {
            request_all_workspace_entry_pages(
                &asset_processor,
                &manifest,
                after_entry_id,
                page_size,
            )
            .await
        } else {
            request_workspace_entry_page(&asset_processor, &manifest, after_entry_id, page_size)
                .await
        }
    })?;

    print_workspace_entry_page(&result);
    Ok(())
}

pub fn record_source_assets(
    name: &str,
    source_paths: &[String],
    schema_type: &str,
    source_root: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;
    if source_root.trim().is_empty() || source_root.trim() != source_root {
        return Err(asset_processor_authority_mismatch(
            "recordSourceAsset",
            "source root must be non-empty and trimmed".to_string(),
        ));
    }
    if schema_type.trim().is_empty() || schema_type.trim() != schema_type {
        return Err(asset_processor_authority_mismatch(
            "recordSourceAsset",
            "schema type must be non-empty and trimmed".to_string(),
        ));
    }
    for source_path in source_paths {
        ensure_cli_asset_db_relative_path(source_path, "recordSourceAsset", "source path")?;
    }

    info!(
        session = %name,
        source_root = %source_root,
        schema_type = %schema_type,
        source_count = source_paths.len(),
        project = %project_path.display(),
        "recording existing source assets"
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let results = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        let snapshot = request_workspace_snapshot(&asset_processor, &manifest)
            .await?
            .ok_or_else(|| {
                asset_processor_authority_mismatch(
                    "recordSourceAsset",
                    format!("session `{name}` has no attached workspace snapshot"),
                )
            })?;
        let workspace_source_root = requested_workspace_source_root(&snapshot, source_root)?;
        request_import_existing_source_assets(
            &asset_processor,
            &manifest,
            workspace_source_root,
            source_root,
            schema_type,
            source_paths,
        )
        .await
    })?;

    for (source_path, result) in source_paths.iter().zip(results) {
        println!(
            "Recorded '{}' as {} (asset {})",
            source_path, schema_type, result.record.asset_guid
        );
    }
    Ok(())
}

pub fn force_reprocess_assets(
    name: &str,
    source_paths: &[String],
    source_root: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;
    if source_root.trim().is_empty() || source_root.trim() != source_root {
        return Err(asset_processor_authority_mismatch(
            "forceReprocessAsset",
            "source root must be non-empty and trimmed".to_string(),
        ));
    }
    for source_path in source_paths {
        ensure_cli_asset_db_relative_path(source_path, "forceReprocessAsset", "source path")?;
    }

    info!(
        session = %name,
        source_root = %source_root,
        source_count = source_paths.len(),
        project = %project_path.display(),
        "force-reprocessing source assets"
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let results = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        request_force_reprocess_assets(&asset_processor, &manifest, source_root, source_paths).await
    })?;

    for (source_path, result) in source_paths.iter().zip(results) {
        println!(
            "Reprocessing '{}': {} job(s) enqueued (asset {}, entry {}, visible jobs {})",
            source_path,
            result.enqueued_jobs,
            result.record.asset_guid,
            result.record.entry.entry_id,
            result.record.entry.jobs.len(),
        );
    }
    Ok(())
}

pub fn workspace_snapshot(
    name: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Reading attached workspace snapshot for session '{}' at {}",
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let snapshot = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        request_workspace_snapshot(&asset_processor, &manifest).await
    })?;

    print_workspace_snapshot(snapshot.as_ref());
    Ok(())
}

pub fn reconcile_asset_sources(
    name: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let reconciled = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        let client: asset_capnp::asset_processor::Client =
            az_rpc::connect_twoparty_bootstrap(&asset_processor.endpoint).await?;
        let capability = asset_write_capability(&manifest, &asset_processor)?;
        request_reconcile_asset_sources(&client, &manifest, &capability).await
    })?;

    println!("Reconciled asset sources for '{name}'");
    println!("  source_roots: {}", reconciled.source_root_count);
    println!(
        "  recorded_sources: {}",
        reconciled.recorded_source_asset_count
    );
    println!(
        "  deleted_sources: {}",
        reconciled.deleted_source_asset_count
    );
    Ok(())
}

pub fn asset_processing_status(
    name: &str,
    platform: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let status = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        let client: asset_capnp::asset_processor::Client =
            az_rpc::connect_twoparty_bootstrap(&asset_processor.endpoint).await?;
        let capability = asset_write_capability(&manifest, &asset_processor)?;
        let mut request = client.processing_status_request();
        (AssetProcessingStatusRequest {
            capability,
            session_id: manifest.id.to_string(),
            platform: platform.to_string(),
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        Ok::<_, CliError>(AssetProcessingStatusResult::from_capnp(
            response.get()?.get_result()?,
        ))
    })?;

    println!("Asset processing status for '{name}' ({platform})");
    println!("  queued: {}", status.queued);
    println!("  leased: {}", status.leased);
    println!("  failed: {}", status.failed);
    println!("  active: {}", status.active());
    Ok(())
}

pub fn asset_health(
    name: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let health = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        let client: asset_capnp::asset_processor::Client =
            az_rpc::connect_twoparty_bootstrap(&asset_processor.endpoint).await?;
        let response = client.health_request().send().promise.await?;
        let health = ServiceHealth::from_capnp(response.get()?.get_health()?)?;
        health
            .require_protocol_version(ProtocolVersion::CURRENT)
            .map_err(|error| CliError::AssetProcessorAuthorityMismatch {
                operation: "health",
                reason: format!("asset-processor must be restarted: {error}"),
            })?;
        Ok::<_, CliError>(health)
    })?;

    println!("Asset processor health for '{name}'");
    println!("  state: {:?}", health.state);
    println!("  ready: {}", health.ready);
    println!("  degraded: {}", health.degraded);
    println!("  run: {}", health.run);
    println!("  uptime_ms: {}", health.uptime_ms);
    if !health.active_operation.is_empty() {
        println!("  active_operation: {}", health.active_operation);
    }
    println!("  message: {}", health.message);
    Ok(())
}

pub fn asset_builders(
    name: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Reading asset builders for session '{}' at {}",
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let catalog = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        request_asset_builder_catalog(&asset_processor, &manifest).await
    })?;

    print_asset_builder_catalog(&catalog);
    Ok(())
}

pub fn catalog_products(
    name: &str,
    platform: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Reading catalog products for session '{}' platform '{}' at {}",
        name,
        platform,
        project_path.display()
    );

    let entries =
        catalog_products_for_session(&project_path, name, platform, daemon_endpoint.as_ref())?;

    print_catalog_products(platform, &entries);
    Ok(())
}

pub fn catalog_products_for_session(
    project_path: &Path,
    name: &str,
    platform: &str,
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
) -> CliResult<Vec<CatalogProductEntry>> {
    let manager = SessionManager::new(project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor = session_supervisor_descriptor_for_command(&manifest, daemon_endpoint)?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        request_catalog_products(&asset_processor, &manifest, platform).await
    })
}

/// Force-publish `assetcatalog.bin` for a session/platform into the
/// project-instance product cache (`Cache/<platform>`).
pub fn publish_asset_catalog(
    name: &str,
    platform: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        session = %name,
        platform = %platform,
        project = %project_path.display(),
        "force-publishing runtime asset catalog"
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let receipt: PublishAssetCatalogResult = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        let client: asset_capnp::asset_processor::Client =
            az_rpc::connect_twoparty_bootstrap(&asset_processor.endpoint).await?;
        let capability = asset_write_capability(&manifest, &asset_processor)?;
        let mut publish = client.publish_asset_catalog_request();
        (PublishAssetCatalogRequest {
            capability,
            session_id: manifest.id.to_string(),
            platform: platform.to_string(),
        })
        .to_capnp(publish.get().init_request())?;
        let response = publish.send().promise.await?;
        Ok::<_, CliError>(PublishAssetCatalogResult::from_capnp(
            response.get()?.get_result()?,
        )?)
    })?;

    info!(
        session = %name,
        platform = %platform,
        catalog_path = %receipt.catalog_path,
        entry_count = receipt.entry_count,
        "force-published runtime asset catalog"
    );
    println!(
        "Published asset catalog: {} entries -> {}",
        receipt.entry_count, receipt.catalog_path
    );
    Ok(())
}

pub fn process_assets(
    name: &str,
    platform: &str,
    skip_build: bool,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<&str>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    if skip_build {
        crate::commands::host_tools::require_prebuilt_project_host_tools()?;
        crate::commands::daemon::start_prebuilt(
            std::slice::from_ref(&project_path),
            None,
            crate::commands::daemon::DEFAULT_DAEMON_START_TIMEOUT_MS,
            daemon_endpoint_kind,
            daemon_endpoint,
        )?;
    } else {
        crate::commands::host_tools::ensure_project_host_tools()?;
        crate::commands::daemon::start(
            std::slice::from_ref(&project_path),
            None,
            crate::commands::daemon::DEFAULT_DAEMON_START_TIMEOUT_MS,
            daemon_endpoint_kind,
            daemon_endpoint,
        )?;
    }
    let resolved = crate::commands::daemon::optional_project_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
        &project_path,
    )?
    .ok_or(CliError::MissingDaemonEndpoint {
        operation: "project asset processing",
    })?;
    let (session, catalog) =
        process_project_assets(&project_path, Some(name), platform, &resolved, skip_build)?;
    println!(
        "Processed assets for session '{session}': {} entries -> {}",
        catalog.entry_count, catalog.catalog_path
    );
    Ok(())
}

#[instrument(
    skip(project_path, daemon_endpoint),
    fields(platform, requested_session)
)]
pub fn process_project_assets(
    project_path: &Path,
    requested_session: Option<&str>,
    platform: &str,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
    services_prebuilt: bool,
) -> CliResult<(String, PublishAssetCatalogResult)> {
    let manager = SessionManager::new(project_path)?;
    let manifest = resolve_asset_processing_session(&manager, requested_session)?;

    info!(
        session = %manifest.slug,
        workspace = %manifest.workspace_root.display(),
        platform,
        "preparing asset processing services"
    );
    let services = ensure_asset_processing_services_through_daemon(
        &manifest,
        services_prebuilt,
        u64::try_from(BUILD_ASSET_PROCESSING_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
        daemon_endpoint,
    )?;

    let manifest = manager.session(&manifest.slug)?;
    let descriptor = services.supervisor;
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let platform = platform.to_string();
    let (receipt, terminal_status) = local.block_on(
        &runtime,
        reconcile_and_publish_asset_catalog(&descriptor, &manifest, &platform),
    )?;
    if terminal_status.failed > 0 {
        info!(
            session = %manifest.slug,
            platform,
            catalog_path = %receipt.catalog_path,
            entry_count = receipt.entry_count,
            failed = terminal_status.failed,
            "published coherent runtime asset catalog before reporting asset failures"
        );
        return Err(CliError::AssetProcessingFailed {
            session: manifest.slug,
            platform,
            failed: terminal_status.failed,
        });
    }
    info!(
        session = %manifest.slug,
        platform,
        catalog_path = %receipt.catalog_path,
        entry_count = receipt.entry_count,
        "project asset cache is runtime-ready"
    );
    Ok((manifest.slug, receipt))
}

/// Resolves the session asset processing runs in: the caller's session when named, otherwise the
/// default build asset session, created on first use.
fn resolve_asset_processing_session(
    manager: &SessionManager,
    requested_session: Option<&str>,
) -> CliResult<SessionManifest> {
    let session_name = requested_session.unwrap_or(BUILD_ASSET_SESSION);
    let manifest = match manager.session(session_name) {
        Ok(manifest) => {
            ensure_active(&manifest)?;
            manifest
        }
        Err(SessionError::SessionNotFound(_)) => {
            manager.create_session(CreateSessionRequest::new(session_name))?
        }
        Err(error) => return Err(error.into()),
    };
    if requested_session.is_none() && manifest.workspace_root != manager.project_root() {
        return Err(CliError::InvalidServicePlan {
            message: format!(
                "default build asset session `{}` is not rooted in the project checkout `{}`",
                manifest.slug,
                manager.project_root().display()
            ),
        });
    }
    Ok(manifest)
}

/// Reconciles asset sources, waits for the processor to park at idle, then publishes the runtime
/// asset catalog. Returns the publish receipt alongside the terminal processing status.
// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn reconcile_and_publish_asset_catalog(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    platform: &str,
) -> CliResult<(PublishAssetCatalogResult, AssetProcessingStatusResult)> {
    let supervisor = connect_session_supervisor(descriptor).await?;
    let asset_processor = resolve_session_service(
        &supervisor,
        descriptor,
        manifest,
        asset_processor_service_id(),
        ServiceRole::AssetProcessor,
    )
    .await?;
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&asset_processor.endpoint).await?;
    let capability = asset_write_capability(manifest, &asset_processor)?;

    let reconciled = request_reconcile_asset_sources(&client, manifest, &capability).await?;
    info!(
        source_root_count = reconciled.source_root_count,
        recorded_source_asset_count = reconciled.recorded_source_asset_count,
        deleted_source_asset_count = reconciled.deleted_source_asset_count,
        platform = %platform,
        "asset source reconciliation completed"
    );

    let terminal_status = {
        let mut wait = client.wait_for_idle_request();
        (AssetProcessingStatusRequest {
            capability: capability.clone(),
            session_id: manifest.id.to_string(),
            platform: platform.to_string(),
        })
        .to_capnp(wait.get().init_request())?;
        let response = tokio::time::timeout(BUILD_ASSET_PROCESSING_TIMEOUT, wait.send().promise)
            .await
            .map_err(|_| CliError::AssetProcessingWaitTimedOut {
                session: manifest.slug.clone(),
                platform: platform.to_string(),
                timeout_ms: u64::try_from(BUILD_ASSET_PROCESSING_TIMEOUT.as_millis())
                    .unwrap_or(u64::MAX),
            })??;
        let status = AssetProcessingStatusResult::from_capnp(response.get()?.get_result()?);
        info!(
            queued = status.queued,
            leased = status.leased,
            active = status.active(),
            failed = status.failed,
            "asset processing reached parked idle state"
        );
        status
    };

    let mut publish = client.publish_asset_catalog_request();
    (PublishAssetCatalogRequest {
        capability,
        session_id: manifest.id.to_string(),
        platform: platform.to_string(),
    })
    .to_capnp(publish.get().init_request())?;
    let response = publish.send().promise.await?;
    Ok((
        PublishAssetCatalogResult::from_capnp(response.get()?.get_result()?)?,
        terminal_status,
    ))
}

fn ensure_asset_processing_services_through_daemon(
    manifest: &SessionManifest,
    services_prebuilt: bool,
    timeout_ms: u64,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<ProjectSessionServicesStartResult> {
    ensure_project_session_services_through_daemon(
        manifest,
        services_prebuilt,
        timeout_ms,
        default_service_endpoint_kind(),
        daemon_endpoint,
        vec!["asset-processor".to_string(), "asset-worker".to_string()],
    )
}

fn ensure_project_session_services_through_daemon(
    manifest: &SessionManifest,
    services_prebuilt: bool,
    timeout_ms: u64,
    endpoint_kind: EndpointKind,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
    requested_services: Vec<String>,
) -> CliResult<ProjectSessionServicesStartResult> {
    let project_id = manifest.project_id.clone();
    let session_slug = manifest.slug.clone();
    let endpoint = daemon_endpoint.endpoint.clone();
    let request_endpoint = endpoint.clone();
    let expected_project_id = project_id.clone();
    let expected_session_slug = session_slug.clone();
    let expected_services = requested_services.clone();

    let result = crate::commands::daemon::with_daemon_progress(
        &endpoint,
        "ensuring project and session services",
        Duration::from_secs(10),
        move |client| async move {
            let mut request = client.ensure_project_session_services_with_progress_request();
            {
                let mut outer = request.get().init_request();
                (EnsureProjectSessionServicesRequest {
                    capability: crate::commands::daemon::daemon_capability(
                        DAEMON_SESSIONS_PERMISSION,
                    ),
                    project_id,
                    session_name: session_slug,
                    endpoint_kind,
                    skip_build: services_prebuilt,
                    start_service_names: requested_services,
                    timeout_ms,
                    daemon_endpoint: request_endpoint,
                })
                .to_capnp(outer.reborrow().init_request())?;
                outer.set_progress_sink(capnp_rpc::new_client(NoopProjectOpenProgressSink));
            }
            let response = request.send().promise.await?;
            Ok(ProjectSessionServicesStartResult::from_capnp(
                response.get()?.get_result()?,
            )?)
        },
    )?;

    if result.manifest.project_id != expected_project_id
        || result.manifest.slug != expected_session_slug
    {
        return Err(CliError::DaemonAuthorityMismatch {
            operation: "ensureProjectSessionServicesWithProgress",
            reason: format!(
                "azd returned session `{}` for project `{}`, expected session `{expected_session_slug}` for project `{expected_project_id}`",
                result.manifest.slug, result.manifest.project_id
            ),
        });
    }
    for service in &expected_services {
        if !result
            .running_service_names
            .iter()
            .any(|running| running == service)
        {
            return Err(CliError::InvalidServicePlan {
                message: format!(
                    "azd did not report requested service `{service}` running for session `{expected_session_slug}`"
                ),
            });
        }
    }

    Ok(result)
}

pub fn runtime_projections(
    name: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    ensure_runtime_host_service_started(
        &project_path,
        name,
        daemon_endpoint_kind,
        daemon_endpoint.clone(),
    )?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Reading runtime projections for session '{}' at {}",
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let catalog = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let runtime_host =
            wait_for_runtime_host_service_ready(&supervisor, &descriptor, &manifest).await?;
        request_runtime_projection_catalog(&runtime_host, &manifest).await
    })?;

    print_runtime_projection_catalog(&catalog);
    Ok(())
}

pub fn inspect_job(
    name: &str,
    job_id: Option<i64>,
    attempt_id: Option<i64>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let selector = inspection_selector(job_id, attempt_id)?;
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        session = %name,
        selector = ?selector,
        project = %project_path.display(),
        "inspecting asset job"
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let inspection = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        request_inspect_job(&asset_processor, &manifest, selector).await
    })?;

    match inspection {
        Some(inspection) => print_job_inspection(&inspection),
        None => println!("Asset job not found"),
    }
    Ok(())
}

fn inspection_selector(
    job_id: Option<i64>,
    attempt_id: Option<i64>,
) -> CliResult<InspectJobSelector> {
    match (job_id, attempt_id) {
        (Some(job_id), None) if job_id > 0 => Ok(InspectJobSelector::Job(job_id)),
        (None, Some(attempt_id)) if attempt_id > 0 => Ok(InspectJobSelector::Attempt(attempt_id)),
        (Some(_), Some(_)) => Err(CliError::InvalidServicePlan {
            message: "inspect job accepts exactly one of --job-id or --attempt-id".to_string(),
        }),
        _ => Err(CliError::InvalidServicePlan {
            message: "inspect job requires a positive --job-id or --attempt-id".to_string(),
        }),
    }
}

pub fn document_snapshot(
    name: &str,
    document: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;
    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let supervisor_descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let result = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&supervisor_descriptor).await?;
        let project_host = resolve_session_service(
            &supervisor,
            &supervisor_descriptor,
            &manifest,
            project_host_service_id(),
            ServiceRole::ProjectHost,
        )
        .await?;
        request_prefab_source_snapshot_on_host(&project_host, &manifest, document).await
    })?;
    let snapshot = ensure_prefab_source_result(document, &result)?;
    print_prefab_source_snapshot(document, snapshot);
    Ok(())
}
pub fn document_load(
    name: &str,
    document: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let (manifest, result) = execute_source_session_lifecycle(
        name,
        document,
        az_proto_project::vnext::SourceSessionCommand::Open,
        None,
        daemon_endpoint_kind,
        daemon_endpoint,
        path,
    )?;
    print_source_session_result(&manifest, document, "opened", &result);
    Ok(())
}
pub fn document_status(
    name: &str,
    document: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let (manifest, result) = execute_source_session_lifecycle(
        name,
        document,
        az_proto_project::vnext::SourceSessionCommand::Status,
        None,
        daemon_endpoint_kind,
        daemon_endpoint,
        path,
    )?;
    print_source_session_result(&manifest, document, "status", &result);
    Ok(())
}
pub fn undo_document(
    name: &str,
    document: &str,
    expected_revision: Option<u64>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let (manifest, result) = execute_source_session_lifecycle(
        name,
        document,
        az_proto_project::vnext::SourceSessionCommand::Undo,
        expected_revision,
        daemon_endpoint_kind,
        daemon_endpoint,
        path,
    )?;
    print_source_session_result(&manifest, document, "undone", &result);
    Ok(())
}
pub fn redo_document(
    name: &str,
    document: &str,
    expected_revision: Option<u64>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let (manifest, result) = execute_source_session_lifecycle(
        name,
        document,
        az_proto_project::vnext::SourceSessionCommand::Redo,
        expected_revision,
        daemon_endpoint_kind,
        daemon_endpoint,
        path,
    )?;
    print_source_session_result(&manifest, document, "redone", &result);
    Ok(())
}
pub fn create_source_file(
    name: &str,
    source_path: &str,
    schema_type: &str,
    from: Option<PathBuf>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Creating source file '{}' for schema '{}' in session '{}' at {}",
        source_path,
        schema_type,
        name,
        project_path.display()
    );

    ensure_cli_asset_db_relative_path(source_path, "createSourceFile", "source path")?;
    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;

    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let uses_payload = from.is_some();
    let result = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        let catalog = request_asset_builder_catalog(&asset_processor, &manifest).await?;
        let workflow = ensure_cli_source_file_workflow_matches_catalog(
            &catalog,
            schema_type,
            source_path,
            uses_payload,
        )?;
        request_create_source_file(
            &asset_processor,
            &manifest,
            &workflow.source_root,
            source_path,
            schema_type,
            from,
        )
        .await
    })?;

    print_source_file_create_result(&manifest, &result);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn launch_runtime(
    name: &str,
    runtime_id: &str,
    role: RuntimeRole,
    include_unsaved_journal: bool,
    launch_profile: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    ensure_runtime_host_service_started(
        &project_path,
        name,
        daemon_endpoint_kind,
        daemon_endpoint.clone(),
    )?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Launching runtime '{}' for session '{}' at {}",
        runtime_id,
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let status = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let asset_processor = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
        )
        .await?;
        let asset_snapshot = required_workspace_snapshot(&asset_processor, &manifest).await?;
        ensure_runtime_launch_workspace_snapshot(&manifest, &asset_snapshot)?;
        let asset_package_roots =
            request_runtime_asset_package_roots(&supervisor, &descriptor, &manifest).await?;
        info!(
            session = %manifest.slug,
            package_roots = asset_package_roots.len(),
            "resolved runtime asset package roots for launch"
        );
        let runtime_host =
            wait_for_runtime_host_service_ready(&supervisor, &descriptor, &manifest).await?;
        let project_host = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            project_host_service_id(),
            ServiceRole::ProjectHost,
        )
        .await?;
        let snapshot = request_runtime_launch_snapshot(
            &project_host,
            &runtime_host,
            &manifest,
            role,
            &asset_snapshot,
            &asset_package_roots,
            include_unsaved_journal,
            launch_profile,
        )
        .await?;
        launch_runtime_on_host(&runtime_host, &manifest, runtime_id, role, snapshot).await
    })?;

    println!("Runtime launched through session '{}'", manifest.slug);
    print_runtime_status(&status);
    Ok(())
}

fn ensure_runtime_host_service_started(
    project_path: &Path,
    session: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
) -> CliResult<()> {
    let manager = SessionManager::new(project_path)?;
    let manifest = manager.session(session)?;
    ensure_active(&manifest)?;

    if runtime_host_service_plan_needs_prepare(&manifest) {
        prepare_services(PrepareServicesOptions {
            name: session.to_string(),
            kind: None,
            recover: false,
            otlp_endpoint: None,
            daemon_endpoint_kind,
            daemon_endpoint: daemon_endpoint.clone(),
            path: Some(project_path.to_path_buf()),
            service_names: Vec::new(),
        })?;
    }

    start_services(StartServicesOptions {
        name: session.to_string(),
        session_supervisor_kind: None,
        session_supervisor_endpoint: None,
        otlp_endpoint: None,
        timeout_ms: RUNTIME_HOST_START_TIMEOUT_MS,
        daemon_endpoint_kind,
        daemon_endpoint,
        path: Some(project_path.to_path_buf()),
        service_names: vec![RUNTIME_HOST_SERVICE_NAME.to_string()],
    })
}

fn runtime_host_service_plan_needs_prepare(manifest: &SessionManifest) -> bool {
    if runtime_host_service_descriptor_record(manifest).is_none() {
        return true;
    }

    matches!(
        current_runtime_host_service_process(manifest).map(|process| process.state),
        None | Some(ServiceProcessState::Failed)
    )
}

fn runtime_host_service_descriptor_record(manifest: &SessionManifest) -> Option<&ServiceRecord> {
    manifest.services.iter().find(|service| {
        service.name == RUNTIME_HOST_SERVICE_NAME
            && service.role == SupervisedServiceRole::RuntimeHost
    })
}

fn current_runtime_host_service_process(
    manifest: &SessionManifest,
) -> Option<&ServiceProcessRecord> {
    manifest.processes.iter().find(|process| {
        process.service_name == RUNTIME_HOST_SERVICE_NAME
            && process.role == SupervisedServiceRole::RuntimeHost
    })
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn wait_for_runtime_host_service_ready(
    supervisor: &session_capnp::session_supervisor::Client,
    supervisor_descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
) -> CliResult<ServiceDescriptor> {
    let runtime_host = resolve_session_service(
        supervisor,
        supervisor_descriptor,
        manifest,
        runtime_host_service_id(),
        ServiceRole::RuntimeHost,
    )
    .await?;
    request_runtime_projection_catalog(&runtime_host, manifest)
        .await
        .map_err(|error| CliError::InvalidServicePlan {
            message: format!(
                "runtime-host terminal start result was not reachable for session `{}`: {error}",
                manifest.slug
            ),
        })?;
    Ok(runtime_host)
}

pub fn runtime_status(
    name: &str,
    runtime_id: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Querying runtime '{}' for session '{}' at {}",
        runtime_id,
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let status = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let runtime_host = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            runtime_host_service_id(),
            ServiceRole::RuntimeHost,
        )
        .await?;
        query_runtime_status_on_host(&runtime_host, &manifest, runtime_id).await
    })?;

    println!("Runtime status for session '{}'", manifest.slug);
    match status {
        Some(status) => print_runtime_status(&status),
        None => println!("  runtime: {runtime_id}\n  state: not-found"),
    }
    Ok(())
}

pub fn runtime_viewport_frame(
    name: &str,
    runtime_id: &str,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    ensure_runtime_host_service_started(
        &project_path,
        name,
        daemon_endpoint_kind,
        daemon_endpoint.clone(),
    )?;
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Reading runtime viewport frame for runtime '{}' in session '{}' at {}",
        runtime_id,
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let frame = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let runtime_host =
            wait_for_runtime_host_service_ready(&supervisor, &descriptor, &manifest).await?;
        request_runtime_viewport_frame(&runtime_host, &manifest, runtime_id).await
    })?;

    println!("Runtime viewport frame for session '{}'", manifest.slug);
    print_runtime_viewport_frame(runtime_id, frame.as_ref());
    Ok(())
}

pub fn stop_runtime(
    name: &str,
    runtime_id: &str,
    preserve: bool,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
    path: Option<PathBuf>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;

    info!(
        "Stopping runtime '{}' for session '{}' at {}",
        runtime_id,
        name,
        project_path.display()
    );

    let manager = SessionManager::new(&project_path)?;
    let manifest = manager.session(name)?;
    ensure_active(&manifest)?;
    let descriptor =
        session_supervisor_descriptor_for_command(&manifest, daemon_endpoint.as_ref())?;
    let runtime = session_cli_rpc_runtime()?;
    let local = LocalSet::new();
    let status = local.block_on(&runtime, async {
        let supervisor = connect_session_supervisor(&descriptor).await?;
        let runtime_host = resolve_session_service(
            &supervisor,
            &descriptor,
            &manifest,
            runtime_host_service_id(),
            ServiceRole::RuntimeHost,
        )
        .await?;
        stop_runtime_on_host(&runtime_host, &manifest, runtime_id, preserve).await
    })?;

    println!("Runtime stopped through session '{}'", manifest.slug);
    print_runtime_status(&status);
    Ok(())
}

fn ensure_active(manifest: &SessionManifest) -> CliResult<()> {
    if manifest.state == SessionState::Active {
        Ok(())
    } else {
        Err(CliError::SessionNotActive {
            session: manifest.slug.clone(),
            state: format_state(manifest.state).to_string(),
        })
    }
}

fn ensure_active_or_recovery(manifest: &SessionManifest, recover: bool) -> CliResult<()> {
    if manifest.state == SessionState::Active
        || (recover && manifest.state == SessionState::FailedPreserved)
    {
        Ok(())
    } else {
        Err(CliError::SessionNotActive {
            session: manifest.slug.clone(),
            state: format_state(manifest.state).to_string(),
        })
    }
}

fn session_supervisor_descriptor(
    manifest: &SessionManifest,
) -> CliResult<az_proto_core::ServiceDescriptor> {
    let id = ServiceId::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
    );
    manifest
        .service_descriptor(&id, ServiceRole::SessionSupervisor)
        .ok_or_else(|| CliError::MissingSessionService {
            session: manifest.slug.clone(),
            service: SESSION_SUPERVISOR_SERVICE_NAME.to_string(),
        })
}

fn session_supervisor_descriptor_for_command(
    manifest: &SessionManifest,
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
) -> CliResult<ServiceDescriptor> {
    if let Some(resolved) = daemon_endpoint {
        match session_supervisor_descriptor_through_daemon(
            &manifest.project_id,
            &manifest.slug,
            &resolved.endpoint,
        ) {
            Ok(Some(descriptor)) => return Ok(descriptor),
            Ok(None) => {
                return Err(CliError::MissingSessionService {
                    session: manifest.slug.clone(),
                    service: SESSION_SUPERVISOR_SERVICE_NAME.to_string(),
                });
            }
            Err(error)
                if resolved.source
                    == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                    && is_daemon_connection_failure(&error) =>
            {
                crate::commands::daemon::handle_stale_runtime_record(&error)?;
            }
            Err(error) => return Err(error),
        }
    }

    session_supervisor_descriptor(manifest)
}

fn live_session_supervisor_descriptor_through_daemon(
    manifest: &SessionManifest,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<ServiceDescriptor> {
    match session_supervisor_descriptor_through_daemon(
        &manifest.project_id,
        &manifest.slug,
        &daemon_endpoint.endpoint,
    ) {
        Ok(Some(descriptor)) => Ok(descriptor),
        Ok(None) => Err(CliError::MissingSessionService {
            session: manifest.slug.clone(),
            service: SESSION_SUPERVISOR_SERVICE_NAME.to_string(),
        }),
        Err(error)
            if daemon_endpoint.source
                == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                && is_daemon_connection_failure(&error) =>
        {
            crate::commands::daemon::handle_stale_runtime_record(&error)?;
            Err(error)
        }
        Err(error) => Err(error),
    }
}

fn live_proto_session_supervisor_descriptor_through_daemon(
    manifest: &ProtoSessionManifest,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<ServiceDescriptor> {
    match session_supervisor_descriptor_through_daemon(
        &manifest.project_id,
        &manifest.slug,
        &daemon_endpoint.endpoint,
    ) {
        Ok(Some(descriptor)) => Ok(descriptor),
        Ok(None) => Err(CliError::MissingSessionService {
            session: manifest.slug.clone(),
            service: SESSION_SUPERVISOR_SERVICE_NAME.to_string(),
        }),
        Err(error)
            if daemon_endpoint.source
                == crate::commands::daemon::DaemonEndpointSource::RuntimeRecord
                && is_daemon_connection_failure(&error) =>
        {
            crate::commands::daemon::handle_stale_runtime_record(&error)?;
            Err(error)
        }
        Err(error) => Err(error),
    }
}

fn session_supervisor_descriptor_through_daemon(
    project_id: &str,
    session_slug: &str,
    endpoint: &Endpoint,
) -> CliResult<Option<ServiceDescriptor>> {
    let project_id = project_id.to_string();
    let session_slug = session_slug.to_string();
    let expected_session_slug = session_slug.clone();
    crate::commands::daemon::with_daemon(endpoint, async move |client| {
        let mut request = client.resolve_session_supervisor_request();
        (ResolveSessionSupervisorRequest {
            capability: crate::commands::daemon::daemon_capability(DAEMON_READ_PERMISSION),
            project_id,
            session_slug,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let descriptor =
            SessionSupervisorResult::from_capnp(response.get()?.get_result()?)?.descriptor;
        if let Some(descriptor) = &descriptor {
            crate::commands::daemon::ensure_daemon_session_supervisor_descriptor_matches_request(
                descriptor,
                &expected_session_slug,
                "resolveSessionSupervisor",
            )?;
        }
        Ok(descriptor)
    })
}

fn session_supervisor_descriptors_through_daemon(
    project_id: &str,
    endpoint: &Endpoint,
) -> CliResult<Vec<az_proto_daemon::SessionSupervisorDescriptor>> {
    let project_id = project_id.to_string();
    crate::commands::daemon::with_daemon(endpoint, async move |client| {
        let mut request = client.list_session_supervisors_request();
        (ListSessionSupervisorsRequest {
            capability: crate::commands::daemon::daemon_capability(DAEMON_READ_PERMISSION),
            project_id,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let supervisors =
            ListSessionSupervisorsResult::from_capnp(response.get()?.get_result()?)?.supervisors;
        crate::commands::daemon::ensure_daemon_session_supervisor_list_is_authoritative(
            &supervisors,
        )?;
        Ok(supervisors)
    })
}

const fn is_daemon_connection_failure(error: &CliError) -> bool {
    matches!(error, CliError::RpcTransport(_))
}

async fn connect_session_supervisor(
    descriptor: &ServiceDescriptor,
) -> CliResult<session_capnp::session_supervisor::Client> {
    validate_session_supervisor_descriptor(descriptor, "connect session-supervisor")?;
    Ok(az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
#[allow(clippy::too_many_arguments)]
async fn request_service_log_chunk(
    supervisor: &session_capnp::session_supervisor::Client,
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    service: &str,
    run: Option<Uuid>,
    stream: ServiceLogStreamArg,
    tail: usize,
    all: bool,
    offset: Option<u64>,
) -> CliResult<ProtoServiceLogResult> {
    let mut request = supervisor.service_log_request();
    (ProtoServiceLogRequest {
        capability: session_read_capability(manifest, descriptor)?,
        slug: manifest.slug.clone(),
        service_name: service.to_string(),
        run,
        stream: proto_service_log_stream(stream),
        all,
        tail_lines: u32::try_from(tail).unwrap_or(u32::MAX),
        offset,
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = ProtoServiceLogResult::from_capnp(response.get()?.get_result()?)?;
    ensure_cli_service_log_result_matches_request(&result, manifest, service, run, stream, offset)?;
    Ok(result)
}

fn ensure_cli_service_log_result_matches_request(
    result: &ProtoServiceLogResult,
    manifest: &SessionManifest,
    service: &str,
    run: Option<Uuid>,
    stream: ServiceLogStreamArg,
    offset: Option<u64>,
) -> CliResult<()> {
    if result.session_slug != manifest.slug {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            format!(
                "returned session `{}`, expected `{}`",
                result.session_slug, manifest.slug
            ),
        ));
    }
    if result.service_name != service {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            format!(
                "returned service `{}`, expected `{service}`",
                result.service_name
            ),
        ));
    }
    if let Some(expected_run) = run {
        if result.run != expected_run {
            return Err(session_supervisor_authority_mismatch(
                "serviceLog",
                format!("returned run {}, expected {expected_run}", result.run),
            ));
        }
    } else if result.run == Uuid::nil() {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            "returned run must not be nil".to_string(),
        ));
    }
    let expected_stream = proto_service_log_stream(stream);
    if result.stream != expected_stream {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            format!(
                "returned {} stream, expected {}",
                proto_service_log_stream_label(result.stream),
                service_log_stream_label(stream)
            ),
        ));
    }
    if result.next_offset < offset.unwrap_or_default() {
        return Err(session_supervisor_authority_mismatch(
            "serviceLog",
            format!(
                "returned next offset {} before requested offset {}",
                result.next_offset,
                offset.unwrap_or_default()
            ),
        ));
    }
    let path = PathBuf::from(&result.path);
    if result.path.trim().is_empty() {
        return Err(CliError::InvalidServiceLogPath(Box::new(
            InvalidServiceLogPathDetails {
                session: manifest.slug.clone(),
                service: service.to_string(),
                path,
                run_dir: manifest.run_dir.clone(),
            },
        )));
    }
    let path = if path.is_absolute() {
        path
    } else {
        manifest.run_dir.join(path)
    };
    if path_has_parent_component(&path) || !path.starts_with(&manifest.run_dir) {
        return Err(CliError::InvalidServiceLogPath(Box::new(
            InvalidServiceLogPathDetails {
                session: manifest.slug.clone(),
                service: service.to_string(),
                path,
                run_dir: manifest.run_dir.clone(),
            },
        )));
    }
    Ok(())
}

const fn session_supervisor_authority_mismatch(
    operation: &'static str,
    reason: String,
) -> CliError {
    CliError::SessionSupervisorAuthorityMismatch { operation, reason }
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn resolve_session_service(
    supervisor: &session_capnp::session_supervisor::Client,
    supervisor_descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    id: ServiceId,
    role: ServiceRole,
) -> CliResult<ServiceDescriptor> {
    let expected = required_manifest_service_descriptor(manifest, &id, role)?;
    ensure_manifest_service_resolution_state(manifest, &expected)?;

    let mut request = supervisor.resolve_service_request();
    (ProtoResolveServiceRequest {
        capability: session_read_capability(manifest, supervisor_descriptor)?,
        slug: manifest.slug.clone(),
        id: id.clone(),
        role,
    })
    .to_capnp(request.get())?;

    let response = request.send().promise.await?;
    let descriptor = ServiceDescriptor::from_capnp(response.get()?.get_descriptor()?)?;
    validate_service_descriptor(&descriptor, &id, role, "resolveService")?;
    if !descriptor.has_same_connection_contract(&expected) {
        return Err(session_supervisor_authority_mismatch(
            "resolveService",
            format!(
                "{} from resolveService endpoint {:?} `{}` does not match the durable session manifest endpoint {:?} `{}` for session `{}`",
                service_descriptor_label(&expected.id, expected.role),
                descriptor.endpoint.kind,
                descriptor.endpoint.address,
                expected.endpoint.kind,
                expected.endpoint.address,
                manifest.slug
            ),
        ));
    }
    Ok(descriptor)
}

fn ensure_manifest_service_resolution_state(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<()> {
    match descriptor.role {
        // These descriptors are attachments to processes owned by the durable
        // project-service manifest. A session manifest is forbidden from
        // carrying their process records, so requiring one here would invert
        // the ownership boundary established by ADR 0039.
        ServiceRole::ProjectHost | ServiceRole::AssetProcessor | ServiceRole::Worker => Ok(()),
        ServiceRole::RuntimeHost => ensure_manifest_service_process_running(manifest, descriptor),
        role => Err(CliError::InvalidServicePlan {
            message: format!(
                "{} cannot be resolved as a session dependency for role {role:?}",
                service_descriptor_label(&descriptor.id, role)
            ),
        }),
    }
}

fn required_manifest_service_descriptor(
    manifest: &SessionManifest,
    id: &ServiceId,
    role: ServiceRole,
) -> CliResult<ServiceDescriptor> {
    manifest
        .service_descriptor(id, role)
        .ok_or_else(|| CliError::MissingSessionService {
            session: manifest.slug.clone(),
            service: service_descriptor_label(id, role),
        })
}

fn ensure_manifest_service_process_running(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<()> {
    let Some(process) = manifest.processes.iter().find(|process| {
        process.service_name == descriptor.id.name && process.role.to_proto() == descriptor.role
    }) else {
        return Err(CliError::SessionServiceNotRunning(Box::new(
            SessionServiceNotRunningDetails {
                session: manifest.slug.clone(),
                service: service_descriptor_label(&descriptor.id, descriptor.role),
                state: "missing process record".to_string(),
            },
        )));
    };

    if process.state != ServiceProcessState::Running {
        return Err(CliError::SessionServiceNotRunning(Box::new(
            SessionServiceNotRunningDetails {
                session: manifest.slug.clone(),
                service: service_descriptor_label(&descriptor.id, descriptor.role),
                state: format_process_state(process.state).to_string(),
            },
        )));
    }

    if process.endpoint_kind.to_proto() != descriptor.endpoint.kind
        || process.endpoint_address != descriptor.endpoint.address
    {
        return Err(session_supervisor_authority_mismatch(
            "resolveService",
            format!(
                "{} descriptor endpoint {:?} `{}` does not match running process endpoint {:?} `{}` for session `{}`",
                service_descriptor_label(&descriptor.id, descriptor.role),
                descriptor.endpoint.kind,
                descriptor.endpoint.address,
                process.endpoint_kind.to_proto(),
                process.endpoint_address,
                manifest.slug
            ),
        ));
    }
    Ok(())
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_workspace_snapshot(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
) -> CliResult<Option<WorkspaceSnapshot>> {
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.workspace_snapshot_request();
    (WorkspaceSnapshotRequest {
        capability: asset_read_capability(manifest, descriptor)?,
        root_scope: AssetRootScope::All,
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = WorkspaceSnapshotResult::from_capnp(response.get()?.get_result()?)?;
    if let Some(snapshot) = &result.snapshot {
        ensure_cli_workspace_snapshot_matches_manifest(snapshot, manifest)?;
    }
    Ok(result.snapshot)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn required_workspace_snapshot(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
) -> CliResult<WorkspaceSnapshot> {
    request_workspace_snapshot(descriptor, manifest)
        .await?
        .ok_or_else(|| CliError::InvalidServicePlan {
            message: format!(
                "asset processor has no attached workspace snapshot for session `{}`",
                manifest.slug
            ),
        })
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_workspace_entry_page(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    after_entry_id: Option<i64>,
    page_size: u32,
) -> CliResult<WorkspaceEntryPageResult> {
    let snapshot = required_workspace_snapshot(descriptor, manifest).await?;
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    request_workspace_entry_page_on_client(
        &client,
        descriptor,
        manifest,
        snapshot.workspace_id,
        after_entry_id,
        page_size,
    )
    .await
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_all_workspace_entry_pages(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    after_entry_id: Option<i64>,
    page_size: u32,
) -> CliResult<WorkspaceEntryPageResult> {
    let snapshot = required_workspace_snapshot(descriptor, manifest).await?;
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut cursor = after_entry_id;
    let mut entries = Vec::new();

    loop {
        let page = request_workspace_entry_page_on_client(
            &client,
            descriptor,
            manifest,
            snapshot.workspace_id,
            cursor,
            page_size,
        )
        .await?;
        let next = next_workspace_entry_cursor(cursor, &page)?;
        entries.extend(page.entries);

        let Some(next) = next else {
            break;
        };
        cursor = Some(next);
    }

    Ok(WorkspaceEntryPageResult {
        entries,
        next_after_entry_id: None,
    })
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_workspace_entry_page_on_client(
    client: &asset_capnp::asset_processor::Client,
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    expected_workspace_id: i64,
    after_entry_id: Option<i64>,
    page_size: u32,
) -> CliResult<WorkspaceEntryPageResult> {
    let mut request = client.workspace_entry_page_request();
    (WorkspaceEntryPageRequest {
        capability: asset_read_capability(manifest, descriptor)?,
        root_scope: AssetRootScope::All,
        after_entry_id,
        page_size,
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = WorkspaceEntryPageResult::from_capnp(response.get()?.get_result()?)?;
    ensure_cli_workspace_entry_page_matches_request(
        &result,
        expected_workspace_id,
        after_entry_id,
        page_size,
    )?;
    Ok(result)
}

fn next_workspace_entry_cursor(
    after_entry_id: Option<i64>,
    page: &WorkspaceEntryPageResult,
) -> CliResult<Option<i64>> {
    let Some(next) = page.next_after_entry_id else {
        return Ok(None);
    };

    if after_entry_id.is_some_and(|after| next <= after) {
        return Err(CliError::InvalidAssetStatusPage {
            message: format!(
                "asset processor returned non-advancing workspace entry cursor {next} after {after_entry_id:?}"
            ),
        });
    }

    Ok(Some(next))
}

fn validate_workspace_entry_paging(after_entry_id: Option<i64>, page_size: u32) -> CliResult<u32> {
    if page_size == 0 {
        return Err(CliError::InvalidAssetStatusPage {
            message: "workspace entry page size must be greater than zero".to_string(),
        });
    }
    if let Some(after) = after_entry_id
        && after <= 0
    {
        return Err(CliError::InvalidAssetStatusPage {
            message: format!("workspace entry --after cursor must be positive, got {after}"),
        });
    }

    Ok(page_size)
}

fn ensure_cli_workspace_snapshot_matches_manifest(
    snapshot: &WorkspaceSnapshot,
    manifest: &SessionManifest,
) -> CliResult<()> {
    ensure_cli_workspace_snapshot_identity(snapshot, manifest)?;
    ensure_cli_workspace_snapshot_roots_are_distinct(snapshot)?;
    ensure_cli_workspace_snapshot_project_assets_root(snapshot, manifest)
}

/// Checks the snapshot header: positive DB id, matching project/workspace identity, sane
/// timestamps, and at least one source root.
fn ensure_cli_workspace_snapshot_identity(
    snapshot: &WorkspaceSnapshot,
    manifest: &SessionManifest,
) -> CliResult<()> {
    if snapshot.workspace_id <= 0 {
        return Err(asset_processor_authority_mismatch(
            "workspaceSnapshot",
            "workspace snapshot must carry a positive DB id".to_string(),
        ));
    }
    if snapshot.project_id != manifest.project_id
        || snapshot.workspace_root != manifest.workspace_root.to_string_lossy()
    {
        return Err(asset_processor_authority_mismatch(
            "workspaceSnapshot",
            format!(
                "asset-processor returned project `{}` workspace `{}` for session `{}` project `{}` workspace `{}`",
                snapshot.project_id,
                snapshot.workspace_root,
                manifest.slug,
                manifest.project_id,
                manifest.workspace_root.display()
            ),
        ));
    }
    if snapshot.branch.trim().is_empty()
        || snapshot.created_unix_ms < 0
        || snapshot.updated_unix_ms < snapshot.created_unix_ms
    {
        return Err(asset_processor_authority_mismatch(
            "workspaceSnapshot",
            format!(
                "workspace snapshot {} has invalid identity metadata",
                snapshot.workspace_id
            ),
        ));
    }
    if snapshot.roots.is_empty() {
        return Err(asset_processor_authority_mismatch(
            "workspaceSnapshot",
            format!(
                "workspace snapshot {} has no source roots",
                snapshot.workspace_id
            ),
        ));
    }
    Ok(())
}

/// Checks every source root carries complete identity and that workspace root ids and portable
/// keys are unique within the snapshot.
fn ensure_cli_workspace_snapshot_roots_are_distinct(snapshot: &WorkspaceSnapshot) -> CliResult<()> {
    let mut root_ids = BTreeSet::new();
    let mut portable_keys = BTreeSet::new();
    for root in &snapshot.roots {
        if root.workspace_id != snapshot.workspace_id
            || root.workspace_root_id <= 0
            || root.root_id <= 0
            || root.owner_id.trim().is_empty()
            || root.source_root.trim().is_empty()
            || root.portable_key.trim().is_empty()
        {
            return Err(asset_processor_authority_mismatch(
                "workspaceSnapshot",
                format!(
                    "workspace snapshot {} contains an invalid source root",
                    snapshot.workspace_id
                ),
            ));
        }
        if !root_ids.insert(root.workspace_root_id)
            || !portable_keys.insert(root.portable_key.as_str())
        {
            return Err(asset_processor_authority_mismatch(
                "workspaceSnapshot",
                format!(
                    "workspace snapshot {} contains duplicate source roots",
                    snapshot.workspace_id
                ),
            ));
        }
    }
    Ok(())
}

/// Checks the snapshot carries the project-assets root and that it is a non-degenerate root
/// nested inside the session workspace.
fn ensure_cli_workspace_snapshot_project_assets_root(
    snapshot: &WorkspaceSnapshot,
    manifest: &SessionManifest,
) -> CliResult<()> {
    let project_assets_key =
        az_project::PortableKey::project_assets(&manifest.project_id).to_string();
    let project_assets = snapshot
        .roots
        .iter()
        .find(|root| root.portable_key == project_assets_key)
        .ok_or_else(|| {
            asset_processor_authority_mismatch(
                "workspaceSnapshot",
                format!(
                    "workspace snapshot {} has no `{project_assets_key}` root",
                    snapshot.workspace_id
                ),
            )
        })?;
    if project_assets.owner_id != manifest.project_id
        || !project_assets.is_root
        || !project_assets.output_prefix.is_empty()
        || !Path::new(&project_assets.source_root).starts_with(&manifest.workspace_root)
        || Path::new(&project_assets.source_root) == manifest.workspace_root
    {
        return Err(asset_processor_authority_mismatch(
            "workspaceSnapshot",
            format!(
                "workspace snapshot {} has an invalid `{project_assets_key}` root",
                snapshot.workspace_id
            ),
        ));
    }

    Ok(())
}

fn ensure_cli_workspace_entry_page_matches_request(
    result: &WorkspaceEntryPageResult,
    expected_workspace_id: i64,
    after_entry_id: Option<i64>,
    page_size: u32,
) -> CliResult<()> {
    if result.entries.len() > page_size as usize {
        return Err(asset_processor_authority_mismatch(
            "workspaceEntryPage",
            format!(
                "asset-processor returned {} entries for requested page size {page_size}",
                result.entries.len()
            ),
        ));
    }

    let mut entry_ids = BTreeSet::new();
    let mut previous_id = after_entry_id.unwrap_or(0);
    for entry in &result.entries {
        ensure_cli_workspace_entry_matches_snapshot(entry, expected_workspace_id)?;
        if entry.entry_id <= previous_id || !entry_ids.insert(entry.entry_id) {
            return Err(asset_processor_authority_mismatch(
                "workspaceEntryPage",
                format!(
                    "asset-processor returned invalid entry cursor {}",
                    entry.entry_id
                ),
            ));
        }
        previous_id = entry.entry_id;
    }

    if let Some(next_after) = result.next_after_entry_id {
        let Some(last) = result.entries.last() else {
            return Err(asset_processor_authority_mismatch(
                "workspaceEntryPage",
                "asset-processor returned a next cursor for an empty page".to_string(),
            ));
        };
        if next_after != last.entry_id {
            return Err(asset_processor_authority_mismatch(
                "workspaceEntryPage",
                format!(
                    "next cursor {next_after} did not match last entry {}",
                    last.entry_id
                ),
            ));
        }
    }

    Ok(())
}

fn ensure_cli_workspace_entry_matches_snapshot(
    entry: &WorkspaceEntry,
    expected_workspace_id: i64,
) -> CliResult<()> {
    if entry.entry_id <= 0
        || entry.workspace_id != expected_workspace_id
        || entry.asset_guid == Uuid::nil()
        || entry.root_id <= 0
        || entry.source_path.trim().is_empty()
        || !is_64_hex_hash(&entry.content_hash)
        || entry.diagnostics_count < 0
        || entry.updated_unix_ms < 0
        || entry
            .schema_type
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err(asset_processor_authority_mismatch(
            "workspaceEntryPage",
            format!(
                "workspace entry `{}` has invalid identity metadata",
                entry.source_path
            ),
        ));
    }
    for activity in &entry.jobs {
        ensure_cli_job_activity_matches_entry(activity, entry)?;
    }
    Ok(())
}

// Clippy reads `job.source_guid != entry.asset_guid` as a copy-paste slip and suggests
// `entry.source_guid`, a field `WorkspaceEntry` does not have; a job's source guid is the
// entry's asset guid, so the comparison is correct and the suggestion would not compile.
#[allow(clippy::suspicious_operation_groupings)]
fn ensure_cli_job_activity_matches_entry(
    activity: &JobActivity,
    entry: &WorkspaceEntry,
) -> CliResult<()> {
    let job = &activity.job;
    if job.job_id <= 0
        || job.workspace_id != entry.workspace_id
        || job.source_guid != entry.asset_guid
        || job.source_path != entry.source_path
        || job.source_root.trim().is_empty()
        || job.key.trim().is_empty()
        || job.platform.trim().is_empty()
        || job.attempts < 0
        || job
            .source_schema_type
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err(asset_processor_authority_mismatch(
            "workspaceEntryPage",
            format!("workspace entry {} contains an invalid job", entry.entry_id),
        ));
    }
    ensure_cli_job_owner(&job.owner, "workspaceEntryPage")?;
    if let Some(attempt) = &activity.attempt
        && (attempt.attempt_id <= 0
            || attempt.job_id != job.job_id
            || attempt.ordinal <= 0
            || attempt.error_count < 0
            || attempt.warning_count < 0
            || attempt.finished_unix_ms.is_some_and(|value| value < 0)
            || attempt
                .owner
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
            || attempt
                .staging
                .as_ref()
                .is_some_and(|value| value.trim().is_empty()))
    {
        return Err(asset_processor_authority_mismatch(
            "workspaceEntryPage",
            format!("job {} contains an invalid attempt", job.job_id),
        ));
    }
    Ok(())
}

fn ensure_cli_job_owner(owner: &JobOwner, operation: &'static str) -> CliResult<()> {
    if matches!(owner, JobOwner::Build(builder) if *builder == Uuid::nil()) {
        return Err(asset_processor_authority_mismatch(
            operation,
            "build job owner cannot use a nil builder guid".to_string(),
        ));
    }
    Ok(())
}

fn ensure_cli_catalog_products_matches_request(
    entries: &[CatalogProductEntry],
    expected_platform: &str,
) -> CliResult<()> {
    let mut product_ids = BTreeSet::new();
    let mut previous_sort_key = None;
    for entry in entries {
        if entry.job_id <= 0
            || entry.product_id <= 0
            || entry.asset_guid == Uuid::nil()
            || entry.builder_guid == Uuid::nil()
            || entry.asset_type == Uuid::nil()
            || entry.platform != expected_platform
            || entry.source_path.trim().is_empty()
            || entry.job_key.trim().is_empty()
            || entry.product_path.trim().is_empty()
            || entry.product_format.trim().is_empty()
            || entry.product_format_version == 0
            || entry.sub_id < 0
            || entry.byte_length < 0
            || !is_64_hex_hash(&entry.content_hash)
        {
            return Err(asset_processor_authority_mismatch(
                "catalogProducts",
                format!("catalog product {} has invalid metadata", entry.product_id),
            ));
        }
        ensure_cli_asset_db_relative_path(&entry.source_path, "catalogProducts", "source path")?;
        ensure_cli_asset_db_relative_path(&entry.product_path, "catalogProducts", "product path")?;
        if !product_ids.insert(entry.product_id) {
            return Err(asset_processor_authority_mismatch(
                "catalogProducts",
                format!("catalog contains duplicate product {}", entry.product_id),
            ));
        }
        let sort_key = (
            &entry.product_path,
            entry.asset_guid,
            entry.sub_id,
            entry.product_id,
        );
        if previous_sort_key
            .as_ref()
            .is_some_and(|previous| previous > &sort_key)
        {
            return Err(asset_processor_authority_mismatch(
                "catalogProducts",
                format!("catalog product `{}` is out of order", entry.product_path),
            ));
        }
        previous_sort_key = Some(sort_key);
        for dependency in &entry.dependencies {
            if dependency.asset_guid == Uuid::nil() || dependency.sub_id < 0 {
                return Err(asset_processor_authority_mismatch(
                    "catalogProducts",
                    format!(
                        "catalog product {} has an invalid dependency",
                        entry.product_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn ensure_cli_job_inspection_matches_request(
    inspection: &JobInspection,
    selector: &InspectJobSelector,
) -> CliResult<()> {
    let job = &inspection.job;
    ensure_cli_job_inspection_answers_selector(inspection, selector)?;

    if job.job_id <= 0
        || job.workspace_id <= 0
        || job.source_guid == Uuid::nil()
        || job.source_path.trim().is_empty()
        || job.source_root.trim().is_empty()
        || job.key.trim().is_empty()
        || job.platform.trim().is_empty()
        || job.attempts < 0
        || job
            .source_schema_type
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err(asset_processor_authority_mismatch(
            "inspectJob",
            format!("job {} has invalid identity metadata", job.job_id),
        ));
    }
    ensure_cli_job_owner(&job.owner, "inspectJob")?;

    if let Some(attempt) = &inspection.attempt
        && (attempt.attempt_id <= 0
            || attempt.job_id != job.job_id
            || attempt.ordinal <= 0
            || attempt.error_count < 0
            || attempt.warning_count < 0
            || attempt.finished_unix_ms.is_some_and(|value| value < 0)
            || attempt
                .owner
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
            || attempt
                .staging
                .as_ref()
                .is_some_and(|value| value.trim().is_empty()))
    {
        return Err(asset_processor_authority_mismatch(
            "inspectJob",
            format!("job {} has an invalid attempt", job.job_id),
        ));
    }

    ensure_cli_job_inspection_products(inspection)?;
    ensure_cli_job_inspection_dependencies(inspection)
}

/// Checks the inspection answers the selector the CLI asked for: the requested job id, or the
/// requested attempt actually present on the returned job.
fn ensure_cli_job_inspection_answers_selector(
    inspection: &JobInspection,
    selector: &InspectJobSelector,
) -> CliResult<()> {
    let job = &inspection.job;
    match selector {
        InspectJobSelector::Job(job_id) if job.job_id != *job_id => {
            Err(asset_processor_authority_mismatch(
                "inspectJob",
                format!(
                    "asset-processor returned job {}, expected {job_id}",
                    job.job_id
                ),
            ))
        }
        InspectJobSelector::Attempt(attempt_id) => {
            let Some(attempt) = &inspection.attempt else {
                return Err(asset_processor_authority_mismatch(
                    "inspectJob",
                    format!("asset-processor omitted requested attempt {attempt_id}"),
                ));
            };
            if attempt.attempt_id == *attempt_id {
                Ok(())
            } else {
                Err(asset_processor_authority_mismatch(
                    "inspectJob",
                    format!(
                        "asset-processor returned attempt {}, expected {attempt_id}",
                        attempt.attempt_id
                    ),
                ))
            }
        }
        InspectJobSelector::Job(_) => Ok(()),
    }
}

/// Checks every product belongs to the inspected job, carries complete product identity and a
/// safe relative path, and that product and dependency-edge ids are unique.
fn ensure_cli_job_inspection_products(inspection: &JobInspection) -> CliResult<()> {
    let job = &inspection.job;
    let mut product_ids = BTreeSet::new();
    for product in &inspection.products {
        if product.product_id <= 0
            || product.job_id != job.job_id
            || product.path.trim().is_empty()
            || product.asset_type == Uuid::nil()
            || product.sub_id < 0
            || product.product_format.trim().is_empty()
            || product.product_format_version == 0
            || product.byte_length < 0
            || !is_64_hex_hash(&product.content_hash)
            || !product_ids.insert(product.product_id)
        {
            return Err(asset_processor_authority_mismatch(
                "inspectJob",
                format!("job {} has invalid products", job.job_id),
            ));
        }
        ensure_cli_asset_db_relative_path(&product.path, "inspectJob", "product path")?;
        let mut edge_ids = BTreeSet::new();
        for edge in &product.edges {
            if edge.product_edge_id <= 0
                || edge.product_id != product.product_id
                || edge.asset_guid == Uuid::nil()
                || edge.sub_id < 0
                || !edge_ids.insert(edge.product_edge_id)
            {
                return Err(asset_processor_authority_mismatch(
                    "inspectJob",
                    format!(
                        "product {} has invalid dependency edges",
                        product.product_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Checks every job dependency belongs to the inspected job, has a unique edge id, and names
/// either a non-nil guid or a safe relative path.
fn ensure_cli_job_inspection_dependencies(inspection: &JobInspection) -> CliResult<()> {
    let job = &inspection.job;
    let mut dependency_ids = BTreeSet::new();
    for dependency in &inspection.dependencies {
        if dependency.job_edge_id <= 0
            || dependency.job_id != job.job_id
            || dependency.key.trim().is_empty()
            || dependency.platform.trim().is_empty()
            || !dependency_ids.insert(dependency.job_edge_id)
        {
            return Err(asset_processor_authority_mismatch(
                "inspectJob",
                format!("job {} has invalid dependency records", job.job_id),
            ));
        }
        match &dependency.target {
            az_proto_asset::JobDependencyTarget::Guid(guid) if *guid != Uuid::nil() => {}
            az_proto_asset::JobDependencyTarget::Path(path)
                if is_cli_safe_asset_db_relative_path(path) => {}
            _ => {
                return Err(asset_processor_authority_mismatch(
                    "inspectJob",
                    format!("job {} has an invalid dependency target", job.job_id),
                ));
            }
        }
    }

    Ok(())
}

fn ensure_cli_asset_builder_catalog_result_matches_request(
    result: &AssetBuilderCatalogResult,
) -> CliResult<()> {
    let mut seen_builder_guids = BTreeSet::new();
    for builder in &result.builders {
        ensure_cli_asset_builder_matches_request(builder)?;
        if !seen_builder_guids.insert(builder.builder_guid.to_string()) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset-processor returned duplicate builder guid {}",
                    builder.builder_guid
                ),
            ));
        }
    }
    let mut seen_source_schemas = BTreeSet::new();
    for source_schema in &result.source_schemas {
        ensure_cli_source_schema_matches_request(source_schema)?;
        if !seen_source_schemas.insert(source_schema.schema_type.as_str()) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset-processor returned duplicate source schema type {}",
                    source_schema.schema_type
                ),
            ));
        }
    }
    for builder in &result.builders {
        for source_schema_type in &builder.source_schema_types {
            if !seen_source_schemas.contains(source_schema_type.as_str()) {
                return Err(asset_processor_authority_mismatch(
                    "builderCatalog",
                    format!(
                        "asset builder `{}` references source schema type `{}` without a source schema descriptor",
                        builder.name, source_schema_type
                    ),
                ));
            }
        }
    }
    let mut seen_product_formats = BTreeSet::new();
    for product_format in &result.product_formats {
        ensure_cli_product_format_matches_request(product_format)?;
        if !seen_product_formats.insert(product_format.id.as_str()) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset-processor returned duplicate product format id {}",
                    product_format.id
                ),
            ));
        }
    }

    Ok(())
}

fn ensure_cli_product_format_matches_request(
    product_format: &ProductFormatDescriptor,
) -> CliResult<()> {
    if product_format.id.trim().is_empty() || product_format.id.trim() != product_format.id {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            "product format id must be non-empty and trimmed".to_string(),
        ));
    }
    if product_format.current_version == 0 {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "product format `{}` current version must be positive",
                product_format.id
            ),
        ));
    }
    if product_format.owner.trim() != product_format.owner {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "product format `{}` owner must be trimmed",
                product_format.id
            ),
        ));
    }
    Ok(())
}

fn ensure_cli_asset_builder_matches_request(builder: &AssetBuilderDescriptor) -> CliResult<()> {
    if builder.name.trim().is_empty() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            "asset builder name cannot be empty".to_string(),
        ));
    }

    if builder.builder_guid == Uuid::nil() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!("asset builder `{}` guid cannot be nil", builder.name),
        ));
    }

    if builder.version == 0 {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!("asset builder `{}` version must be positive", builder.name),
        ));
    }

    if builder.patterns.is_empty() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "asset builder `{}` must declare at least one pattern",
                builder.name
            ),
        ));
    }

    for pattern in &builder.patterns {
        ensure_cli_asset_builder_pattern_matches_request(builder, pattern)?;
    }

    let mut seen_source_schema_types = BTreeSet::new();
    for source_schema_type in &builder.source_schema_types {
        if source_schema_type.trim().is_empty() {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset builder `{}` contains an empty source schema type",
                    builder.name
                ),
            ));
        }
        if !seen_source_schema_types.insert(source_schema_type.as_str()) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "asset builder `{}` contains duplicate source schema type `{}`",
                    builder.name, source_schema_type
                ),
            ));
        }
    }

    Ok(())
}

fn ensure_cli_source_schema_matches_request(
    source_schema: &SourceSchemaDescriptor,
) -> CliResult<()> {
    if source_schema.schema_type.trim().is_empty() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            "source schema type cannot be empty".to_string(),
        ));
    }
    if source_schema.owner.trim() != source_schema.owner
        || source_schema.label.trim() != source_schema.label
        || source_schema.category.trim() != source_schema.category
    {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "source schema `{}` display metadata must be trimmed",
                source_schema.schema_type
            ),
        ));
    }
    match &source_schema.authoring {
        SourceSchemaAuthoring::File { workflow } => {
            ensure_cli_source_file_workflow_matches_request(&source_schema.schema_type, workflow)?;
        }
        SourceSchemaAuthoring::ProjectDocument { schema_type } => {
            if schema_type.trim().is_empty() {
                return Err(asset_processor_authority_mismatch(
                    "builderCatalog",
                    format!(
                        "source schema `{}` project document schema type cannot be empty",
                        source_schema.schema_type
                    ),
                ));
            }
        }
    }
    ensure_cli_source_file_templates_match_request(source_schema)
}

fn ensure_cli_source_file_templates_match_request(
    source_schema: &SourceSchemaDescriptor,
) -> CliResult<()> {
    if !source_schema.file_templates.is_empty() {
        match &source_schema.authoring {
            SourceSchemaAuthoring::File { workflow } if workflow.can_create => {}
            SourceSchemaAuthoring::File { .. } => {
                return Err(asset_processor_authority_mismatch(
                    "builderCatalog",
                    format!(
                        "source schema `{}` exposes file templates but is not default-creatable",
                        source_schema.schema_type
                    ),
                ));
            }
            SourceSchemaAuthoring::ProjectDocument { .. } => {
                return Err(asset_processor_authority_mismatch(
                    "builderCatalog",
                    format!(
                        "source schema `{}` exposes file templates but is project-document backed",
                        source_schema.schema_type
                    ),
                ));
            }
        }
    }

    let mut seen_paths = BTreeSet::new();
    for template in &source_schema.file_templates {
        if template.owner.trim() != template.owner
            || template.label.trim() != template.label
            || template.description.trim() != template.description
        {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{}` file template display metadata must be trimmed",
                    source_schema.schema_type
                ),
            ));
        }
        if !is_safe_asset_relative_path(&template.source_path) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{}` file template has invalid source path `{}`",
                    source_schema.schema_type, template.source_path
                ),
            ));
        }
        if !seen_paths.insert(template.source_path.as_str()) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{}` repeats file template source path `{}`",
                    source_schema.schema_type, template.source_path
                ),
            ));
        }
    }
    Ok(())
}

fn is_safe_asset_relative_path(path: &str) -> bool {
    !path.trim().is_empty()
        && path.trim() == path
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn ensure_cli_source_file_workflow_matches_request(
    source_schema_type: &str,
    workflow: &SourceFileWorkflowDescriptor,
) -> CliResult<()> {
    if workflow.default_path_prefix.trim() != workflow.default_path_prefix
        || workflow.default_path_prefix.contains('\\')
        || workflow.default_path_prefix.contains(':')
        || workflow.default_path_prefix.starts_with('/')
        || workflow.default_path_prefix.ends_with('/')
    {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "source schema `{source_schema_type}` file workflow has invalid default path prefix `{}`",
                workflow.default_path_prefix
            ),
        ));
    }
    for component in workflow.default_path_prefix.split('/') {
        if !workflow.default_path_prefix.is_empty()
            && (component.is_empty() || component == "." || component == "..")
        {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{source_schema_type}` file workflow has invalid default path prefix `{}`",
                    workflow.default_path_prefix
                ),
            ));
        }
    }
    if workflow.extensions.is_empty() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!("source schema `{source_schema_type}` file workflow has no extensions"),
        ));
    }
    let mut seen = BTreeSet::new();
    for extension in &workflow.extensions {
        if extension.trim() != extension
            || extension.is_empty()
            || extension.starts_with('.')
            || extension.contains('/')
            || extension.contains('\\')
            || extension.contains(':')
            || extension.contains('*') && extension != "*"
            || extension.contains('?')
        {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{source_schema_type}` file workflow has invalid extension `{extension}`"
                ),
            ));
        }
        if !seen.insert(extension.as_str()) {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{source_schema_type}` file workflow repeats extension `{extension}`"
                ),
            ));
        }
    }
    if workflow.extensions.iter().any(|extension| extension == "*") && workflow.extensions.len() > 1
    {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "source schema `{source_schema_type}` file workflow catch-all extension must stand alone"
            ),
        ));
    }
    if workflow.can_create && workflow.extensions.iter().any(|extension| extension == "*") {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "source schema `{source_schema_type}` creatable file workflow cannot use the catch-all extension"
            ),
        ));
    }
    Ok(())
}

fn ensure_cli_source_file_workflow_matches_catalog<'a>(
    asset_builder_catalog: &'a AssetBuilderCatalogResult,
    schema_type: &str,
    source_path: &str,
    uses_payload: bool,
) -> CliResult<&'a SourceFileWorkflowDescriptor> {
    ensure_cli_asset_db_relative_path(source_path, "createSourceFile", "source path")?;

    let source_schemas = asset_builder_catalog
        .source_schemas
        .iter()
        .filter(|source_schema| source_schema.schema_type == schema_type)
        .collect::<Vec<_>>();
    let source_schema = match source_schemas.as_slice() {
        [source_schema] => *source_schema,
        [] => {
            return Err(CliError::InvalidAuthoredEdit {
                message: format!(
                    "source schema `{schema_type}` is not registered by asset-processor; register the source schema, template, build rule, product type, product format, and loader/inspector workflow before creating it"
                ),
            });
        }
        duplicates => {
            return Err(asset_processor_authority_mismatch(
                "builderCatalog",
                format!(
                    "source schema `{schema_type}` is registered {} times",
                    duplicates.len()
                ),
            ));
        }
    };

    match &source_schema.authoring {
        SourceSchemaAuthoring::File { workflow } => {
            ensure_cli_source_file_path_matches_workflow(
                schema_type,
                source_path,
                workflow,
                "createSourceFile",
            )?;
            if uses_payload || workflow.can_create {
                return Ok(workflow);
            }
            Err(CliError::InvalidAuthoredEdit {
                message: format!(
                    "source schema `{schema_type}` is file-backed but has no default create workflow; pass `--from` to import a source payload or register a default source-file template"
                ),
            })
        }
        SourceSchemaAuthoring::ProjectDocument {
            schema_type: document_schema,
        } => Err(CliError::InvalidAuthoredEdit {
            message: format!(
                "source schema `{schema_type}` is project-document backed by `{document_schema}`; use `azoth session document create` for that workflow"
            ),
        }),
    }
}

fn ensure_cli_source_file_path_matches_workflow(
    source_schema_type: &str,
    source_path: &str,
    workflow: &SourceFileWorkflowDescriptor,
    operation: &'static str,
) -> CliResult<()> {
    let source_path_lower = source_path.to_ascii_lowercase();
    let matches = workflow
        .extensions
        .iter()
        .filter(|extension| extension.as_str() != "*")
        .any(|extension| {
            source_path_lower.ends_with(&format!(".{}", extension.to_ascii_lowercase()))
        });
    if matches {
        return Ok(());
    }

    Err(asset_processor_authority_mismatch(
        operation,
        format!(
            "source path `{source_path}` does not match creatable file workflow for `{source_schema_type}`; expected one of extensions {:?}",
            workflow.extensions
        ),
    ))
}

fn ensure_cli_asset_builder_pattern_matches_request(
    builder: &AssetBuilderDescriptor,
    pattern: &AssetBuilderPatternDescriptor,
) -> CliResult<()> {
    if pattern.pattern.trim().is_empty() {
        return Err(asset_processor_authority_mismatch(
            "builderCatalog",
            format!(
                "asset builder `{}` contains an empty {:?} pattern",
                builder.name, pattern.kind
            ),
        ));
    }

    Ok(())
}

fn is_64_hex_hash(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn ensure_cli_asset_db_relative_path(
    value: &str,
    operation: &'static str,
    label: &'static str,
) -> CliResult<()> {
    if is_cli_safe_asset_db_relative_path(value) {
        Ok(())
    } else {
        Err(asset_processor_authority_mismatch(
            operation,
            format!("{label} `{value}` must be an asset-db relative path"),
        ))
    }
}

fn is_cli_safe_asset_db_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::Normal(segment) if !segment.to_string_lossy().trim().is_empty() => {
                has_normal_component = true;
            }
            Component::Normal(_)
            | Component::CurDir
            | Component::ParentDir
            | Component::RootDir => {
                return false;
            }
            Component::Prefix(_) => return false,
        }
    }
    has_normal_component
}

const fn asset_processor_authority_mismatch(operation: &'static str, reason: String) -> CliError {
    CliError::AssetProcessorAuthorityMismatch { operation, reason }
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_inspect_job(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    selector: InspectJobSelector,
) -> CliResult<Option<JobInspection>> {
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.inspect_job_request();
    (InspectJobRequest {
        capability: asset_read_capability(manifest, descriptor)?,
        selector: selector.clone(),
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = InspectJobResult::from_capnp(response.get()?.get_result()?)?;
    if let Some(inspection) = &result.inspection {
        ensure_cli_job_inspection_matches_request(inspection, &selector)?;
    }
    Ok(result.inspection)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_catalog_products(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    platform: &str,
) -> CliResult<Vec<CatalogProductEntry>> {
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.catalog_products_request();
    (CatalogProductsRequest {
        capability: asset_read_capability(manifest, descriptor)?,
        platform: platform.to_string(),
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = CatalogProductsResult::from_capnp(response.get()?.get_result()?)?;
    ensure_cli_catalog_products_matches_request(&result.entries, platform)?;
    Ok(result.entries)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_asset_builder_catalog(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
) -> CliResult<AssetBuilderCatalogResult> {
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.builder_catalog_request();
    (AssetBuilderCatalogRequest {
        capability: asset_read_capability(manifest, descriptor)?,
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = AssetBuilderCatalogResult::from_capnp(response.get()?.get_result()?)?;
    ensure_cli_asset_builder_catalog_result_matches_request(&result)?;
    Ok(result)
}

fn requested_workspace_source_root<'a>(
    snapshot: &'a WorkspaceSnapshot,
    requested: &str,
) -> CliResult<&'a WorkspaceRoot> {
    let project_source_key =
        az_project::PortableKey::project_assets(&snapshot.project_id).to_string();
    let mut candidates = snapshot.roots.iter().filter(|root| {
        if requested == PROJECT_SOURCE_ROOT {
            root.portable_key == project_source_key
                && root.owner_id == snapshot.project_id
                && root.is_root
                && root.output_prefix.is_empty()
        } else {
            root.portable_key == requested
        }
    });
    let Some(root) = candidates.next() else {
        return Err(asset_processor_authority_mismatch(
            "recordSourceAsset",
            format!(
                "workspace snapshot {} has no source root `{requested}`",
                snapshot.workspace_id
            ),
        ));
    };
    if candidates.next().is_some() {
        return Err(asset_processor_authority_mismatch(
            "recordSourceAsset",
            format!(
                "workspace snapshot {} returned multiple source roots for `{requested}`",
                snapshot.workspace_id
            ),
        ));
    }
    Ok(root)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_import_existing_source_assets(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    workspace_source_root: &WorkspaceRoot,
    source_root: &str,
    schema_type: &str,
    source_paths: &[String],
) -> CliResult<Vec<SourceFileCreateResult>> {
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let capability = asset_write_capability(manifest, descriptor)?;
    let mut results = Vec::with_capacity(source_paths.len());
    for source_path in source_paths {
        let native_path = Path::new(&workspace_source_root.source_root).join(source_path);
        let source_bytes = std::fs::read(&native_path)?;
        let changed_unix_ms = existing_source_file_modified_unix_ms(&native_path)?;
        let content = SourceFileCreateContent::Payload(Box::new(
            source_create_payload_side_channel(&capability, &source_bytes)?,
        ));
        let mut request = client.create_source_file_request();
        (SourceFileCreateRequest {
            capability: capability.clone(),
            session_id: manifest.id.to_string(),
            source_root: source_root.to_string(),
            source_path: source_path.clone(),
            schema_type: schema_type.to_string(),
            changed_unix_ms,
            content,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let result = SourceFileCreateResult::from_capnp(response.get()?.get_result()?)?;
        ensure_cli_source_file_create_result_matches_request(&result, source_path, schema_type)?;
        results.push(result);
    }
    Ok(results)
}

fn existing_source_file_modified_unix_ms(path: &Path) -> CliResult<i64> {
    let changed_unix_ms = path
        .metadata()?
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|source| {
            CliError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("source modification time is before Unix epoch: {source}"),
            ))
        })?
        .as_millis();
    let changed_unix_ms = i64::try_from(changed_unix_ms).map_err(|_| {
        CliError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "source modification timestamp for {} exceeds i64",
                path.display()
            ),
        ))
    })?;
    Ok(changed_unix_ms)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_force_reprocess_assets(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    source_root: &str,
    source_paths: &[String],
) -> CliResult<Vec<ForceReprocessAssetResult>> {
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let capability = asset_write_capability(manifest, descriptor)?;
    let mut results = Vec::with_capacity(source_paths.len());
    for source_path in source_paths {
        let mut request = client.force_reprocess_asset_request();
        (ForceReprocessAssetRequest {
            capability: capability.clone(),
            session_id: manifest.id.to_string(),
            source_root: source_root.to_string(),
            source_path: source_path.clone(),
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let result = ForceReprocessAssetResult::from_capnp(response.get()?.get_result()?)?;
        ensure_cli_force_reprocess_result_matches_request(&result, source_path)?;
        results.push(result);
    }
    Ok(results)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_reconcile_asset_sources(
    client: &asset_capnp::asset_processor::Client,
    manifest: &SessionManifest,
    capability: &Capability,
) -> Result<ReconcileAssetSourcesResult, CliError> {
    let mut reconcile = client.reconcile_asset_sources_request();
    (ReconcileAssetSourcesRequest {
        capability: capability.clone(),
        session_id: manifest.id.to_string(),
        root_scope: AssetRootScope::All,
    })
    .to_capnp(reconcile.get().init_request())?;
    let response = reconcile.send().promise.await?;
    ReconcileAssetSourcesResult::from_capnp(response.get()?.get_result()?).map_err(Into::into)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_create_source_file(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    source_root: &str,
    source_path: &str,
    schema_type: &str,
    from: Option<PathBuf>,
) -> CliResult<SourceFileCreateResult> {
    let client: asset_capnp::asset_processor::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let capability = asset_write_capability(manifest, descriptor)?;
    let content = match from {
        Some(path) => {
            let bytes = std::fs::read(&path)?;
            SourceFileCreateContent::Payload(Box::new(source_create_payload_side_channel(
                &capability,
                &bytes,
            )?))
        }
        None => SourceFileCreateContent::DefaultTemplate,
    };
    let mut request = client.create_source_file_request();
    (SourceFileCreateRequest {
        capability,
        session_id: manifest.id.to_string(),
        source_root: source_root.to_string(),
        source_path: source_path.to_string(),
        schema_type: schema_type.to_string(),
        changed_unix_ms: current_unix_ms_i64()?,
        content,
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = SourceFileCreateResult::from_capnp(response.get()?.get_result()?)?;
    ensure_cli_source_file_create_result_matches_request(&result, source_path, schema_type)?;
    Ok(result)
}

fn source_create_payload_side_channel(
    capability: &Capability,
    bytes: &[u8],
) -> CliResult<SideChannelHandle> {
    let staging_root = std::env::temp_dir()
        .join("azoth")
        .join("source-create")
        .join(Uuid::now_v7().as_simple().to_string());
    let written = write_content_addressed_staging_file(&staging_root, "source-create", bytes)
        .map_err(|error| {
            asset_processor_authority_mismatch(
                "createSourceFile",
                format!("failed to stage source payload side channel: {error}"),
            )
        })?;
    Ok(SideChannelHandle::staging_file(
        written.path.to_string_lossy(),
        written.byte_length,
        written.content_hash,
        std::env::consts::OS,
    )
    .with_capability(capability.clone()))
}

fn current_unix_ms_i64() -> CliResult<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| {
            CliError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("system time is before Unix epoch: {source}"),
            ))
        })?;
    let millis = duration.as_millis();
    i64::try_from(millis).map_err(|_| {
        CliError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "current Unix timestamp does not fit in i64 milliseconds",
        ))
    })
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is a distinct input the `runtimeLaunchSnapshot` request needs — \
              the project-host and runtime-host descriptors, the session manifest, the \
              runtime role, the asset snapshot and package roots, and the two launch flags — \
              and they are assembled straight into `RuntimeLaunchSnapshotRequest`, so a \
              CLI-side struct would only duplicate that wire message"
)]
async fn request_runtime_launch_snapshot(
    descriptor: &ServiceDescriptor,
    runtime_descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    role: RuntimeRole,
    asset_snapshot: &WorkspaceSnapshot,
    asset_package_roots: &[RuntimeAssetPackageRoot],
    include_unsaved_journal: bool,
    launch_profile: &str,
) -> CliResult<az_proto_core::SideChannelHandle> {
    let client: project_capnp::project_host::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.runtime_launch_snapshot_request();
    let capability = project_runtime_launch_capability(manifest, descriptor)?;
    let runtime_launch_capability =
        runtime_project_host_control_capability(manifest, runtime_descriptor)?;
    RuntimeLaunchSnapshotRequest {
        capability,
        runtime_launch_capability: runtime_launch_capability.clone(),
        role,
        project_id: manifest.project_id.clone(),
        session_id: manifest.id.0,
        session_slug: manifest.slug.clone(),
        project_root: manifest.project_root.to_string_lossy().into_owned(),
        workspace_path: manifest.workspace_root.to_string_lossy().into_owned(),
        workspace_id: asset_snapshot.workspace_id,
        include_unsaved_journal,
        launch_profile: launch_profile.to_string(),
        asset_source_roots: runtime_asset_source_roots(asset_snapshot),
        asset_package_roots: asset_package_roots.to_vec(),
    }
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    Ok(
        ProjectSideChannelResult::from_capnp((response.get()?, &runtime_launch_capability))?
            .snapshot,
    )
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_runtime_asset_package_roots(
    supervisor: &session_capnp::session_supervisor::Client,
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
) -> CliResult<Vec<RuntimeAssetPackageRoot>> {
    let mut request = supervisor.runtime_asset_package_roots_request();
    (RuntimeAssetPackageRootsRequest {
        capability: session_read_capability(manifest, descriptor)?,
        slug: manifest.slug.clone(),
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    Ok(RuntimeAssetPackageRootsResult::from_capnp(response.get()?.get_result()?)?.roots)
}

fn ensure_runtime_launch_workspace_snapshot(
    manifest: &SessionManifest,
    snapshot: &WorkspaceSnapshot,
) -> CliResult<()> {
    if snapshot.roots.is_empty() {
        return Err(CliError::MissingRuntimeAssetSourceRoots {
            session: manifest.slug.clone(),
            workspace_id: snapshot.workspace_id,
        });
    }
    ensure_cli_workspace_snapshot_matches_manifest(snapshot, manifest)
}

fn runtime_asset_source_roots(snapshot: &WorkspaceSnapshot) -> Vec<RuntimeAssetSourceRoot> {
    snapshot
        .roots
        .iter()
        .map(|root| RuntimeAssetSourceRoot {
            workspace_root_id: root.workspace_root_id,
            workspace_id: root.workspace_id,
            root_id: root.root_id,
            owner_id: root.owner_id.clone(),
            source_root: root.source_root.clone(),
            display_name: root.display_name.clone(),
            portable_key: root.portable_key.clone(),
            output_prefix: root.output_prefix.clone(),
            is_root: root.is_root,
        })
        .collect()
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_prefab_source_snapshot_on_host(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    source_path: &str,
) -> CliResult<az_proto_project::vnext::PrefabRpcResult> {
    let client: project_capnp::project_host::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.prefab_source_snapshot_request();
    {
        let mut parameters = request.get();
        az_proto_core::Capability::to_capnp(
            &project_document_read_capability(manifest, descriptor)?,
            parameters.reborrow().init_capability(),
        )?;
        parameters.set_source_path(source_path);
    }
    let response = request.send().promise.await?;
    Ok(az_proto_project::vnext::PrefabRpcResult::from_capnp(
        response.get()?.get_result()?,
    )?)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_source_session_lifecycle_on_host(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    source_path: &str,
    command: az_proto_project::vnext::SourceSessionCommand,
    expected_revision: u64,
) -> CliResult<az_proto_project::vnext::SourceSessionResult> {
    let client: project_capnp::project_host::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.source_session_lifecycle_request();
    {
        let mut parameters = request.get();
        let capability = match command {
            az_proto_project::vnext::SourceSessionCommand::Open
            | az_proto_project::vnext::SourceSessionCommand::Status => {
                project_document_read_capability(manifest, descriptor)?
            }
            _ => project_document_write_capability(manifest, descriptor)?,
        };
        az_proto_core::Capability::to_capnp(&capability, parameters.reborrow().init_capability())?;
        parameters.set_source_path(source_path);
        parameters.set_command((command).to_capnp());
        parameters.set_expected_revision(expected_revision);
    }
    let response = request.send().promise.await?;
    Ok(az_proto_project::vnext::SourceSessionResult::from_capnp(
        response.get()?.get_result()?,
    )?)
}

fn ensure_cli_source_file_create_result_matches_request(
    result: &SourceFileCreateResult,
    requested_source_path: &str,
    requested_schema_type: &str,
) -> CliResult<()> {
    let record = &result.record;
    let entry = &record.entry;
    if record.asset_guid == Uuid::nil() {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            format!("asset record for `{requested_source_path}` returned a nil asset guid"),
        ));
    }
    if entry.entry_id <= 0
        || entry.workspace_id <= 0
        || entry.asset_guid == Uuid::nil()
        || entry.root_id <= 0
    {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            format!("asset record for `{requested_source_path}` did not carry DB-owned ids"),
        ));
    }
    if entry.source_path != requested_source_path
        || entry.schema_type.as_deref() != Some(requested_schema_type)
    {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            format!(
                "asset record source identity `{}` / `{:?}` did not match requested `{requested_source_path}` / `{requested_schema_type}`",
                entry.source_path, entry.schema_type
            ),
        ));
    }
    if !is_64_hex_hash(&entry.content_hash) {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            format!(
                "asset record for `{requested_source_path}` returned invalid content hash `{}`",
                entry.content_hash
            ),
        ));
    }
    if matches!(
        entry.diff,
        ProtoWorkspaceEntryDiff::Deleted | ProtoWorkspaceEntryDiff::Conflicted
    ) || entry.diagnostics_count < 0
    {
        return Err(asset_processor_authority_mismatch(
            "createSourceFile",
            format!("asset record for `{requested_source_path}` returned invalid status metadata"),
        ));
    }
    Ok(())
}

fn ensure_cli_force_reprocess_result_matches_request(
    result: &ForceReprocessAssetResult,
    requested_source_path: &str,
) -> CliResult<()> {
    let record = &result.record;
    let entry = &record.entry;
    if result.enqueued_jobs == 0 {
        return Err(asset_processor_authority_mismatch(
            "forceReprocessAsset",
            format!("asset record for `{requested_source_path}` enqueued no jobs"),
        ));
    }
    if record.asset_guid == Uuid::nil() {
        return Err(asset_processor_authority_mismatch(
            "forceReprocessAsset",
            format!("asset record for `{requested_source_path}` returned a nil asset guid"),
        ));
    }
    if entry.entry_id <= 0
        || entry.workspace_id <= 0
        || entry.asset_guid == Uuid::nil()
        || entry.root_id <= 0
    {
        return Err(asset_processor_authority_mismatch(
            "forceReprocessAsset",
            format!("asset record for `{requested_source_path}` did not carry DB-owned ids"),
        ));
    }
    if entry.source_path != requested_source_path || !is_64_hex_hash(&entry.content_hash) {
        return Err(asset_processor_authority_mismatch(
            "forceReprocessAsset",
            format!(
                "asset record source identity `{}` / `{}` did not match requested `{requested_source_path}`",
                entry.source_path, entry.content_hash
            ),
        ));
    }
    if matches!(
        entry.diff,
        ProtoWorkspaceEntryDiff::Deleted | ProtoWorkspaceEntryDiff::Conflicted
    ) || entry.diagnostics_count < 0
    {
        return Err(asset_processor_authority_mismatch(
            "forceReprocessAsset",
            format!("asset record for `{requested_source_path}` returned invalid status metadata"),
        ));
    }
    Ok(())
}

fn ensure_cli_runtime_status_matches_request(
    status: &RuntimeStatus,
    expected_runtime_id: &str,
    expected_role: Option<RuntimeRole>,
    operation: &'static str,
) -> CliResult<()> {
    if status.runtime_id != expected_runtime_id {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime-host returned runtime `{}`, expected `{}`",
                status.runtime_id, expected_runtime_id
            ),
        ));
    }

    if status.runtime_id.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            operation,
            "runtime status id cannot be empty".to_string(),
        ));
    }

    if let Some(expected_role) = expected_role
        && status.role != expected_role
    {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` reported role {:?}, expected {:?}",
                status.runtime_id, status.role, expected_role
            ),
        ));
    }

    if status.project_id.trim().is_empty() || status.session_slug.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` status project/session identity cannot be empty",
                status.runtime_id
            ),
        ));
    }

    Ok(())
}

fn ensure_cli_runtime_viewport_frame_matches_request(
    frame: &RuntimeViewportFrame,
    expected_runtime_id: &str,
    operation: &'static str,
) -> CliResult<()> {
    if frame.runtime_id != expected_runtime_id {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime-host returned viewport frame for runtime `{}`, expected `{}`",
                frame.runtime_id, expected_runtime_id
            ),
        ));
    }

    if frame.runtime_id.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            operation,
            "runtime viewport frame id cannot be empty".to_string(),
        ));
    }

    ensure_cli_runtime_viewport_frame_geometry(frame, operation)?;
    ensure_cli_runtime_viewport_frame_side_channel(frame, operation)
}

/// Checks the frame describes a real image: non-zero dimensions, a known pixel format, and a row
/// pitch wide enough for one row of pixels.
fn ensure_cli_runtime_viewport_frame_geometry(
    frame: &RuntimeViewportFrame,
    operation: &'static str,
) -> CliResult<()> {
    if frame.width == 0 || frame.height == 0 {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport dimensions must be non-zero, got {}x{}",
                frame.runtime_id, frame.width, frame.height
            ),
        ));
    }

    let bytes_per_pixel = frame.format.bytes_per_pixel().ok_or_else(|| {
        runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport pixel format cannot be unknown",
                frame.runtime_id
            ),
        )
    })?;
    let min_row_pitch = frame.width.checked_mul(bytes_per_pixel).ok_or_else(|| {
        runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport row pitch overflow for {} pixels",
                frame.runtime_id, frame.width
            ),
        )
    })?;
    if frame.row_pitch < min_row_pitch {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport row pitch {} is smaller than minimum {}",
                frame.runtime_id, frame.row_pitch, min_row_pitch
            ),
        ));
    }

    Ok(())
}

/// Checks the colour side channel is large enough for the declared frame, is fully addressed, and
/// is a live channel (staging/mmap handles must resolve; CAS blobs are rejected).
fn ensure_cli_runtime_viewport_frame_side_channel(
    frame: &RuntimeViewportFrame,
    operation: &'static str,
) -> CliResult<()> {
    let min_byte_length = u64::from(frame.row_pitch)
        .checked_mul(u64::from(frame.height))
        .ok_or_else(|| {
            runtime_host_authority_mismatch(
                operation,
                format!(
                    "runtime `{}` viewport byte length overflow for row pitch {} and height {}",
                    frame.runtime_id, frame.row_pitch, frame.height
                ),
            )
        })?;
    if frame.color.byte_length < min_byte_length {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport side channel length {} is smaller than minimum {}",
                frame.runtime_id, frame.color.byte_length, min_byte_length
            ),
        ));
    }

    if frame.color.locator.trim().is_empty() || frame.color.platform.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            operation,
            format!(
                "runtime `{}` viewport side-channel locator/platform cannot be empty",
                frame.runtime_id
            ),
        ));
    }

    match frame.color.kind {
        SideChannelKind::StagingFile => {
            validated_staging_file_path(&frame.color).map_err(|error| {
                runtime_host_authority_mismatch(
                    operation,
                    format!(
                        "runtime `{}` viewport staging-file handle is invalid: {error}",
                        frame.runtime_id
                    ),
                )
            })?;
        }
        SideChannelKind::MmapFile => {
            validated_mmap_file_path(&frame.color).map_err(|error| {
                runtime_host_authority_mismatch(
                    operation,
                    format!(
                        "runtime `{}` viewport mmap-file handle is invalid: {error}",
                        frame.runtime_id
                    ),
                )
            })?;
        }
        SideChannelKind::SharedMemory | SideChannelKind::GpuSurface => {}
        SideChannelKind::CasBlob => {
            return Err(runtime_host_authority_mismatch(
                operation,
                format!(
                    "runtime `{}` viewport frames must use live side channels, not CAS blobs",
                    frame.runtime_id
                ),
            ));
        }
    }

    Ok(())
}

fn ensure_cli_runtime_projection_catalog_matches_request(
    result: &RuntimeProjectionCatalogResult,
) -> CliResult<()> {
    let mut seen_names = BTreeSet::new();
    for projection in &result.projections {
        ensure_cli_runtime_projection_descriptor_matches_request(projection)?;
        if !seen_names.insert(projection.name.as_str()) {
            return Err(runtime_host_authority_mismatch(
                "projectionCatalog",
                format!(
                    "runtime-host returned duplicate projection `{}`",
                    projection.name
                ),
            ));
        }
    }

    Ok(())
}

fn ensure_cli_runtime_projection_descriptor_matches_request(
    projection: &RuntimeProjectionDescriptor,
) -> CliResult<()> {
    if projection.name.trim().is_empty() {
        return Err(runtime_host_authority_mismatch(
            "projectionCatalog",
            "runtime projection name cannot be empty".to_string(),
        ));
    }

    let mut seen_profiles = BTreeSet::new();
    for profile in &projection.launch_profiles {
        if profile.trim().is_empty() {
            return Err(runtime_host_authority_mismatch(
                "projectionCatalog",
                format!(
                    "runtime projection `{}` launch profile cannot be empty",
                    projection.name
                ),
            ));
        }
        if !seen_profiles.insert(profile.as_str()) {
            return Err(runtime_host_authority_mismatch(
                "projectionCatalog",
                format!(
                    "runtime projection `{}` returned duplicate launch profile `{}`",
                    projection.name, profile
                ),
            ));
        }
    }

    Ok(())
}

const fn runtime_host_authority_mismatch(operation: &'static str, reason: String) -> CliError {
    CliError::RuntimeHostAuthorityMismatch { operation, reason }
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn launch_runtime_on_host(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    runtime_id: &str,
    role: RuntimeRole,
    snapshot: az_proto_core::SideChannelHandle,
) -> CliResult<RuntimeStatus> {
    let client: runtime_capnp::runtime_host::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.launch_request();
    (LaunchRuntimeRequest {
        capability: runtime_control_capability(manifest, descriptor)?,
        runtime_id: runtime_id.to_string(),
        role,
        snapshot,
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let status = RuntimeStatus::from_capnp(response.get()?.get_status()?)?;
    ensure_cli_runtime_status_matches_request(&status, runtime_id, Some(role), "launch")?;
    Ok(status)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn query_runtime_status_on_host(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    runtime_id: &str,
) -> CliResult<Option<RuntimeStatus>> {
    let client: runtime_capnp::runtime_host::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.status_request();
    (RuntimeStatusRequest {
        capability: runtime_read_capability(manifest, descriptor)?,
        runtime_id: runtime_id.to_string(),
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = RuntimeStatusResult::from_capnp(response.get()?.get_result()?)?;
    if let Some(status) = &result.status {
        ensure_cli_runtime_status_matches_request(status, runtime_id, None, "status")?;
    }
    Ok(result.status)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_runtime_projection_catalog(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
) -> CliResult<RuntimeProjectionCatalogResult> {
    let client: runtime_capnp::runtime_host::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.projection_catalog_request();
    (RuntimeProjectionCatalogRequest {
        capability: runtime_read_capability(manifest, descriptor)?,
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let result = RuntimeProjectionCatalogResult::from_capnp(response.get()?.get_result()?)?;
    ensure_cli_runtime_projection_catalog_matches_request(&result)?;
    Ok(result)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn request_runtime_viewport_frame(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    runtime_id: &str,
) -> CliResult<Option<RuntimeViewportFrame>> {
    let client: runtime_capnp::runtime_host::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let capability = runtime_read_capability(manifest, descriptor)?;
    let mut request = client.viewport_frame_request();
    (RuntimeViewportRequest {
        capability: capability.clone(),
        runtime_id: runtime_id.to_string(),
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let frame = RuntimeViewportResult::from_capnp(response.get()?.get_result()?)?.frame;
    if let Some(frame) = &frame {
        ensure_cli_runtime_viewport_frame_matches_request(frame, runtime_id, "viewportFrame")?;
        validate_side_channel_capability_matches(
            &frame.color,
            &capability,
            "runtime viewport frame",
        )?;
    }
    Ok(frame)
}

// capnp-rpc clients and requests are `Rc`-based (`ClientHook`/`RequestHook` are not
// `Send`/`Sync`), so every session RPC future is single-threaded by design and runs on a
// `LocalSet`; there is no `Send` form of this future short of replacing capnp-rpc.
#[allow(clippy::future_not_send)]
async fn stop_runtime_on_host(
    descriptor: &ServiceDescriptor,
    manifest: &SessionManifest,
    runtime_id: &str,
    preserve: bool,
) -> CliResult<RuntimeStatus> {
    let client: runtime_capnp::runtime_host::Client =
        az_rpc::connect_twoparty_bootstrap(&descriptor.endpoint).await?;
    let mut request = client.stop_request();
    (StopRuntimeRequest {
        capability: runtime_control_capability(manifest, descriptor)?,
        runtime_id: runtime_id.to_string(),
        preserve,
    })
    .to_capnp(request.get().init_request())?;
    let response = request.send().promise.await?;
    let status = RuntimeStatus::from_capnp(response.get()?.get_status()?)?;
    ensure_cli_runtime_status_matches_request(&status, runtime_id, None, "stop")?;
    Ok(status)
}

fn project_host_service_id() -> ServiceId {
    ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME)
}

fn asset_processor_service_id() -> ServiceId {
    ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME)
}

fn runtime_host_service_id() -> ServiceId {
    ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME)
}

fn session_supervisor_service_id() -> ServiceId {
    ServiceId::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
    )
}

fn print_session_exec_result(result: &ProtoExecCommandResult) {
    print!("{}", result.stdout);
    eprint!("{}", result.stderr);
    if result.stdout_truncated {
        eprintln!("\n[session-supervisor stdout truncated]");
    }
    if result.stderr_truncated {
        eprintln!("\n[session-supervisor stderr truncated]");
    }
}

fn split_exec_command(command: Vec<String>) -> CliResult<(String, Vec<String>)> {
    let mut command = command.into_iter();
    let Some(program) = command.next() else {
        return Err(CliError::MissingSessionExecCommand);
    };
    Ok((program, command.collect()))
}

fn session_exec_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    session_capability(manifest, descriptor, [SESSION_EXEC_PERMISSION])
}

fn session_manage_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    session_capability(manifest, descriptor, [SESSION_MANAGE_PERMISSION])
}

fn proto_session_manage_capability(
    manifest: &ProtoSessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    proto_session_capability(manifest, descriptor, [SESSION_MANAGE_PERMISSION])
}

fn session_read_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    session_capability(manifest, descriptor, [SESSION_READ_PERMISSION])
}

fn proto_session_read_capability(
    manifest: &ProtoSessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    proto_session_capability(manifest, descriptor, [SESSION_READ_PERMISSION])
}

fn proto_session_capability(
    manifest: &ProtoSessionManifest,
    descriptor: &ServiceDescriptor,
    permissions: impl IntoIterator<Item = &'static str>,
) -> CliResult<Capability> {
    validate_session_supervisor_descriptor(descriptor, "session capability")?;
    let permissions = permissions.into_iter().collect::<Vec<_>>();
    descriptor
        .brokered_capability_template(
            ServiceRole::Editor,
            SESSION_SUPERVISOR_AUDIENCE,
            permissions.as_slice(),
            Some(manifest.id),
        )
        .ok_or_else(|| {
            CliError::MissingServiceCapability(Box::new(MissingServiceCapabilityDetails {
                session: manifest.slug.clone(),
                service: service_label(&descriptor.id),
                audience: SESSION_SUPERVISOR_AUDIENCE.to_string(),
                permissions: permissions.join(", "),
            }))
        })
}

fn unscoped_session_read_capability(descriptor: &ServiceDescriptor) -> CliResult<Capability> {
    validate_session_supervisor_descriptor(descriptor, "session list capability")?;
    let permissions = [SESSION_READ_PERMISSION];
    descriptor
        .brokered_capability_template(
            ServiceRole::Editor,
            SESSION_SUPERVISOR_AUDIENCE,
            &permissions,
            None,
        )
        .ok_or_else(|| {
            CliError::MissingServiceCapability(Box::new(MissingServiceCapabilityDetails {
                session: "<unscoped>".to_string(),
                service: service_label(&descriptor.id),
                audience: SESSION_SUPERVISOR_AUDIENCE.to_string(),
                permissions: permissions.join(", "),
            }))
        })
}

fn session_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    permissions: impl IntoIterator<Item = &'static str>,
) -> CliResult<Capability> {
    validate_session_supervisor_descriptor(descriptor, "session capability")?;
    let permissions: Vec<&'static str> = permissions.into_iter().collect();
    session_scoped_descriptor_capability_template(
        manifest,
        descriptor,
        ServiceRole::Editor,
        SESSION_SUPERVISOR_AUDIENCE,
        &permissions,
    )
}

fn asset_read_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    validate_asset_processor_descriptor(descriptor, "asset read capability")?;
    let permissions = [ASSET_READ_PERMISSION];
    project_scoped_descriptor_capability_template(
        manifest,
        descriptor,
        ServiceRole::Editor,
        ASSET_PROCESSOR_AUDIENCE,
        &permissions,
    )
}

fn asset_write_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    validate_asset_processor_descriptor(descriptor, "asset write capability")?;
    let permissions = [ASSET_WRITE_PERMISSION];
    project_scoped_descriptor_capability_template(
        manifest,
        descriptor,
        ServiceRole::SessionSupervisor,
        ASSET_PROCESSOR_AUDIENCE,
        &permissions,
    )
}

fn project_runtime_launch_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    project_host_capability(manifest, descriptor, [PROJECT_RUNTIME_LAUNCH_PERMISSION])
}

fn project_document_read_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    project_host_capability(manifest, descriptor, [PROJECT_DOCUMENT_READ_PERMISSION])
}

fn project_document_write_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    project_host_capability(manifest, descriptor, [PROJECT_DOCUMENT_WRITE_PERMISSION])
}

fn project_host_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    permissions: impl IntoIterator<Item = &'static str>,
) -> CliResult<Capability> {
    validate_project_host_descriptor(descriptor, "project-host capability")?;
    let permissions: Vec<&'static str> = permissions.into_iter().collect();
    project_scoped_descriptor_capability_template(
        manifest,
        descriptor,
        ServiceRole::Editor,
        PROJECT_HOST_AUDIENCE,
        &permissions,
    )
}

fn runtime_read_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    runtime_capability(manifest, descriptor, [RUNTIME_READ_PERMISSION])
}

fn runtime_control_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    runtime_capability(manifest, descriptor, [RUNTIME_CONTROL_PERMISSION])
}

fn runtime_project_host_control_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
) -> CliResult<Capability> {
    validate_runtime_host_descriptor(descriptor, "runtime project-host control capability")?;
    let permissions = [RUNTIME_CONTROL_PERMISSION];
    session_scoped_descriptor_capability_template(
        manifest,
        descriptor,
        ServiceRole::ProjectHost,
        RUNTIME_HOST_AUDIENCE,
        &permissions,
    )
}

fn runtime_capability(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    permissions: impl IntoIterator<Item = &'static str>,
) -> CliResult<Capability> {
    validate_runtime_host_descriptor(descriptor, "runtime capability")?;
    let permissions: Vec<&'static str> = permissions.into_iter().collect();
    session_scoped_descriptor_capability_template(
        manifest,
        descriptor,
        ServiceRole::Editor,
        RUNTIME_HOST_AUDIENCE,
        &permissions,
    )
}

fn validate_project_host_descriptor(
    descriptor: &ServiceDescriptor,
    operation: &'static str,
) -> CliResult<()> {
    validate_service_descriptor(
        descriptor,
        &project_host_service_id(),
        ServiceRole::ProjectHost,
        operation,
    )
}

fn validate_asset_processor_descriptor(
    descriptor: &ServiceDescriptor,
    operation: &'static str,
) -> CliResult<()> {
    validate_service_descriptor(
        descriptor,
        &asset_processor_service_id(),
        ServiceRole::AssetProcessor,
        operation,
    )
}

fn validate_runtime_host_descriptor(
    descriptor: &ServiceDescriptor,
    operation: &'static str,
) -> CliResult<()> {
    validate_service_descriptor(
        descriptor,
        &runtime_host_service_id(),
        ServiceRole::RuntimeHost,
        operation,
    )
}

fn validate_session_supervisor_descriptor(
    descriptor: &ServiceDescriptor,
    operation: &'static str,
) -> CliResult<()> {
    validate_service_descriptor(
        descriptor,
        &session_supervisor_service_id(),
        ServiceRole::SessionSupervisor,
        operation,
    )
}

fn validate_service_descriptor(
    descriptor: &ServiceDescriptor,
    expected_id: &ServiceId,
    expected_role: ServiceRole,
    operation: &'static str,
) -> CliResult<()> {
    if &descriptor.id != expected_id || descriptor.role != expected_role {
        return Err(CliError::UnexpectedServiceDescriptor {
            operation,
            expected: service_descriptor_label(expected_id, expected_role),
            actual: service_descriptor_label(&descriptor.id, descriptor.role),
        });
    }

    descriptor
        .validate_brokered_capability_templates()
        .map_err(|error| CliError::InvalidServiceDescriptor {
            operation,
            service: service_descriptor_label(&descriptor.id, descriptor.role),
            reason: error.to_string(),
        })
}

fn service_descriptor_label(id: &ServiceId, role: ServiceRole) -> String {
    format!("{}/{} role {role:?}", id.namespace, id.name)
}

fn project_scoped_descriptor_capability_template(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    role: ServiceRole,
    audience: &str,
    permissions: &[&str],
) -> CliResult<Capability> {
    descriptor
        .brokered_capability_template(role, audience, permissions, None)
        .ok_or_else(|| missing_service_capability(manifest, &descriptor.id, audience, permissions))
}

fn session_scoped_descriptor_capability_template(
    manifest: &SessionManifest,
    descriptor: &ServiceDescriptor,
    role: ServiceRole,
    audience: &str,
    permissions: &[&str],
) -> CliResult<Capability> {
    descriptor
        .brokered_capability_template(role, audience, permissions, Some(manifest.id.0))
        .ok_or_else(|| missing_service_capability(manifest, &descriptor.id, audience, permissions))
}

fn missing_service_capability(
    manifest: &SessionManifest,
    service_id: &ServiceId,
    audience: &str,
    permissions: &[&str],
) -> CliError {
    CliError::MissingServiceCapability(Box::new(MissingServiceCapabilityDetails {
        session: manifest.slug.clone(),
        service: service_label(service_id),
        audience: audience.to_string(),
        permissions: permissions.join(", "),
    }))
}

fn service_label(service_id: &ServiceId) -> String {
    format!("{}/{}", service_id.namespace, service_id.name)
}

const fn default_service_endpoint_kind() -> EndpointKind {
    if cfg!(windows) {
        EndpointKind::WindowsNamedPipe
    } else {
        EndpointKind::UnixDomainSocket
    }
}

const fn endpoint_kind_arg(kind: EndpointKind, operation: &'static str) -> CliResult<&'static str> {
    match kind {
        EndpointKind::WindowsNamedPipe => Ok("windows-named-pipe"),
        EndpointKind::UnixDomainSocket => Ok("unix-domain-socket"),
        EndpointKind::Tcp => Ok("tcp"),
        EndpointKind::InProcess => Err(CliError::UnsupportedEndpointKind { operation, kind }),
    }
}

fn validate_public_endpoint_kind(kind: EndpointKind, operation: &'static str) -> CliResult<()> {
    endpoint_kind_arg(kind, operation).map(|_| ())
}

fn print_manifest_summary(manifest: &SessionManifest) {
    println!("  id: {}", manifest.id);
    println!("  project: {}", manifest.project_id);
    println!("  state: {}", format_state(manifest.state));
    println!("  workspace: {}", manifest.workspace_root.display());
    println!("  metadata: {}", manifest.run_dir.display());
    if let Some(reason) = failure_summary_line(manifest) {
        println!("  failure: {reason}");
    }
    if !manifest.services.is_empty() {
        println!("  services: {}", manifest.services.len());
    }
    if !manifest.processes.is_empty() {
        println!("  service processes: {}", manifest.processes.len());
    }
}

fn print_proto_session_workspace_status(status: &ProtoSessionWorkspaceStatus) {
    println!("Session '{}' status", status.manifest.slug);
    print_proto_manifest_summary(&status.manifest);
}

fn print_proto_manifest_summary(manifest: &ProtoSessionManifest) {
    println!("  id: {}", manifest.id);
    println!("  project: {}", manifest.project_id);
    println!("  state: {}", format_proto_state(manifest.state));
    println!("  workspace: {}", manifest.workspace_root);
    println!("  metadata: {}", manifest.run_dir);
    if let Some(reason) = failure_summary_line_from_run_dir(Path::new(&manifest.run_dir)) {
        println!("  failure: {reason}");
    }
    if !manifest.services.is_empty() {
        println!("  services: {}", manifest.services.len());
    }
    if !manifest.processes.is_empty() {
        println!("  service processes: {}", manifest.processes.len());
    }
}

fn print_proto_services_status(status: &ProtoSessionWorkspaceStatus) {
    println!("Session '{}' services", status.manifest.slug);
    print_proto_manifest_summary(&status.manifest);

    if status.manifest.services.is_empty() {
        println!("Services: none");
    } else {
        println!("Services:");
        for service in &status.manifest.services {
            print_service_descriptor(service);
        }
    }

    if status.manifest.processes.is_empty() {
        println!("Service processes: none");
    } else {
        println!("Service processes:");
        for process in &status.manifest.processes {
            print_proto_service_process(process);
        }
    }
}

fn print_service_registration(service: &str, endpoint: &Endpoint) {
    println!(
        "  service: {service} ({:?}, {})",
        endpoint.kind, endpoint.address
    );
}

fn print_service_descriptor(service: &ServiceDescriptor) {
    println!(
        "  {}.{}: {} run {} ({:?} {})",
        service.id.namespace,
        service.id.name,
        format_proto_service_role(service.role),
        service.run,
        service.endpoint.kind,
        service.endpoint.address
    );
    println!(
        "    protocol: {}.{}.{}",
        service.protocol.major, service.protocol.minor, service.protocol.patch
    );
    if service.capabilities.is_empty() {
        println!("    capabilities: none");
        return;
    }

    println!("    capabilities:");
    for capability in &service.capabilities {
        let permissions = if capability.permissions.is_empty() {
            "none".to_string()
        } else {
            capability.permissions.join(", ")
        };
        println!(
            "      {}.{} as {} -> {} [{}]",
            capability.service.namespace,
            capability.service.name,
            format_proto_service_role(capability.role),
            capability.audience,
            permissions
        );
    }
}

fn print_service_record(service: &ServiceRecord) {
    println!(
        "  {}.{}: {} run {} ({:?} {})",
        service.namespace,
        service.name,
        format_service_role(service.role),
        service.run,
        service.endpoint_kind,
        service.endpoint_address
    );
    println!(
        "    protocol: {}.{}.{}",
        service.protocol_major, service.protocol_minor, service.protocol_patch
    );
    if service.capabilities.is_empty() {
        println!("    capabilities: none");
        return;
    }

    println!("    capabilities:");
    for capability in &service.capabilities {
        let permissions = if capability.permissions.is_empty() {
            "none".to_string()
        } else {
            capability.permissions.join(", ")
        };
        println!(
            "      {}.{} as {} -> {} [{}]",
            capability.service_namespace,
            capability.service_name,
            format_service_role(capability.role),
            capability.audience,
            permissions
        );
    }
}

fn print_proto_service_process(process: &ProtoServiceProcessRecord) {
    let service_label = service_process_display_name(&process.owner_id, &process.service_name);
    println!(
        "  {}: {} run {} ({:?} {})",
        service_label,
        format_proto_process_state(process.state),
        process.run,
        process.endpoint.kind,
        process.endpoint.address
    );
    println!("    role: {}", format_proto_service_role(process.role));
    if !process.owner_root.trim().is_empty() {
        println!("    owner_root: {}", process.owner_root);
    }
    if let Some(pid) = process.pid {
        println!("    pid: {pid}");
    }
    if let Some(exit_code) = process.exit_code {
        println!("    exit_code: {exit_code}");
    }
    if let Some(failure) = process.failure.as_deref() {
        println!("    failure: {failure}");
    }
    println!("    cwd: {}", process.cwd);
    println!("    stdout: {}", process.stdout_log);
    println!("    stderr: {}", process.stderr_log);
    println!("    structured: {}", process.structured_log);
    println!(
        "    command: {}",
        shell_command(&process.program, &process.args)
    );
}

fn print_service_process(process: &ServiceProcessRecord) {
    let service_label = service_process_display_name(&process.owner_id, &process.service_name);
    println!(
        "  {}: {} run {} ({:?} {})",
        service_label,
        format_process_state(process.state),
        process.run,
        process.endpoint_kind,
        process.endpoint_address
    );
    println!("    role: {}", format_service_role(process.role));
    if !process.owner_root.as_os_str().is_empty() {
        println!("    owner_root: {}", process.owner_root.display());
    }
    if let Some(pid) = process.pid {
        println!("    pid: {pid}");
    }
    if let Some(exit_code) = process.exit_code {
        println!("    exit_code: {exit_code}");
    }
    if let Some(failure) = process.failure.as_deref() {
        println!("    failure: {failure}");
    }
    println!("    cwd: {}", process.cwd.display());
    println!("    stdout: {}", process.stdout_log.display());
    println!("    stderr: {}", process.stderr_log.display());
    if !process.structured_log.as_os_str().is_empty() {
        println!("    structured: {}", process.structured_log.display());
    }
    if let Some(ready_file) = process.ready_file.as_ref() {
        println!("    ready: {}", ready_file.display());
    }
    println!(
        "    command: {}",
        shell_command(&process.program, &process.args)
    );
}

fn service_process_display_name(owner_id: &str, service_name: &str) -> String {
    if owner_id.is_empty() {
        service_name.to_string()
    } else {
        format!("{owner_id}:{service_name}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceLogSelection {
    session_slug: String,
    service_name: String,
    run: Uuid,
    stream: ServiceLogStreamArg,
    path: PathBuf,
}

fn select_local_service_log(
    manifest: &SessionManifest,
    service: &str,
    run: Option<Uuid>,
    stream: ServiceLogStreamArg,
) -> CliResult<ServiceLogSelection> {
    let process = manifest
        .processes
        .iter()
        .find(|process| process.service_name == service)
        .ok_or_else(|| missing_service_process_error(&manifest.slug, service, run))?;
    let (run, previous) = match run {
        None => (process.run, false),
        Some(run) if run == process.run => (run, false),
        Some(run) if process.previous_run == Some(run) => (run, true),
        Some(run) => {
            return Err(missing_service_process_error(
                &manifest.slug,
                service,
                Some(run),
            ));
        }
    };

    Ok(ServiceLogSelection {
        session_slug: manifest.slug.clone(),
        service_name: process.service_name.clone(),
        run,
        stream,
        path: local_service_log_path(manifest, process, stream, previous)?,
    })
}

fn local_service_log_path(
    manifest: &SessionManifest,
    process: &ServiceProcessRecord,
    stream: ServiceLogStreamArg,
    previous: bool,
) -> CliResult<PathBuf> {
    let raw_path = match stream {
        ServiceLogStreamArg::Stdout => &process.stdout_log,
        ServiceLogStreamArg::Stderr => &process.stderr_log,
        ServiceLogStreamArg::Structured => &process.structured_log,
    };
    let raw_path = if previous {
        previous_log_path(raw_path)
    } else {
        raw_path.clone()
    };
    let path = if raw_path.is_absolute() {
        raw_path
    } else {
        manifest.run_dir.join(raw_path)
    };
    if path_has_parent_component(&path) || !path.starts_with(&manifest.run_dir) {
        return Err(CliError::InvalidServiceLogPath(Box::new(
            InvalidServiceLogPathDetails {
                session: manifest.slug.clone(),
                service: process.service_name.clone(),
                path,
                run_dir: manifest.run_dir.clone(),
            },
        )));
    }
    Ok(path)
}

fn path_has_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn same_protocol_path(left: &Path, right: &Path) -> bool {
    let left = comparable_protocol_path(left);
    let right = comparable_protocol_path(right);
    #[cfg(windows)]
    {
        comparable_protocol_path_text(&left) == comparable_protocol_path_text(&right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn comparable_protocol_path(path: &Path) -> PathBuf {
    if path.exists() {
        path.canonicalize().map_or_else(
            |_| cli_compatible_protocol_path(path.to_path_buf()),
            cli_compatible_protocol_path,
        )
    } else {
        cli_compatible_protocol_path(path.to_path_buf())
    }
}

#[cfg(windows)]
fn comparable_protocol_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', r"\")
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn cli_compatible_protocol_path(path: PathBuf) -> PathBuf {
    let path_text = path.to_string_lossy();
    if let Some(rest) = path_text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = path_text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[cfg(not(windows))]
fn cli_compatible_protocol_path(path: PathBuf) -> PathBuf {
    path
}

fn missing_service_process_error(session: &str, service: &str, run: Option<Uuid>) -> CliError {
    CliError::MissingServiceProcess(Box::new(MissingServiceProcessDetails {
        session: session.to_string(),
        service: service.to_string(),
        run: run.map(|run| format!(" run {run}")).unwrap_or_default(),
    }))
}

fn print_proto_service_log(result: &ProtoServiceLogResult) -> CliResult<()> {
    println!(
        "Session '{}' service '{}' {} log",
        result.session_slug,
        result.service_name,
        proto_service_log_stream_label(result.stream)
    );
    println!("  run: {}", result.run);
    println!("  path: {}", result.path);
    println!("---");
    for line in &result.lines {
        println!("{line}");
    }
    std::io::stdout().flush()?;
    Ok(())
}

fn print_service_log(
    selection: &ServiceLogSelection,
    tail: usize,
    all: bool,
    follow: bool,
) -> CliResult<()> {
    let (lines, follow_offset) = service_log_output(selection, tail, all)?;
    for line in lines {
        println!("{line}");
    }
    if follow {
        match selection.stream {
            ServiceLogStreamArg::Structured => {
                follow_structured_log_file_from(&selection.path, follow_offset)?;
            }
            ServiceLogStreamArg::Stdout | ServiceLogStreamArg::Stderr => {
                follow_log_file_from(&selection.path, follow_offset)?;
            }
        }
    }
    Ok(())
}

fn follow_session_service_log(
    path: &str,
    stream: ServiceLogStreamArg,
    start_offset: u64,
) -> CliResult<()> {
    let path = Path::new(path);
    match stream {
        ServiceLogStreamArg::Structured => follow_structured_log_file_from(path, start_offset),
        ServiceLogStreamArg::Stdout | ServiceLogStreamArg::Stderr => {
            follow_log_file_from(path, start_offset)
        }
    }
}

fn service_log_output(
    selection: &ServiceLogSelection,
    tail: usize,
    all: bool,
) -> CliResult<(Vec<String>, u64)> {
    let mut output = vec![
        format!(
            "Session '{}' service '{}' {} log",
            selection.session_slug,
            selection.service_name,
            service_log_stream_label(selection.stream)
        ),
        format!("  run: {}", selection.run),
        format!("  path: {}", selection.path.display()),
        "---".to_string(),
    ];
    let (log_lines, follow_offset) = match selection.stream {
        ServiceLogStreamArg::Structured => read_structured_log_lines_with_offset(
            &selection.path,
            if all { None } else { Some(tail) },
        )?,
        ServiceLogStreamArg::Stdout | ServiceLogStreamArg::Stderr => {
            read_log_lines_with_offset(&selection.path, if all { None } else { Some(tail) })?
        }
    };
    output.extend(log_lines);
    Ok((output, follow_offset))
}

fn read_log_lines_with_offset(path: &Path, tail: Option<usize>) -> CliResult<(Vec<String>, u64)> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let lines = match tail {
        None => reader
            .by_ref()
            .lines()
            .collect::<Result<Vec<_>, _>>()
            .map_err(CliError::from)?,
        Some(0) => {
            let offset = reader.seek(SeekFrom::End(0))?;
            return Ok((Vec::new(), offset));
        }
        Some(limit) => {
            let mut lines = VecDeque::with_capacity(limit);
            for line in reader.by_ref().lines() {
                if lines.len() == limit {
                    lines.pop_front();
                }
                lines.push_back(line?);
            }
            lines.into_iter().collect()
        }
    };
    let offset = reader.stream_position()?;
    Ok((lines, offset))
}

fn follow_log_file_from(path: &Path, start_offset: u64) -> CliResult<()> {
    let events = FileChangeEvents::new(path)?;
    let mut offset = start_offset;

    loop {
        drain_log_file_from(path, &mut offset)?;
        if events.wait()? {
            offset = 0;
        }
    }
}

fn drain_log_file_from(path: &Path, offset: &mut u64) -> CliResult<()> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let length = file.metadata()?.len();
    if length < *offset {
        *offset = 0;
    }
    file.seek(SeekFrom::Start(*offset))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut stdout = std::io::stdout();
    while reader.read_line(&mut line)? != 0 {
        writeln!(stdout, "{}", trim_log_line_end(&line))?;
        line.clear();
    }
    stdout.flush()?;
    *offset = reader.stream_position()?;
    Ok(())
}

fn read_structured_log_lines_with_offset(
    path: &Path,
    tail: Option<usize>,
) -> CliResult<(Vec<String>, u64)> {
    if matches!(tail, Some(0)) {
        let offset = File::open(path)?.metadata()?.len();
        return Ok((Vec::new(), offset));
    }

    let read = read_observed_log_file_from_offset(path, 0).map_err(|error| {
        CliError::StructuredServiceLog {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    let records = match tail {
        None => read.records,
        Some(limit) => {
            let keep = limit.min(read.records.len());
            let skip = read.records.len().saturating_sub(keep);
            read.records.into_iter().skip(skip).collect()
        }
    };
    Ok((
        records
            .iter()
            .map(format_log_record_for_console)
            .collect::<Vec<_>>(),
        read.next_offset,
    ))
}

fn follow_structured_log_file_from(path: &Path, start_offset: u64) -> CliResult<()> {
    let events = FileChangeEvents::new(path)?;
    let mut offset = start_offset;
    loop {
        if path.exists() {
            let length = match File::open(path) {
                Ok(file) => file.metadata()?.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if events.wait()? {
                        offset = 0;
                    }
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if length < offset {
                offset = 0;
            }
            let read = match read_observed_log_file_from_offset(path, offset) {
                Ok(read) => read,
                Err(ObservedLogFileError::Open { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    if events.wait()? {
                        offset = 0;
                    }
                    continue;
                }
                Err(error) => {
                    return Err(CliError::StructuredServiceLog {
                        path: path.to_path_buf(),
                        message: error.to_string(),
                    });
                }
            };
            for record in &read.records {
                println!("{}", format_log_record_for_console(record));
            }
            std::io::stdout().flush()?;
            offset = read.next_offset;
        }
        if events.wait()? {
            offset = 0;
        }
    }
}

fn trim_log_line_end(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

const fn service_log_stream_label(stream: ServiceLogStreamArg) -> &'static str {
    match stream {
        ServiceLogStreamArg::Stdout => "stdout",
        ServiceLogStreamArg::Stderr => "stderr",
        ServiceLogStreamArg::Structured => "structured",
    }
}

const fn proto_service_log_stream(stream: ServiceLogStreamArg) -> ProtoServiceLogStream {
    match stream {
        ServiceLogStreamArg::Stdout => ProtoServiceLogStream::Stdout,
        ServiceLogStreamArg::Stderr => ProtoServiceLogStream::Stderr,
        ServiceLogStreamArg::Structured => ProtoServiceLogStream::Structured,
    }
}

const fn proto_service_log_stream_label(stream: ProtoServiceLogStream) -> &'static str {
    match stream {
        ProtoServiceLogStream::Stdout => "stdout",
        ProtoServiceLogStream::Stderr => "stderr",
        ProtoServiceLogStream::Structured => "structured",
    }
}

fn print_runtime_status(status: &RuntimeStatus) {
    println!("  runtime: {}", status.runtime_id);
    println!("  role: {}", format_runtime_role(status.role));
    println!("  state: {}", format_runtime_state(status.state));
    println!("  project: {}", status.project_id);
    println!("  session: {}", status.session_slug);
    println!("  authored_revision: {}", status.authored_revision);
    if !status.diagnostic.is_empty() {
        println!("  diagnostic: {}", status.diagnostic);
    }
}

fn print_runtime_viewport_frame(runtime_id: &str, frame: Option<&RuntimeViewportFrame>) {
    let Some(frame) = frame else {
        println!("  runtime: {runtime_id}");
        println!("  frame: none");
        return;
    };

    println!("  runtime: {}", frame.runtime_id);
    println!("  dimensions: {}x{}", frame.width, frame.height);
    println!("  row_pitch: {}", frame.row_pitch);
    println!("  format: {}", format_viewport_pixel_format(frame.format));
    print_side_channel_handle("  color", &frame.color);
}

fn print_side_channel_handle(label: &str, handle: &SideChannelHandle) {
    println!("{label}:");
    println!("    kind: {}", format_side_channel_kind(handle.kind));
    println!("    locator: {}", handle.locator);
    println!("    platform: {}", empty_label(&handle.platform));
    println!("    byte_length: {}", handle.byte_length);
    println!(
        "    content_hash: {}",
        format_optional_hex_bytes(&handle.content_hash)
    );
    println!(
        "    capability: {}",
        handle
            .capability
            .as_ref()
            .map_or_else(|| "none".to_string(), format_side_channel_capability)
    );
}

const fn format_viewport_pixel_format(format: ViewportPixelFormat) -> &'static str {
    match format {
        ViewportPixelFormat::Unknown => "unknown",
        ViewportPixelFormat::Bgra8Unorm => "bgra8Unorm",
        ViewportPixelFormat::Rgba8Unorm => "rgba8Unorm",
        ViewportPixelFormat::Rgba16Float => "rgba16Float",
        ViewportPixelFormat::Depth32Float => "depth32Float",
    }
}

const fn format_side_channel_kind(kind: SideChannelKind) -> &'static str {
    match kind {
        SideChannelKind::SharedMemory => "sharedMemory",
        SideChannelKind::CasBlob => "casBlob",
        SideChannelKind::StagingFile => "stagingFile",
        SideChannelKind::GpuSurface => "gpuSurface",
        SideChannelKind::MmapFile => "mmapFile",
    }
}

fn format_side_channel_capability(capability: &Capability) -> String {
    let permissions = if capability.permissions.is_empty() {
        "none".to_string()
    } else {
        capability.permissions.join(", ")
    };
    let session = capability
        .session
        .map_or_else(|| "none".to_string(), |session| session.to_string());

    format!(
        "{}.{} as {} -> {} session={} permissions=[{}] expires_unix_ms={} token_hash={}",
        capability.service.namespace,
        capability.service.name,
        format_proto_service_role(capability.role),
        empty_label(&capability.audience),
        session,
        permissions,
        capability.expires_unix_ms,
        format_optional_hex_bytes(&capability.token_hash),
    )
}

const fn empty_label(value: &str) -> &str {
    if value.is_empty() { "none" } else { value }
}

fn format_optional_hex_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        "none".to_string()
    } else {
        format_hex_bytes(bytes)
    }
}

fn format_hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn ensure_source_session_result(
    source_path: &str,
    command: az_proto_project::vnext::SourceSessionCommand,
    result: &az_proto_project::vnext::SourceSessionResult,
) -> CliResult<()> {
    if result.diagnostics.is_empty() {
        return Ok(());
    }
    Err(CliError::InvalidServicePlan {
        message: format!(
            "project-host rejected {} for prefab source '{}': {}",
            source_session_command_label(command),
            source_path,
            prefab_diagnostics_label(&result.diagnostics)
        ),
    })
}

fn ensure_prefab_source_result<'a>(
    source_path: &str,
    result: &'a az_proto_project::vnext::PrefabRpcResult,
) -> CliResult<&'a az_proto_project::vnext::PrefabSourceSnapshot> {
    if !result.diagnostics.is_empty() {
        return Err(CliError::InvalidServicePlan {
            message: format!(
                "project-host could not read prefab source '{}': {}",
                source_path,
                prefab_diagnostics_label(&result.diagnostics)
            ),
        });
    }
    result
        .snapshot
        .as_ref()
        .ok_or_else(|| CliError::InvalidServicePlan {
            message: format!(
                "project-host returned no snapshot or diagnostic for prefab source '{source_path}'"
            ),
        })
}

fn prefab_diagnostics_label(diagnostics: &[az_proto_project::vnext::PrefabDiagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{} {}: {}",
                prefab_diagnostic_severity_label(diagnostic.severity),
                diagnostic.code,
                diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

const fn prefab_diagnostic_severity_label(
    severity: az_proto_project::vnext::DiagnosticSeverity,
) -> &'static str {
    match severity {
        az_proto_project::vnext::DiagnosticSeverity::Info => "info",
        az_proto_project::vnext::DiagnosticSeverity::Warning => "warning",
        az_proto_project::vnext::DiagnosticSeverity::Error => "error",
    }
}

const fn source_session_command_label(
    command: az_proto_project::vnext::SourceSessionCommand,
) -> &'static str {
    match command {
        az_proto_project::vnext::SourceSessionCommand::Open => "open",
        az_proto_project::vnext::SourceSessionCommand::Save => "save",
        az_proto_project::vnext::SourceSessionCommand::SaveRecovery => "save recovery",
        az_proto_project::vnext::SourceSessionCommand::Undo => "undo",
        az_proto_project::vnext::SourceSessionCommand::Redo => "redo",
        az_proto_project::vnext::SourceSessionCommand::Close => "close",
        az_proto_project::vnext::SourceSessionCommand::Status => "status",
    }
}

fn print_source_session_result(
    manifest: &SessionManifest,
    source_path: &str,
    operation: &str,
    result: &az_proto_project::vnext::SourceSessionResult,
) {
    println!(
        "Prefab source '{}' {} through session '{}'",
        source_path, operation, manifest.slug
    );
    println!(
        "  open={} revision={} dirty={} undo={} redo={}",
        result.status.open,
        result.status.revision,
        result.status.dirty,
        result.status.undo_depth,
        result.status.redo_depth
    );
    if let Some(snapshot) = &result.snapshot {
        print_prefab_source_snapshot(source_path, snapshot);
    }
}

fn print_prefab_source_snapshot(
    source_path: &str,
    snapshot: &az_proto_project::vnext::PrefabSourceSnapshot,
) {
    println!("Prefab source '{source_path}'");
    println!("  document_version: {}", snapshot.document_version);
    println!("  revision: {}", snapshot.revision);
    println!("  entities: {}", snapshot.entities.len());
    println!("  components: {}", snapshot.components.len());
    println!("  instances: {}", snapshot.instances.len());
    println!("  overrides: {}", snapshot.overrides.len());
    if !snapshot.components.is_empty() {
        println!("  component types:");
        for component in &snapshot.components {
            println!("    {}: {}", component.entity_alias, component.type_path);
        }
    }
}
fn print_source_file_create_result(manifest: &SessionManifest, result: &SourceFileCreateResult) {
    let entry = &result.record.entry;
    println!(
        "Source file '{}' created through session '{}'",
        entry.source_path, manifest.slug
    );
    println!(
        "  schema_type: {}",
        entry.schema_type.as_deref().unwrap_or("")
    );
    println!("  content_hash: {}", entry.content_hash);
    println!("  asset_guid: {}", result.record.asset_guid);
    println!("  workspace_asset_entry: {}", entry.entry_id);
}

fn print_workspace_entry_page(page: &WorkspaceEntryPageResult) {
    if page.entries.is_empty() {
        println!("No workspace entries found");
    } else {
        println!("Workspace entries:");
        for entry in &page.entries {
            print_workspace_entry(entry);
        }
    }

    match page.next_after_entry_id {
        Some(next) => println!("next_after_entry_id: {next}"),
        None => println!("next_after_entry_id: none"),
    }
}

fn print_workspace_snapshot(snapshot: Option<&WorkspaceSnapshot>) {
    let Some(snapshot) = snapshot else {
        println!("Attached workspace snapshot not found");
        return;
    };

    println!("Workspace snapshot {}", snapshot.workspace_id);
    println!("  project: {}", snapshot.project_id);
    println!("  workspace_root: {}", snapshot.workspace_root);
    println!("  branch: {}", snapshot.branch);
    println!("  created_unix_ms: {}", snapshot.created_unix_ms);
    println!("  updated_unix_ms: {}", snapshot.updated_unix_ms);
    if snapshot.roots.is_empty() {
        println!("  roots: none");
    } else {
        println!("  roots:");
        for root in &snapshot.roots {
            println!(
                "    {} {} {} -> {}",
                root.workspace_root_id, root.owner_id, root.portable_key, root.source_root
            );
            println!(
                "      scan_folder: {} display: {} output_prefix: {}",
                root.root_id, root.display_name, root.output_prefix
            );
        }
    }
}

fn print_asset_builder_catalog(catalog: &AssetBuilderCatalogResult) {
    if catalog.builders.is_empty() && catalog.source_schemas.is_empty() {
        println!("No asset builders or source schemas registered");
        return;
    }

    println!("Asset builders: {}", catalog.builders.len());
    for builder in &catalog.builders {
        print_asset_builder(builder);
    }
    println!("Source schemas: {}", catalog.source_schemas.len());
    for source_schema in &catalog.source_schemas {
        print_asset_source_schema(source_schema);
    }
}

fn print_asset_builder(builder: &AssetBuilderDescriptor) {
    println!(
        "  {} {} v{}",
        builder.name, builder.builder_guid, builder.version
    );
    if builder.patterns.is_empty() {
        println!("    patterns: none");
    } else {
        println!("    patterns:");
        for pattern in &builder.patterns {
            println!(
                "      {} {}",
                format_asset_builder_pattern_kind(pattern.kind),
                pattern.pattern
            );
        }
    }
    if builder.source_schema_types.is_empty() {
        println!("    source_schemas: any");
    } else {
        println!("    source_schemas:");
        for schema_type in &builder.source_schema_types {
            println!("      {schema_type}");
        }
    }
}

const fn format_asset_builder_pattern_kind(kind: AssetBuilderPatternKind) -> &'static str {
    match kind {
        AssetBuilderPatternKind::Wildcard => "wildcard",
        AssetBuilderPatternKind::Regex => "regex",
    }
}

fn print_asset_source_schema(source_schema: &SourceSchemaDescriptor) {
    println!("  {}", source_schema.schema_type);
    if !source_schema.owner.is_empty() {
        println!("    owner: {}", source_schema.owner);
    }
    if !source_schema.label.is_empty() {
        println!("    label: {}", source_schema.label);
    }
    if !source_schema.category.is_empty() {
        println!("    category: {}", source_schema.category);
    }
    match &source_schema.authoring {
        SourceSchemaAuthoring::File { workflow } => {
            println!("    authoring: file");
            println!("    file_source_root: {}", workflow.source_root);
            println!(
                "    file_default_path_prefix: {}",
                if workflow.default_path_prefix.is_empty() {
                    "<source-root>"
                } else {
                    workflow.default_path_prefix.as_str()
                }
            );
            println!("    file_extensions: {}", workflow.extensions.join(", "));
            println!("    file_can_create: {}", workflow.can_create);
            println!("    file_can_edit: {}", workflow.can_edit);
        }
        SourceSchemaAuthoring::ProjectDocument { schema_type } => {
            println!("    authoring: project-document {schema_type}");
        }
    }
    if !source_schema.file_templates.is_empty() {
        println!("    file_templates: {}", source_schema.file_templates.len());
        for template in source_schema.file_templates.iter().take(8) {
            let label = if template.label.is_empty() {
                template.source_path.as_str()
            } else {
                template.label.as_str()
            };
            println!("      {} -> {}", label, template.source_path);
        }
        if source_schema.file_templates.len() > 8 {
            println!("      ... {} more", source_schema.file_templates.len() - 8);
        }
    }
}

fn print_runtime_projection_catalog(catalog: &RuntimeProjectionCatalogResult) {
    if catalog.projections.is_empty() {
        println!("No runtime projections registered");
        return;
    }

    println!("Runtime projections: {}", catalog.projections.len());
    for projection in &catalog.projections {
        print_runtime_projection(projection);
    }
}

fn print_runtime_projection(projection: &RuntimeProjectionDescriptor) {
    println!("  {} priority {}", projection.name, projection.priority);
    println!(
        "    roles: {}",
        runtime_projection_roles_label(&projection.roles)
    );
    println!(
        "    launch_profiles: {}",
        runtime_projection_launch_profiles_label(&projection.launch_profiles)
    );
}

fn runtime_projection_roles_label(roles: &[RuntimeRole]) -> String {
    if roles.is_empty() {
        "all".to_string()
    } else {
        roles
            .iter()
            .map(|role| format_runtime_role(*role))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn runtime_projection_launch_profiles_label(profiles: &[String]) -> String {
    if profiles.is_empty() {
        "all".to_string()
    } else {
        profiles.join(", ")
    }
}

fn print_workspace_entry(entry: &WorkspaceEntry) {
    println!(
        "  {}: {} ({})",
        entry.entry_id,
        entry.source_path,
        format_workspace_entry_diff(entry.diff)
    );
    println!("    workspace: {}", entry.workspace_id);
    println!("    asset_guid: {}", entry.asset_guid);
    println!("    scan_folder: {}", entry.root_id);
    println!("    content_hash: {}", entry.content_hash);
    println!("    diagnostics: {}", entry.diagnostics_count);
    println!("    updated_unix_ms: {}", entry.updated_unix_ms);
    for activity in &entry.jobs {
        let job = &activity.job;
        println!(
            "    job {}: {} ({})",
            job.job_id,
            job.key,
            format_job_status(job.status)
        );
        println!("      owner: {}", format_job_owner(&job.owner));
        println!("      platform: {}", job.platform);
        if let Some(attempt) = &activity.attempt {
            println!(
                "      attempt {} ordinal {} ({})",
                attempt.attempt_id,
                attempt.ordinal,
                format_attempt_status(attempt.status)
            );
            println!("        errors: {}", attempt.error_count);
            println!("        warnings: {}", attempt.warning_count);
        }
    }
}

fn print_job_inspection(inspection: &JobInspection) {
    let job = &inspection.job;
    println!("Asset job {}", job.job_id);
    println!("  status: {}", format_job_status(job.status));
    println!("  owner: {}", format_job_owner(&job.owner));
    println!("  workspace: {}", job.workspace_id);
    println!("  source_guid: {}", job.source_guid);
    println!("  source_path: {}", job.source_path);
    println!("  source_root: {}", job.source_root);
    println!("  key: {}", job.key);
    println!("  platform: {}", job.platform);
    if let Some(attempt) = &inspection.attempt {
        println!(
            "  attempt: {} ordinal {} ({})",
            attempt.attempt_id,
            attempt.ordinal,
            format_attempt_status(attempt.status)
        );
        if let Some(owner) = attempt.owner.as_deref() {
            println!("    owner: {owner}");
        }
        if let Some(staging) = attempt.staging.as_deref() {
            println!("    staging: {staging}");
        }
        if let Some(finished) = attempt.finished_unix_ms {
            println!("    finished_unix_ms: {finished}");
        }
        println!("    errors: {}", attempt.error_count);
        println!("    warnings: {}", attempt.warning_count);
    }
    println!("  products: {}", inspection.products.len());
    for product in &inspection.products {
        println!(
            "  {}: {} type {} sub {}",
            product.product_id, product.path, product.asset_type, product.sub_id
        );
        println!(
            "    format: {} v{}",
            product.product_format, product.product_format_version
        );
        println!("    content_hash: {}", product.content_hash);
        println!("    bytes: {}", product.byte_length);
        for edge in &product.edges {
            println!(
                "    dependency edge {}: {} sub {} flags 0x{:x}",
                edge.product_edge_id, edge.asset_guid, edge.sub_id, edge.flags
            );
        }
    }
    println!("  dependencies: {}", inspection.dependencies.len());
    for dependency in &inspection.dependencies {
        println!(
            "    {}: {}:{} ({})",
            dependency.job_edge_id,
            dependency.key,
            dependency.platform,
            format_job_dependency_kind(dependency.kind)
        );
    }
}

fn print_catalog_products(platform: &str, entries: &[CatalogProductEntry]) {
    if entries.is_empty() {
        println!("No catalog products found for platform {platform}");
        return;
    }

    println!(
        "Catalog products for platform {platform}: {}",
        entries.len()
    );
    for entry in entries {
        println!(
            "  {}: {} type {} sub {}",
            entry.product_id, entry.product_path, entry.asset_type, entry.sub_id
        );
        println!("    source: {} ({})", entry.source_path, entry.asset_guid);
        println!("    builder: {} job {}", entry.builder_guid, entry.job_key);
        println!("    content_hash: {}", entry.content_hash);
        println!("    bytes: {}", entry.byte_length);
    }
}

const fn format_job_dependency_kind(kind: JobDependencyKind) -> &'static str {
    match kind {
        JobDependencyKind::Order => "order",
        JobDependencyKind::Fingerprint => "fingerprint",
        JobDependencyKind::OrderOnly => "order-only",
    }
}

fn failure_summary_line(manifest: &SessionManifest) -> Option<String> {
    SessionManager::failure_reason_for_manifest(manifest)
        .ok()
        .flatten()
        .and_then(|reason| first_non_empty_line(&reason).map(str::to_string))
}

fn failure_summary_line_from_run_dir(run_dir: &Path) -> Option<String> {
    std::fs::read_to_string(run_dir.join("failure.txt"))
        .ok()
        .and_then(|reason| first_non_empty_line(&reason).map(str::to_string))
}

fn first_non_empty_line(reason: &str) -> Option<&str> {
    reason.lines().map(str::trim).find(|line| !line.is_empty())
}

fn shell_command(program: &str, args: &[String]) -> String {
    let mut command = shell_arg(program);
    for arg in args {
        command.push(' ');
        command.push_str(&shell_arg(arg));
    }
    command
}

fn shell_arg(arg: &str) -> String {
    if !arg.is_empty()
        && arg.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '/' | '\\')
        })
    {
        arg.to_string()
    } else {
        format!("\"{}\"", arg.replace('"', "\\\""))
    }
}

const fn format_state(state: SessionState) -> &'static str {
    match state {
        SessionState::Preparing => "preparing",
        SessionState::Active => "active",
        SessionState::FailedPreserved => "failed-preserved",
        SessionState::Removed => "removed",
    }
}

const fn format_proto_state(state: ProtoSessionState) -> &'static str {
    match state {
        ProtoSessionState::Preparing => "preparing",
        ProtoSessionState::Active => "active",
        ProtoSessionState::FailedPreserved => "failed-preserved",
        ProtoSessionState::Removed => "removed",
    }
}

const fn format_process_state(state: ServiceProcessState) -> &'static str {
    match state {
        ServiceProcessState::Planned => "planned",
        ServiceProcessState::Starting => "starting",
        ServiceProcessState::Running => "running",
        ServiceProcessState::Exited => "exited",
        ServiceProcessState::Failed => "failed",
    }
}

const fn format_proto_process_state(state: ProtoServiceProcessState) -> &'static str {
    match state {
        ProtoServiceProcessState::Planned => "planned",
        ProtoServiceProcessState::Starting => "starting",
        ProtoServiceProcessState::Running => "running",
        ProtoServiceProcessState::Exited => "exited",
        ProtoServiceProcessState::Failed => "failed",
    }
}

const fn format_service_role(role: SupervisedServiceRole) -> &'static str {
    match role {
        SupervisedServiceRole::Editor => "editor",
        SupervisedServiceRole::Daemon => "daemon",
        SupervisedServiceRole::SessionSupervisor => "session-supervisor",
        SupervisedServiceRole::ProjectHost => "project-host",
        SupervisedServiceRole::AssetProcessor => "asset-processor",
        SupervisedServiceRole::RuntimeHost => "runtime-host",
        SupervisedServiceRole::Worker => "worker",
    }
}

const fn format_proto_service_role(role: ServiceRole) -> &'static str {
    match role {
        ServiceRole::Unknown => "unknown",
        ServiceRole::Editor => "editor",
        ServiceRole::Daemon => "daemon",
        ServiceRole::SessionSupervisor => "session-supervisor",
        ServiceRole::ProjectHost => "project-host",
        ServiceRole::AssetProcessor => "asset-processor",
        ServiceRole::RuntimeHost => "runtime-host",
        ServiceRole::Worker => "worker",
    }
}

const fn format_runtime_role(role: RuntimeRole) -> &'static str {
    match role {
        RuntimeRole::EditorWorld => "editor-world",
        RuntimeRole::PlayPreview => "play-preview",
        RuntimeRole::ServerPreview => "server-preview",
        RuntimeRole::Validation => "validation",
        RuntimeRole::Thumbnail => "thumbnail",
        RuntimeRole::Bake => "bake",
    }
}

const fn format_runtime_state(state: RuntimeState) -> &'static str {
    match state {
        RuntimeState::Stopped => "stopped",
        RuntimeState::Starting => "starting",
        RuntimeState::Running => "running",
        RuntimeState::Failed => "failed",
    }
}

const fn format_workspace_entry_diff(diff: ProtoWorkspaceEntryDiff) -> &'static str {
    match diff {
        ProtoWorkspaceEntryDiff::Clean => "clean",
        ProtoWorkspaceEntryDiff::Added => "added",
        ProtoWorkspaceEntryDiff::Modified => "modified",
        ProtoWorkspaceEntryDiff::Deleted => "deleted",
        ProtoWorkspaceEntryDiff::Conflicted => "conflicted",
    }
}

const fn format_job_status(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Leased => "leased",
        JobStatus::Succeeded => "succeeded",
        JobStatus::Failed => "failed",
    }
}

const fn format_attempt_status(status: AttemptStatus) -> &'static str {
    match status {
        AttemptStatus::Leased => "leased",
        AttemptStatus::Succeeded => "succeeded",
        AttemptStatus::Failed => "failed",
        AttemptStatus::Abandoned => "abandoned",
    }
}

fn format_job_owner(owner: &JobOwner) -> String {
    match owner {
        JobOwner::Plan => "plan".to_string(),
        JobOwner::Build(builder_guid) => format!("build:{builder_guid}"),
    }
}

#[cfg(test)]
mod tests {
    use az_proto_asset::CatalogPathRegistration;
    use az_service_catalog::session_supervisor_service_descriptor;
    use az_session::{EDITOR_SERVICE_NAME, EDITOR_SERVICE_NAMESPACE, SessionId};

    use super::*;

    fn test_run(value: u8) -> Uuid {
        Uuid::from_bytes([value; 16])
    }

    fn write_test_project_manifest(root: &std::path::Path, id: &str) {
        az_project::write_project_manifest(
            root,
            &az_project::ProjectManifest::new(id, "Session CLI Test", "0.1.0"),
        )
        .unwrap();
        az_project::refresh_project_lock(root).unwrap();
    }

    fn test_session_manifest() -> SessionManifest {
        let temp = tempfile::tempdir().unwrap();
        SessionManifest::new(
            SessionId::new(),
            "local.test_session".to_string(),
            "editor-work".to_string(),
            temp.path().join("project"),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        )
    }

    fn valid_service_log_result(manifest: &SessionManifest) -> ProtoServiceLogResult {
        ProtoServiceLogResult {
            session_slug: manifest.slug.clone(),
            service_name: "project-host".to_string(),
            run: test_run(3),
            stream: ProtoServiceLogStream::Structured,
            path: manifest
                .run_dir
                .join("logs")
                .join("project-host.capnp.log")
                .to_string_lossy()
                .into_owned(),
            lines: vec!["INFO project-host ready".to_string()],
            next_offset: 128,
        }
    }

    fn valid_exec_result() -> ProtoExecCommandResult {
        ProtoExecCommandResult {
            success: true,
            exited: true,
            exit_code: 0,
            stdout: "ok\n".to_string(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    fn valid_session_workspace_status(manifest: &SessionManifest) -> ProtoSessionWorkspaceStatus {
        ProtoSessionWorkspaceStatus {
            manifest: az_session::session_manifest_to_proto(manifest),
            failure_reason: None,
        }
    }

    fn assert_session_supervisor_authority_mismatch(
        error: CliError,
        expected_operation: &'static str,
        expected_reason: &str,
    ) {
        match error {
            CliError::SessionSupervisorAuthorityMismatch { operation, reason } => {
                assert_eq!(operation, expected_operation);
                assert!(
                    reason.contains(expected_reason),
                    "expected `{reason}` to contain `{expected_reason}`"
                );
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    fn assert_asset_processor_authority_mismatch(
        error: CliError,
        expected_operation: &'static str,
        expected_reason: &str,
    ) {
        match error {
            CliError::AssetProcessorAuthorityMismatch { operation, reason } => {
                assert_eq!(operation, expected_operation);
                assert!(
                    reason.contains(expected_reason),
                    "expected `{reason}` to contain `{expected_reason}`"
                );
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    fn assert_runtime_host_authority_mismatch(
        error: CliError,
        expected_operation: &'static str,
        expected_reason: &str,
    ) {
        match error {
            CliError::RuntimeHostAuthorityMismatch { operation, reason } => {
                assert_eq!(operation, expected_operation);
                assert!(
                    reason.contains(expected_reason),
                    "expected `{reason}` to contain `{expected_reason}`"
                );
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    fn valid_job_activity(
        job_id: i64,
        workspace_id: i64,
        asset_guid: Uuid,
        source_path: &str,
    ) -> JobActivity {
        JobActivity {
            job: az_proto_asset::JobRecord {
                job_id,
                workspace_id,
                source_guid: asset_guid,
                source_path: source_path.to_string(),
                source_root: "projects/example/assets".to_string(),
                source_schema_type: Some("az.tests.Prefab".to_string()),
                owner: JobOwner::Build(Uuid::from_bytes([8; 16])),
                key: "BuildPrefab".to_string(),
                platform: "pc".to_string(),
                status: JobStatus::Succeeded,
                ready: true,
                attempts: 1,
            },
            attempt: Some(az_proto_asset::JobAttemptRecord {
                attempt_id: job_id + 1_000,
                job_id,
                ordinal: 1,
                status: AttemptStatus::Succeeded,
                owner: None,
                staging: Some("projects/example/.azoth/staging".to_string()),
                finished_unix_ms: Some(20),
                error_count: 0,
                warning_count: 0,
            }),
        }
    }

    fn valid_workspace_entry(entry_id: i64, workspace_id: i64) -> WorkspaceEntry {
        let source_path = format!("prefabs/asset-{entry_id}.prefab.ron");
        let asset_guid = Uuid::from_u128(
            u128::try_from(entry_id).expect("workspace entry ids in fixtures are non-negative") + 1,
        );
        WorkspaceEntry {
            entry_id,
            workspace_id,
            asset_guid,
            root_id: 40,
            source_path: source_path.clone(),
            schema_type: Some("az.tests.Prefab".to_string()),
            content_hash: "ab".repeat(32),
            diff: ProtoWorkspaceEntryDiff::Clean,
            diagnostics_count: 0,
            updated_unix_ms: 30,
            jobs: vec![valid_job_activity(
                entry_id + 100,
                workspace_id,
                asset_guid,
                &source_path,
            )],
        }
    }

    fn valid_job_inspection(job_id: i64) -> JobInspection {
        let activity = valid_job_activity(
            job_id,
            7,
            Uuid::from_bytes([9; 16]),
            "prefabs/job.prefab.ron",
        );
        JobInspection {
            job: activity.job,
            attempt: activity.attempt,
            products: vec![az_proto_asset::JobProductRecord {
                product_id: job_id + 100,
                job_id,
                path: "products/job.azasset".to_string(),
                asset_type: Uuid::from_bytes([10; 16]),
                sub_id: 0,
                product_format: "az.test.raw".to_string(),
                product_format_version: 1,
                catalog_path_registration: CatalogPathRegistration::Registered,
                content_hash: "cd".repeat(32),
                byte_length: 128,
                aliases: Vec::new(),
                edges: vec![az_proto_asset::JobProductEdgeRecord {
                    product_edge_id: job_id + 200,
                    product_id: job_id + 100,
                    asset_guid: Uuid::from_bytes([11; 16]),
                    sub_id: 0,
                    flags: 0,
                }],
            }],
            dependencies: vec![az_proto_asset::JobDependencyRecord {
                job_edge_id: job_id + 300,
                job_id,
                target: az_proto_asset::JobDependencyTarget::Path(
                    "materials/base.material.ron".to_string(),
                ),
                key: "BuildMaterial".to_string(),
                platform: "pc".to_string(),
                kind: JobDependencyKind::Fingerprint,
            }],
        }
    }

    fn valid_catalog_product(product_id: i64, product_path: &str) -> CatalogProductEntry {
        CatalogProductEntry {
            job_id: product_id + 100,
            product_id,
            asset_guid: Uuid::from_bytes([11; 16]),
            source_path: format!("prefabs/source-{product_id}.prefab.ron"),
            builder_guid: Uuid::from_bytes([12; 16]),
            job_key: "BuildPrefab".to_string(),
            platform: "pc".to_string(),
            product_path: product_path.to_string(),
            asset_type: Uuid::from_bytes([13; 16]),
            sub_id: 0,
            product_format: "az.test.raw".to_string(),
            product_format_version: 1,
            content_hash: "ef".repeat(32),
            catalog_aliases: Vec::new(),
            catalog_path_registration: CatalogPathRegistration::Registered,
            byte_length: 128,
            dependencies: Vec::new(),
        }
    }

    fn valid_asset_builder(builder_guid: Uuid) -> AssetBuilderDescriptor {
        AssetBuilderDescriptor {
            name: "PrefabBuilder".to_string(),
            builder_guid,
            version: 1,
            analysis_fingerprint: String::new(),
            patterns: vec![AssetBuilderPatternDescriptor {
                kind: AssetBuilderPatternKind::Wildcard,
                pattern: "*.prefab.ron".to_string(),
            }],
            source_schema_types: vec!["az.tests.Prefab".to_string()],
        }
    }

    fn valid_source_schema() -> SourceSchemaDescriptor {
        source_schema_creating("az.tests.Prefab", "az.tests.Prefab")
    }

    fn source_schema_creating(
        source_schema_type: &str,
        document_schema_type: &str,
    ) -> SourceSchemaDescriptor {
        SourceSchemaDescriptor {
            schema_type: source_schema_type.to_string(),
            owner: "session-tests".to_string(),
            label: "Prefab".to_string(),
            category: "Authoring".to_string(),
            authoring: SourceSchemaAuthoring::ProjectDocument {
                schema_type: document_schema_type.to_string(),
            },
            file_templates: Vec::new(),
        }
    }

    fn source_schema_file_only(source_schema_type: &str) -> SourceSchemaDescriptor {
        SourceSchemaDescriptor {
            schema_type: source_schema_type.to_string(),
            owner: "session-tests".to_string(),
            label: "Prefab".to_string(),
            category: "Authoring".to_string(),
            authoring: SourceSchemaAuthoring::File {
                workflow: SourceFileWorkflowDescriptor {
                    source_root: az_proto_asset::PROJECT_SOURCE_ROOT.to_string(),
                    default_path_prefix: "legacy".to_string(),
                    extensions: vec!["xml".to_string()],
                    can_create: false,
                    can_edit: false,
                },
            },
            file_templates: Vec::new(),
        }
    }

    fn source_schema_creatable_file(
        source_schema_type: &str,
        extensions: Vec<&str>,
    ) -> SourceSchemaDescriptor {
        SourceSchemaDescriptor {
            schema_type: source_schema_type.to_string(),
            owner: "session-tests".to_string(),
            label: "GameData Table".to_string(),
            category: "Authoring".to_string(),
            authoring: SourceSchemaAuthoring::File {
                workflow: SourceFileWorkflowDescriptor {
                    source_root: az_proto_asset::PROJECT_SOURCE_ROOT.to_string(),
                    default_path_prefix: "gamedata/tables".to_string(),
                    extensions: extensions.into_iter().map(str::to_string).collect(),
                    can_create: true,
                    can_edit: true,
                },
            },
            file_templates: Vec::new(),
        }
    }

    fn valid_runtime_status(runtime_id: &str, role: RuntimeRole) -> RuntimeStatus {
        RuntimeStatus {
            runtime_id: runtime_id.to_string(),
            role,
            state: RuntimeState::Running,
            project_id: "local.test_session".to_string(),
            session_slug: "editor-work".to_string(),
            authored_revision: 42,
            diagnostic: String::new(),
        }
    }

    fn valid_runtime_viewport_frame(runtime_id: &str) -> RuntimeViewportFrame {
        RuntimeViewportFrame {
            runtime_id: runtime_id.to_string(),
            width: 1280,
            height: 720,
            row_pitch: 5120,
            format: ViewportPixelFormat::Bgra8Unorm,
            color: SideChannelHandle {
                kind: SideChannelKind::GpuSurface,
                capability: None,
                locator: "dxgi://adapter/0/editor-world/3".to_string(),
                byte_length: 3_686_400,
                content_hash: Vec::new(),
                platform: "windows".to_string(),
            },
        }
    }

    fn valid_runtime_projection_catalog() -> RuntimeProjectionCatalogResult {
        RuntimeProjectionCatalogResult {
            projections: vec![RuntimeProjectionDescriptor {
                name: "editor-world".to_string(),
                priority: 100,
                roles: vec![RuntimeRole::EditorWorld],
                launch_profiles: vec!["default".to_string()],
            }],
        }
    }

    fn runtime_workspace_snapshot(
        manifest: &SessionManifest,
        roots: Vec<az_proto_asset::WorkspaceRoot>,
    ) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            workspace_id: 42,
            project_id: manifest.project_id.clone(),
            workspace_root: manifest.workspace_root.to_string_lossy().into_owned(),
            branch: "main".to_string(),
            created_unix_ms: 100,
            updated_unix_ms: 200,
            roots,
        }
    }

    fn validate_runtime_workspace_snapshot(
        manifest: &SessionManifest,
        snapshot: &WorkspaceSnapshot,
    ) -> CliResult<()> {
        ensure_runtime_launch_workspace_snapshot(manifest, snapshot)
    }

    fn runtime_project_source_root(manifest: &SessionManifest) -> az_proto_asset::WorkspaceRoot {
        az_proto_asset::WorkspaceRoot {
            workspace_root_id: 900,
            workspace_id: 42,
            root_id: 901,
            declared_root_id: "project.assets".to_string(),
            owner_id: "local.test_session".to_string(),
            source_root: manifest
                .workspace_root
                .join("assets")
                .to_string_lossy()
                .into_owned(),
            display_name: "Project Assets".to_string(),
            portable_key: "project:local.test_session:assets".to_string(),
            mount: "@assets@".to_string(),
            recursive: true,
            watch: true,
            writable: true,
            output_prefix: String::new(),
            is_root: true,
        }
    }

    fn runtime_gem_asset_source_root() -> az_proto_asset::WorkspaceRoot {
        az_proto_asset::WorkspaceRoot {
            workspace_root_id: 902,
            workspace_id: 42,
            root_id: 903,
            declared_root_id: "gem.azoth.physics.assets".to_string(),
            owner_id: "azoth.physics".to_string(),
            source_root: "projects/example/.azoth/workspaces/editor-work/gems/physics/assets"
                .to_string(),
            display_name: "Physics Assets".to_string(),
            portable_key: "gem:azoth.physics:assets".to_string(),
            mount: "@gems/physics/assets@".to_string(),
            recursive: true,
            watch: true,
            writable: false,
            output_prefix: "gems/azoth.physics".to_string(),
            is_root: false,
        }
    }

    #[test]
    fn cli_service_log_validation_requires_echo_and_safe_path() {
        let manifest = test_session_manifest();
        let mut result = valid_service_log_result(&manifest);

        ensure_cli_service_log_result_matches_request(
            &result,
            &manifest,
            "project-host",
            Some(test_run(3)),
            ServiceLogStreamArg::Structured,
            Some(64),
        )
        .unwrap();

        result.session_slug = "other-session".to_string();
        let error = ensure_cli_service_log_result_matches_request(
            &result,
            &manifest,
            "project-host",
            Some(test_run(3)),
            ServiceLogStreamArg::Structured,
            Some(64),
        )
        .unwrap_err();
        assert_session_supervisor_authority_mismatch(error, "serviceLog", "expected `editor-work`");

        let mut result = valid_service_log_result(&manifest);
        result.service_name = "runtime-host".to_string();
        let error = ensure_cli_service_log_result_matches_request(
            &result,
            &manifest,
            "project-host",
            Some(test_run(3)),
            ServiceLogStreamArg::Structured,
            Some(64),
        )
        .unwrap_err();
        assert_session_supervisor_authority_mismatch(
            error,
            "serviceLog",
            "expected `project-host`",
        );

        let mut result = valid_service_log_result(&manifest);
        result.path = "../outside.log".to_string();
        let error = ensure_cli_service_log_result_matches_request(
            &result,
            &manifest,
            "project-host",
            Some(test_run(3)),
            ServiceLogStreamArg::Structured,
            Some(64),
        )
        .unwrap_err();
        assert!(matches!(error, CliError::InvalidServiceLogPath(_)));
    }

    #[test]
    fn cli_service_log_validation_requires_run_stream_and_offset() {
        let manifest = test_session_manifest();
        let mut result = valid_service_log_result(&manifest);
        result.run = test_run(2);
        let error = ensure_cli_service_log_result_matches_request(
            &result,
            &manifest,
            "project-host",
            Some(test_run(3)),
            ServiceLogStreamArg::Structured,
            Some(64),
        )
        .unwrap_err();
        assert_session_supervisor_authority_mismatch(error, "serviceLog", &test_run(3).to_string());

        let mut result = valid_service_log_result(&manifest);
        result.stream = ProtoServiceLogStream::Stdout;
        let error = ensure_cli_service_log_result_matches_request(
            &result,
            &manifest,
            "project-host",
            Some(test_run(3)),
            ServiceLogStreamArg::Structured,
            Some(64),
        )
        .unwrap_err();
        assert_session_supervisor_authority_mismatch(error, "serviceLog", "expected structured");

        let mut result = valid_service_log_result(&manifest);
        result.next_offset = 63;
        let error = ensure_cli_service_log_result_matches_request(
            &result,
            &manifest,
            "project-host",
            Some(test_run(3)),
            ServiceLogStreamArg::Structured,
            Some(64),
        )
        .unwrap_err();
        assert_session_supervisor_authority_mismatch(
            error,
            "serviceLog",
            "before requested offset",
        );

        let mut result = valid_service_log_result(&manifest);
        result.run = Uuid::nil();
        let error = ensure_cli_service_log_result_matches_request(
            &result,
            &manifest,
            "project-host",
            None,
            ServiceLogStreamArg::Structured,
            None,
        )
        .unwrap_err();
        assert_session_supervisor_authority_mismatch(error, "serviceLog", "must not be nil");
    }

    #[test]
    fn cli_exec_result_validation_requires_coherent_exit_state() {
        let result = valid_exec_result();
        ensure_cli_exec_command_result_matches_request(&result, 1024).unwrap();

        let mut result = valid_exec_result();
        result.success = true;
        result.exited = false;
        let error = ensure_cli_exec_command_result_matches_request(&result, 1024).unwrap_err();
        assert_session_supervisor_authority_mismatch(error, "execCommand", "exited process");

        let mut result = valid_exec_result();
        result.exit_code = 7;
        let error = ensure_cli_exec_command_result_matches_request(&result, 1024).unwrap_err();
        assert_session_supervisor_authority_mismatch(error, "execCommand", "exit code 0");

        let mut result = valid_exec_result();
        result.success = false;
        result.exited = false;
        result.exit_code = 9;
        let error = ensure_cli_exec_command_result_matches_request(&result, 1024).unwrap_err();
        assert_session_supervisor_authority_mismatch(
            error,
            "execCommand",
            "cannot carry exit code",
        );
    }

    #[test]
    fn cli_exec_result_validation_bounds_printed_output() {
        let mut result = valid_exec_result();
        result.stdout = "abcd".to_string();
        let error = ensure_cli_exec_command_result_matches_request(&result, 1).unwrap_err();
        assert_session_supervisor_authority_mismatch(error, "execCommand", "stdout length");

        let mut result = valid_exec_result();
        result.success = false;
        result.exit_code = 1;
        result.stderr = "abcd".to_string();
        let error = ensure_cli_exec_command_result_matches_request(&result, 1).unwrap_err();
        assert_session_supervisor_authority_mismatch(error, "execCommand", "stderr length");
    }

    #[test]
    fn cli_session_workspace_status_validation_requires_identity_and_failure_context() {
        let manifest = test_session_manifest();
        let status = valid_session_workspace_status(&manifest);
        ensure_cli_session_workspace_status_matches_manifest(
            &status,
            &manifest,
            "session-supervisor status",
        )
        .unwrap();

        let mut failed_manifest = manifest;
        failed_manifest.state = SessionState::FailedPreserved;
        let failed_status = valid_session_workspace_status(&failed_manifest);
        let error = ensure_cli_session_workspace_status_matches_manifest(
            &failed_status,
            &failed_manifest,
            "session-supervisor status",
        )
        .unwrap_err();
        assert_session_supervisor_authority_mismatch(
            error,
            "session-supervisor status",
            "failure reason",
        );
    }

    #[test]
    fn cli_create_source_file_workflow_requires_creatable_file_schema() {
        let asset_builder_catalog = AssetBuilderCatalogResult {
            builders: vec![valid_asset_builder(Uuid::from_bytes([26; 16]))],
            source_schemas: vec![source_schema_creatable_file(
                "azoth.gamedata.TableSource",
                vec!["ron"],
            )],
            product_formats: Vec::new(),
        };

        ensure_cli_source_file_workflow_matches_catalog(
            &asset_builder_catalog,
            "azoth.gamedata.TableSource",
            "achievementdata/achievementdatatable.ron",
            false,
        )
        .unwrap();

        let error = ensure_cli_source_file_workflow_matches_catalog(
            &asset_builder_catalog,
            "azoth.gamedata.TableSource",
            "achievementdata/achievementdatatable.xml",
            false,
        )
        .unwrap_err();
        assert_asset_processor_authority_mismatch(error, "createSourceFile", "extensions");

        let file_only_catalog = AssetBuilderCatalogResult {
            builders: vec![valid_asset_builder(Uuid::from_bytes([27; 16]))],
            source_schemas: vec![source_schema_file_only("azoth.gamedata.TableSource")],
            product_formats: Vec::new(),
        };
        let error = ensure_cli_source_file_workflow_matches_catalog(
            &file_only_catalog,
            "azoth.gamedata.TableSource",
            "achievementdata/achievementdatatable.xml",
            false,
        )
        .unwrap_err();
        assert!(matches!(error, CliError::InvalidAuthoredEdit { .. }));
        assert!(error.to_string().contains("no default create workflow"));

        ensure_cli_source_file_workflow_matches_catalog(
            &file_only_catalog,
            "azoth.gamedata.TableSource",
            "legacy/imported.xml",
            true,
        )
        .unwrap();
    }

    #[test]
    fn cli_create_source_file_workflow_rejects_project_document_schema() {
        let asset_builder_catalog = AssetBuilderCatalogResult {
            builders: vec![valid_asset_builder(Uuid::from_bytes([28; 16]))],
            source_schemas: vec![source_schema_creating(
                "az.cli.tests.SetFieldSource",
                "az.cli.tests.SetFieldTemplate",
            )],
            product_formats: Vec::new(),
        };

        let error = ensure_cli_source_file_workflow_matches_catalog(
            &asset_builder_catalog,
            "az.cli.tests.SetFieldSource",
            "prefabs/door.prefab.ron",
            false,
        )
        .unwrap_err();

        assert!(matches!(error, CliError::InvalidAuthoredEdit { .. }));
        assert!(error.to_string().contains("session document create"));
    }

    #[test]
    fn cli_workspace_snapshot_validation_requires_attached_identity_and_roots() {
        let manifest = test_session_manifest();
        let snapshot =
            runtime_workspace_snapshot(&manifest, vec![runtime_project_source_root(&manifest)]);
        ensure_cli_workspace_snapshot_matches_manifest(&snapshot, &manifest).unwrap();

        let mut wrong_project = snapshot.clone();
        wrong_project.project_id = "local.other".to_string();
        let error =
            ensure_cli_workspace_snapshot_matches_manifest(&wrong_project, &manifest).unwrap_err();
        assert_asset_processor_authority_mismatch(error, "workspaceSnapshot", "returned project");

        let mut duplicate_root = runtime_project_source_root(&manifest);
        duplicate_root.workspace_root_id += 1;
        duplicate_root.root_id += 1;
        let duplicate_snapshot = runtime_workspace_snapshot(
            &manifest,
            vec![runtime_project_source_root(&manifest), duplicate_root],
        );
        let error = ensure_cli_workspace_snapshot_matches_manifest(&duplicate_snapshot, &manifest)
            .unwrap_err();
        assert_asset_processor_authority_mismatch(
            error,
            "workspaceSnapshot",
            "duplicate source roots",
        );

        let mut wrong_owner = snapshot.clone();
        wrong_owner.roots[0].owner_id = "azoth.physics".to_string();
        let error =
            ensure_cli_workspace_snapshot_matches_manifest(&wrong_owner, &manifest).unwrap_err();
        assert_asset_processor_authority_mismatch(error, "workspaceSnapshot", "invalid");

        let mut wrong_path = snapshot.clone();
        wrong_path.roots[0].source_root = "projects/example/.azoth/workspaces/stale".to_string();
        let error =
            ensure_cli_workspace_snapshot_matches_manifest(&wrong_path, &manifest).unwrap_err();
        assert_asset_processor_authority_mismatch(error, "workspaceSnapshot", "invalid");

        let mut not_root = snapshot.clone();
        not_root.roots[0].is_root = false;
        let error =
            ensure_cli_workspace_snapshot_matches_manifest(&not_root, &manifest).unwrap_err();
        assert_asset_processor_authority_mismatch(error, "workspaceSnapshot", "invalid");

        let mut prefixed = snapshot;
        prefixed.roots[0].output_prefix = "prefabs".to_string();
        let error =
            ensure_cli_workspace_snapshot_matches_manifest(&prefixed, &manifest).unwrap_err();
        assert_asset_processor_authority_mismatch(error, "workspaceSnapshot", "invalid");
    }

    #[test]
    fn cli_workspace_entry_page_validation_requires_ordered_attached_entries() {
        let result = WorkspaceEntryPageResult {
            entries: vec![valid_workspace_entry(1, 7), valid_workspace_entry(2, 7)],
            next_after_entry_id: Some(2),
        };
        ensure_cli_workspace_entry_page_matches_request(&result, 7, None, 8).unwrap();

        let wrong_workspace = WorkspaceEntryPageResult {
            entries: vec![valid_workspace_entry(1, 8)],
            next_after_entry_id: None,
        };
        let error = ensure_cli_workspace_entry_page_matches_request(&wrong_workspace, 7, None, 8)
            .unwrap_err();
        assert_asset_processor_authority_mismatch(error, "workspaceEntryPage", "invalid identity");

        let bad_cursor = WorkspaceEntryPageResult {
            entries: vec![valid_workspace_entry(1, 7)],
            next_after_entry_id: Some(99),
        };
        let error =
            ensure_cli_workspace_entry_page_matches_request(&bad_cursor, 7, None, 8).unwrap_err();
        assert_asset_processor_authority_mismatch(
            error,
            "workspaceEntryPage",
            "did not match last entry",
        );

        let mut bad_activity = valid_workspace_entry(3, 7);
        bad_activity.jobs[0].job.source_path = "prefabs/other.prefab.ron".to_string();
        let error = ensure_cli_workspace_entry_page_matches_request(
            &WorkspaceEntryPageResult {
                entries: vec![bad_activity],
                next_after_entry_id: None,
            },
            7,
            None,
            8,
        )
        .unwrap_err();
        assert_asset_processor_authority_mismatch(
            error,
            "workspaceEntryPage",
            "contains an invalid job",
        );
    }

    #[test]
    fn cli_job_inspection_validation_requires_selector_and_canonical_owner() {
        let inspection = valid_job_inspection(42);
        ensure_cli_job_inspection_matches_request(&inspection, &InspectJobSelector::Job(42))
            .unwrap();
        ensure_cli_job_inspection_matches_request(&inspection, &InspectJobSelector::Attempt(1_042))
            .unwrap();

        let error =
            ensure_cli_job_inspection_matches_request(&inspection, &InspectJobSelector::Job(41))
                .unwrap_err();
        assert_asset_processor_authority_mismatch(error, "inspectJob", "expected 41");

        let mut nil_owner = inspection;
        nil_owner.job.owner = JobOwner::Build(Uuid::nil());
        let error =
            ensure_cli_job_inspection_matches_request(&nil_owner, &InspectJobSelector::Job(42))
                .unwrap_err();
        assert_asset_processor_authority_mismatch(error, "inspectJob", "nil builder guid");
    }

    #[test]
    fn cli_job_inspection_validation_rejects_invalid_products_and_dependencies() {
        let mut duplicate_product = valid_job_inspection(42);
        duplicate_product
            .products
            .push(duplicate_product.products[0].clone());
        let error = ensure_cli_job_inspection_matches_request(
            &duplicate_product,
            &InspectJobSelector::Job(42),
        )
        .unwrap_err();
        assert_asset_processor_authority_mismatch(error, "inspectJob", "invalid products");

        let mut unsafe_dependency = valid_job_inspection(42);
        unsafe_dependency.dependencies[0].target =
            az_proto_asset::JobDependencyTarget::Path("../materials/base.material.ron".to_string());
        let error = ensure_cli_job_inspection_matches_request(
            &unsafe_dependency,
            &InspectJobSelector::Job(42),
        )
        .unwrap_err();
        assert_asset_processor_authority_mismatch(error, "inspectJob", "invalid dependency target");
    }

    #[test]
    fn cli_catalog_products_validation_requires_platform_unique_sorted_entries() {
        let entries = vec![
            valid_catalog_product(1, "products/a.azasset"),
            valid_catalog_product(2, "products/b.azasset"),
        ];
        ensure_cli_catalog_products_matches_request(&entries, "pc").unwrap();

        let mut wrong_platform = valid_catalog_product(1, "products/a.azasset");
        wrong_platform.platform = "android".to_string();
        let error =
            ensure_cli_catalog_products_matches_request(&[wrong_platform], "pc").unwrap_err();
        assert_asset_processor_authority_mismatch(error, "catalogProducts", "invalid metadata");

        let duplicate = vec![
            valid_catalog_product(1, "products/a.azasset"),
            valid_catalog_product(1, "products/a.azasset"),
        ];
        let error = ensure_cli_catalog_products_matches_request(&duplicate, "pc").unwrap_err();
        assert_asset_processor_authority_mismatch(error, "catalogProducts", "duplicate product");

        let unsorted = vec![
            valid_catalog_product(2, "products/b.azasset"),
            valid_catalog_product(1, "products/a.azasset"),
        ];
        let error = ensure_cli_catalog_products_matches_request(&unsorted, "pc").unwrap_err();
        assert_asset_processor_authority_mismatch(error, "catalogProducts", "out of order");
    }

    #[test]
    fn cli_catalog_products_validation_requires_canonical_identity_paths() {
        let mut bad_source = valid_catalog_product(1, "products/a.azasset");
        bad_source.source_path = "../source.prefab.ron".to_string();
        let error = ensure_cli_catalog_products_matches_request(&[bad_source], "pc").unwrap_err();
        assert_asset_processor_authority_mismatch(
            error,
            "catalogProducts",
            "asset-db relative path",
        );

        let mut nil_type = valid_catalog_product(1, "products/a.azasset");
        nil_type.asset_type = Uuid::nil();
        let error = ensure_cli_catalog_products_matches_request(&[nil_type], "pc").unwrap_err();
        assert_asset_processor_authority_mismatch(error, "catalogProducts", "invalid metadata");

        let mut bad_hash = valid_catalog_product(1, "products/a.azasset");
        bad_hash.content_hash = "not-a-hash".to_string();
        let error = ensure_cli_catalog_products_matches_request(&[bad_hash], "pc").unwrap_err();
        assert_asset_processor_authority_mismatch(error, "catalogProducts", "invalid metadata");
    }

    #[test]
    fn cli_asset_builder_catalog_validation_requires_unique_valid_builders() {
        let builder = valid_asset_builder(Uuid::from_bytes([11; 16]));
        ensure_cli_asset_builder_catalog_result_matches_request(&AssetBuilderCatalogResult {
            builders: vec![builder.clone()],
            source_schemas: vec![valid_source_schema()],
            product_formats: vec![ProductFormatDescriptor {
                id: "az.test.prefab".to_string(),
                current_version: 1,
                owner: "az-test".to_string(),
            }],
        })
        .unwrap();

        let error =
            ensure_cli_asset_builder_catalog_result_matches_request(&AssetBuilderCatalogResult {
                builders: vec![builder.clone(), builder.clone()],
                source_schemas: vec![valid_source_schema()],
                product_formats: Vec::new(),
            })
            .unwrap_err();
        assert_asset_processor_authority_mismatch(
            error,
            "builderCatalog",
            "duplicate builder guid",
        );

        let mut empty_pattern = builder;
        empty_pattern.patterns[0].pattern.clear();
        let error =
            ensure_cli_asset_builder_catalog_result_matches_request(&AssetBuilderCatalogResult {
                builders: vec![empty_pattern],
                source_schemas: vec![valid_source_schema()],
                product_formats: Vec::new(),
            })
            .unwrap_err();
        assert_asset_processor_authority_mismatch(error, "builderCatalog", "empty");

        let mut duplicate_schema = valid_asset_builder(Uuid::from_bytes([12; 16]));
        duplicate_schema
            .source_schema_types
            .push("az.tests.Prefab".to_string());
        let error =
            ensure_cli_asset_builder_catalog_result_matches_request(&AssetBuilderCatalogResult {
                builders: vec![duplicate_schema],
                source_schemas: vec![valid_source_schema()],
                product_formats: Vec::new(),
            })
            .unwrap_err();
        assert_asset_processor_authority_mismatch(
            error,
            "builderCatalog",
            "duplicate source schema type",
        );
    }

    #[test]
    fn cli_asset_builder_catalog_validation_requires_valid_schemas_and_product_formats() {
        let source_schema = valid_source_schema();
        let error =
            ensure_cli_asset_builder_catalog_result_matches_request(&AssetBuilderCatalogResult {
                builders: vec![valid_asset_builder(Uuid::from_bytes([13; 16]))],
                source_schemas: vec![source_schema.clone(), source_schema],
                product_formats: Vec::new(),
            })
            .unwrap_err();
        assert_asset_processor_authority_mismatch(
            error,
            "builderCatalog",
            "duplicate source schema type",
        );

        let error =
            ensure_cli_asset_builder_catalog_result_matches_request(&AssetBuilderCatalogResult {
                builders: vec![valid_asset_builder(Uuid::from_bytes([14; 16]))],
                source_schemas: Vec::new(),
                product_formats: Vec::new(),
            })
            .unwrap_err();
        assert_asset_processor_authority_mismatch(
            error,
            "builderCatalog",
            "without a source schema descriptor",
        );

        let mut import_with_template = source_schema_file_only("az.compat.LegacyMaterialSource");
        import_with_template
            .file_templates
            .push(az_proto_asset::SourceFileTemplateDescriptor {
                owner: "legacy-materials".to_string(),
                source_path: "materials/default.mtl".to_string(),
                label: "Default Material".to_string(),
                description: "Empty material".to_string(),
            });
        let mut import_builder = valid_asset_builder(Uuid::from_bytes([15; 16]));
        import_builder.source_schema_types = vec![import_with_template.schema_type.clone()];
        let error =
            ensure_cli_asset_builder_catalog_result_matches_request(&AssetBuilderCatalogResult {
                builders: vec![import_builder],
                source_schemas: vec![import_with_template],
                product_formats: Vec::new(),
            })
            .unwrap_err();
        assert_asset_processor_authority_mismatch(error, "builderCatalog", "not default-creatable");

        let product_format = ProductFormatDescriptor {
            id: "az.test.prefab".to_string(),
            current_version: 1,
            owner: "az-test".to_string(),
        };
        let error =
            ensure_cli_asset_builder_catalog_result_matches_request(&AssetBuilderCatalogResult {
                builders: Vec::new(),
                source_schemas: Vec::new(),
                product_formats: vec![product_format.clone(), product_format],
            })
            .unwrap_err();
        assert_asset_processor_authority_mismatch(
            error,
            "builderCatalog",
            "duplicate product format id",
        );
    }

    #[test]
    fn cli_runtime_status_validation_requires_echo_role_and_identity() {
        let status = valid_runtime_status("editor-world", RuntimeRole::EditorWorld);
        ensure_cli_runtime_status_matches_request(
            &status,
            "editor-world",
            Some(RuntimeRole::EditorWorld),
            "launch",
        )
        .unwrap();

        let error =
            ensure_cli_runtime_status_matches_request(&status, "play-preview", None, "status")
                .unwrap_err();
        assert_runtime_host_authority_mismatch(error, "status", "expected `play-preview`");

        let mut wrong_role = status.clone();
        wrong_role.role = RuntimeRole::PlayPreview;
        let error = ensure_cli_runtime_status_matches_request(
            &wrong_role,
            "editor-world",
            Some(RuntimeRole::EditorWorld),
            "launch",
        )
        .unwrap_err();
        assert_runtime_host_authority_mismatch(error, "launch", "reported role");

        let mut missing_identity = status;
        missing_identity.project_id.clear();
        let error = ensure_cli_runtime_status_matches_request(
            &missing_identity,
            "editor-world",
            None,
            "status",
        )
        .unwrap_err();
        assert_runtime_host_authority_mismatch(error, "status", "project/session identity");
    }

    #[test]
    fn cli_runtime_viewport_frame_validation_requires_echo_and_layout() {
        let frame = valid_runtime_viewport_frame("editor-world");
        ensure_cli_runtime_viewport_frame_matches_request(&frame, "editor-world", "viewportFrame")
            .unwrap();

        let error = ensure_cli_runtime_viewport_frame_matches_request(
            &frame,
            "play-preview",
            "viewportFrame",
        )
        .unwrap_err();
        assert_runtime_host_authority_mismatch(error, "viewportFrame", "expected `play-preview`");

        let mut short_row = frame.clone();
        short_row.row_pitch = 1;
        let error = ensure_cli_runtime_viewport_frame_matches_request(
            &short_row,
            "editor-world",
            "viewportFrame",
        )
        .unwrap_err();
        assert_runtime_host_authority_mismatch(error, "viewportFrame", "smaller than minimum");

        let mut cas_frame = frame;
        cas_frame.color.kind = SideChannelKind::CasBlob;
        let error = ensure_cli_runtime_viewport_frame_matches_request(
            &cas_frame,
            "editor-world",
            "viewportFrame",
        )
        .unwrap_err();
        assert_runtime_host_authority_mismatch(error, "viewportFrame", "live side channels");
    }

    #[test]
    fn cli_runtime_projection_catalog_validation_rejects_duplicates_and_empty_profiles() {
        let catalog = valid_runtime_projection_catalog();
        ensure_cli_runtime_projection_catalog_matches_request(&catalog).unwrap();

        let mut duplicate = catalog.clone();
        duplicate.projections.push(duplicate.projections[0].clone());
        let error = ensure_cli_runtime_projection_catalog_matches_request(&duplicate).unwrap_err();
        assert_runtime_host_authority_mismatch(error, "projectionCatalog", "duplicate projection");

        let mut empty_profile = catalog;
        empty_profile.projections[0].launch_profiles[0].clear();
        let error =
            ensure_cli_runtime_projection_catalog_matches_request(&empty_profile).unwrap_err();
        assert_runtime_host_authority_mismatch(
            error,
            "projectionCatalog",
            "launch profile cannot be empty",
        );
    }

    #[test]
    fn next_workspace_entry_cursor_accepts_advancing_cursor() {
        let page = WorkspaceEntryPageResult {
            entries: Vec::new(),
            next_after_entry_id: Some(8),
        };

        assert_eq!(
            next_workspace_entry_cursor(Some(4), &page).unwrap(),
            Some(8)
        );
        assert_eq!(next_workspace_entry_cursor(None, &page).unwrap(), Some(8));
    }

    #[test]
    fn next_workspace_entry_cursor_rejects_repeated_cursor() {
        let page = WorkspaceEntryPageResult {
            entries: Vec::new(),
            next_after_entry_id: Some(4),
        };

        assert!(matches!(
            next_workspace_entry_cursor(Some(4), &page),
            Err(CliError::InvalidAssetStatusPage { .. })
        ));
    }

    #[test]
    fn workspace_entry_paging_rejects_zero_limit_and_non_positive_cursor() {
        assert_eq!(validate_workspace_entry_paging(Some(5), 64).unwrap(), 64);

        assert!(matches!(
            validate_workspace_entry_paging(None, 0),
            Err(CliError::InvalidAssetStatusPage { .. })
        ));
        assert!(matches!(
            validate_workspace_entry_paging(Some(0), 64),
            Err(CliError::InvalidAssetStatusPage { .. })
        ));
        assert!(matches!(
            validate_workspace_entry_paging(Some(-1), 64),
            Err(CliError::InvalidAssetStatusPage { .. })
        ));
    }

    #[test]
    fn runtime_launch_workspace_snapshot_requires_roots() {
        let manifest = test_session_manifest();
        let snapshot = runtime_workspace_snapshot(&manifest, Vec::new());

        let error = validate_runtime_workspace_snapshot(&manifest, &snapshot).unwrap_err();

        match error {
            CliError::MissingRuntimeAssetSourceRoots {
                session,
                workspace_id,
            } => {
                assert_eq!(session, "editor-work");
                assert_eq!(workspace_id, 42);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn runtime_launch_workspace_snapshot_requires_root_identity() {
        let manifest = test_session_manifest();
        let mut source_root = runtime_project_source_root(&manifest);
        source_root.workspace_root_id = 0;
        let snapshot = runtime_workspace_snapshot(&manifest, vec![source_root]);

        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("invalid source root")
        ));

        let mut source_root = runtime_project_source_root(&manifest);
        source_root.root_id = 0;
        let snapshot = runtime_workspace_snapshot(&manifest, vec![source_root]);

        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("invalid source root")
        ));
    }

    #[test]
    fn runtime_launch_workspace_snapshot_requires_identity() {
        let manifest = test_session_manifest();
        let mut snapshot =
            runtime_workspace_snapshot(&manifest, vec![runtime_project_source_root(&manifest)]);

        snapshot.workspace_id = 0;
        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("positive DB id")
        ));

        let mut snapshot =
            runtime_workspace_snapshot(&manifest, vec![runtime_project_source_root(&manifest)]);
        snapshot.created_unix_ms = -1;
        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("invalid identity metadata")
        ));

        let mut snapshot =
            runtime_workspace_snapshot(&manifest, vec![runtime_project_source_root(&manifest)]);
        snapshot.updated_unix_ms = snapshot.created_unix_ms - 1;
        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("invalid identity metadata")
        ));
    }

    #[test]
    fn runtime_launch_workspace_snapshot_rejects_duplicate_roots() {
        let manifest = test_session_manifest();
        let project_source = runtime_project_source_root(&manifest);
        let mut duplicate_id = runtime_gem_asset_source_root();
        duplicate_id.workspace_root_id = project_source.workspace_root_id;
        let snapshot =
            runtime_workspace_snapshot(&manifest, vec![project_source.clone(), duplicate_id]);

        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("duplicate source roots")
        ));

        let mut duplicate_key = runtime_gem_asset_source_root();
        duplicate_key.portable_key = project_source.portable_key.clone();
        let snapshot = runtime_workspace_snapshot(&manifest, vec![project_source, duplicate_key]);

        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("duplicate source roots")
        ));
    }

    #[test]
    fn runtime_launch_workspace_snapshot_requires_project_root() {
        let manifest = test_session_manifest();
        let snapshot = runtime_workspace_snapshot(&manifest, vec![runtime_gem_asset_source_root()]);

        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("has no `project:local.test_session:assets` root")
        ));
    }

    #[test]
    fn runtime_launch_workspace_snapshot_rejects_project_root_owner_mismatch() {
        let manifest = test_session_manifest();
        let mut source_root = runtime_project_source_root(&manifest);
        source_root.owner_id = "azoth.physics".to_string();
        let snapshot = runtime_workspace_snapshot(&manifest, vec![source_root]);

        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("invalid")
        ));
    }

    #[test]
    fn runtime_launch_workspace_snapshot_rejects_project_root_shape_mismatch() {
        let manifest = test_session_manifest();

        let mut source_root = runtime_project_source_root(&manifest);
        source_root.is_root = false;
        let snapshot = runtime_workspace_snapshot(&manifest, vec![source_root]);
        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("invalid")
        ));

        let mut source_root = runtime_project_source_root(&manifest);
        source_root.output_prefix = "prefabs".to_string();
        let snapshot = runtime_workspace_snapshot(&manifest, vec![source_root]);
        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &snapshot),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("invalid")
        ));
    }

    #[test]
    fn runtime_launch_workspace_snapshot_must_match_attached_identity() {
        let manifest = test_session_manifest();
        let snapshot =
            runtime_workspace_snapshot(&manifest, vec![runtime_project_source_root(&manifest)]);
        validate_runtime_workspace_snapshot(&manifest, &snapshot).unwrap();

        let mut wrong_project = snapshot.clone();
        wrong_project.project_id = "local.other".to_string();
        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &wrong_project),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("project") && reason.contains("local.other")
        ));

        let mut wrong_root = snapshot.clone();
        wrong_root.workspace_root = "projects/example/.azoth/workspaces/stale".to_string();
        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &wrong_root),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("workspace") && reason.contains("stale")
        ));

        let mut invalid_branch = snapshot;
        invalid_branch.branch.clear();
        assert!(matches!(
            validate_runtime_workspace_snapshot(&manifest, &invalid_branch),
            Err(CliError::AssetProcessorAuthorityMismatch { operation: "workspaceSnapshot", reason })
                if reason.contains("invalid identity metadata")
        ));
    }

    #[test]
    fn runtime_asset_source_roots_preserve_workspace_metadata() {
        let manifest = test_session_manifest();
        let snapshot = runtime_workspace_snapshot(
            &manifest,
            vec![
                runtime_project_source_root(&manifest),
                runtime_gem_asset_source_root(),
            ],
        );

        validate_runtime_workspace_snapshot(&manifest, &snapshot).unwrap();
        let roots = runtime_asset_source_roots(&snapshot);
        let root = roots
            .iter()
            .find(|root| root.portable_key == "gem:azoth.physics:assets")
            .expect("asset source root copied from workspace snapshot");

        assert_eq!(roots.len(), 2);
        assert_eq!(root.workspace_root_id, 902);
        assert_eq!(root.workspace_id, 42);
        assert_eq!(root.root_id, 903);
        assert_eq!(root.owner_id, "azoth.physics");
        assert_eq!(
            root.source_root,
            "projects/example/.azoth/workspaces/editor-work/gems/physics/assets"
        );
        assert_eq!(root.display_name, "Physics Assets");
        assert_eq!(root.output_prefix, "gems/azoth.physics");
        assert!(!root.is_root);
    }

    #[test]
    fn runtime_cli_replans_missing_or_failed_runtime_host_service() {
        let mut manifest = test_session_manifest();

        assert!(runtime_host_service_plan_needs_prepare(&manifest));

        let endpoint = Endpoint::in_process("runtime-host:launch");
        let descriptor =
            runtime_host_service_descriptor(manifest.id.0, test_run(2), endpoint.clone());
        manifest
            .services
            .push(ServiceRecord::from_descriptor(&descriptor).unwrap());

        assert!(runtime_host_service_plan_needs_prepare(&manifest));

        manifest.processes.push(runtime_host_process(
            descriptor.run,
            &endpoint,
            ServiceProcessState::Planned,
        ));
        assert!(!runtime_host_service_plan_needs_prepare(&manifest));

        manifest.processes.clear();
        manifest.processes.push(runtime_host_process(
            descriptor.run,
            &endpoint,
            ServiceProcessState::Failed,
        ));
        assert!(runtime_host_service_plan_needs_prepare(&manifest));
    }

    #[test]
    fn runtime_launch_uses_session_manifest_project_identity_without_local_manifest_reload() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/session.rs"
        ))
        .expect("read session command source");
        let launch_start = source
            .find("pub fn launch_runtime(")
            .expect("find launch_runtime");
        let launch_end = source[launch_start..]
            .find("\nfn ensure_runtime_host_service_started(")
            .map(|offset| launch_start + offset)
            .expect("find function after launch_runtime");
        let launch_source = &source[launch_start..launch_end];
        let snapshot_start = source
            .find("async fn request_runtime_launch_snapshot(")
            .expect("find runtime snapshot request");
        let snapshot_end = source[snapshot_start..]
            .find("\nfn ensure_runtime_launch_workspace_snapshot(")
            .map(|offset| snapshot_start + offset)
            .expect("find function after runtime snapshot request");
        let snapshot_source = &source[snapshot_start..snapshot_end];

        assert!(
            !launch_source.contains("load_project_manifest"),
            "runtime launch must not reconstruct project identity by reloading azoth.toml"
        );
        assert!(
            snapshot_source.contains("project_id: manifest.project_id.clone()"),
            "runtime snapshot requests must use the stable project id from the session manifest"
        );
    }

    #[test]
    fn runtime_launch_resolves_project_host_at_snapshot_handoff() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/session.rs"
        ))
        .expect("read session command source");
        let launch_start = source
            .find("pub fn launch_runtime(")
            .expect("find launch_runtime");
        let launch_end = source[launch_start..]
            .find("\nfn ensure_runtime_host_service_started(")
            .map(|offset| launch_start + offset)
            .expect("find function after launch_runtime");
        let launch_source = &source[launch_start..launch_end];

        let workspace_snapshot_guard = launch_source
            .find("ensure_runtime_launch_workspace_snapshot(&manifest, &asset_snapshot)?;")
            .expect("find workspace snapshot guard");
        let runtime_host_ready = launch_source
            .find("let runtime_host =")
            .expect("find runtime-host readiness resolution");
        let project_host_resolve = launch_source
            .find("let project_host = resolve_session_service(")
            .expect("find project-host resolution");
        let snapshot_request = launch_source
            .find("let snapshot = request_runtime_launch_snapshot(")
            .expect("find runtime snapshot request");

        assert!(
            workspace_snapshot_guard < runtime_host_ready,
            "runtime-host descriptor must be resolved after workspace snapshot identity is validated"
        );
        assert!(
            runtime_host_ready < project_host_resolve,
            "project-host must be resolved after runtime-host readiness so snapshot capabilities target the current runtime-host"
        );
        assert!(
            project_host_resolve < snapshot_request,
            "project-host must be resolved immediately before requesting runtimeLaunchSnapshot"
        );
        assert!(
            !launch_source[..workspace_snapshot_guard].contains("project_host_service_id()"),
            "runtime launch must not keep an early project-host descriptor across runtime-host startup or workspace snapshot reads"
        );
    }

    #[test]
    fn cli_resolve_session_service_uses_supervisor_without_scanning_source_control() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/session.rs"
        ))
        .expect("read session command source");
        let helper_start = source
            .find("async fn resolve_session_service(")
            .expect("find resolve_session_service");
        let helper_end = source[helper_start..]
            .find("\nasync fn request_workspace_snapshot(")
            .map(|offset| helper_start + offset)
            .expect("find function after resolve_session_service helpers");
        let helper_source = &source[helper_start..helper_end];

        assert!(
            helper_source.contains("required_manifest_service_descriptor("),
            "CLI service resolution must validate the durable session manifest before resolveService"
        );
        assert!(
            helper_source.contains("ensure_manifest_service_resolution_state"),
            "CLI service resolution must validate availability according to the descriptor's supervision scope"
        );
        assert!(
            helper_source.contains("supervisor.resolve_service_request()"),
            "CLI service resolution must ask the supervisor for its authoritative descriptor"
        );
        assert!(
            !helper_source.contains("request_session_status_from_supervisor("),
            "resolving a service must not trigger a full source-control status scan"
        );
    }

    #[test]
    fn build_asset_processing_has_one_daemon_lifecycle_authority() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/session.rs"
        ))
        .expect("read session command source");
        let process_start = source
            .find("pub fn process_project_assets(")
            .expect("find process_project_assets");
        let process_end = source[process_start..]
            .find("\nfn ensure_asset_processing_services_through_daemon(")
            .map(|offset| process_start + offset)
            .expect("find asset-processing daemon helper");
        let process_source = &source[process_start..process_end];

        assert!(
            process_source.contains("ensure_asset_processing_services_through_daemon("),
            "build asset processing must recover, prepare, and start services through azd"
        );
        assert!(
            !process_source.contains("prepare_selected_services_with_build_policy("),
            "build asset processing must not split daemon preparation from service startup"
        );
        assert!(
            !process_source.contains("start_services("),
            "build asset processing must not launch a competing session supervisor directly"
        );
        assert!(
            process_source.contains("manager.session(&manifest.slug)?"),
            "build asset processing must reload daemon-authored capabilities before RPC use"
        );
    }

    #[test]
    fn build_asset_processing_publishes_successful_products_before_reporting_failures() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/session.rs"
        ))
        .expect("read session command source");
        let process_start = source
            .find("pub fn process_project_assets(")
            .expect("find process_project_assets");
        let process_end = source[process_start..]
            .find("\nfn ensure_asset_processing_services_through_daemon(")
            .map(|offset| process_start + offset)
            .expect("find asset-processing daemon helper");
        let process_source = &source[process_start..process_end];
        // Publication lives in `reconcile_and_publish_asset_catalog`; the caller must run it
        // before it inspects the terminal status, so this window covers both.
        let publish = process_source
            .find("reconcile_and_publish_asset_catalog(")
            .expect("find runtime catalog publication");
        let report_failure = process_source
            .find("CliError::AssetProcessingFailed")
            .expect("find terminal asset failure report");

        assert!(
            publish < report_failure,
            "successful current products must be published through the normal asset-processing path before independent failures are reported"
        );

        let helper_start = process_source
            .find("async fn reconcile_and_publish_asset_catalog(")
            .expect("find runtime catalog publication helper");
        assert!(
            process_source[helper_start..].contains("client.publish_asset_catalog_request()"),
            "the catalog publication helper must publish through the normal asset-processing path"
        );
    }

    #[test]
    fn session_service_start_has_one_daemon_lifecycle_authority() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/session.rs"
        ))
        .expect("read session command source");
        let start = source
            .find("pub fn start_services(")
            .expect("find start_services");
        let end = source[start..]
            .find("\npub fn stop_services(")
            .map(|offset| start + offset)
            .expect("find function after start_services");
        let start_source = &source[start..end];

        assert!(
            start_source.contains("ensure_project_session_services_through_daemon("),
            "session service startup must coordinate project-instance and session scopes through azd"
        );
        assert!(
            !start_source.contains("request_session_service_start("),
            "session service startup must not bypass azd and start only session-owned processes"
        );
        assert!(
            !start_source.contains("spawn_sessiond_process("),
            "session service startup must not launch a competing session supervisor directly"
        );
    }

    #[test]
    fn asset_processing_skip_build_never_prepares_source_host_tools() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/session.rs"
        ))
        .expect("read session command source");
        let process_start = source
            .find("pub fn process_assets(")
            .expect("find process_assets");
        let process_end = source[process_start..]
            .find("\n#[instrument(")
            .map(|offset| process_start + offset)
            .expect("find function after process_assets");
        let process_source = &source[process_start..process_end];

        assert!(
            process_source.contains("if skip_build {")
                && process_source.contains("require_prebuilt_project_host_tools()?")
                && process_source.contains("daemon::start_prebuilt("),
            "--skip-build must resolve installed host tools and start the prebuilt daemon without Cargo"
        );
        assert!(
            process_source.contains("ensure_project_host_tools()?")
                && process_source.contains("daemon::start("),
            "the normal asset-processing path must retain source host-tool preparation"
        );
    }

    fn runtime_host_process(
        run: Uuid,
        endpoint: &Endpoint,
        state: ServiceProcessState,
    ) -> ServiceProcessRecord {
        let temp = tempfile::tempdir().unwrap();
        let mut process = ServiceProcessRecord::planned(
            RUNTIME_HOST_SERVICE_NAME,
            SupervisedServiceRole::RuntimeHost,
            run,
            endpoint,
            "runtime-host",
            temp.path().to_path_buf(),
            Vec::new(),
            temp.path().join("runtime-host.out"),
            temp.path().join("runtime-host.err"),
            temp.path().join("runtime-host.capnp.log"),
            None,
            0,
        );
        match state {
            ServiceProcessState::Planned => {}
            ServiceProcessState::Starting => process.mark_starting(1),
            ServiceProcessState::Running => process
                .mark_running(
                    ProcessIdentity {
                        process_id: 1234,
                        process_start_time: 9_001,
                    },
                    1,
                )
                .unwrap(),
            ServiceProcessState::Exited => process.mark_exited(Some(0), None, 1),
            ServiceProcessState::Failed => {
                process.mark_exited(Some(1), Some("runtime-host failed".to_string()), 1);
            }
        }
        process
    }

    #[test]
    fn project_service_resolution_does_not_require_session_process_records() {
        let manifest = test_session_manifest();
        let descriptor = az_service_catalog::asset_processor_service_descriptor(
            test_run(2),
            Endpoint::in_process("asset-processor:project"),
        );

        ensure_manifest_service_resolution_state(&manifest, &descriptor).unwrap();
    }

    #[test]
    fn runtime_host_resolution_requires_its_session_owned_process_record() {
        let mut manifest = test_session_manifest();
        let endpoint = Endpoint::in_process("runtime-host:session");
        let descriptor =
            runtime_host_service_descriptor(manifest.id.0, test_run(2), endpoint.clone());

        let error = ensure_manifest_service_resolution_state(&manifest, &descriptor).unwrap_err();
        assert!(matches!(
            error,
            CliError::SessionServiceNotRunning(details)
                if details.state == "missing process record"
        ));

        manifest.processes.push(runtime_host_process(
            descriptor.run,
            &endpoint,
            ServiceProcessState::Running,
        ));
        ensure_manifest_service_resolution_state(&manifest, &descriptor).unwrap();
    }

    #[test]
    fn select_local_service_log_resolves_current_and_previous_runs() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("run");
        let mut manifest = SessionManifest::new(
            SessionId::new(),
            "local.cli_manual_service_registration".to_string(),
            "editor-work".to_string(),
            temp.path().to_path_buf(),
            temp.path().join("workspace"),
            run_dir.clone(),
            0,
        );
        let current_run = test_run(2);
        let previous_run = test_run(1);
        let mut process = ServiceProcessRecord::planned(
            "project-host",
            SupervisedServiceRole::ProjectHost,
            current_run,
            &Endpoint::new(EndpointKind::Tcp, "127.0.0.1:2"),
            "project-host",
            temp.path().to_path_buf(),
            Vec::new(),
            run_dir.join("project-host.out"),
            run_dir.join("project-host.err"),
            run_dir.join("project-host.capnp.log"),
            None,
            2,
        );
        process.previous_run = Some(previous_run);
        manifest.processes.push(process);

        let latest =
            select_local_service_log(&manifest, "project-host", None, ServiceLogStreamArg::Stderr)
                .unwrap();
        assert_eq!(latest.run, current_run);
        assert!(latest.path.ends_with("project-host.err"));

        let stdout = select_local_service_log(
            &manifest,
            "project-host",
            Some(previous_run),
            ServiceLogStreamArg::Stdout,
        )
        .unwrap();
        assert_eq!(stdout.run, previous_run);
        assert!(stdout.path.ends_with("project-host.previous.out"));

        let structured = select_local_service_log(
            &manifest,
            "project-host",
            Some(current_run),
            ServiceLogStreamArg::Structured,
        )
        .unwrap();
        assert_eq!(structured.run, current_run);
        assert!(structured.path.ends_with("project-host.capnp.log"));

        assert!(matches!(
            select_local_service_log(
                &manifest,
                "project-host",
                Some(test_run(9)),
                ServiceLogStreamArg::Stderr,
            ),
            Err(CliError::MissingServiceProcess(_))
        ));
    }

    #[test]
    fn select_local_service_log_rejects_paths_outside_session_run_dir() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("run");
        let mut manifest = SessionManifest::new(
            SessionId::new(),
            "local.cli_log_escape".to_string(),
            "editor-work".to_string(),
            temp.path().to_path_buf(),
            temp.path().join("workspace"),
            run_dir.clone(),
            0,
        );
        manifest.processes.push(ServiceProcessRecord::planned(
            "project-host",
            SupervisedServiceRole::ProjectHost,
            test_run(1),
            &Endpoint::new(EndpointKind::Tcp, "127.0.0.1:1"),
            "project-host",
            temp.path().to_path_buf(),
            Vec::new(),
            temp.path().join("outside.out"),
            PathBuf::from("logs/../outside.err"),
            run_dir.join("project-host.capnp.log"),
            None,
            1,
        ));

        let absolute_error =
            select_local_service_log(&manifest, "project-host", None, ServiceLogStreamArg::Stdout)
                .unwrap_err();
        assert!(matches!(
            absolute_error,
            CliError::InvalidServiceLogPath(details)
                if details.session == "editor-work" && details.service == "project-host"
        ));

        let parent_error =
            select_local_service_log(&manifest, "project-host", None, ServiceLogStreamArg::Stderr)
                .unwrap_err();
        assert!(matches!(
            parent_error,
            CliError::InvalidServiceLogPath(details)
                if details.session == "editor-work" && details.service == "project-host"
        ));
    }

    #[test]
    fn select_local_service_log_reports_missing_service() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = SessionManifest::new(
            SessionId::new(),
            "local.test_session".to_string(),
            "editor-work".to_string(),
            temp.path().to_path_buf(),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        );

        assert!(matches!(
            select_local_service_log(&manifest, "asset-processor", None, ServiceLogStreamArg::Stderr),
            Err(CliError::MissingServiceProcess(details))
                if details.session == "editor-work" && details.service == "asset-processor"
        ));
    }

    #[test]
    fn read_log_lines_tails_recorded_log() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service.stderr.log");
        std::fs::write(&path, "one\ntwo\nthree\nfour\n").unwrap();

        assert_eq!(
            read_log_lines_with_offset(&path, Some(2)).unwrap(),
            (
                vec!["three".to_string(), "four".to_string()],
                "one\ntwo\nthree\nfour\n".len() as u64
            )
        );
        assert_eq!(
            read_log_lines_with_offset(&path, None).unwrap(),
            (
                vec![
                    "one".to_string(),
                    "two".to_string(),
                    "three".to_string(),
                    "four".to_string()
                ],
                "one\ntwo\nthree\nfour\n".len() as u64
            )
        );
        assert_eq!(
            read_log_lines_with_offset(&path, Some(0)).unwrap(),
            (Vec::<String>::new(), "one\ntwo\nthree\nfour\n".len() as u64)
        );
    }

    #[test]
    fn log_drain_advances_across_append_and_resets_after_truncate() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service.stderr.log");
        std::fs::write(&path, "one\n").unwrap();
        let mut offset = "one\n".len() as u64;

        std::fs::write(&path, "one\ntwo\n").unwrap();
        drain_log_file_from(&path, &mut offset).unwrap();
        assert_eq!(offset, "one\ntwo\n".len() as u64);

        std::fs::write(&path, "new\n").unwrap();
        drain_log_file_from(&path, &mut offset).unwrap();
        assert_eq!(offset, "new\n".len() as u64);
    }

    #[test]
    fn trim_log_line_end_strips_platform_line_endings() {
        assert_eq!(trim_log_line_end("one\n"), "one");
        assert_eq!(trim_log_line_end("two\r\n"), "two");
        assert_eq!(trim_log_line_end("three\r"), "three");
        assert_eq!(trim_log_line_end("four"), "four");
        assert_eq!(trim_log_line_end("\n"), "");
    }

    #[test]
    fn daemon_endpoint_fallback_is_disabled_for_explicit_endpoint() {
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:41234");
        let explicit = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint: endpoint.clone(),
            source: crate::commands::daemon::DaemonEndpointSource::Explicit,
        };
        let runtime_record = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint,
            source: crate::commands::daemon::DaemonEndpointSource::RuntimeRecord,
        };

        assert!(!can_fallback_to_local_session(Some(&explicit)));
        assert!(can_fallback_to_local_session(Some(&runtime_record)));
        assert!(can_fallback_to_local_session(None));

        let missing_grant =
            CliError::MissingServiceCapability(Box::new(MissingServiceCapabilityDetails {
                session: "editor-work".to_string(),
                service: "azoth/session-supervisor".to_string(),
                audience: SESSION_SUPERVISOR_AUDIENCE.to_string(),
                permissions: SESSION_READ_PERMISSION.to_string(),
            }));
        assert!(!can_fallback_after_supervisor_error(None, &missing_grant));
        assert!(!can_fallback_after_supervisor_error(
            Some(&runtime_record),
            &missing_grant
        ));
        let invalid_descriptor = CliError::InvalidServiceDescriptor {
            operation: "session capability",
            service: "azoth/session-supervisor role SessionSupervisor".to_string(),
            reason: "capability template lifetime is invalid".to_string(),
        };
        assert!(!can_fallback_after_supervisor_error(
            None,
            &invalid_descriptor
        ));
        assert!(!can_fallback_after_supervisor_error(
            Some(&runtime_record),
            &invalid_descriptor
        ));

        let transient = CliError::MissingSessionService {
            session: "editor-work".to_string(),
            service: "session-supervisor".to_string(),
        };
        assert!(can_fallback_after_supervisor_error(None, &transient));
        assert!(can_fallback_after_supervisor_error(
            Some(&runtime_record),
            &transient
        ));
        assert!(!can_fallback_after_supervisor_error(
            Some(&explicit),
            &transient
        ));
    }

    #[test]
    fn session_exec_stays_on_supervisor_boundary_without_direct_workspace_fallback() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/session.rs"
        ))
        .expect("read session command source");
        let exec_start = source.find("pub fn exec(").expect("find session exec");
        let status_start = source[exec_start..]
            .find("\npub fn status(")
            .map(|offset| exec_start + offset)
            .expect("find session status after exec");
        let exec_source = &source[exec_start..status_start];

        assert!(
            exec_source.contains("request_session_exec_command"),
            "session exec must call SessionSupervisor.execCommand"
        );
        for forbidden in [
            "run_external_command",
            "validate_session_workspace",
            "can_fallback_to_local_session",
            "can_fallback_after_supervisor_error",
            "falling back to direct workspace command",
        ] {
            assert!(
                !exec_source.contains(forbidden),
                "session exec must not bypass session-supervisor with `{forbidden}`"
            );
        }
    }

    #[test]
    fn service_planning_can_route_through_azd_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest =
            az_project::ProjectManifest::new("local.cli_services_rpc", "Services RPC", "0.1.0");
        manifest
            .tools
            .service_targets
            .push(az_project::ProjectServiceTarget::cargo_bin(
                "project-host",
                az_project::ProjectServiceRole::ProjectHost,
                "game",
                "project-host",
            ));
        manifest
            .tools
            .service_targets
            .push(az_project::ProjectServiceTarget::cargo_bin(
                "asset-processor",
                az_project::ProjectServiceRole::AssetProcessor,
                "game",
                "asset-processor",
            ));
        az_project::write_project_manifest(temp.path(), &manifest).unwrap();
        az_project::refresh_project_lock(temp.path()).unwrap();
        let daemon = az_daemon::AzDaemon::with_data_home(az_filesystem::AzothDataHome::new(
            temp.path().join("azoth-home"),
        ))
        .unwrap();
        let server = az_daemon::start_az_daemon_rpc_server_with_daemon(
            daemon,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();
        let daemon_endpoint = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint: server.endpoint().clone(),
            source: crate::commands::daemon::DaemonEndpointSource::Explicit,
        };

        let plan = plan_project_services_for_session(
            temp.path(),
            temp.path(),
            "editor-work".to_string(),
            EndpointKind::Tcp,
            Some(&daemon_endpoint),
            vec!["asset-processor".to_string()],
        )
        .unwrap();

        assert_eq!(plan.build_commands.len(), 1);
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].service_name, "asset-processor");
        assert_eq!(plan.commands[0].role, ServiceRole::AssetProcessor);
        assert_eq!(plan.commands[0].endpoint.kind, EndpointKind::Tcp);
        assert!(
            !plan.commands[0].args.iter().any(|arg| arg == "--session"),
            "project-scoped services must not inherit session ownership arguments"
        );

        server.stop();
    }

    #[test]
    fn service_planning_requires_azd_endpoint() {
        let temp = tempfile::tempdir().unwrap();

        let error = plan_project_services_for_session(
            temp.path(),
            temp.path(),
            "editor-work".to_string(),
            EndpointKind::Tcp,
            None,
            Vec::new(),
        )
        .unwrap_err();

        match error {
            CliError::MissingDaemonEndpoint { operation } => {
                assert_eq!(operation, "session service planning");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn session_supervisor_descriptor_can_resolve_through_azd_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let project_manifest =
            az_project::ProjectManifest::new("local.cli_runtime_rpc", "Runtime RPC", "0.1.0");
        az_project::write_project_manifest(&project_root, &project_manifest).unwrap();
        az_project::refresh_project_lock(&project_root).unwrap();
        let mut session_manifest = SessionManifest::new(
            SessionId::new(),
            "local.cli_runtime_rpc".to_string(),
            "editor-work".to_string(),
            project_root.clone(),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        );
        let local_descriptor = az_service_catalog::session_supervisor_service_descriptor(
            session_manifest.id.0,
            test_run(1),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:1"),
        );
        session_manifest.upsert_service_descriptor(&local_descriptor, 1);

        let daemon = az_daemon::AzDaemon::with_data_home(az_filesystem::AzothDataHome::new(
            temp.path().join("azoth-home"),
        ))
        .unwrap();
        let project = daemon.register_project_root(&project_root).unwrap();
        let daemon_descriptor = az_service_catalog::session_supervisor_service_descriptor(
            session_manifest.id.0,
            test_run(2),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:2"),
        );
        daemon
            .register_session_supervisor(&project.project_id, "editor-work", &daemon_descriptor)
            .unwrap();
        let server = az_daemon::start_az_daemon_rpc_server_with_daemon(
            daemon,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();
        let daemon_endpoint = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint: server.endpoint().clone(),
            source: crate::commands::daemon::DaemonEndpointSource::Explicit,
        };

        let resolved =
            session_supervisor_descriptor_for_command(&session_manifest, Some(&daemon_endpoint))
                .unwrap();

        assert_eq!(resolved.endpoint, daemon_descriptor.endpoint);
        assert_eq!(resolved.run, test_run(2));
        server.stop();
    }

    #[test]
    fn daemon_session_discovery_rejects_mismatched_session_project_identity() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let other_root = temp.path().join("other");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&other_root).unwrap();
        let project = ProjectRecord {
            project_id: "local.cli_discovery".to_string(),
            name: "Discovery".to_string(),
            root: project_root.to_string_lossy().into_owned(),
            manifest_path: project_root
                .join("azoth.toml")
                .to_string_lossy()
                .into_owned(),
            engine_version: "0.1.0".to_string(),
        };
        let mut manifest = az_session::session_manifest_to_proto(&SessionManifest::new(
            SessionId::new(),
            project.project_id.clone(),
            "editor-work".to_string(),
            project_root.clone(),
            project_root
                .join(".azoth")
                .join("workspaces")
                .join("editor-work"),
            project_root
                .join(".azoth")
                .join("sessions")
                .join(Uuid::new_v4().to_string()),
            0,
        ));

        validate_proto_sessions_match_project(&project, vec![manifest.clone()], "session list")
            .unwrap();

        manifest.project_id = "local.other".to_string();
        let mismatch =
            validate_proto_sessions_match_project(&project, vec![manifest.clone()], "session list")
                .unwrap_err();
        assert!(matches!(
            mismatch,
            CliError::SessionDiscoveryMismatch {
                operation: "session list",
                session,
                reason
            } if session == "editor-work"
                && reason.contains("local.other")
                && reason.contains("local.cli_discovery")
        ));

        manifest.project_id = project.project_id.clone();
        manifest.project_root = other_root.to_string_lossy().into_owned();
        let mismatch =
            validate_proto_sessions_match_project(&project, vec![manifest], "session list")
                .unwrap_err();
        assert!(matches!(
            mismatch,
            CliError::SessionDiscoveryMismatch {
                operation: "session list",
                session,
                reason
            } if session == "editor-work"
                && reason.contains("project root")
                && reason.contains("daemon project root")
        ));
    }

    #[test]
    fn session_supervisor_responses_must_match_requested_manifest_identity() {
        let expected = test_session_manifest();
        let mut response = az_session::session_manifest_to_proto(&expected);

        ensure_proto_session_response_matches_manifest(
            &response,
            &expected,
            "session-supervisor status",
        )
        .unwrap();

        response.id = SessionId::new().0;
        let mismatch = ensure_proto_session_response_matches_manifest(
            &response,
            &expected,
            "session-supervisor status",
        )
        .unwrap_err();
        assert!(matches!(
            mismatch,
            CliError::SessionDiscoveryMismatch {
                operation: "session-supervisor status",
                session,
                reason
            } if session == "editor-work" && reason.contains("session id")
        ));

        response = az_session::session_manifest_to_proto(&expected);
        response.workspace_root = "projects/other-workspace".to_string();
        let mismatch = ensure_proto_session_response_matches_manifest(
            &response,
            &expected,
            "session-supervisor status",
        )
        .unwrap_err();
        assert!(matches!(
            mismatch,
            CliError::SessionDiscoveryMismatch {
                operation: "session-supervisor status",
                session,
                reason
            } if session == "editor-work" && reason.contains("workspace")
        ));
    }

    #[test]
    fn session_service_registration_response_must_include_requested_descriptor() {
        let expected = test_session_manifest();
        let descriptor = runtime_host_service_descriptor(
            expected.id.0,
            test_run(3),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37641"),
        );
        let mut response = az_session::session_manifest_to_proto(&expected);

        response.services.push(descriptor.clone());
        ensure_registered_service_response_contains_descriptor(&response, &descriptor).unwrap();

        response.services.clear();
        let missing =
            ensure_registered_service_response_contains_descriptor(&response, &descriptor)
                .unwrap_err();
        assert!(matches!(
            missing,
            CliError::SessionDiscoveryMismatch {
                operation: "session service registration",
                session,
                reason
            } if session == "editor-work" && reason.contains("did not include")
        ));

        let mut rewritten = descriptor.clone();
        rewritten.run = test_run(4);
        response.services.push(rewritten.clone());
        ensure_registered_service_response_contains_descriptor(&response, &descriptor).unwrap();

        rewritten.endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37642");
        response.services.clear();
        response.services.push(rewritten);
        let mismatch =
            ensure_registered_service_response_contains_descriptor(&response, &descriptor)
                .unwrap_err();
        assert!(matches!(
            mismatch,
            CliError::SessionDiscoveryMismatch {
                operation: "session service registration",
                session,
                reason
            } if session == "editor-work"
                && reason.contains("does not match requested")
                && reason.contains("37642")
                && reason.contains("37641")
        ));
    }

    #[test]
    fn daemon_active_session_selection_rejects_registered_supervisor_for_other_project() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let supervisor_root = temp.path().join("supervisor");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&supervisor_root).unwrap();
        write_test_project_manifest(&project_root, "local.cli_discovery");
        write_test_project_manifest(&supervisor_root, "local.other_project");

        let session_id = SessionId::new();
        let run_dir = SessionManager::with_data_home(
            &supervisor_root,
            az_filesystem::AzothDataHome::new(temp.path().join("azoth-home")),
        )
        .unwrap()
        .sessions_dir()
        .join(session_id.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        let mut session_manifest = SessionManifest::new(
            session_id,
            "local.other_project".to_string(),
            "editor-work".to_string(),
            supervisor_root.clone(),
            supervisor_root.clone(),
            run_dir,
            0,
        );
        session_manifest.activate(0);
        std::fs::write(
            session_manifest.manifest_path(),
            toml::to_string(&session_manifest).unwrap(),
        )
        .unwrap();

        let mut supervisor_descriptor = session_supervisor_service_descriptor(
            session_manifest.id.0,
            test_run(1),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        let supervisor_server = az_session::start_session_supervisor_rpc_server_with_manager(
            SessionManager::with_data_home(
                &supervisor_root,
                az_filesystem::AzothDataHome::new(temp.path().join("azoth-home")),
            )
            .unwrap(),
            supervisor_descriptor.endpoint.clone(),
            &session_manifest.slug,
        )
        .unwrap();
        supervisor_descriptor.endpoint = supervisor_server.endpoint().clone();
        session_manifest
            .upsert_service_descriptor(&supervisor_descriptor, 1)
            .unwrap();
        std::fs::write(
            session_manifest.manifest_path(),
            toml::to_string(&session_manifest).unwrap(),
        )
        .unwrap();

        let daemon = az_daemon::AzDaemon::with_data_home(az_filesystem::AzothDataHome::new(
            temp.path().join("azoth-home"),
        ))
        .unwrap();
        let project = daemon.register_project_root(&project_root).unwrap();
        daemon
            .register_session_supervisor(
                &project.project_id,
                &session_manifest.slug,
                &supervisor_descriptor,
            )
            .unwrap();
        let daemon_server = az_daemon::start_az_daemon_rpc_server_with_daemon(
            daemon,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();
        let daemon_endpoint = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint: daemon_server.endpoint().clone(),
            source: crate::commands::daemon::DaemonEndpointSource::Explicit,
        };

        let error =
            active_session_slug_through_daemon(&project_root, &daemon_endpoint, "runtime launch")
                .unwrap_err();

        match error {
            CliError::SessionDiscoveryMismatch {
                operation: "runtime launch",
                session,
                reason,
            } => {
                assert_eq!(session, "editor-work");
                assert!(reason.contains("local.other_project"), "{reason}");
                assert!(reason.contains("local.cli_discovery"), "{reason}");
            }
            other => panic!("unexpected error: {other}"),
        }

        supervisor_server.stop();
        daemon_server.stop();
    }

    #[test]
    fn formats_runtime_viewport_side_channel_metadata() {
        assert_eq!(
            format_viewport_pixel_format(ViewportPixelFormat::Bgra8Unorm),
            "bgra8Unorm"
        );
        assert_eq!(
            format_side_channel_kind(SideChannelKind::MmapFile),
            "mmapFile"
        );
        assert_eq!(format_optional_hex_bytes(&[]), "none");
        assert_eq!(format_optional_hex_bytes(&[0xab, 0xcd]), "abcd");

        let capability = Capability::new(
            ServiceId::new(RUNTIME_HOST_NAMESPACE, RUNTIME_HOST_SERVICE_NAME),
            ServiceRole::RuntimeHost,
        )
        .with_session(SessionId::new().0)
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_READ_PERMISSION])
        .with_expires_unix_ms(42)
        .with_token_hash([0x41, 0x42]);

        let summary = format_side_channel_capability(&capability);

        assert!(summary.contains("azoth.runtime-host"));
        assert!(summary.contains("runtime-host"));
        assert!(summary.contains("runtime.read"));
        assert!(summary.contains("expires_unix_ms=42"));
        assert!(summary.contains("4142"));
    }

    #[test]
    fn cli_capability_helpers_use_passed_descriptor_templates() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = SessionManifest::new(
            SessionId::new(),
            "local.test_session".to_string(),
            "editor-work".to_string(),
            temp.path().join("project"),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        );
        let project_capability = Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_audience(PROJECT_HOST_AUDIENCE)
        .with_permissions([PROJECT_DOCUMENT_READ_PERMISSION])
        .with_token_hash([0x41]);
        let asset_capability = Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_audience(ASSET_PROCESSOR_AUDIENCE)
        .with_permissions([ASSET_READ_PERMISSION])
        .with_token_hash([0x42]);
        let runtime_capability = Capability::new(
            ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
            ServiceRole::Editor,
        )
        .with_session(manifest.id.0)
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_CONTROL_PERMISSION])
        .with_token_hash([0x43]);
        let runtime_project_host_capability = Capability::new(
            ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
        )
        .with_session(manifest.id.0)
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_CONTROL_PERMISSION])
        .with_token_hash([0x45]);
        let project_descriptor = ServiceDescriptor::new(
            project_host_service_id(),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_capability(project_capability);
        let asset_descriptor = ServiceDescriptor::new(
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
            Endpoint::in_process("asset-processor:test"),
        )
        .with_capability(asset_capability);
        let runtime_descriptor = ServiceDescriptor::new(
            runtime_host_service_id(),
            ServiceRole::RuntimeHost,
            Endpoint::in_process("runtime-host:test"),
        )
        .with_capability(runtime_capability)
        .with_capability(runtime_project_host_capability);
        let project = project_document_read_capability(&manifest, &project_descriptor).unwrap();
        assert_eq!(project.token_hash, vec![0x41]);
        assert_eq!(project.session, None);

        let asset = asset_read_capability(&manifest, &asset_descriptor).unwrap();
        assert_eq!(asset.token_hash, vec![0x42]);
        assert_eq!(asset.session, None);

        let runtime = runtime_control_capability(&manifest, &runtime_descriptor).unwrap();
        assert_eq!(runtime.token_hash, vec![0x43]);
        assert_eq!(runtime.session, Some(manifest.id.0));

        let runtime_project =
            runtime_project_host_control_capability(&manifest, &runtime_descriptor).unwrap();
        assert_eq!(runtime_project.token_hash, vec![0x45]);
        assert_eq!(runtime_project.session, Some(manifest.id.0));
    }

    #[test]
    fn cli_capability_helpers_reject_missing_descriptor_grants() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = SessionManifest::new(
            SessionId::new(),
            "local.test_session".to_string(),
            "editor-work".to_string(),
            temp.path().join("project"),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        );
        let descriptor = ServiceDescriptor::new(
            project_host_service_id(),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_permissions([PROJECT_DOCUMENT_WRITE_PERMISSION])
            .with_token_hash([0x41]),
        );

        let error = project_document_read_capability(&manifest, &descriptor).unwrap_err();

        assert!(matches!(
            error,
            CliError::MissingServiceCapability(details)
                if details.service == "azoth/project-host"
                    && details.audience == PROJECT_HOST_AUDIENCE
                    && details.permissions == PROJECT_DOCUMENT_READ_PERMISSION
        ));
    }

    #[test]
    fn cli_capability_helpers_reject_unbrokered_descriptor_grants() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = SessionManifest::new(
            SessionId::new(),
            "local.test_session".to_string(),
            "editor-work".to_string(),
            temp.path().join("project"),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        );
        let descriptor = ServiceDescriptor::new(
            project_host_service_id(),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_permissions([PROJECT_DOCUMENT_READ_PERMISSION]),
        );

        let error = project_document_read_capability(&manifest, &descriptor).unwrap_err();

        assert!(matches!(
            error,
            CliError::InvalidServiceDescriptor {
                operation: "project-host capability",
                reason,
                ..
            } if reason.contains("brokered token hash")
        ));
    }

    #[test]
    fn cli_capability_helpers_reject_expired_descriptor_grants() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = SessionManifest::new(
            SessionId::new(),
            "local.test_session".to_string(),
            "editor-work".to_string(),
            temp.path().join("project"),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        );
        let descriptor = ServiceDescriptor::new(
            project_host_service_id(),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:test"),
        )
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_permissions([PROJECT_DOCUMENT_READ_PERMISSION])
            .with_expires_unix_ms(1)
            .with_token_hash([0x41]),
        );

        let error = project_document_read_capability(&manifest, &descriptor).unwrap_err();

        assert!(matches!(
            error,
            CliError::InvalidServiceDescriptor {
                operation: "project-host capability",
                reason,
                ..
            } if reason.contains("lifetime is invalid")
        ));
    }

    #[test]
    fn cli_capability_helpers_reject_wrong_descriptor_identity() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = SessionManifest::new(
            SessionId::new(),
            "local.test_session".to_string(),
            "editor-work".to_string(),
            temp.path().join("project"),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        );
        let wrong_descriptor = ServiceDescriptor::new(
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
            Endpoint::in_process("asset-processor:test"),
        )
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_audience(PROJECT_HOST_AUDIENCE)
            .with_permissions([PROJECT_DOCUMENT_READ_PERMISSION])
            .with_token_hash([0x41]),
        );

        let error = project_document_read_capability(&manifest, &wrong_descriptor).unwrap_err();

        assert!(matches!(
            error,
            CliError::UnexpectedServiceDescriptor {
                operation: "project-host capability",
                expected,
                actual,
            } if expected.contains("azoth/project-host")
                && expected.contains("ProjectHost")
                && actual.contains("azoth/asset-processor")
                && actual.contains("AssetProcessor")
        ));
    }

    #[test]
    fn resolved_session_service_descriptor_must_match_requested_identity() {
        let wrong_descriptor = ServiceDescriptor::new(
            asset_processor_service_id(),
            ServiceRole::AssetProcessor,
            Endpoint::in_process("asset-processor:test"),
        );

        let error = validate_service_descriptor(
            &wrong_descriptor,
            &project_host_service_id(),
            ServiceRole::ProjectHost,
            "resolveService",
        )
        .unwrap_err();

        assert!(matches!(
            error,
            CliError::UnexpectedServiceDescriptor {
                operation: "resolveService",
                expected,
                actual,
            } if expected.contains("azoth/project-host")
                && expected.contains("ProjectHost")
                && actual.contains("azoth/asset-processor")
                && actual.contains("AssetProcessor")
        ));
    }

    #[test]
    fn unscoped_session_list_capability_comes_from_descriptor() {
        let session_id = SessionId::new();
        let descriptor = session_supervisor_service_descriptor(
            session_id.0,
            test_run(1),
            Endpoint::in_process("session-supervisor:test"),
        );

        let capability = unscoped_session_read_capability(&descriptor).unwrap();

        assert_eq!(capability.session, Some(session_id.0));
        assert_eq!(capability.audience, SESSION_SUPERVISOR_AUDIENCE);
        assert!(capability.has_permissions(&[SESSION_READ_PERMISSION]));
        assert!(!capability.token_hash.is_empty());
    }

    #[test]
    fn unscoped_session_list_rejects_descriptor_without_read_grant() {
        let descriptor = ServiceDescriptor::new(
            session_supervisor_service_id(),
            ServiceRole::SessionSupervisor,
            Endpoint::in_process("session-supervisor:test"),
        )
        .with_capability(
            Capability::new(
                ServiceId::new(EDITOR_SERVICE_NAMESPACE, EDITOR_SERVICE_NAME),
                ServiceRole::Editor,
            )
            .with_session(SessionId::new().0)
            .with_audience(SESSION_SUPERVISOR_AUDIENCE)
            .with_permissions([SESSION_EXEC_PERMISSION])
            .with_token_hash([0x44]),
        );

        let error = unscoped_session_read_capability(&descriptor).unwrap_err();

        assert!(matches!(
            error,
            CliError::MissingServiceCapability(details)
                if details.service == "azoth/session-supervisor"
                    && details.audience == SESSION_SUPERVISOR_AUDIENCE
                    && details.permissions == SESSION_READ_PERMISSION
        ));
    }

    #[test]
    fn sessiond_args_include_session_and_explicit_endpoints() {
        let project_path = Path::new("projects/example");
        let daemon_endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612");

        let args = sessiond_args(
            project_path,
            &SessiondLaunch {
                session: "editor-work",
                session_supervisor_kind: Some(EndpointKind::Tcp),
                session_supervisor_endpoint: Some("127.0.0.1:37613"),
                daemon_endpoint: Some(&daemon_endpoint),
                otlp_endpoint: None,
                keep_alive: false,
                start_service_names: &[],
            },
        )
        .unwrap();

        assert_eq!(
            args,
            vec![
                "--project",
                "projects/example",
                "--session",
                "editor-work",
                "--endpoint-kind",
                "tcp",
                "--endpoint",
                "127.0.0.1:37613",
                "--daemon-endpoint-kind",
                "tcp",
                "--daemon-endpoint",
                "127.0.0.1:37612",
            ]
        );
    }

    #[test]
    fn sessiond_args_reject_in_process_endpoint_kinds() {
        let error = sessiond_args(
            Path::new("projects/example"),
            &SessiondLaunch {
                session: "editor-work",
                session_supervisor_kind: Some(EndpointKind::InProcess),
                session_supervisor_endpoint: Some("sessiond:test"),
                daemon_endpoint: None,
                otlp_endpoint: None,
                keep_alive: false,
                start_service_names: &[],
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            CliError::UnsupportedEndpointKind {
                operation: "session-supervisor launch",
                kind: EndpointKind::InProcess
            }
        ));

        let daemon_endpoint = Endpoint::in_process("azd:test");
        let error = sessiond_args(
            Path::new("projects/example"),
            &SessiondLaunch {
                session: "editor-work",
                session_supervisor_kind: None,
                session_supervisor_endpoint: None,
                daemon_endpoint: Some(&daemon_endpoint),
                otlp_endpoint: None,
                keep_alive: false,
                start_service_names: &[],
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            CliError::UnsupportedEndpointKind {
                operation: "session-supervisor daemon endpoint",
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn session_service_endpoint_validation_rejects_in_process() {
        let error =
            validate_public_endpoint_kind(EndpointKind::InProcess, "session service planning")
                .unwrap_err();

        assert!(matches!(
            error,
            CliError::UnsupportedEndpointKind {
                operation: "session service planning",
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn sessiond_launcher_forwards_only_explicit_daemon_endpoints() {
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612");
        let explicit = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint: endpoint.clone(),
            source: crate::commands::daemon::DaemonEndpointSource::Explicit,
        };
        let runtime_record = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint,
            source: crate::commands::daemon::DaemonEndpointSource::RuntimeRecord,
        };

        assert!(forwarded_sessiond_daemon_endpoint(Some(&explicit)).is_some());
        assert!(forwarded_sessiond_daemon_endpoint(Some(&runtime_record)).is_none());
    }

    #[test]
    fn sessiond_launch_command_passes_absolute_project_root_to_child_process() {
        let sessiond_executable = Path::new("bin/az-sessiond.exe");
        let command = sessiond_launch_command(
            sessiond_executable,
            Path::new("."),
            &SessiondLaunch {
                session: "editor-work",
                session_supervisor_kind: None,
                session_supervisor_endpoint: None,
                daemon_endpoint: None,
                otlp_endpoint: None,
                keep_alive: false,
                start_service_names: &[],
            },
        )
        .expect("build sessiond launch command");
        let project_root_index = command
            .args
            .iter()
            .position(|arg| arg == "--project")
            .expect("--project arg exists");
        assert!(Path::new(&command.args[project_root_index + 1]).is_absolute());
        assert_eq!(command.program, sessiond_executable.to_string_lossy());
        assert!(!command.args.iter().any(|arg| arg == "run"));
    }

    #[test]
    fn sessiond_args_keep_alive_for_background_start() {
        let args = sessiond_args(
            Path::new("projects/example"),
            &SessiondLaunch {
                session: "editor-work",
                session_supervisor_kind: None,
                session_supervisor_endpoint: None,
                daemon_endpoint: None,
                otlp_endpoint: None,
                keep_alive: true,
                start_service_names: &[],
            },
        )
        .unwrap();

        assert!(args.iter().any(|arg| arg == "--keep-alive"));
    }

    #[test]
    fn sessiond_args_can_scope_startup_services() {
        let args = sessiond_args(
            Path::new("projects/example"),
            &SessiondLaunch {
                session: "editor-work",
                session_supervisor_kind: None,
                session_supervisor_endpoint: None,
                daemon_endpoint: None,
                otlp_endpoint: None,
                keep_alive: true,
                start_service_names: &["project-host".to_string(), "asset-processor".to_string()],
            },
        )
        .unwrap();

        assert!(
            args.windows(2)
                .any(|pair| pair == ["--start-service", "project-host"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--start-service", "asset-processor"])
        );
    }

    #[test]
    fn sessiond_args_can_forward_otlp_endpoint() {
        let args = sessiond_args(
            Path::new("projects/example"),
            &SessiondLaunch {
                session: "editor-work",
                session_supervisor_kind: None,
                session_supervisor_endpoint: None,
                daemon_endpoint: None,
                otlp_endpoint: Some("http://127.0.0.1:4317"),
                keep_alive: false,
                start_service_names: &[],
            },
        )
        .unwrap();

        assert!(
            args.windows(2)
                .any(|pair| { pair == ["--otlp-endpoint", "http://127.0.0.1:4317"] })
        );
    }

    #[test]
    fn failure_summary_line_reads_first_non_empty_failure_line() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(
            run_dir.join("failure.txt"),
            "\n  project-host exited during bootstrap\nretry preserved\n",
        )
        .unwrap();
        let manifest = SessionManifest::new(
            SessionId::new(),
            "local.test_session".to_string(),
            "editor-work".to_string(),
            temp.path().join("project"),
            temp.path().join("workspace"),
            run_dir,
            0,
        );

        assert_eq!(
            failure_summary_line(&manifest).as_deref(),
            Some("project-host exited during bootstrap")
        );
    }

    #[test]
    fn prepare_recovery_accepts_failed_preserved_sessions_only_when_requested() {
        let temp = tempfile::tempdir().unwrap();
        let mut manifest = SessionManifest::new(
            SessionId::new(),
            "local.test_session".to_string(),
            "editor-work".to_string(),
            temp.path().join("project"),
            temp.path().join("workspace"),
            temp.path().join("run"),
            0,
        );
        manifest.preserve_failure(1);

        assert!(ensure_active_or_recovery(&manifest, true).is_ok());
        assert!(ensure_active_or_recovery(&manifest, false).is_err());
    }

    #[test]
    fn shell_command_quotes_paths_with_spaces() {
        let manifest_path = ["C:", "Projects", "Sample Game", "Cargo.toml"].join("\\");
        let args = vec![
            "run".to_string(),
            "--manifest-path".to_string(),
            manifest_path.clone(),
        ];

        assert_eq!(
            shell_command("cargo", &args),
            format!("cargo run --manifest-path \"{manifest_path}\"")
        );
    }

    #[test]
    fn shell_command_quotes_empty_arguments() {
        let args = vec![
            "pr".to_string(),
            "create".to_string(),
            "--body".to_string(),
            String::new(),
        ];

        assert_eq!(shell_command("gh", &args), "gh pr create --body \"\"");
    }

    #[test]
    fn split_exec_command_requires_program() {
        assert!(matches!(
            split_exec_command(Vec::new()),
            Err(CliError::MissingSessionExecCommand)
        ));
    }

    #[test]
    fn split_exec_command_preserves_args() {
        let (program, args) = split_exec_command(vec![
            "cargo".to_string(),
            "test".to_string(),
            "-p".to_string(),
            "azoth".to_string(),
        ])
        .unwrap();

        assert_eq!(program, "cargo");
        assert_eq!(
            args,
            vec!["test".to_string(), "-p".to_string(), "azoth".to_string()]
        );
    }
    fn sessiond_image_name() -> String {
        format!("az-sessiond{}", std::env::consts::EXE_SUFFIX)
    }

    fn argv(arguments: &[&str]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| (*argument).to_string())
            .collect()
    }

    fn sessiond_argv(project: &str, session: &str) -> Vec<String> {
        argv(&["az-sessiond", "--project", project, "--session", session])
    }

    fn swept_process(pid: u32, executable: &Path, arguments: Vec<String>) -> SweptProcess {
        SweptProcess {
            pid,
            executable: executable.to_path_buf(),
            argv: arguments,
        }
    }

    #[test]
    fn sweep_target_reads_project_and_session_from_command_line() {
        let argv = sessiond_argv("projects/sample", "main");

        assert_eq!(
            sessiond_target(&argv),
            Some((PathBuf::from("projects/sample"), "main".to_string()))
        );
    }

    #[test]
    fn sweep_target_requires_both_project_and_session() {
        for arguments in [
            argv(&["az-sessiond", "--project", "projects/sample"]),
            argv(&["az-sessiond", "--session", "main"]),
            argv(&["az-sessiond", "--project", "projects/p", "--session"]),
            argv(&["az-sessiond", "--project", "  ", "--session", "main"]),
        ] {
            assert_eq!(
                sessiond_target(&arguments),
                None,
                "argv {arguments:?} names no stoppable target"
            );
        }
    }

    #[test]
    fn sweep_target_keeps_values_containing_spaces() {
        // The OS hands back an already-split argv, so a quoted project path
        // arrives as one entry and must survive whole.
        let argv = sessiond_argv("builds/sample world", "feature main");

        assert_eq!(
            sessiond_target(&argv),
            Some((
                PathBuf::from("builds/sample world"),
                "feature main".to_string()
            ))
        );
    }

    #[test]
    fn sweep_selects_only_supervisors_running_the_workspace_image() {
        let workspace = PathBuf::from("builds/azoth/target/debug").join(sessiond_image_name());
        let elsewhere = PathBuf::from("builds/elsewhere/target/debug").join(sessiond_image_name());
        let editor = PathBuf::from("builds/azoth/target/debug/az-editor");
        let processes = [
            swept_process(11, &workspace, sessiond_argv("projects/sample", "main")),
            swept_process(12, &elsewhere, sessiond_argv("projects/other", "main")),
            swept_process(13, &editor, argv(&["az-editor"])),
        ];

        let supervisors =
            workspace_supervisors(&processes, &az_filesystem::normalize(&workspace)).unwrap();

        assert_eq!(
            supervisors,
            vec![SweptSupervisor {
                pid: 11,
                project: PathBuf::from("projects/sample"),
                session: "main".to_string(),
            }]
        );
    }

    #[test]
    fn sweep_dedupes_supervisors_sharing_a_project_and_session() {
        let workspace = PathBuf::from("builds/azoth/target/debug").join(sessiond_image_name());
        let processes = [
            swept_process(21, &workspace, sessiond_argv("projects/sample", "main")),
            swept_process(22, &workspace, sessiond_argv("projects/sample", "main")),
            swept_process(23, &workspace, sessiond_argv("projects/sample", "build")),
        ];

        let supervisors =
            workspace_supervisors(&processes, &az_filesystem::normalize(&workspace)).unwrap();

        assert_eq!(
            supervisors
                .iter()
                .map(|supervisor| (supervisor.pid, supervisor.session.as_str()))
                .collect::<Vec<_>>(),
            vec![(21, "main"), (23, "build")]
        );
    }

    #[test]
    fn sweep_rejects_a_workspace_supervisor_with_no_target() {
        let workspace = PathBuf::from("builds/azoth/target/debug").join(sessiond_image_name());
        let processes = [swept_process(
            31,
            &workspace,
            argv(&["az-sessiond", "--project"]),
        )];

        let error = workspace_supervisors(&processes, &az_filesystem::normalize(&workspace))
            .expect_err("a supervisor with no addressable target must fail the sweep");

        assert!(
            error.to_string().contains("31"),
            "the error must name the process that cannot be stopped: {error}"
        );
    }
}
