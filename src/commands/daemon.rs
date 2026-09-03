use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use az_endpoint_discovery::{
    DaemonEndpointRecord, daemon_endpoint_record_path_in, default_daemon_endpoint,
    default_daemon_endpoint_kind, default_project_registry_path, project_daemon_endpoint,
    project_daemon_endpoint_record_path_in, read_daemon_endpoint_record,
    read_project_daemon_endpoint_record, read_project_daemon_endpoint_record_in,
    remove_daemon_endpoint_record, remove_project_daemon_endpoint_record,
};
use az_filesystem::AzothDataHome;
use az_proto_core::{
    Capability, Endpoint, EndpointKind, ProtocolVersion, ServiceDescriptor, ServiceHealth,
    ServiceId, ServiceRole,
};
use az_proto_daemon::{
    DAEMON_AUDIENCE, DAEMON_CONTROL_PERMISSION, DAEMON_LEASE_PERMISSION,
    DAEMON_PROJECTS_PERMISSION, DAEMON_READ_PERMISSION, DAEMON_SESSIONS_PERMISSION,
    ListProjectsRequest, ListProjectsResult, ListSessionSupervisorsRequest,
    ListSessionSupervisorsResult, PlanProjectBuildRequest, PlanProjectServicesRequest,
    ProcessIdentity as ProtoProcessIdentity, ProjectBuildCommand, ProjectBuildPackageProfile,
    ProjectBuildPlan, ProjectRecord, ProjectResult, ProjectServiceCommand, ProjectServicePlan,
    RegisterProjectRootRequest, ResolveProjectRequest, ResolveSessionSupervisorRequest,
    SessionSupervisorDescriptor, SessionSupervisorResult, ShutdownDaemonRequest,
    ShutdownDaemonResult, TouchEditorLeaseRequest, TouchEditorLeaseResult,
    UnregisterSessionSupervisorRequest, UnregisterSessionSupervisorResult, daemon_capnp,
    editor_process_lease_id, forget_project_registration, read_project_registry,
};
use az_proto_session::{
    SESSION_READ_PERMISSION, SESSION_SUPERVISOR_AUDIENCE, SESSION_SUPERVISOR_NAMESPACE,
    SESSION_SUPERVISOR_SERVICE_NAME, SessionCapabilityRequest,
};
use az_service_catalog::{DAEMON_SERVICE_NAME, DAEMON_SERVICE_NAMESPACE};
#[cfg(test)]
use az_service_supervision::previous_log_path;
use az_service_supervision::{
    ProcessIdentity, ServiceLifecycleEvent, ServiceLifecycleEvents, ServiceProcessError,
    rotate_log_at_plan_time,
};
use az_session::connect_session_supervisor_rpc_client;
use tokio::runtime::Builder;
use tokio::task::LocalSet;
use tokio::time::{interval, timeout};
use tracing::{info, warn};

use crate::error::{CliError, CliResult, CommandFailedDetails};

const DAEMON_STOP_RPC_TIMEOUT: Duration = Duration::from_secs(3);
pub const DAEMON_RPC_PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
/// A first project registration may have to repair the authored/generated
/// resolution snapshot and regenerate target metadata before azd can publish
/// its endpoint. The launcher still fails immediately when the child exits;
/// this timeout only bounds a healthy but legitimately long bootstrap.
pub const DEFAULT_DAEMON_START_TIMEOUT_MS: u64 = 300_000;

pub fn serve(
    project_roots: &[PathBuf],
    project_registry: Option<&Path>,
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
) -> CliResult<()> {
    let daemon_executable = crate::commands::host_tools::ensure_daemon()?;
    let command = daemon_launch_command(
        &daemon_executable,
        kind,
        endpoint,
        project_roots,
        project_registry,
        DaemonLaunchOptions::default(),
    )?;

    info!(
        program = %command.program,
        args = ?command.args,
        cwd = %command.cwd.display(),
        "launching foreground azd process"
    );

    let mut process = Command::new(&command.program);
    process.args(&command.args).current_dir(&command.cwd);
    configure_daemon_engine_root(&mut process, &command);
    let status = process.status()?;

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

pub fn start(
    project_roots: &[PathBuf],
    project_registry: Option<&Path>,
    timeout_ms: u64,
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
) -> CliResult<()> {
    start_with_options(
        project_roots,
        project_registry,
        timeout_ms,
        kind,
        endpoint,
        DaemonLaunchOptions::default(),
        false,
    )
}

pub fn start_prebuilt(
    project_roots: &[PathBuf],
    project_registry: Option<&Path>,
    timeout_ms: u64,
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
) -> CliResult<()> {
    start_with_options(
        project_roots,
        project_registry,
        timeout_ms,
        kind,
        endpoint,
        DaemonLaunchOptions::default(),
        true,
    )
}

pub fn start_for_editor(
    project_roots: &[PathBuf],
    project_registry: Option<&Path>,
    timeout_ms: u64,
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
) -> CliResult<()> {
    start_with_options(
        project_roots,
        project_registry,
        timeout_ms,
        kind,
        endpoint,
        DaemonLaunchOptions {
            editor_owner_process: Some(ProcessIdentity::current()?),
            shutdown_when_editor_leases_gone: true,
        },
        false,
    )
}

fn start_with_options(
    project_roots: &[PathBuf],
    project_registry: Option<&Path>,
    timeout_ms: u64,
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
    options: DaemonLaunchOptions,
    host_tools_prebuilt: bool,
) -> CliResult<()> {
    let primary_project_root = project_roots.first().map(PathBuf::as_path);
    if let Some(record) = existing_reachable_daemon(kind, endpoint, primary_project_root)? {
        if let Some(owner_process) = options.editor_owner_process {
            touch_editor_owner_process(&record.endpoint, owner_process)?;
        }
        println!(
            "azd already running: {:?} {}",
            record.endpoint.kind, record.endpoint.address
        );
        println!("process_id: {}", record.process_id);
        std::io::stdout().flush()?;
        return Ok(());
    }

    let daemon_executable = if host_tools_prebuilt {
        crate::commands::host_tools::require_prebuilt_daemon()?
    } else {
        crate::commands::host_tools::ensure_daemon()?
    };
    let command = daemon_launch_command(
        &daemon_executable,
        kind,
        endpoint,
        project_roots,
        project_registry,
        options,
    )?;
    let requested_endpoint = daemon_endpoint_from_options(kind, endpoint, primary_project_root)?;
    let log_path = daemon_log_path(primary_project_root)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let (stdout, stderr) = open_rotated_daemon_logs(&log_path)?;

    info!(
        program = %command.program,
        args = ?command.args,
        cwd = %command.cwd.display(),
        log = %log_path.display(),
        "starting background azd process"
    );

    println!("starting azd");
    println!("log: {}", log_path.display());
    std::io::stdout().flush()?;

    let mut child = spawn_daemon_process(&command, stdout, stderr)?;

    let record = wait_for_daemon_start(
        primary_project_root,
        &mut child,
        &command,
        &requested_endpoint,
        timeout_ms,
        &log_path,
    )?;
    println!(
        "azd started: {:?} {}",
        record.endpoint.kind, record.endpoint.address
    );
    println!("process_id: {}", record.process_id);
    println!("log: {}", log_path.display());
    std::io::stdout().flush()?;
    Ok(())
}

fn open_rotated_daemon_logs(path: &Path) -> CliResult<(std::fs::File, std::fs::File)> {
    rotate_log_at_plan_time(path)?;
    let stdout = OpenOptions::new().create(true).append(true).open(path)?;
    let stderr = stdout.try_clone()?;
    Ok((stdout, stderr))
}

pub fn stop(
    reason: Option<String>,
    project_roots: Vec<PathBuf>,
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<()> {
    let reason = stop_reason(reason);
    let project_roots = project_roots
        .into_iter()
        .filter(|root| !root.as_os_str().is_empty())
        .collect::<Vec<_>>();

    if project_roots.is_empty() {
        let stopped = stop_discovered_daemons(kind, endpoint, &reason)?;
        if stopped == 0 {
            println!("No reachable azd endpoint found");
        }
        return Ok(());
    }

    if endpoint.is_some() {
        return Err(CliError::InvalidArgument {
            message: "`azoth daemon --endpoint <address> stop --project <path>` is ambiguous; pass either an explicit endpoint or one or more project roots".to_string(),
        });
    }

    for project_root in project_roots {
        let endpoint = project_daemon_endpoint_for_stop(kind, &project_root)?;
        if stop_reachable_daemon_endpoint(&endpoint, &reason)? {
            continue;
        }
        println!(
            "No reachable azd endpoint found for project {}",
            project_root.display()
        );
    }

    Ok(())
}

fn stop_discovered_daemons(
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
    reason: &str,
) -> CliResult<usize> {
    let explicit_endpoint = endpoint.is_some() || kind.is_some();
    let mut stopped = 0;
    if let Some(endpoint) = daemon_endpoint_for_stop(kind, endpoint)? {
        if explicit_endpoint {
            stop_daemon_endpoint(&endpoint, reason)?;
            stopped += 1;
        } else if stop_reachable_daemon_endpoint(&endpoint, reason)? {
            stopped += 1;
        } else {
            remove_daemon_endpoint_record()?;
        }
    }
    if !explicit_endpoint {
        stopped += stop_known_project_daemons(kind, reason)?;
    }
    Ok(stopped)
}

fn stop_known_project_daemons(kind: Option<EndpointKind>, reason: &str) -> CliResult<usize> {
    AzothDataHome::resolve().prepare()?;
    let projects = read_project_registry(&default_project_registry_path())?;
    let mut stopped = 0;
    for project in projects {
        let root = Path::new(&project.root);
        // A registry entry outlives its project whenever the manifest is
        // deleted but the registration is not, and resolving that project's
        // endpoint then fails on the missing `azoth.toml`. Propagating the
        // error aborted the whole sweep: every project after the stale entry
        // kept its daemon, and `stop` reported failure even for the daemons
        // that had already accepted shutdown. A project we cannot resolve has
        // no endpoint to stop, so skip it and keep going.
        let endpoint = match project_daemon_endpoint_for_stop(kind, root) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                warn!(
                    project = %project.project_id,
                    root = %root.display(),
                    %error,
                    "skipping unresolvable project while stopping azd daemons"
                );
                continue;
            }
        };
        if stop_reachable_daemon_endpoint(&endpoint, reason)? {
            stopped += 1;
        }
    }
    Ok(stopped)
}

fn daemon_endpoint_for_stop(
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<Option<Endpoint>> {
    if kind.is_none() && endpoint.is_none() {
        return Ok(read_daemon_endpoint_record()?.map(|record| record.endpoint));
    }

    daemon_endpoint(kind, endpoint).map(Some)
}

fn stop_daemon_endpoint(endpoint: &Endpoint, reason: &str) -> CliResult<()> {
    info!(
        endpoint = %endpoint.address,
        reason,
        "requesting azd shutdown"
    );

    let result = stop_daemon_through_daemon(endpoint, reason.to_string())?;
    if result.accepted {
        println!("azd shutdown accepted");
    } else {
        println!("azd shutdown rejected");
    }
    if !result.reason.is_empty() {
        println!("reason: {}", result.reason);
    }
    Ok(())
}

fn stop_reachable_daemon_endpoint(endpoint: &Endpoint, reason: &str) -> CliResult<bool> {
    if probe_daemon(endpoint).is_err() {
        return Ok(false);
    }
    stop_daemon_endpoint(endpoint, reason)?;
    Ok(true)
}

fn project_daemon_endpoint_for_stop(
    kind: Option<EndpointKind>,
    project_root: &Path,
) -> CliResult<Endpoint> {
    project_daemon_endpoint_for_stop_in(&AzothDataHome::resolve(), kind, project_root)
}

fn project_daemon_endpoint_for_stop_in(
    data_home: &AzothDataHome,
    kind: Option<EndpointKind>,
    project_root: &Path,
) -> CliResult<Endpoint> {
    validate_public_endpoint_kind(kind, "azd project endpoint")?;
    if kind.is_none()
        && let Some(record) = read_project_daemon_endpoint_record_in(data_home, project_root)?
    {
        return Ok(record.endpoint);
    }

    let kind = kind.unwrap_or_else(default_daemon_endpoint_kind);
    Ok(project_daemon_endpoint(kind, project_root)?)
}

pub fn register_project(
    path: Option<PathBuf>,
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<()> {
    let project_path = path.unwrap_or_else(|| PathBuf::from("."));
    let endpoint = daemon_endpoint(kind, endpoint)?;

    info!(
        project_root = %project_path.display(),
        endpoint = %endpoint.address,
        "registering project root with azd"
    );

    let requested_root = project_path.clone();
    let project = with_daemon(&endpoint, async move |client| {
        let mut request = client.register_project_root_request();
        (RegisterProjectRootRequest {
            capability: daemon_capability(DAEMON_PROJECTS_PERMISSION),
            root: project_path.to_string_lossy().into_owned(),
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let project = ProjectRecord::from_capnp(response.get()?.get_project()?)?;
        ensure_daemon_project_record_matches_request(
            &project,
            None,
            Some(&requested_root),
            "registerProjectRoot",
        )?;
        Ok(project)
    })?;

    println!(
        "Registered project '{}' ({})",
        project.project_id, project.root
    );
    Ok(())
}

pub fn forget_project(project_id: &str) -> CliResult<()> {
    AzothDataHome::resolve().prepare()?;
    let registry_path = default_project_registry_path();

    info!(
        project_id = %project_id,
        registry = %registry_path.display(),
        "forgetting project registration"
    );

    // Deliberately a registry edit rather than a daemon round-trip. The reason
    // to forget a project is usually that its manifest is gone, and a daemon
    // cannot resolve such a project to act on it -- routing this through azd
    // would fail for exactly the registrations that most need removing.
    let Some(removed) = forget_project_registration(&registry_path, project_id)? else {
        println!("No project registered as '{project_id}'");
        return Ok(());
    };

    println!("Forgot project '{}' ({})", removed.project_id, removed.root);
    if Path::new(&removed.manifest_path).exists() {
        println!(
            "note: {} still exists; `azoth daemon register-project` re-registers it",
            removed.manifest_path
        );
    }
    Ok(())
}

pub fn list_projects(kind: Option<EndpointKind>, endpoint: Option<String>) -> CliResult<()> {
    let endpoint = daemon_endpoint(kind, endpoint)?;

    info!(endpoint = %endpoint.address, "listing projects from azd");

    let result = with_daemon(&endpoint, async move |client| {
        let mut request = client.list_projects_request();
        (ListProjectsRequest {
            capability: daemon_capability(DAEMON_READ_PERMISSION),
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let result = ListProjectsResult::from_capnp(response.get()?.get_result()?)?;
        ensure_daemon_project_list_is_authoritative(&result)?;
        Ok(result)
    })?;

    if result.projects.is_empty() {
        println!("No projects registered with azd");
    } else {
        for project in result.projects {
            println!("{}\t{}\t{}", project.project_id, project.name, project.root);
        }
    }
    Ok(())
}

pub fn status(kind: Option<EndpointKind>, endpoint: Option<String>) -> CliResult<()> {
    if kind.is_some() || endpoint.is_some() {
        let endpoint = daemon_endpoint(kind, endpoint)?;
        println!("azd endpoint: {:?} {}", endpoint.kind, endpoint.address);
        println!("source: command-line");
        return Ok(());
    }

    if let Some(record) = read_daemon_endpoint_record()? {
        println!(
            "azd endpoint: {:?} {}",
            record.endpoint.kind, record.endpoint.address
        );
        println!("process_id: {}", record.process_id);
        println!("source: runtime record");
        match probe_daemon(&record.endpoint) {
            Ok(()) => println!("state: reachable"),
            Err(error) => {
                remove_daemon_endpoint_record()?;
                println!("state: stale");
                println!("stale_record_removed: true");
                println!("error: {error}");
            }
        }
    } else {
        let endpoint = default_daemon_endpoint(default_daemon_endpoint_kind())?;
        println!("No azd endpoint record found");
        println!("default endpoint: {:?} {}", endpoint.kind, endpoint.address);
    }
    Ok(())
}

pub fn resolve_project(
    project_id: String,
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<()> {
    let endpoint = daemon_endpoint(kind, endpoint)?;

    info!(
        project_id = %project_id,
        endpoint = %endpoint.address,
        "resolving project from azd"
    );

    let expected_project_id = project_id.clone();
    let result = with_daemon(&endpoint, async move |client| {
        let mut request = client.resolve_project_request();
        (ResolveProjectRequest {
            capability: daemon_capability(DAEMON_READ_PERMISSION),
            project_id,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let result = ProjectResult::from_capnp(response.get()?.get_result()?)?;
        if let Some(project) = &result.project {
            ensure_daemon_project_record_matches_request(
                project,
                Some(&expected_project_id),
                None,
                "resolveProject",
            )?;
        }
        Ok(result)
    })?;

    if let Some(project) = result.project {
        println!("project_id: {}", project.project_id);
        println!("name: {}", project.name);
        println!("root: {}", project.root);
        println!("manifest: {}", project.manifest_path);
        println!("engine_version: {}", project.engine_version);
    } else {
        println!("Project not registered");
    }
    Ok(())
}

pub fn list_session_supervisors(
    project_id: String,
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<()> {
    let endpoint = daemon_endpoint(kind, endpoint)?;

    info!(
        project_id = %project_id,
        endpoint = %endpoint.address,
        "listing session supervisors from azd"
    );

    let supervisors = list_session_supervisors_through_daemon(&endpoint, project_id)?;

    if supervisors.is_empty() {
        println!("No session supervisors registered with azd");
    } else {
        for supervisor in &supervisors {
            print_session_supervisor_descriptor(supervisor);
        }
    }
    Ok(())
}

pub fn resolve_session_supervisor(
    project_id: String,
    session: String,
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<()> {
    let endpoint = daemon_endpoint(kind, endpoint)?;

    info!(
        project_id = %project_id,
        session = %session,
        endpoint = %endpoint.address,
        "resolving session supervisor from azd"
    );

    match resolve_session_supervisor_through_daemon(&endpoint, project_id, session)? {
        Some(descriptor) => print_service_descriptor(&descriptor),
        None => println!("Session supervisor not registered"),
    }
    Ok(())
}

pub fn unregister_session_supervisor(
    project_id: String,
    session: String,
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<()> {
    let endpoint = daemon_endpoint(kind, endpoint)?;

    info!(
        project_id = %project_id,
        session = %session,
        endpoint = %endpoint.address,
        "unregistering session supervisor from azd"
    );

    if unregister_session_supervisor_through_daemon(&endpoint, project_id, session)? {
        println!("Unregistered session supervisor");
    } else {
        println!("Session supervisor not registered or descriptor changed");
    }
    Ok(())
}

pub fn prune_session_supervisors(
    project_id: &str,
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<()> {
    let endpoint = daemon_endpoint(kind, endpoint)?;

    info!(
        project_id = %project_id,
        endpoint = %endpoint.address,
        "pruning unreachable session supervisors from azd"
    );

    let report = prune_session_supervisors_through_daemon(&endpoint, project_id)?;
    if report.checked == 0 {
        println!("No session supervisors registered with azd");
    } else {
        println!(
            "Checked {} session supervisor(s): kept={}, removed={}, skipped={}",
            report.checked, report.kept, report.removed, report.skipped
        );
        for event in report.events {
            println!("{event}");
        }
    }
    Ok(())
}

pub fn plan_build(
    project_id: String,
    profile: String,
    package_selectors: Vec<String>,
    target: Option<String>,
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<()> {
    let endpoint = daemon_endpoint(kind, endpoint)?;

    info!(
        project_id = %project_id,
        profile = %profile,
        endpoint = %endpoint.address,
        "planning project build through azd"
    );

    let expected_project_id = project_id.clone();
    let plan = with_daemon(&endpoint, async move |client| {
        let mut request = client.plan_project_build_request();
        (PlanProjectBuildRequest {
            capability: daemon_capability(DAEMON_PROJECTS_PERMISSION),
            project_id,
            profile,
            target_triple: target,
            package_selectors,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let plan = ProjectBuildPlan::from_capnp(response.get()?.get_plan()?)?;
        ensure_daemon_project_build_plan_matches_request(&plan, &expected_project_id)?;
        Ok(plan)
    })?;

    if plan.commands.is_empty() {
        println!("No build commands planned");
    } else {
        if let Some(profile) = &plan.package_profile {
            print_build_package_profile(profile);
        }
        for command in &plan.commands {
            print_build_command(command);
        }
    }
    Ok(())
}

pub fn plan_services(
    project_id: String,
    session: String,
    service_endpoint_kind: Option<EndpointKind>,
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<()> {
    let endpoint = daemon_endpoint(kind, endpoint)?;
    let service_endpoint_kind = service_endpoint_kind.unwrap_or_else(default_service_endpoint_kind);
    validate_public_endpoint_kind(
        Some(service_endpoint_kind),
        "project service endpoint planning",
    )?;

    info!(
        project_id = %project_id,
        session = %session,
        endpoint = %endpoint.address,
        service_endpoint_kind = ?service_endpoint_kind,
        "planning project services through azd"
    );

    let expected_project_id = project_id.clone();
    let expected_session = session.clone();
    let plan = with_daemon(&endpoint, async move |client| {
        let mut request = client.plan_project_services_request();
        (PlanProjectServicesRequest {
            capability: daemon_capability(DAEMON_PROJECTS_PERMISSION),
            project_id,
            session_slug: session,
            endpoint_kind: service_endpoint_kind,
            workspace_root: None,
            service_names: Vec::new(),
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let plan = ProjectServicePlan::from_capnp(response.get()?.get_plan()?)?;
        ensure_daemon_project_service_plan_matches_request(
            &plan,
            &expected_project_id,
            &expected_session,
        )?;
        Ok(plan)
    })?;

    print_service_plan(&plan);
    Ok(())
}

fn stop_daemon_through_daemon(
    endpoint: &Endpoint,
    reason: String,
) -> CliResult<ShutdownDaemonResult> {
    with_daemon_timeout(
        endpoint,
        "stop azd",
        DAEMON_STOP_RPC_TIMEOUT,
        async move |client| {
            let mut request = client.shutdown_request();
            (ShutdownDaemonRequest {
                capability: daemon_capability(DAEMON_CONTROL_PERMISSION),
                reason,
            })
            .to_capnp(request.get().init_request())?;
            let response = request.send().promise.await?;
            Ok(ShutdownDaemonResult::from_capnp(
                response.get()?.get_result()?,
            )?)
        },
    )
}

fn stop_reason(reason: Option<String>) -> String {
    reason
        .filter(|reason| !reason.trim().is_empty())
        .unwrap_or_else(|| "azoth daemon stop".to_string())
}

pub fn ensure_daemon_project_record_matches_request(
    project: &ProjectRecord,
    expected_project_id: Option<&str>,
    expected_root: Option<&Path>,
    operation: &'static str,
) -> CliResult<()> {
    if let Some(expected_project_id) = expected_project_id
        && project.project_id != expected_project_id
    {
        return Err(daemon_authority_mismatch(
            operation,
            format!(
                "daemon returned project `{}`, expected `{expected_project_id}`",
                project.project_id
            ),
        ));
    }
    if project.project_id.trim().is_empty()
        || project.name.trim().is_empty()
        || project.root.trim().is_empty()
        || project.manifest_path.trim().is_empty()
        || project.engine_version.trim().is_empty()
    {
        return Err(daemon_authority_mismatch(
            operation,
            "project record identity fields cannot be empty".to_string(),
        ));
    }

    let root = Path::new(&project.root);
    let manifest_path = Path::new(&project.manifest_path);
    if !root.is_absolute() || !manifest_path.is_absolute() {
        return Err(daemon_authority_mismatch(
            operation,
            format!(
                "project `{}` returned non-absolute root or manifest path",
                project.project_id
            ),
        ));
    }

    let expected_manifest_path = root.join("azoth.toml");
    if !same_daemon_path(manifest_path, &expected_manifest_path) {
        return Err(daemon_authority_mismatch(
            operation,
            format!(
                "project `{}` manifest path `{}` does not match root `{}`",
                project.project_id, project.manifest_path, project.root
            ),
        ));
    }

    if let Some(expected_root) = expected_root
        && !same_daemon_path(root, expected_root)
    {
        return Err(daemon_authority_mismatch(
            operation,
            format!(
                "project `{}` root `{}` does not match requested root `{}`",
                project.project_id,
                project.root,
                expected_root.display()
            ),
        ));
    }
    Ok(())
}

fn ensure_daemon_project_list_is_authoritative(result: &ListProjectsResult) -> CliResult<()> {
    result
        .protocol_version
        .require(ProtocolVersion::CURRENT)
        .map_err(|error| {
            daemon_authority_mismatch(
                "listProjects",
                format!("azd unavailable until restarted: {error}"),
            )
        })?;

    let mut seen = BTreeSet::new();
    for project in &result.projects {
        ensure_daemon_project_record_matches_request(project, None, None, "listProjects")?;
        if !seen.insert(project.project_id.as_str()) {
            return Err(daemon_authority_mismatch(
                "listProjects",
                format!("daemon returned duplicate project `{}`", project.project_id),
            ));
        }
    }
    Ok(())
}

pub fn ensure_daemon_session_supervisor_descriptor_matches_request(
    descriptor: &ServiceDescriptor,
    expected_session_slug: &str,
    operation: &'static str,
) -> CliResult<()> {
    if expected_session_slug.trim().is_empty() {
        return Err(daemon_authority_mismatch(
            operation,
            "requested session slug cannot be empty".to_string(),
        ));
    }
    validate_daemon_session_supervisor_descriptor(descriptor)
        .map_err(|reason| daemon_authority_mismatch(operation, reason))
}

pub fn ensure_daemon_session_supervisor_list_is_authoritative(
    supervisors: &[SessionSupervisorDescriptor],
) -> CliResult<()> {
    let mut seen = BTreeSet::new();
    for supervisor in supervisors {
        if supervisor.session_slug.trim().is_empty() {
            return Err(daemon_authority_mismatch(
                "listSessionSupervisors",
                "daemon returned a session-supervisor with an empty slug".to_string(),
            ));
        }
        if !seen.insert(supervisor.session_slug.as_str()) {
            return Err(daemon_authority_mismatch(
                "listSessionSupervisors",
                format!(
                    "daemon returned duplicate session-supervisor slug `{}`",
                    supervisor.session_slug
                ),
            ));
        }
        validate_daemon_session_supervisor_descriptor(&supervisor.descriptor).map_err(
            |reason| {
                daemon_authority_mismatch(
                    "listSessionSupervisors",
                    format!(
                        "daemon returned invalid descriptor for session `{}`: {reason}",
                        supervisor.session_slug
                    ),
                )
            },
        )?;
    }
    Ok(())
}

pub fn ensure_daemon_project_build_plan_matches_request(
    plan: &ProjectBuildPlan,
    expected_project_id: &str,
) -> CliResult<()> {
    if expected_project_id.trim().is_empty() {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            "requested project id cannot be empty".to_string(),
        ));
    }
    if plan.commands.is_empty() {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            format!("daemon returned no build commands for project `{expected_project_id}`"),
        ));
    }
    if let Some(profile) = &plan.package_profile {
        ensure_daemon_project_build_package_profile_is_traceable(profile)?;
    }
    for command in &plan.commands {
        ensure_daemon_project_build_command_is_traceable(command, expected_project_id)?;
    }
    Ok(())
}

pub fn ensure_daemon_project_service_plan_matches_request(
    plan: &ProjectServicePlan,
    expected_project_id: &str,
    expected_session_slug: &str,
) -> CliResult<()> {
    if expected_project_id.trim().is_empty() || expected_session_slug.trim().is_empty() {
        return Err(daemon_authority_mismatch(
            "planProjectServices",
            "requested project id and session slug cannot be empty".to_string(),
        ));
    }
    if plan.commands.is_empty() {
        return Err(daemon_authority_mismatch(
            "planProjectServices",
            format!(
                "daemon returned no service commands for project `{expected_project_id}` session `{expected_session_slug}`"
            ),
        ));
    }

    for command in &plan.build_commands {
        ensure_daemon_project_build_command_is_traceable(command, expected_project_id)?;
    }

    let mut seen_services = BTreeSet::new();
    for command in &plan.commands {
        ensure_daemon_project_service_command_is_traceable(
            command,
            expected_project_id,
            expected_session_slug,
        )?;
        if !seen_services.insert(command.service_name.as_str()) {
            return Err(daemon_authority_mismatch(
                "planProjectServices",
                format!(
                    "daemon returned duplicate service command `{}` for session `{expected_session_slug}`",
                    command.service_name
                ),
            ));
        }
    }
    Ok(())
}

fn ensure_daemon_project_build_command_is_traceable(
    command: &ProjectBuildCommand,
    expected_project_id: &str,
) -> CliResult<()> {
    if command.owner_id.trim().is_empty()
        || command.owner_root.trim().is_empty()
        || command.target_name.trim().is_empty()
        || command.program.trim().is_empty()
        || command.cwd.trim().is_empty()
    {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            format!("daemon returned incomplete build command for project `{expected_project_id}`"),
        ));
    }

    let owner_root = Path::new(&command.owner_root);
    let cwd = Path::new(&command.cwd);
    if !owner_root.is_absolute() || !cwd.is_absolute() {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            format!(
                "daemon returned non-absolute build command paths for owner `{}`",
                command.owner_id
            ),
        ));
    }
    if command.args.is_empty() {
        return Err(daemon_authority_mismatch(
            "planProjectBuild",
            format!(
                "daemon returned build command `{}` for owner `{}` without args",
                command.target_name, command.owner_id
            ),
        ));
    }
    Ok(())
}

fn ensure_daemon_project_build_package_profile_is_traceable(
    profile: &ProjectBuildPackageProfile,
) -> CliResult<()> {
    for (field, value) in [
        ("name", profile.name.as_str()),
        ("asset platform", profile.asset_platform.as_str()),
        ("cargo profile", profile.cargo_profile.as_str()),
        ("container", profile.container.as_str()),
        ("compression", profile.compression.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(daemon_authority_mismatch(
                "planProjectBuild",
                format!("daemon returned package profile with empty {field}"),
            ));
        }
    }

    match (
        profile.compression.as_str(),
        profile.oodle_compressor.as_deref(),
        profile.oodle_effort.as_deref(),
    ) {
        ("none", None, None) => {}
        ("oodle", Some(compressor), Some(effort))
            if !compressor.trim().is_empty() && !effort.trim().is_empty() => {}
        ("oodle", _, _) => {
            return Err(daemon_authority_mismatch(
                "planProjectBuild",
                format!(
                    "daemon returned package profile `{}` with oodle compression but incomplete oodle settings",
                    profile.name
                ),
            ));
        }
        (compression, None, None) => {
            return Err(daemon_authority_mismatch(
                "planProjectBuild",
                format!(
                    "daemon returned package profile `{}` with unsupported compression `{compression}`",
                    profile.name
                ),
            ));
        }
        (_, _, _) => {
            return Err(daemon_authority_mismatch(
                "planProjectBuild",
                format!(
                    "daemon returned package profile `{}` with oodle settings but non-oodle compression `{}`",
                    profile.name, profile.compression
                ),
            ));
        }
    }

    az_asset::PackagePayloadPolicy::from_profile(&az_asset::PackageManifestProfile {
        name: profile.name.clone(),
        asset_platform: profile.asset_platform.clone(),
        cargo_profile: profile.cargo_profile.clone(),
        container: profile.container.clone(),
        compression: profile.compression.clone(),
        oodle_compressor: profile.oodle_compressor.clone(),
        oodle_effort: profile.oodle_effort.clone(),
    })
    .map_err(|error| {
        daemon_authority_mismatch(
            "planProjectBuild",
            format!(
                "daemon returned unsupported package backend policy `{}`/`{}` for profile `{}`: {error}",
                profile.container, profile.compression, profile.name
            ),
        )
    })?;

    Ok(())
}

fn ensure_daemon_project_service_command_is_traceable(
    command: &ProjectServiceCommand,
    expected_project_id: &str,
    expected_session_slug: &str,
) -> CliResult<()> {
    if command.owner_id.trim().is_empty()
        || command.owner_root.trim().is_empty()
        || command.build_output_root.trim().is_empty()
        || command.service_name.trim().is_empty()
        || command.program.trim().is_empty()
        || command.cwd.trim().is_empty()
        || command.endpoint.address.trim().is_empty()
    {
        return Err(daemon_authority_mismatch(
            "planProjectServices",
            format!(
                "daemon returned incomplete service command for project `{expected_project_id}` session `{expected_session_slug}`"
            ),
        ));
    }
    if command.role == ServiceRole::Unknown || command.role == ServiceRole::Editor {
        return Err(daemon_authority_mismatch(
            "planProjectServices",
            format!(
                "daemon returned unsupported role {:?} for service `{}`",
                command.role, command.service_name
            ),
        ));
    }
    if command.endpoint.kind == EndpointKind::InProcess {
        return Err(daemon_authority_mismatch(
            "planProjectServices",
            format!(
                "daemon returned in-process endpoint for service `{}`",
                command.service_name
            ),
        ));
    }
    let owner_root = Path::new(&command.owner_root);
    let build_output_root = Path::new(&command.build_output_root);
    let cwd = Path::new(&command.cwd);
    let program = Path::new(&command.program);
    if !owner_root.is_absolute()
        || !build_output_root.is_absolute()
        || !cwd.is_absolute()
        || !program.is_absolute()
    {
        return Err(daemon_authority_mismatch(
            "planProjectServices",
            format!(
                "daemon returned non-absolute service command paths for service `{}`",
                command.service_name
            ),
        ));
    }
    if command.args.is_empty() {
        return Err(daemon_authority_mismatch(
            "planProjectServices",
            format!(
                "daemon returned service command `{}` without args",
                command.service_name
            ),
        ));
    }
    Ok(())
}

fn same_daemon_path(left: &Path, right: &Path) -> bool {
    let left = comparable_daemon_path(left);
    let right = comparable_daemon_path(right);
    #[cfg(windows)]
    {
        comparable_daemon_path_text(&left) == comparable_daemon_path_text(&right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn comparable_daemon_path(path: &Path) -> PathBuf {
    if path.exists() {
        path.canonicalize().map_or_else(
            |_| cli_compatible_daemon_path(path.to_path_buf()),
            cli_compatible_daemon_path,
        )
    } else {
        cli_compatible_daemon_path(path.to_path_buf())
    }
}

#[cfg(windows)]
fn comparable_daemon_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', r"\")
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn cli_compatible_daemon_path(path: PathBuf) -> PathBuf {
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
fn cli_compatible_daemon_path(path: PathBuf) -> PathBuf {
    path
}

const fn daemon_authority_mismatch(operation: &'static str, reason: String) -> CliError {
    CliError::DaemonAuthorityMismatch { operation, reason }
}

fn list_session_supervisors_through_daemon(
    endpoint: &Endpoint,
    project_id: String,
) -> CliResult<Vec<SessionSupervisorDescriptor>> {
    with_daemon(endpoint, async move |client| {
        let mut request = client.list_session_supervisors_request();
        (ListSessionSupervisorsRequest {
            capability: daemon_capability(DAEMON_READ_PERMISSION),
            project_id,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let supervisors =
            ListSessionSupervisorsResult::from_capnp(response.get()?.get_result()?)?.supervisors;
        ensure_daemon_session_supervisor_list_is_authoritative(&supervisors)?;
        Ok(supervisors)
    })
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PruneSessionSupervisorsReport {
    checked: usize,
    kept: usize,
    removed: usize,
    skipped: usize,
    events: Vec<String>,
}

fn prune_session_supervisors_through_daemon(
    endpoint: &Endpoint,
    project_id: &str,
) -> CliResult<PruneSessionSupervisorsReport> {
    let supervisors = list_session_supervisors_through_daemon(endpoint, project_id.to_string())?;
    let mut report = PruneSessionSupervisorsReport {
        checked: supervisors.len(),
        ..Default::default()
    };

    for supervisor in supervisors {
        match probe_session_supervisor_descriptor(&supervisor.descriptor) {
            Ok(()) => {
                report.kept += 1;
                report.events.push(format!(
                    "kept {}\t{:?} {}",
                    supervisor.session_slug,
                    supervisor.descriptor.endpoint.kind,
                    supervisor.descriptor.endpoint.address
                ));
            }
            Err(error) => {
                let removed = unregister_session_supervisor_descriptor_through_daemon(
                    endpoint,
                    project_id.to_string(),
                    supervisor.session_slug.clone(),
                    supervisor.descriptor.clone(),
                )?;
                if removed {
                    report.removed += 1;
                    report.events.push(format!(
                        "removed {}\t{:?} {}\t{}",
                        supervisor.session_slug,
                        supervisor.descriptor.endpoint.kind,
                        supervisor.descriptor.endpoint.address,
                        error
                    ));
                } else {
                    report.skipped += 1;
                    report.events.push(format!(
                        "skipped {}\tdescriptor changed before prune",
                        supervisor.session_slug
                    ));
                }
            }
        }
    }

    Ok(report)
}

fn unregister_session_supervisor_through_daemon(
    endpoint: &Endpoint,
    project_id: String,
    session: String,
) -> CliResult<bool> {
    let Some(descriptor) =
        resolve_session_supervisor_through_daemon(endpoint, project_id.clone(), session.clone())?
    else {
        return Ok(false);
    };

    unregister_session_supervisor_descriptor_through_daemon(
        endpoint, project_id, session, descriptor,
    )
}

fn unregister_session_supervisor_descriptor_through_daemon(
    endpoint: &Endpoint,
    project_id: String,
    session: String,
    descriptor: ServiceDescriptor,
) -> CliResult<bool> {
    with_daemon(endpoint, async move |client| {
        let mut request = client.unregister_session_supervisor_request();
        (UnregisterSessionSupervisorRequest {
            capability: daemon_capability(DAEMON_SESSIONS_PERMISSION),
            project_id,
            session_slug: session,
            descriptor,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        Ok(UnregisterSessionSupervisorResult::from_capnp(response.get()?.get_result()?)?.removed)
    })
}

fn probe_session_supervisor_descriptor(descriptor: &ServiceDescriptor) -> Result<(), String> {
    let runtime = Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(|error| error.to_string())?;
    let local = LocalSet::new();
    let descriptor = descriptor.clone();

    local.block_on(&runtime, async move {
        let client = connect_session_supervisor_rpc_client(&descriptor.endpoint)
            .await
            .map_err(|error| error.to_string())?;
        let mut request = client.list_request();
        (SessionCapabilityRequest {
            capability: daemon_session_read_capability(&descriptor)?,
        })
        .to_capnp(request.get())
        .map_err(|error| error.to_string())?;
        let response = request
            .send()
            .promise
            .await
            .map_err(|error| error.to_string())?;
        response
            .get()
            .map_err(|error| error.to_string())?
            .get_sessions()
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

fn daemon_session_read_capability(descriptor: &ServiceDescriptor) -> Result<Capability, String> {
    validate_daemon_session_supervisor_descriptor(descriptor)?;
    let permissions = [SESSION_READ_PERMISSION];
    descriptor
        .brokered_capability_template(
            ServiceRole::Daemon,
            SESSION_SUPERVISOR_AUDIENCE,
            &permissions,
            None,
        )
        .ok_or_else(|| {
            format!(
                "session-supervisor descriptor {}.{} did not grant `{}` capability `{}` to {}.{}",
                descriptor.id.namespace,
                descriptor.id.name,
                SESSION_SUPERVISOR_AUDIENCE,
                permissions.join(", "),
                DAEMON_SERVICE_NAMESPACE,
                DAEMON_SERVICE_NAME
            )
        })
}

fn validate_daemon_session_supervisor_descriptor(
    descriptor: &ServiceDescriptor,
) -> Result<(), String> {
    let expected_id = ServiceId::new(
        SESSION_SUPERVISOR_NAMESPACE,
        SESSION_SUPERVISOR_SERVICE_NAME,
    );
    if descriptor.id != expected_id || descriptor.role != ServiceRole::SessionSupervisor {
        return Err(format!(
            "session-supervisor probe expected descriptor {}.{} role {:?}, got {}.{} role {:?}",
            expected_id.namespace,
            expected_id.name,
            ServiceRole::SessionSupervisor,
            descriptor.id.namespace,
            descriptor.id.name,
            descriptor.role
        ));
    }
    if descriptor.run == uuid::Uuid::nil() {
        return Err("session-supervisor descriptor run must not be nil".to_string());
    }
    validate_public_endpoint_kind(
        Some(descriptor.endpoint.kind),
        "session-supervisor descriptor",
    )
    .map_err(|error| error.to_string())?;

    descriptor
        .validate_brokered_capability_templates()
        .map_err(|error| {
            format!(
                "session-supervisor descriptor {}.{} has invalid brokered capability templates: {error}",
                descriptor.id.namespace, descriptor.id.name
            )
        })
}

fn resolve_session_supervisor_through_daemon(
    endpoint: &Endpoint,
    project_id: String,
    session: String,
) -> CliResult<Option<ServiceDescriptor>> {
    let expected_session = session.clone();
    with_daemon(endpoint, async move |client| {
        let mut request = client.resolve_session_supervisor_request();
        (ResolveSessionSupervisorRequest {
            capability: daemon_capability(DAEMON_READ_PERMISSION),
            project_id,
            session_slug: session,
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let descriptor =
            SessionSupervisorResult::from_capnp(response.get()?.get_result()?)?.descriptor;
        if let Some(descriptor) = &descriptor {
            ensure_daemon_session_supervisor_descriptor_matches_request(
                descriptor,
                &expected_session,
                "resolveSessionSupervisor",
            )?;
        }
        Ok(descriptor)
    })
}

pub fn daemon_endpoint(
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<Endpoint> {
    validate_public_endpoint_kind(kind, "azd endpoint")?;
    if kind.is_none()
        && endpoint.is_none()
        && let Some(record) = read_daemon_endpoint_record()?
    {
        return Ok(record.endpoint);
    }

    let kind = kind.unwrap_or_else(default_daemon_endpoint_kind);
    Ok(match endpoint {
        Some(address) => Endpoint::new(kind, address),
        None => default_daemon_endpoint(kind)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonEndpointSource {
    Explicit,
    RuntimeRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalDaemonEndpoint {
    pub endpoint: Endpoint,
    pub source: DaemonEndpointSource,
}

pub fn optional_daemon_endpoint_with_source(
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
) -> CliResult<Option<OptionalDaemonEndpoint>> {
    if kind.is_some() || endpoint.is_some() {
        Ok(Some(OptionalDaemonEndpoint {
            endpoint: daemon_endpoint(kind, endpoint)?,
            source: DaemonEndpointSource::Explicit,
        }))
    } else {
        Ok(
            read_daemon_endpoint_record()?.map(|record| OptionalDaemonEndpoint {
                endpoint: record.endpoint,
                source: DaemonEndpointSource::RuntimeRecord,
            }),
        )
    }
}

pub fn optional_project_daemon_endpoint_with_source(
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
    project_root: &Path,
) -> CliResult<Option<OptionalDaemonEndpoint>> {
    if kind.is_some() || endpoint.is_some() {
        Ok(Some(OptionalDaemonEndpoint {
            endpoint: daemon_endpoint_from_options(kind, endpoint, Some(project_root))?,
            source: DaemonEndpointSource::Explicit,
        }))
    } else {
        Ok(
            read_project_daemon_endpoint_record(project_root)?.map(|record| {
                OptionalDaemonEndpoint {
                    endpoint: record.endpoint,
                    source: DaemonEndpointSource::RuntimeRecord,
                }
            }),
        )
    }
}

pub fn handle_stale_runtime_record(error: &CliError) -> CliResult<()> {
    warn!(
        error = %error,
        "azd runtime endpoint record was stale; removing record"
    );
    remove_daemon_endpoint_record()?;
    Ok(())
}

pub fn handle_stale_project_runtime_record(error: &CliError, project_root: &Path) -> CliResult<()> {
    warn!(
        error = %error,
        project_root = %project_root.display(),
        "project azd runtime endpoint record was stale; removing record"
    );
    remove_project_daemon_endpoint_record(project_root)?;
    Ok(())
}

pub fn is_daemon_connection_failure(error: &CliError) -> bool {
    match error {
        CliError::RpcTransport(_) => true,
        CliError::ServiceProtocol(error) => error.kind == capnp::ErrorKind::Disconnected,
        _ => false,
    }
}

fn existing_reachable_daemon(
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
    project_root: Option<&Path>,
) -> CliResult<Option<DaemonEndpointRecord>> {
    if kind.is_none()
        && endpoint.is_none()
        && let Some(project_root) = project_root
    {
        if let Some(record) = read_project_daemon_endpoint_record(project_root)? {
            if probe_daemon(&record.endpoint).is_ok() {
                return Ok(Some(record));
            }
            remove_project_daemon_endpoint_record(project_root)?;
        }
    } else if let Some(record) = read_daemon_endpoint_record()? {
        if probe_daemon(&record.endpoint).is_ok() {
            return Ok(Some(record));
        }
        remove_daemon_endpoint_record()?;
    }

    let endpoint = daemon_endpoint_from_options(kind, endpoint, project_root)?;
    if probe_daemon(&endpoint).is_ok() {
        return Ok(Some(DaemonEndpointRecord {
            endpoint,
            process_id: 0,
            protocol_version: ProtocolVersion::CURRENT,
        }));
    }

    Ok(None)
}

fn daemon_endpoint_from_options(
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
    project_root: Option<&Path>,
) -> CliResult<Endpoint> {
    validate_public_endpoint_kind(kind, "azd endpoint")?;
    let kind = kind.unwrap_or_else(default_daemon_endpoint_kind);
    Ok(match (endpoint, project_root) {
        (Some(address), _) => Endpoint::new(kind, address),
        (None, Some(root)) => project_daemon_endpoint(kind, root)?,
        (None, None) => default_daemon_endpoint(kind)?,
    })
}

fn wait_for_daemon_start(
    project_root: Option<&Path>,
    child: &mut Child,
    command: &DaemonLaunchCommand,
    requested_endpoint: &Endpoint,
    timeout_ms: u64,
    log_path: &Path,
) -> CliResult<DaemonEndpointRecord> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let record_path = match daemon_endpoint_record_path_for(project_root) {
        Ok(path) => path,
        Err(error) => {
            abort_daemon_process(child);
            return Err(error);
        }
    };
    let process_identity = match capture_daemon_process_identity(child) {
        Ok(identity) => identity,
        Err(error) => {
            abort_daemon_process(child);
            return Err(error);
        }
    };
    let lifecycle = ServiceLifecycleEvents::new();
    if let Err(error) = lifecycle.add_identity(process_identity) {
        abort_daemon_process(child);
        return Err(error.into());
    }
    let mut ready = match lifecycle.subscribe_ready([record_path.as_path()]) {
        Ok(ready) => ready,
        Err(error) => {
            abort_daemon_process(child);
            let _ = lifecycle.retire_exit(process_identity);
            return Err(error.into());
        }
    };

    let result = (|| {
        loop {
            let record = match project_root {
                Some(root) => read_project_daemon_endpoint_record(root)?,
                None => read_daemon_endpoint_record()?,
            };
            if let Some(record) = record
                && endpoint_matches_requested(&record.endpoint, requested_endpoint)
                && probe_daemon(&record.endpoint).is_ok()
            {
                return Ok(record);
            }

            let event = lifecycle.wait_until(deadline)?;
            observe_daemon_start_event(
                event,
                child,
                command,
                process_identity,
                timeout_ms,
                log_path,
            )?;
        }
    })();

    match result {
        Ok(record) => {
            let release = lifecycle.cancel_exit_wait(process_identity);
            let ready = ready.finish();
            release?;
            ready?;
            Ok(record)
        }
        Err(error) => {
            abort_daemon_process(child);
            if let Err(cleanup) = ready.finish() {
                warn!(error = %cleanup, "failed to close azd readiness watcher after bootstrap failure");
            }
            if let Err(cleanup) = lifecycle.retire_exit(process_identity) {
                warn!(error = %cleanup, "failed to retire azd exit wait after bootstrap failure");
            }
            Err(error)
        }
    }
}

/// Resolves the endpoint record `azd` publishes: project-scoped when a project root is given,
/// otherwise the machine-wide record.
fn daemon_endpoint_record_path_for(project_root: Option<&Path>) -> CliResult<PathBuf> {
    let data_home = AzothDataHome::resolve();
    project_root.map_or_else(
        || daemon_endpoint_record_path_in(&data_home).map_err(CliError::from),
        |root| project_daemon_endpoint_record_path_in(&data_home, root).map_err(CliError::from),
    )
}

fn capture_daemon_process_identity(child: &Child) -> CliResult<ProcessIdentity> {
    match ProcessIdentity::capture(child.id()) {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => Err(ServiceProcessError::ProcessIdentityUnavailable { pid: child.id() }.into()),
        Err(source) => Err(ServiceProcessError::ProcessIdentityCapture {
            pid: child.id(),
            source,
        }
        .into()),
    }
}

/// Interprets one lifecycle event observed while waiting for `azd` to publish its endpoint.
/// `Ok(())` means keep polling; the launched process exiting, its exit binding failing, or the
/// deadline elapsing all end the wait with an error.
fn observe_daemon_start_event(
    event: Option<ServiceLifecycleEvent>,
    child: &mut Child,
    command: &DaemonLaunchCommand,
    process_identity: ProcessIdentity,
    timeout_ms: u64,
    log_path: &Path,
) -> CliResult<()> {
    match event {
        Some(ServiceLifecycleEvent::ProcessExited(identity)) if identity == process_identity => {
            let status = child.wait()?;
            Err(CliError::CommandFailed(Box::new(CommandFailedDetails {
                program: command.program.clone(),
                args: command.args.clone(),
                cwd: command.cwd.clone(),
                status: status.code(),
            })))
        }
        Some(ServiceLifecycleEvent::ProcessExitWaitFailed { identity, reason })
            if identity == process_identity =>
        {
            Err(ServiceProcessError::ProcessExitBindingUnavailable { identity, reason }.into())
        }
        Some(
            ServiceLifecycleEvent::ReadyFileChanged
            | ServiceLifecycleEvent::ProcessExited(_)
            | ServiceLifecycleEvent::ProcessExitWaitFailed { .. },
        ) => Ok(()),
        None => Err(CliError::DaemonStartTimedOut {
            timeout_ms,
            log_path: log_path.to_path_buf(),
        }),
    }
}

fn abort_daemon_process(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn spawn_daemon_process(
    command: &DaemonLaunchCommand,
    stdout: std::fs::File,
    stderr: std::fs::File,
) -> std::io::Result<Child> {
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    configure_daemon_engine_root(&mut process, command);
    configure_background_process(&mut process);
    process.spawn()
}

fn configure_daemon_engine_root(process: &mut Command, command: &DaemonLaunchCommand) {
    if let Some(engine_root) = &command.engine_root {
        process.env(az_project::AZOTH_ENGINE_ROOT_ENV, engine_root);
    }
}

#[cfg(windows)]
fn configure_background_process(process: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    process.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_background_process(_process: &mut Command) {}

fn endpoint_matches_requested(record: &Endpoint, requested: &Endpoint) -> bool {
    record.kind == requested.kind
        && (record.address == requested.address || requested.address.ends_with(":0"))
}

fn daemon_log_path(project_root: Option<&Path>) -> CliResult<PathBuf> {
    daemon_log_path_in(&AzothDataHome::resolve(), project_root)
}

fn daemon_log_path_in(
    data_home: &AzothDataHome,
    project_root: Option<&Path>,
) -> CliResult<PathBuf> {
    let record_path = match project_root {
        Some(root) => project_daemon_endpoint_record_path_in(data_home, root)?,
        None => daemon_endpoint_record_path_in(data_home)?,
    };
    Ok(record_path.with_file_name("azd.log"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonLaunchCommand {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    engine_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DaemonLaunchOptions {
    editor_owner_process: Option<ProcessIdentity>,
    shutdown_when_editor_leases_gone: bool,
}

fn daemon_launch_command(
    daemon_executable: &Path,
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
    project_roots: &[PathBuf],
    project_registry: Option<&Path>,
    options: DaemonLaunchOptions,
) -> CliResult<DaemonLaunchCommand> {
    let daemon_args = daemon_args(kind, endpoint, project_roots, project_registry, options)?;
    Ok(DaemonLaunchCommand {
        program: daemon_executable.to_string_lossy().into_owned(),
        args: daemon_args,
        cwd: std::env::current_dir()?,
        engine_root: source_engine_root_for_host_tool(daemon_executable),
    })
}

fn source_engine_root_for_host_tool(executable: &Path) -> Option<PathBuf> {
    let executable = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    executable
        .ancestors()
        .find(|candidate| candidate.join(az_project::ENGINE_MANIFEST_FILE).is_file())
        .map(Path::to_path_buf)
}

fn daemon_args(
    kind: Option<EndpointKind>,
    endpoint: Option<&str>,
    project_roots: &[PathBuf],
    project_registry: Option<&Path>,
    options: DaemonLaunchOptions,
) -> CliResult<Vec<String>> {
    let mut args = Vec::new();
    if let Some(kind) = kind {
        args.extend([
            "--endpoint-kind".to_string(),
            endpoint_kind_arg(kind)?.to_string(),
        ]);
    }
    if let Some(endpoint) = endpoint {
        args.extend(["--endpoint".to_string(), endpoint.to_string()]);
    }
    for root in project_roots {
        args.extend([
            "--project".to_string(),
            child_path(root)?.to_string_lossy().into_owned(),
        ]);
    }
    if let Some(project_registry) = project_registry {
        args.extend([
            "--project-registry".to_string(),
            child_path(project_registry)?.to_string_lossy().into_owned(),
        ]);
    }
    if let Some(owner_process) = options.editor_owner_process {
        args.extend([
            "--editor-owner-process".to_string(),
            format!(
                "{}:{}",
                owner_process.process_id, owner_process.process_start_time
            ),
        ]);
    }
    if options.shutdown_when_editor_leases_gone {
        args.push("--shutdown-when-editor-leases-gone".to_string());
    }
    Ok(args)
}

fn child_path(path: &Path) -> CliResult<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

const fn endpoint_kind_arg(kind: EndpointKind) -> CliResult<&'static str> {
    match kind {
        EndpointKind::WindowsNamedPipe => Ok("windows-named-pipe"),
        EndpointKind::UnixDomainSocket => Ok("unix-domain-socket"),
        EndpointKind::Tcp => Ok("tcp"),
        EndpointKind::InProcess => Err(CliError::UnsupportedEndpointKind {
            operation: "azd launch",
            kind,
        }),
    }
}

const fn validate_public_endpoint_kind(
    kind: Option<EndpointKind>,
    operation: &'static str,
) -> CliResult<()> {
    if matches!(kind, Some(EndpointKind::InProcess)) {
        return Err(CliError::UnsupportedEndpointKind {
            operation,
            kind: EndpointKind::InProcess,
        });
    }
    Ok(())
}

pub fn daemon_capability(permission: &str) -> Capability {
    Capability::new(ServiceId::new("azoth", "cli"), ServiceRole::Editor)
        .with_audience(DAEMON_AUDIENCE)
        .with_permissions([permission])
}

pub fn with_daemon<F, Fut, T>(endpoint: &Endpoint, future: F) -> CliResult<T>
where
    F: FnOnce(daemon_capnp::az_daemon::Client) -> Fut + 'static,
    Fut: std::future::Future<Output = CliResult<T>> + 'static,
    T: 'static,
{
    with_daemon_inner(endpoint, None, None, "azd RPC", future)
}

pub fn with_daemon_progress<F, Fut, T>(
    endpoint: &Endpoint,
    operation: &'static str,
    progress_interval: Duration,
    future: F,
) -> CliResult<T>
where
    F: FnOnce(daemon_capnp::az_daemon::Client) -> Fut + 'static,
    Fut: std::future::Future<Output = CliResult<T>> + 'static,
    T: 'static,
{
    with_daemon_inner(endpoint, None, Some(progress_interval), operation, future)
}

fn with_daemon_timeout<F, Fut, T>(
    endpoint: &Endpoint,
    operation: &'static str,
    deadline: Duration,
    future: F,
) -> CliResult<T>
where
    F: FnOnce(daemon_capnp::az_daemon::Client) -> Fut + 'static,
    Fut: std::future::Future<Output = CliResult<T>> + 'static,
    T: 'static,
{
    with_daemon_inner(endpoint, Some(deadline), None, operation, future)
}

fn with_daemon_inner<F, Fut, T>(
    endpoint: &Endpoint,
    deadline: Option<Duration>,
    progress_interval: Option<Duration>,
    operation: &'static str,
    future: F,
) -> CliResult<T>
where
    F: FnOnce(daemon_capnp::az_daemon::Client) -> Fut + 'static,
    Fut: std::future::Future<Output = CliResult<T>> + 'static,
    T: 'static,
{
    let endpoint = endpoint.clone();
    let endpoint_label = endpoint.address.clone();
    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let local = LocalSet::new();
    local.block_on(&runtime, async move {
        let rpc = async move {
            let client: daemon_capnp::az_daemon::Client =
                az_rpc::connect_twoparty_bootstrap(&endpoint).await?;
            let response = client.health_request().send().promise.await?;
            let health = ServiceHealth::from_capnp(response.get()?.get_health()?)?;
            health
                .require_protocol_version(ProtocolVersion::CURRENT)
                .map_err(|error| CliError::DaemonAuthorityMismatch {
                    operation: "health",
                    reason: format!("azd unavailable until restarted: {error}"),
                })?;
            future(client).await
        };
        match deadline {
            Some(deadline) => {
                timeout(deadline, rpc)
                    .await
                    .map_err(|_| CliError::DaemonRpcTimedOut {
                        operation,
                        endpoint: endpoint_label,
                        timeout_ms: u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
                    })?
            }
            None => {
                if let Some(progress_interval) = progress_interval {
                    let started = Instant::now();
                    let mut ticks = interval(progress_interval);
                    ticks.tick().await;
                    tokio::pin!(rpc);
                    loop {
                        tokio::select! {
                            result = &mut rpc => break result,
                            _ = ticks.tick() => {
                                eprintln!(
                                    "{operation}: still waiting on azd at {endpoint_label} ({}s elapsed)",
                                    started.elapsed().as_secs()
                                );
                                let _ = std::io::stderr().flush();
                            }
                        }
                    }
                } else {
                    rpc.await
                }
            }
        }
    })
}

fn probe_daemon(endpoint: &Endpoint) -> CliResult<()> {
    with_daemon_timeout(
        endpoint,
        "probe azd",
        DAEMON_STOP_RPC_TIMEOUT,
        async move |client| {
            let mut request = client.list_projects_request();
            (ListProjectsRequest {
                capability: daemon_capability(DAEMON_READ_PERMISSION),
            })
            .to_capnp(request.get().init_request())?;
            request.send().promise.await?;
            Ok(())
        },
    )
}

fn touch_editor_owner_process(
    endpoint: &Endpoint,
    owner_process: ProcessIdentity,
) -> CliResult<()> {
    let owner_process = ProtoProcessIdentity {
        process_id: owner_process.process_id,
        process_start_time: owner_process.process_start_time,
    };
    let lease_id = editor_process_lease_id(owner_process);
    with_daemon(endpoint, move |client| async move {
        let mut request = client.touch_editor_lease_request();
        (TouchEditorLeaseRequest {
            capability: daemon_capability(DAEMON_LEASE_PERMISSION),
            lease_id: lease_id.clone(),
            owner_process,
            purpose: "azoth editor launcher".to_string(),
        })
        .to_capnp(request.get().init_request())?;
        let response = request.send().promise.await?;
        let result = TouchEditorLeaseResult::from_capnp(response.get()?.get_result()?)?;
        if !result.accepted || result.lease_id != lease_id {
            return Err(CliError::DaemonAuthorityMismatch {
                operation: "touchEditorLease",
                reason: format!(
                    "daemon returned lease `{}` accepted={} for requested lease `{lease_id}`",
                    result.lease_id, result.accepted
                ),
            });
        }
        Ok(())
    })
}

fn print_build_command(command: &ProjectBuildCommand) {
    println!("build {}:{}", command.owner_id, command.target_name);
    println!("  owner_root: {}", command.owner_root);
    println!("  cwd: {}", command.cwd);
    println!(
        "  command: {}",
        shell_command_line(&command.program, &command.args)
    );
}

fn print_build_package_profile(profile: &ProjectBuildPackageProfile) {
    println!("package profile {}", profile.name);
    println!("  asset_platform: {}", profile.asset_platform);
    println!("  cargo_profile: {}", profile.cargo_profile);
    println!("  container: {}", profile.container);
    println!("  compression: {}", profile.compression);
    if let Some(compressor) = &profile.oodle_compressor {
        println!("  oodle_compressor: {compressor}");
    }
    if let Some(effort) = &profile.oodle_effort {
        println!("  oodle_effort: {effort}");
    }
}

fn print_service_plan(plan: &ProjectServicePlan) {
    if plan.build_commands.is_empty() {
        println!("No service build commands planned");
    } else {
        println!("Build commands:");
        for command in &plan.build_commands {
            print_build_command(command);
        }
    }

    if plan.commands.is_empty() {
        println!("No service launch commands planned");
    } else {
        println!("Service commands:");
        for command in &plan.commands {
            print_service_command(command);
        }
    }
}

fn print_service_command(command: &ProjectServiceCommand) {
    println!(
        "service {}:{} {:?}",
        command.owner_id, command.service_name, command.role
    );
    println!("  owner_root: {}", command.owner_root);
    println!(
        "  endpoint: {:?} {}",
        command.endpoint.kind, command.endpoint.address
    );
    println!("  cwd: {}", command.cwd);
    println!(
        "  command: {}",
        shell_command_line(&command.program, &command.args)
    );
}

fn print_session_supervisor_descriptor(supervisor: &SessionSupervisorDescriptor) {
    println!("session {}", supervisor.session_slug);
    print_service_descriptor(&supervisor.descriptor);
}

fn print_service_descriptor(descriptor: &ServiceDescriptor) {
    println!(
        "  service: {}.{} {:?} run {}",
        descriptor.id.namespace, descriptor.id.name, descriptor.role, descriptor.run
    );
    println!(
        "  endpoint: {:?} {}",
        descriptor.endpoint.kind, descriptor.endpoint.address
    );
}

fn shell_command_line(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':' | '\\'))
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

pub const fn default_service_endpoint_kind() -> EndpointKind {
    if cfg!(windows) {
        EndpointKind::WindowsNamedPipe
    } else {
        EndpointKind::UnixDomainSocket
    }
}

#[cfg(test)]
mod tests {
    use az_daemon::{
        AzDaemon, start_az_daemon_rpc_server_with_daemon,
        start_az_daemon_rpc_server_with_daemon_and_shutdown,
    };
    use az_proto_daemon::ProjectRecord;
    use uuid::Uuid;

    use super::*;

    fn test_run(value: u8) -> Uuid {
        Uuid::from_bytes([value; 16])
    }

    fn write_endpoint_test_manifest(root: &Path) {
        std::fs::write(
            root.join("azoth.toml"),
            "[project]\nid = \"local.cli_endpoint_test\"\n",
        )
        .unwrap();
    }

    #[test]
    fn remote_application_failure_does_not_mark_daemon_endpoint_stale() {
        let error = CliError::ServiceProtocol(capnp::Error::failed(
            "project manifest rejected".to_string(),
        ));

        assert!(!is_daemon_connection_failure(&error));
    }

    #[test]
    fn disconnected_service_protocol_marks_daemon_endpoint_stale() {
        let error = CliError::ServiceProtocol(capnp::Error::disconnected(
            "daemon connection closed".to_string(),
        ));

        assert!(is_daemon_connection_failure(&error));
    }

    #[test]
    fn daemon_args_forward_explicit_endpoint_and_project_roots() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let project_registry = temp.path().join("azd.projects.toml");
        let expected_root = project_root.to_string_lossy().into_owned();
        let expected_registry = project_registry.to_string_lossy().into_owned();

        let args = daemon_args(
            Some(EndpointKind::Tcp),
            Some("127.0.0.1:37612"),
            &[project_root],
            Some(&project_registry),
            DaemonLaunchOptions::default(),
        )
        .unwrap();

        assert_eq!(
            args,
            vec![
                "--endpoint-kind",
                "tcp",
                "--endpoint",
                "127.0.0.1:37612",
                "--project",
                expected_root.as_str(),
                "--project-registry",
                expected_registry.as_str(),
            ]
        );
    }

    #[test]
    fn daemon_args_reject_in_process_endpoint_kind() {
        let error = daemon_args(
            Some(EndpointKind::InProcess),
            None,
            &[],
            None,
            DaemonLaunchOptions::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            CliError::UnsupportedEndpointKind {
                operation: "azd launch",
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn explicit_daemon_endpoint_rejects_in_process_kind() {
        let error = daemon_endpoint(Some(EndpointKind::InProcess), Some("azd:test".to_string()))
            .unwrap_err();

        assert!(matches!(
            error,
            CliError::UnsupportedEndpointKind {
                operation: "azd endpoint",
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn daemon_launch_command_uses_separate_azd_binary_target() {
        let daemon_executable = std::env::temp_dir().join("azoth/bin/azd.exe");
        let command = daemon_launch_command(
            &daemon_executable,
            Some(EndpointKind::Tcp),
            Some("127.0.0.1:37612"),
            &[],
            None,
            DaemonLaunchOptions::default(),
        )
        .expect("build daemon launch command");

        assert_eq!(command.program, daemon_executable.to_string_lossy());
        assert_eq!(
            command.args,
            ["--endpoint-kind", "tcp", "--endpoint", "127.0.0.1:37612"]
        );
        assert!(!command.args.iter().any(|arg| arg == "run"));
    }

    #[test]
    fn source_daemon_launch_carries_runtime_engine_root() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(az_project::ENGINE_MANIFEST_FILE),
            "[engine]\n",
        )
        .unwrap();
        let daemon_executable = temp.path().join("target/debug/azd.exe");
        let command = daemon_launch_command(
            &daemon_executable,
            None,
            None,
            &[],
            None,
            DaemonLaunchOptions::default(),
        )
        .expect("build daemon launch command");

        assert_eq!(command.engine_root.as_deref(), Some(temp.path()));
    }

    #[test]
    fn daemon_launch_command_passes_absolute_project_roots_to_child_process() {
        let command = daemon_launch_command(
            &std::env::temp_dir().join("azoth/bin/azd.exe"),
            None,
            None,
            &[PathBuf::from(".")],
            None,
            DaemonLaunchOptions::default(),
        )
        .unwrap();
        let root_index = command
            .args
            .iter()
            .position(|arg| arg == "--project")
            .expect("--project arg exists");

        assert!(Path::new(&command.args[root_index + 1]).is_absolute());
    }

    #[test]
    fn daemon_launch_command_passes_absolute_project_registry_to_child_process() {
        let command = daemon_launch_command(
            &std::env::temp_dir().join("azoth/bin/azd.exe"),
            None,
            None,
            &[],
            Some(Path::new("azd.projects.toml")),
            DaemonLaunchOptions::default(),
        )
        .unwrap();
        let registry_index = command
            .args
            .iter()
            .position(|arg| arg == "--project-registry")
            .expect("project-registry arg exists");

        assert!(Path::new(&command.args[registry_index + 1]).is_absolute());
    }

    #[test]
    fn daemon_launch_command_supports_editor_owner_lease_shutdown() {
        let command = daemon_launch_command(
            &std::env::temp_dir().join("azoth/bin/azd.exe"),
            None,
            None,
            &[],
            None,
            DaemonLaunchOptions {
                editor_owner_process: Some(ProcessIdentity {
                    process_id: 1234,
                    process_start_time: 9_001,
                }),
                shutdown_when_editor_leases_gone: true,
            },
        )
        .unwrap();

        assert!(
            command
                .args
                .windows(2)
                .any(|args| args[0] == "--editor-owner-process" && args[1] == "1234:9001")
        );
        assert!(
            command
                .args
                .iter()
                .any(|arg| arg == "--shutdown-when-editor-leases-gone")
        );
    }

    #[test]
    fn explicit_optional_daemon_endpoint_reports_source() {
        let endpoint = optional_daemon_endpoint_with_source(Some(EndpointKind::Tcp), None)
            .unwrap()
            .unwrap();

        assert_eq!(endpoint.source, DaemonEndpointSource::Explicit);
        assert_eq!(
            endpoint.endpoint,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612")
        );
    }

    #[test]
    fn stop_daemon_routes_control_shutdown_through_azd_rpc() {
        let temp = tempfile::tempdir().unwrap();
        let shutdown = az_work::CancellationToken::new();
        let server = start_az_daemon_rpc_server_with_daemon_and_shutdown(
            AzDaemon::with_data_home(AzothDataHome::new(temp.path().join("azoth-home"))).unwrap(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
            shutdown.clone(),
            test_run(1),
        )
        .unwrap();

        let result =
            stop_daemon_through_daemon(server.endpoint(), "cli requested stop".to_string())
                .unwrap();

        assert!(result.accepted);
        assert_eq!(result.reason, "cli requested stop");
        assert!(shutdown.is_cancelled());
        server.stop();
    }

    #[test]
    fn project_daemon_stop_uses_project_endpoint_record() {
        let temp = tempfile::tempdir().unwrap();
        write_endpoint_test_manifest(temp.path());
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39123");
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));
        let _record = az_endpoint_discovery::write_project_daemon_endpoint_record_in(
            &data_home,
            temp.path(),
            &endpoint,
        )
        .unwrap();

        let resolved = project_daemon_endpoint_for_stop_in(&data_home, None, temp.path()).unwrap();

        assert_eq!(resolved, endpoint);
    }

    #[test]
    fn project_daemon_stop_without_record_uses_deterministic_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        write_endpoint_test_manifest(temp.path());
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));

        let resolved = project_daemon_endpoint_for_stop_in(&data_home, None, temp.path()).unwrap();
        let expected =
            project_daemon_endpoint(default_daemon_endpoint_kind(), temp.path()).unwrap();

        assert_eq!(resolved, expected);
    }

    #[test]
    fn stop_reason_defaults_blank_values() {
        assert_eq!(stop_reason(None), "azoth daemon stop");
        assert_eq!(stop_reason(Some("  ".to_string())), "azoth daemon stop");
        assert_eq!(stop_reason(Some("maintenance".to_string())), "maintenance");
    }

    fn valid_project_record_for_root(root: &Path) -> ProjectRecord {
        ProjectRecord {
            project_id: "local.cli_daemon".to_string(),
            name: "CLI Daemon".to_string(),
            root: root.to_string_lossy().into_owned(),
            manifest_path: root.join("azoth.toml").to_string_lossy().into_owned(),
            engine_version: "0.1.0".to_string(),
        }
    }

    fn valid_build_command(root: &Path) -> ProjectBuildCommand {
        ProjectBuildCommand {
            owner_id: "local.cli_daemon".to_string(),
            owner_root: root.to_string_lossy().into_owned(),
            target_name: "game".to_string(),
            program: "cargo".to_string(),
            cwd: root.to_string_lossy().into_owned(),
            args: vec!["build".to_string()],
            cargo_target_dir: None,
        }
    }

    fn valid_package_profile() -> ProjectBuildPackageProfile {
        ProjectBuildPackageProfile {
            name: "pc-release".to_string(),
            asset_platform: "pc".to_string(),
            cargo_profile: "release".to_string(),
            container: "azpack".to_string(),
            compression: "oodle".to_string(),
            oodle_compressor: Some("kraken".to_string()),
            oodle_effort: Some("normal".to_string()),
        }
    }

    fn valid_service_command(root: &Path, service_name: &str) -> ProjectServiceCommand {
        ProjectServiceCommand {
            owner_id: "local.cli_daemon".to_string(),
            owner_root: root.to_string_lossy().into_owned(),
            build_output_root: root.join("target").to_string_lossy().into_owned(),
            service_name: service_name.to_string(),
            role: ServiceRole::ProjectHost,
            endpoint: Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39020"),
            program: root
                .join("target")
                .join("debug")
                .join(service_name)
                .to_string_lossy()
                .into_owned(),
            cwd: root.to_string_lossy().into_owned(),
            args: vec!["--service".to_string(), service_name.to_string()],
        }
    }

    fn assert_daemon_authority_mismatch(
        error: CliError,
        expected_operation: &'static str,
        expected_reason: &str,
    ) {
        match error {
            CliError::DaemonAuthorityMismatch { operation, reason } => {
                assert_eq!(operation, expected_operation);
                assert!(
                    reason.contains(expected_reason),
                    "expected reason containing `{expected_reason}`, got `{reason}`"
                );
            }
            other => panic!("expected daemon authority mismatch, got {other:?}"),
        }
    }

    #[test]
    fn daemon_project_record_must_echo_requested_project_id() {
        let temp = tempfile::tempdir().unwrap();
        let mut project = valid_project_record_for_root(temp.path());
        project.project_id = "local.other".to_string();

        let error = ensure_daemon_project_record_matches_request(
            &project,
            Some("local.cli_daemon"),
            None,
            "resolveProject",
        )
        .unwrap_err();

        assert_daemon_authority_mismatch(error, "resolveProject", "expected `local.cli_daemon`");
    }

    #[test]
    fn daemon_project_record_must_echo_requested_root() {
        let temp = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let project = valid_project_record_for_root(temp.path());

        let error = ensure_daemon_project_record_matches_request(
            &project,
            None,
            Some(other.path()),
            "registerProjectRoot",
        )
        .unwrap_err();

        assert_daemon_authority_mismatch(error, "registerProjectRoot", "requested root");
    }

    #[test]
    fn daemon_project_list_rejects_protocol_skew_and_duplicate_project_ids() {
        let temp = tempfile::tempdir().unwrap();
        let project = valid_project_record_for_root(temp.path());
        let mut result = ListProjectsResult {
            protocol_version: ProtocolVersion::CURRENT,
            projects: vec![project.clone(), project],
        };

        let error = ensure_daemon_project_list_is_authoritative(&result).unwrap_err();
        assert_daemon_authority_mismatch(error, "listProjects", "duplicate project");

        result.protocol_version = ProtocolVersion {
            major: 0,
            minor: 1,
            patch: 0,
        };
        let error = ensure_daemon_project_list_is_authoritative(&result).unwrap_err();
        assert_daemon_authority_mismatch(error, "listProjects", "unavailable until restarted");
    }

    #[test]
    fn daemon_session_supervisor_list_rejects_duplicate_slugs() {
        let descriptor = az_service_catalog::session_supervisor_service_descriptor(
            az_session::SessionId::new().0,
            test_run(4),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39010"),
        );
        let supervisors = vec![
            SessionSupervisorDescriptor {
                session_slug: "editor".to_string(),
                descriptor: descriptor.clone(),
            },
            SessionSupervisorDescriptor {
                session_slug: "editor".to_string(),
                descriptor,
            },
        ];

        let error =
            ensure_daemon_session_supervisor_list_is_authoritative(&supervisors).unwrap_err();

        assert_daemon_authority_mismatch(error, "listSessionSupervisors", "duplicate");
    }

    #[test]
    fn daemon_session_supervisor_descriptor_must_be_live_authority() {
        let descriptor = az_service_catalog::session_supervisor_service_descriptor(
            az_session::SessionId::new().0,
            Uuid::nil(),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39011"),
        );

        let error = ensure_daemon_session_supervisor_descriptor_matches_request(
            &descriptor,
            "editor",
            "resolveSessionSupervisor",
        )
        .unwrap_err();

        assert_daemon_authority_mismatch(error, "resolveSessionSupervisor", "run must not be nil");
    }

    #[test]
    fn daemon_project_build_plan_must_be_traceable() {
        let temp = tempfile::tempdir().unwrap();
        let mut command = valid_build_command(temp.path());
        command.args.clear();
        let plan = ProjectBuildPlan {
            commands: vec![command],
            package_profile: None,
        };

        let error = ensure_daemon_project_build_plan_matches_request(&plan, "local.cli_daemon")
            .unwrap_err();

        assert_daemon_authority_mismatch(error, "planProjectBuild", "without args");
    }

    #[test]
    fn daemon_project_build_plan_rejects_unsupported_package_compression() {
        let temp = tempfile::tempdir().unwrap();
        let mut package_profile = valid_package_profile();
        package_profile.compression = "brotli".to_string();
        package_profile.oodle_compressor = None;
        package_profile.oodle_effort = None;
        let plan = ProjectBuildPlan {
            commands: vec![valid_build_command(temp.path())],
            package_profile: Some(package_profile),
        };

        let error = ensure_daemon_project_build_plan_matches_request(&plan, "local.cli_daemon")
            .unwrap_err();

        assert_daemon_authority_mismatch(error, "planProjectBuild", "unsupported compression");
    }

    #[test]
    fn daemon_project_build_plan_rejects_incoherent_oodle_package_policy() {
        let temp = tempfile::tempdir().unwrap();
        let mut package_profile = valid_package_profile();
        package_profile.compression = "none".to_string();
        let plan = ProjectBuildPlan {
            commands: vec![valid_build_command(temp.path())],
            package_profile: Some(package_profile),
        };

        let error = ensure_daemon_project_build_plan_matches_request(&plan, "local.cli_daemon")
            .unwrap_err();

        assert_daemon_authority_mismatch(
            error,
            "planProjectBuild",
            "oodle settings but non-oodle compression",
        );
    }

    #[test]
    fn daemon_project_build_plan_rejects_unimplemented_package_backend_policy() {
        let temp = tempfile::tempdir().unwrap();
        let mut package_profile = valid_package_profile();
        package_profile.container = "loose".to_string();
        let plan = ProjectBuildPlan {
            commands: vec![valid_build_command(temp.path())],
            package_profile: Some(package_profile),
        };

        let error = ensure_daemon_project_build_plan_matches_request(&plan, "local.cli_daemon")
            .unwrap_err();

        assert_daemon_authority_mismatch(
            error,
            "planProjectBuild",
            "unsupported package backend policy",
        );
    }

    #[test]
    fn daemon_project_service_plan_must_be_traceable() {
        let temp = tempfile::tempdir().unwrap();
        let mut command = valid_service_command(temp.path(), "project-host");
        command.program = "project-host".to_string();
        let plan = ProjectServicePlan {
            build_commands: vec![valid_build_command(temp.path())],
            commands: vec![command],
        };

        let error =
            ensure_daemon_project_service_plan_matches_request(&plan, "local.cli_daemon", "editor")
                .unwrap_err();

        assert_daemon_authority_mismatch(error, "planProjectServices", "non-absolute");
    }

    #[test]
    fn daemon_project_service_plan_rejects_duplicate_services() {
        let temp = tempfile::tempdir().unwrap();
        let plan = ProjectServicePlan {
            build_commands: Vec::new(),
            commands: vec![
                valid_service_command(temp.path(), "project-host"),
                valid_service_command(temp.path(), "project-host"),
            ],
        };

        let error =
            ensure_daemon_project_service_plan_matches_request(&plan, "local.cli_daemon", "editor")
                .unwrap_err();

        assert_daemon_authority_mismatch(error, "planProjectServices", "duplicate service");
    }

    #[test]
    fn daemon_session_probe_capability_comes_from_descriptor() {
        let session_id = az_session::SessionId::new();
        let descriptor = az_service_catalog::session_supervisor_service_descriptor(
            session_id.0,
            test_run(4),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39012"),
        );

        let capability = daemon_session_read_capability(&descriptor).unwrap();

        assert_eq!(
            capability.service,
            ServiceId::new(DAEMON_SERVICE_NAMESPACE, DAEMON_SERVICE_NAME)
        );
        assert_eq!(capability.role, ServiceRole::Daemon);
        assert_eq!(capability.session, Some(session_id.0));
        assert_eq!(capability.audience, SESSION_SUPERVISOR_AUDIENCE);
        assert!(
            capability
                .permissions
                .iter()
                .any(|permission| permission == SESSION_READ_PERMISSION)
        );
        assert!(!capability.token_hash.is_empty());
    }

    #[test]
    fn daemon_session_probe_rejects_missing_descriptor_grant() {
        let session_id = az_session::SessionId::new();
        let mut descriptor = az_service_catalog::session_supervisor_service_descriptor(
            session_id.0,
            test_run(4),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39013"),
        );
        descriptor
            .capabilities
            .retain(|capability| capability.role == ServiceRole::Editor);

        let error = daemon_session_read_capability(&descriptor).unwrap_err();

        assert!(error.contains("did not grant"));
        assert!(error.contains(SESSION_SUPERVISOR_AUDIENCE));
        assert!(error.contains(DAEMON_SERVICE_NAME));
    }

    #[test]
    fn daemon_session_probe_rejects_wrong_descriptor_identity() {
        let session_id = az_session::SessionId::new();
        let mut descriptor = az_service_catalog::session_supervisor_service_descriptor(
            session_id.0,
            test_run(4),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39014"),
        );
        descriptor.id = ServiceId::new("azoth", "project-host");

        let error = daemon_session_read_capability(&descriptor).unwrap_err();

        assert!(error.contains("expected descriptor"));
        assert!(error.contains("project-host"));
    }

    #[test]
    fn daemon_session_probe_rejects_unbrokered_descriptor_grant() {
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("azoth", "session-supervisor"),
            ServiceRole::SessionSupervisor,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39015"),
        )
        .with_run(test_run(4))
        .with_capability(
            Capability::new(
                ServiceId::new(DAEMON_SERVICE_NAMESPACE, DAEMON_SERVICE_NAME),
                ServiceRole::Daemon,
            )
            .with_audience(SESSION_SUPERVISOR_AUDIENCE)
            .with_permissions([SESSION_READ_PERMISSION]),
        );

        let error = daemon_session_read_capability(&descriptor).unwrap_err();

        assert!(error.contains("invalid brokered capability templates"));
        assert!(error.contains("brokered token hash"));
    }

    #[test]
    fn daemon_session_probe_rejects_expired_descriptor_grant() {
        let session_id = az_session::SessionId::new();
        let mut descriptor = az_service_catalog::session_supervisor_service_descriptor(
            session_id.0,
            test_run(4),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39016"),
        );
        for capability in &mut descriptor.capabilities {
            capability.expires_unix_ms = 1;
        }

        let error = daemon_session_read_capability(&descriptor).unwrap_err();

        assert!(error.contains("invalid brokered capability templates"));
        assert!(error.contains("expired"));
    }

    #[test]
    fn endpoint_match_treats_tcp_port_zero_as_bound_port_wildcard() {
        let requested = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0");
        let bound = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:43210");
        let other_kind = Endpoint::new(EndpointKind::InProcess, "127.0.0.1:43210");

        assert!(endpoint_matches_requested(&bound, &requested));
        assert!(!endpoint_matches_requested(&other_kind, &requested));
    }

    #[test]
    fn daemon_log_path_sits_next_to_runtime_endpoint_record() {
        let temp = tempfile::tempdir().unwrap();
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));
        let log = daemon_log_path_in(&data_home, None).unwrap();
        let record = daemon_endpoint_record_path_in(&data_home).unwrap();

        assert_eq!(log.parent(), record.parent());
        assert_eq!(
            log.file_name().and_then(|name| name.to_str()),
            Some("azd.log")
        );
    }

    #[test]
    fn daemon_restart_retains_exactly_current_and_previous_output_logs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("azd.log");
        let previous = previous_log_path(&path);
        std::fs::write(&path, "first launch").unwrap();
        std::fs::write(&previous, "obsolete launch").unwrap();

        let (stdout, stderr) = open_rotated_daemon_logs(&path).unwrap();
        drop((stdout, stderr));
        assert_eq!(std::fs::read_to_string(&previous).unwrap(), "first launch");
        std::fs::write(&path, "second launch").unwrap();

        let (stdout, stderr) = open_rotated_daemon_logs(&path).unwrap();
        drop((stdout, stderr));

        assert_eq!(std::fs::read_to_string(&previous).unwrap(), "second launch");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    }

    #[test]
    fn daemon_start_endpoint_uses_project_scoped_endpoint_for_project_root() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = daemon_endpoint_from_options(
            Some(EndpointKind::WindowsNamedPipe),
            None,
            Some(temp.path()),
        )
        .unwrap();
        let expected =
            project_daemon_endpoint(EndpointKind::WindowsNamedPipe, temp.path()).unwrap();

        assert_eq!(endpoint, expected);
    }

    #[test]
    fn daemon_log_path_sits_next_to_project_endpoint_record() {
        let temp = tempfile::tempdir().unwrap();
        write_endpoint_test_manifest(temp.path());
        let data_home = AzothDataHome::new(temp.path().join("azoth-home"));
        let log = daemon_log_path_in(&data_home, Some(temp.path())).unwrap();
        let record = project_daemon_endpoint_record_path_in(&data_home, temp.path()).unwrap();

        assert_eq!(log.parent(), record.parent());
        assert_eq!(
            log.file_name().and_then(|name| name.to_str()),
            Some("azd.log")
        );
    }

    #[test]
    fn session_supervisor_registry_can_route_through_azd_rpc() {
        let project = tempfile::tempdir().unwrap();
        let project_root = project.path().to_string_lossy().into_owned();
        let manifest_path = project.path().join("azoth.toml");
        let daemon =
            AzDaemon::with_data_home(AzothDataHome::new(project.path().join("azoth-home")))
                .unwrap();
        daemon
            .register_project(&ProjectRecord {
                project_id: "local.cli_daemon_sessions".to_string(),
                name: "CLI Daemon Sessions".to_string(),
                root: project_root,
                manifest_path: manifest_path.to_string_lossy().into_owned(),
                engine_version: "0.1.0".to_string(),
            })
            .unwrap();
        let descriptor = az_service_catalog::session_supervisor_service_descriptor(
            az_session::SessionId::new().0,
            test_run(7),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39001"),
        );
        daemon
            .register_session_supervisor("local.cli_daemon_sessions", "editor-work", &descriptor)
            .unwrap();
        let server = start_az_daemon_rpc_server_with_daemon(
            daemon,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();

        let supervisors = list_session_supervisors_through_daemon(
            server.endpoint(),
            "local.cli_daemon_sessions".to_string(),
        )
        .unwrap();
        let resolved = resolve_session_supervisor_through_daemon(
            server.endpoint(),
            "local.cli_daemon_sessions".to_string(),
            "editor-work".to_string(),
        )
        .unwrap();

        assert_eq!(supervisors.len(), 1);
        assert_eq!(supervisors[0].session_slug, "editor-work");
        assert_eq!(supervisors[0].descriptor, descriptor);
        assert_eq!(resolved, Some(descriptor));

        let removed = unregister_session_supervisor_through_daemon(
            server.endpoint(),
            "local.cli_daemon_sessions".to_string(),
            "editor-work".to_string(),
        )
        .unwrap();
        let resolved_after_unregister = resolve_session_supervisor_through_daemon(
            server.endpoint(),
            "local.cli_daemon_sessions".to_string(),
            "editor-work".to_string(),
        )
        .unwrap();

        assert!(removed);
        assert_eq!(resolved_after_unregister, None);
        server.stop();
    }

    #[test]
    fn prune_session_supervisors_removes_unreachable_descriptor() {
        let project = tempfile::tempdir().unwrap();
        let project_root = project.path().to_string_lossy().into_owned();
        let manifest_path = project.path().join("azoth.toml");
        let daemon =
            AzDaemon::with_data_home(AzothDataHome::new(project.path().join("azoth-home")))
                .unwrap();
        daemon
            .register_project(&ProjectRecord {
                project_id: "local.cli_daemon_prune".to_string(),
                name: "CLI Daemon Prune".to_string(),
                root: project_root,
                manifest_path: manifest_path.to_string_lossy().into_owned(),
                engine_version: "0.1.0".to_string(),
            })
            .unwrap();
        let descriptor = az_service_catalog::session_supervisor_service_descriptor(
            az_session::SessionId::new().0,
            test_run(1),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:9"),
        );
        daemon
            .register_session_supervisor("local.cli_daemon_prune", "stale", &descriptor)
            .unwrap();
        let server = start_az_daemon_rpc_server_with_daemon(
            daemon,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();

        let report =
            prune_session_supervisors_through_daemon(server.endpoint(), "local.cli_daemon_prune")
                .unwrap();
        let resolved = resolve_session_supervisor_through_daemon(
            server.endpoint(),
            "local.cli_daemon_prune".to_string(),
            "stale".to_string(),
        )
        .unwrap();

        assert_eq!(report.checked, 1);
        assert_eq!(report.removed, 1);
        assert_eq!(report.kept, 0);
        assert_eq!(resolved, None);
        server.stop();
    }
}
