use crate::error::{CliError, CliResult, CommandFailedDetails, SessionServiceNotRunningDetails};
use az_proto_asset::{
    ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME, ASSET_WORKER_SERVICE_NAME,
    ASSET_WORKER_SERVICE_NAMESPACE,
};
use az_proto_core::{Endpoint, EndpointKind, ServiceRole};
use az_proto_project::{PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME};
use az_proto_session::{
    ServiceProcessRecord as ProtoServiceProcessRecord,
    ServiceProcessState as ProtoServiceProcessState, SessionManifest as ProtoSessionManifest,
    SessionState as ProtoSessionState, SessionWorkspaceStatus as ProtoSessionWorkspaceStatus,
};
#[cfg(test)]
use az_service_supervision::ProcessIdentity;
use az_service_supervision::{
    ServiceProcessRecord, ServiceProcessState, ServiceRecord, SupervisedServiceRole,
};
use az_session::{
    CreateSessionRequest, SessionError, SessionManager, SessionManifest, SessionState,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

const DEFAULT_EDITOR_SESSION: &str = "main";
const EDITOR_ATTACH_OPERATION: &str = "editor attach";
const EDITOR_START_SERVICES_REASON: &str = "azoth editor ensure-services";
const EDITOR_DAEMON_START_TIMEOUT_MS: u64 = 10_000;
const EDITOR_SESSION_SERVICE_START_TIMEOUT_MS: u64 = 60 * 60 * 1_000;

#[derive(Debug, Clone)]
struct EditorSessionResolution {
    slug: String,
    live_manifest: Option<ProtoSessionManifest>,
}

impl EditorSessionResolution {
    const fn local(slug: String) -> Self {
        Self {
            slug,
            live_manifest: None,
        }
    }

    fn live(manifest: ProtoSessionManifest) -> Self {
        Self {
            slug: manifest.slug.clone(),
            live_manifest: Some(manifest),
        }
    }
}

pub fn execute(
    path: Option<PathBuf>,
    session: Option<String>,
    ensure_services: bool,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<String>,
) -> CliResult<()> {
    let Some(project_path) = path else {
        validate_unbound_launcher_args(
            session.as_deref(),
            daemon_endpoint_kind,
            daemon_endpoint.as_deref(),
        )?;
        let command = editor_launcher_launch_command()?;
        info!(
            program = %command.program,
            args = ?command.args,
            "launching az-editor project launcher"
        );
        return run_editor_launch_command(command);
    };

    let project_path = child_project_path(&project_path)?;
    let session = if ensure_services {
        let ensured = ensure_editor_session_services(
            &project_path,
            session.as_deref(),
            daemon_endpoint_kind,
            daemon_endpoint.as_deref(),
        )?;
        Some(ensured)
    } else {
        session
    };
    let ensured_daemon_endpoint = if ensure_services {
        Some(crate::commands::daemon::daemon_endpoint(
            daemon_endpoint_kind,
            daemon_endpoint.clone(),
        )?)
    } else {
        None
    };
    let daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;
    let editor_daemon_endpoint = forwarded_editor_daemon_endpoint(daemon_endpoint.as_ref())
        .or(ensured_daemon_endpoint.as_ref());
    let command = editor_launch_command(&project_path, session.as_deref(), editor_daemon_endpoint)?;

    info!(
        root = %project_path.display(),
        program = %command.program,
        args = ?command.args,
        "launching az-editor"
    );

    run_editor_launch_command(command)
}

fn validate_unbound_launcher_args(
    session: Option<&str>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<&str>,
) -> CliResult<()> {
    if session.is_some() {
        return Err(CliError::InvalidArgument {
            message: "`azoth editor --session <name>` requires `--project <DIR>`; omit both to show the project launcher".to_string(),
        });
    }
    if daemon_endpoint_kind.is_some() || daemon_endpoint.is_some() {
        return Err(CliError::InvalidArgument {
            message: "daemon endpoint options require `--project <DIR>` because daemon attach is project-bound".to_string(),
        });
    }
    Ok(())
}

fn run_editor_launch_command(command: EditorLaunchCommand) -> CliResult<()> {
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

fn ensure_editor_session_services(
    project_path: &Path,
    requested_session: Option<&str>,
    daemon_endpoint_kind: Option<EndpointKind>,
    daemon_endpoint: Option<&str>,
) -> CliResult<String> {
    az_project_scaffold::project_contract::sync_project_contract(project_path)?;
    let project_roots = [project_path.to_path_buf()];
    crate::commands::daemon::start_for_editor(
        &project_roots,
        None,
        EDITOR_DAEMON_START_TIMEOUT_MS,
        daemon_endpoint_kind,
        daemon_endpoint,
    )?;
    crate::commands::daemon::register_project(
        Some(project_path.to_path_buf()),
        daemon_endpoint_kind,
        daemon_endpoint.map(str::to_string),
    )?;
    let live_daemon_endpoint = crate::commands::daemon::optional_daemon_endpoint_with_source(
        daemon_endpoint_kind,
        daemon_endpoint.map(str::to_string),
    )?
    .ok_or(CliError::MissingDaemonEndpoint {
        operation: "editor service readiness",
    })?;

    let resolution = resolve_editor_session_for_services(
        project_path,
        requested_session,
        &live_daemon_endpoint,
    )?;
    let session = resolution.slug.clone();
    if ensure_live_editor_services_from_resolution(&resolution, &live_daemon_endpoint)? {
        return Ok(session);
    }

    let manager = SessionManager::new(project_path)?;
    let manifest = manager.session(&session)?;
    if editor_service_plan_needs_prepare_for_session(&manifest, &live_daemon_endpoint)? {
        crate::commands::session::prepare_services(
            crate::commands::session::PrepareServicesOptions {
                name: session.clone(),
                kind: None,
                recover: false,
                otlp_endpoint: None,
                daemon_endpoint_kind,
                daemon_endpoint: daemon_endpoint.map(str::to_string),
                path: Some(project_path.to_path_buf()),
                service_names: Vec::new(),
            },
        )?;
    }
    crate::commands::session::start_services(crate::commands::session::StartServicesOptions {
        name: session.clone(),
        session_supervisor_kind: None,
        session_supervisor_endpoint: None,
        otlp_endpoint: None,
        timeout_ms: EDITOR_SESSION_SERVICE_START_TIMEOUT_MS,
        daemon_endpoint_kind,
        daemon_endpoint: daemon_endpoint.map(str::to_string),
        path: Some(project_path.to_path_buf()),
        service_names: required_editor_service_names(),
    })?;
    Ok(session)
}

fn resolve_editor_session_for_services(
    project_path: &Path,
    requested_session: Option<&str>,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<EditorSessionResolution> {
    let manager = SessionManager::new(project_path)?;
    resolve_editor_session_for_services_with_manager(
        project_path,
        requested_session,
        daemon_endpoint,
        &manager,
    )
}

fn resolve_editor_session_for_services_with_manager(
    project_path: &Path,
    requested_session: Option<&str>,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
    manager: &SessionManager,
) -> CliResult<EditorSessionResolution> {
    if let Some(session) = requested_session {
        return match crate::commands::session::requested_session_manifest_through_daemon(
            project_path,
            session,
            daemon_endpoint,
            EDITOR_ATTACH_OPERATION,
        ) {
            Ok(Some(manifest)) => Ok(EditorSessionResolution::live(manifest)),
            Ok(None) => ensure_editor_session_with_manager(manager, Some(session))
                .map(EditorSessionResolution::local),
            Err(error) => Err(error),
        };
    }

    match crate::commands::session::active_session_manifest_through_daemon(
        project_path,
        daemon_endpoint,
        EDITOR_ATTACH_OPERATION,
    ) {
        Ok(Some(manifest)) => Ok(EditorSessionResolution::live(manifest)),
        Ok(None) | Err(CliError::NoActiveSession { .. }) => {
            ensure_editor_session_with_manager(manager, None).map(EditorSessionResolution::local)
        }
        Err(error) => Err(error),
    }
}

fn ensure_live_editor_services_from_resolution(
    resolution: &EditorSessionResolution,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<bool> {
    let Some(manifest) = &resolution.live_manifest else {
        return Ok(false);
    };

    match crate::commands::session::live_session_status_for_proto_manifest_through_daemon(
        manifest,
        daemon_endpoint,
    ) {
        Ok(status) => {
            ensure_proto_editor_session_active(&status.manifest)?;
            if editor_service_status_needs_prepare(&status) {
                return Ok(false);
            }
            if first_unready_editor_service_status(&status).is_none() {
                return Ok(true);
            }

            let result =
                crate::commands::session::start_services_for_proto_manifest_through_daemon(
                    manifest,
                    daemon_endpoint,
                    EDITOR_START_SERVICES_REASON,
                    required_editor_service_names(),
                )?;
            ensure_proto_editor_session_active(&result.status.manifest)?;
            if let Some((service, state)) = first_unready_editor_service_status(&result.status) {
                return Err(CliError::SessionServiceNotRunning(Box::new(
                    SessionServiceNotRunningDetails {
                        session: result.status.manifest.slug,
                        service: service.name.to_string(),
                        state: state.label().to_string(),
                    },
                )));
            }
            Ok(true)
        }
        Err(CliError::MissingSessionService { .. }) => Ok(false),
        Err(error) => {
            info!(
                session = %manifest.slug,
                error = %error,
                "live supervisor status was unavailable while checking editor service readiness; falling back to local session metadata"
            );
            Ok(false)
        }
    }
}

fn ensure_editor_session_with_manager(
    manager: &SessionManager,
    requested_session: Option<&str>,
) -> CliResult<String> {
    if let Some(session) = requested_session {
        return match manager.session(session) {
            Ok(manifest) => ensure_editor_session_active_or_recover(manager, manifest),
            Err(SessionError::SessionNotFound(_)) => {
                println!("creating editor session '{session}'");
                Ok(manager
                    .create_session(CreateSessionRequest::new(session))?
                    .slug)
            }
            Err(error) => Err(error.into()),
        };
    }

    let active = manager
        .list_sessions()?
        .into_iter()
        .filter(|session| session.state == SessionState::Active)
        .collect::<Vec<_>>();
    match active.as_slice() {
        [session] => Ok(session.slug.clone()),
        [] => match manager.session(DEFAULT_EDITOR_SESSION) {
            Ok(manifest) => ensure_editor_session_active_or_recover(manager, manifest),
            Err(SessionError::SessionNotFound(_)) => {
                println!("creating default editor session '{DEFAULT_EDITOR_SESSION}'");
                Ok(manager
                    .create_session(CreateSessionRequest::new(DEFAULT_EDITOR_SESSION))?
                    .slug)
            }
            Err(error) => Err(error.into()),
        },
        sessions => Err(CliError::AmbiguousActiveSessions {
            operation: "editor attach",
            sessions: sessions
                .iter()
                .map(|session| session.slug.clone())
                .collect(),
        }),
    }
}

fn ensure_editor_session_active_or_recover(
    manager: &SessionManager,
    manifest: SessionManifest,
) -> CliResult<String> {
    match manifest.state {
        SessionState::Active => Ok(manifest.slug),
        SessionState::FailedPreserved => {
            println!("recovering editor session '{}'", manifest.slug);
            Ok(manager.recover_session(&manifest.slug, true)?.slug)
        }
        state => Err(CliError::SessionNotActive {
            session: manifest.slug,
            state: session_state_label(state).to_string(),
        }),
    }
}

// The strict form of the editor session-state check. Production attaches through
// `ensure_editor_session_active_or_recover`, which recovers a failed-preserved session instead
// of rejecting it, so this variant exists only for the rejection test below.
#[cfg(test)]
fn ensure_editor_session_active(manifest: &SessionManifest) -> CliResult<()> {
    if manifest.state == SessionState::Active {
        Ok(())
    } else {
        Err(CliError::SessionNotActive {
            session: manifest.slug.clone(),
            state: session_state_label(manifest.state).to_string(),
        })
    }
}

fn editor_service_plan_needs_prepare_for_session(
    manifest: &SessionManifest,
    daemon_endpoint: &crate::commands::daemon::OptionalDaemonEndpoint,
) -> CliResult<bool> {
    match crate::commands::session::live_session_status_through_daemon(manifest, daemon_endpoint) {
        Ok(status) => {
            ensure_proto_editor_session_active(&status.manifest)?;
            Ok(editor_service_status_needs_prepare(&status))
        }
        Err(CliError::MissingSessionService { .. }) => {
            Ok(editor_service_plan_needs_prepare(manifest))
        }
        Err(error) => {
            info!(
                session = %manifest.slug,
                error = %error,
                "live supervisor status was unavailable while deciding editor service plan; falling back to local session metadata"
            );
            Ok(editor_service_plan_needs_prepare(manifest))
        }
    }
}

fn editor_service_plan_needs_prepare(manifest: &SessionManifest) -> bool {
    required_editor_services().iter().any(|service| {
        let Some(descriptor) = editor_service_descriptor(manifest, *service) else {
            return true;
        };
        let Some(process) = current_editor_service_process(manifest, *service) else {
            return true;
        };
        process.state == ServiceProcessState::Failed
            || process.endpoint_kind != descriptor.endpoint_kind
            || process.endpoint_address != descriptor.endpoint_address
    })
}

fn editor_service_status_needs_prepare(status: &ProtoSessionWorkspaceStatus) -> bool {
    required_editor_services().iter().any(|service| {
        let Some(descriptor) = status.manifest.services.iter().find(|descriptor| {
            descriptor.id.namespace == service.namespace
                && descriptor.id.name == service.name
                && descriptor.role == service.proto_role()
        }) else {
            return true;
        };
        let Some(process) = current_proto_editor_service_process(&status.manifest, *service) else {
            return true;
        };
        process.state == ProtoServiceProcessState::Failed || process.endpoint != descriptor.endpoint
    })
}

fn ensure_proto_editor_session_active(manifest: &ProtoSessionManifest) -> CliResult<()> {
    if manifest.state == ProtoSessionState::Active {
        Ok(())
    } else {
        Err(CliError::SessionNotActive {
            session: manifest.slug.clone(),
            state: proto_session_state_label(manifest.state).to_string(),
        })
    }
}

const fn proto_session_state_label(state: ProtoSessionState) -> &'static str {
    match state {
        ProtoSessionState::Preparing => "preparing",
        ProtoSessionState::Active => "active",
        ProtoSessionState::FailedPreserved => "failed-preserved",
        ProtoSessionState::Removed => "removed",
    }
}

#[cfg(test)]
fn first_unready_editor_service(
    manifest: &SessionManifest,
) -> Option<(RequiredEditorService, EditorServiceReadiness)> {
    required_editor_services().into_iter().find_map(|service| {
        let Some(descriptor) = editor_service_descriptor(manifest, service) else {
            return Some((service, EditorServiceReadiness::MissingDescriptor));
        };

        let Some(process) = current_editor_service_process(manifest, service) else {
            return Some((service, EditorServiceReadiness::MissingProcess));
        };
        if process.endpoint_kind != descriptor.endpoint_kind
            || process.endpoint_address != descriptor.endpoint_address
        {
            return Some((service, EditorServiceReadiness::EndpointMismatch));
        }

        match process.state {
            ServiceProcessState::Running => None,
            ServiceProcessState::Planned => Some((service, EditorServiceReadiness::Planned)),
            ServiceProcessState::Starting => Some((service, EditorServiceReadiness::Starting)),
            ServiceProcessState::Exited => Some((service, EditorServiceReadiness::Exited)),
            ServiceProcessState::Failed => Some((service, EditorServiceReadiness::Failed)),
        }
    })
}

fn first_unready_editor_service_status(
    status: &ProtoSessionWorkspaceStatus,
) -> Option<(RequiredEditorService, EditorServiceReadiness)> {
    required_editor_services().into_iter().find_map(|service| {
        let Some(descriptor) = status.manifest.services.iter().find(|descriptor| {
            descriptor.id.namespace == service.namespace
                && descriptor.id.name == service.name
                && descriptor.role == service.proto_role()
        }) else {
            return Some((service, EditorServiceReadiness::MissingDescriptor));
        };

        let Some(process) = current_proto_editor_service_process(&status.manifest, service) else {
            return Some((service, EditorServiceReadiness::MissingProcess));
        };
        if process.endpoint != descriptor.endpoint {
            return Some((service, EditorServiceReadiness::EndpointMismatch));
        }

        match process.state {
            ProtoServiceProcessState::Running => None,
            ProtoServiceProcessState::Planned => Some((service, EditorServiceReadiness::Planned)),
            ProtoServiceProcessState::Starting => Some((service, EditorServiceReadiness::Starting)),
            ProtoServiceProcessState::Exited => Some((service, EditorServiceReadiness::Exited)),
            ProtoServiceProcessState::Failed => Some((service, EditorServiceReadiness::Failed)),
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorServiceReadiness {
    MissingDescriptor,
    MissingProcess,
    EndpointMismatch,
    Planned,
    Starting,
    Exited,
    Failed,
}

impl EditorServiceReadiness {
    const fn label(self) -> &'static str {
        match self {
            Self::MissingDescriptor => "missing descriptor",
            Self::MissingProcess => "missing process",
            Self::EndpointMismatch => "endpoint mismatch",
            Self::Planned => "planned",
            Self::Starting => "starting",
            Self::Exited => "exited",
            Self::Failed => "failed",
        }
    }

    #[cfg(test)]
    const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::MissingDescriptor | Self::MissingProcess | Self::EndpointMismatch | Self::Failed
        )
    }
}

fn editor_service_descriptor(
    manifest: &SessionManifest,
    service: RequiredEditorService,
) -> Option<&ServiceRecord> {
    manifest
        .services
        .iter()
        .find(|record| record.name == service.name && record.role == service.role)
}

// Clippy reads `process.service_name == service.name && process.role == service.role` as a
// copy-paste slip and suggests `service.service_name`, a field `RequiredEditorService` does not
// have; the comparison is correct as written and the suggestion would not compile.
#[allow(clippy::suspicious_operation_groupings)]
fn current_editor_service_process(
    manifest: &SessionManifest,
    service: RequiredEditorService,
) -> Option<&ServiceProcessRecord> {
    manifest
        .processes
        .iter()
        .find(|process| process.service_name == service.name && process.role == service.role)
}

fn current_proto_editor_service_process(
    manifest: &ProtoSessionManifest,
    service: RequiredEditorService,
) -> Option<&ProtoServiceProcessRecord> {
    manifest.processes.iter().find(|process| {
        process.service_name == service.name && process.role == service.proto_role()
    })
}

#[derive(Debug, Clone, Copy)]
struct RequiredEditorService {
    namespace: &'static str,
    name: &'static str,
    role: SupervisedServiceRole,
}

impl RequiredEditorService {
    const fn proto_role(self) -> ServiceRole {
        match self.role {
            SupervisedServiceRole::ProjectHost => ServiceRole::ProjectHost,
            SupervisedServiceRole::AssetProcessor => ServiceRole::AssetProcessor,
            SupervisedServiceRole::Worker => ServiceRole::Worker,
            _ => ServiceRole::Unknown,
        }
    }
}

const fn required_editor_services() -> [RequiredEditorService; 3] {
    [
        RequiredEditorService {
            namespace: PROJECT_HOST_NAMESPACE,
            name: PROJECT_HOST_SERVICE_NAME,
            role: SupervisedServiceRole::ProjectHost,
        },
        RequiredEditorService {
            namespace: ASSET_PROCESSOR_NAMESPACE,
            name: ASSET_PROCESSOR_SERVICE_NAME,
            role: SupervisedServiceRole::AssetProcessor,
        },
        RequiredEditorService {
            namespace: ASSET_WORKER_SERVICE_NAMESPACE,
            name: ASSET_WORKER_SERVICE_NAME,
            role: SupervisedServiceRole::Worker,
        },
    ]
}

fn required_editor_service_names() -> Vec<String> {
    required_editor_services()
        .into_iter()
        .map(|service| service.name.to_string())
        .collect()
}

const fn session_state_label(state: SessionState) -> &'static str {
    match state {
        SessionState::Preparing => "preparing",
        SessionState::Active => "active",
        SessionState::FailedPreserved => "failed-preserved",
        SessionState::Removed => "removed",
    }
}

fn forwarded_editor_daemon_endpoint(
    daemon_endpoint: Option<&crate::commands::daemon::OptionalDaemonEndpoint>,
) -> Option<&Endpoint> {
    daemon_endpoint
        .filter(|resolved| {
            resolved.source == crate::commands::daemon::DaemonEndpointSource::Explicit
        })
        .map(|resolved| &resolved.endpoint)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorLaunchCommand {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
}

fn editor_launch_command(
    project_path: &Path,
    session: Option<&str>,
    daemon_endpoint: Option<&Endpoint>,
) -> CliResult<EditorLaunchCommand> {
    let project_path = child_project_path(project_path)?;
    let editor_args = editor_args(&project_path, session, daemon_endpoint)?;
    editor_process_command(editor_args, project_path)
}

fn editor_launcher_launch_command() -> CliResult<EditorLaunchCommand> {
    editor_process_command(Vec::new(), std::env::current_dir()?)
}

fn editor_process_command(
    editor_args: Vec<String>,
    sibling_cwd: PathBuf,
) -> CliResult<EditorLaunchCommand> {
    if let Some(editor) = sibling_editor_executable()? {
        return Ok(EditorLaunchCommand {
            program: editor.to_string_lossy().into_owned(),
            args: editor_args,
            cwd: sibling_cwd,
        });
    }

    let mut args = vec![
        "run".to_string(),
        "-p".to_string(),
        "az-editor".to_string(),
        "--bin".to_string(),
        "az-editor".to_string(),
        "--".to_string(),
    ];
    args.extend(editor_args);
    Ok(EditorLaunchCommand {
        program: "cargo".to_string(),
        args,
        cwd: workspace_root(),
    })
}

fn child_project_path(project_path: &Path) -> CliResult<PathBuf> {
    if project_path.is_absolute() {
        Ok(project_path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(project_path))
    }
}

fn editor_args(
    project_path: &Path,
    session: Option<&str>,
    daemon_endpoint: Option<&Endpoint>,
) -> CliResult<Vec<String>> {
    let mut args = vec![
        "--project".to_string(),
        project_path.to_string_lossy().into_owned(),
    ];
    if let Some(session) = session {
        args.extend(["--session".to_string(), session.to_string()]);
    }
    if let Some(endpoint) = daemon_endpoint {
        args.extend([
            "--daemon-endpoint-kind".to_string(),
            endpoint_kind_arg(endpoint.kind)?.to_string(),
            "--daemon-endpoint".to_string(),
            endpoint.address.clone(),
        ]);
    }
    Ok(args)
}

const fn endpoint_kind_arg(kind: EndpointKind) -> CliResult<&'static str> {
    match kind {
        EndpointKind::WindowsNamedPipe => Ok("windows-named-pipe"),
        EndpointKind::UnixDomainSocket => Ok("unix-domain-socket"),
        EndpointKind::Tcp => Ok("tcp"),
        EndpointKind::InProcess => Err(CliError::UnsupportedEditorDaemonEndpoint { kind }),
    }
}

fn sibling_editor_executable() -> CliResult<Option<PathBuf>> {
    let current = std::env::current_exe()?;
    let Some(dir) = current.parent() else {
        return Ok(None);
    };
    let candidate = dir.join(editor_executable_name());
    Ok(candidate.is_file().then_some(candidate))
}

const fn editor_executable_name() -> &'static str {
    if cfg!(windows) {
        "az-editor.exe"
    } else {
        "az-editor"
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_architecture_guard::production_source_without_cfg_test_modules;
    use az_service_catalog::{
        asset_processor_service_descriptor, asset_worker_service_descriptor,
        project_host_service_descriptor,
    };
    use uuid::Uuid;

    fn test_run(value: u8) -> Uuid {
        Uuid::from_bytes([value; 16])
    }
    use az_session::{SESSION_MANIFEST_FILE, SessionId};

    #[test]
    fn editor_args_target_project_path_and_session() {
        let args = editor_args(Path::new("projects/example"), Some("lighting"), None).unwrap();

        assert_eq!(
            args,
            vec!["--project", "projects/example", "--session", "lighting"]
        );
    }

    #[test]
    fn fallback_launch_uses_separate_az_editor_binary_target() {
        let project_path = std::env::temp_dir().join("azoth-editor-example");
        let project_path_arg = project_path.to_string_lossy().into_owned();
        let command = editor_launch_command(&project_path, Some("lighting"), None)
            .expect("build fallback command");

        if command.program == "cargo" {
            assert_eq!(
                command.args,
                vec![
                    "run",
                    "-p",
                    "az-editor",
                    "--bin",
                    "az-editor",
                    "--",
                    "--project",
                    &project_path_arg,
                    "--session",
                    "lighting",
                ]
            );
            assert_eq!(command.cwd, workspace_root());
        } else {
            assert!(command.program.ends_with(editor_executable_name()));
        }
    }

    #[test]
    fn unbound_launcher_passes_no_project_attach_args() {
        let command = editor_launcher_launch_command().unwrap();

        if command.program == "cargo" {
            assert_eq!(
                command.args,
                vec!["run", "-p", "az-editor", "--bin", "az-editor", "--"]
            );
            assert_eq!(command.cwd, workspace_root());
        } else {
            assert!(command.program.ends_with(editor_executable_name()));
            assert!(command.args.is_empty());
        }
    }

    #[test]
    fn unbound_launcher_rejects_project_bound_attach_args() {
        let error = validate_unbound_launcher_args(Some("main"), None, None).unwrap_err();
        assert!(matches!(error, CliError::InvalidArgument { message }
            if message.contains("--session") && message.contains("--project")));

        let error =
            validate_unbound_launcher_args(None, Some(EndpointKind::Tcp), None).unwrap_err();
        assert!(matches!(error, CliError::InvalidArgument { message }
            if message.contains("daemon endpoint") && message.contains("--project")));
    }

    #[test]
    fn launch_command_passes_absolute_project_path_to_child_process() {
        let command = editor_launch_command(Path::new("."), None, None).unwrap();
        let path_index = command
            .args
            .iter()
            .position(|arg| arg == "--project")
            .expect("path arg exists");
        assert!(Path::new(&command.args[path_index + 1]).is_absolute());
    }

    #[test]
    fn editor_args_include_daemon_endpoint_when_requested() {
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612");
        let args = editor_args(
            Path::new("projects/example"),
            Some("lighting"),
            Some(&endpoint),
        )
        .unwrap();

        assert_eq!(
            args,
            vec![
                "--project",
                "projects/example",
                "--session",
                "lighting",
                "--daemon-endpoint-kind",
                "tcp",
                "--daemon-endpoint",
                "127.0.0.1:37612",
            ]
        );
    }

    #[test]
    fn editor_launcher_rejects_in_process_daemon_endpoint() {
        let endpoint = Endpoint::in_process("azd:test");

        let error = editor_launch_command(Path::new("projects/example"), None, Some(&endpoint))
            .unwrap_err();

        assert!(matches!(
            error,
            CliError::UnsupportedEditorDaemonEndpoint {
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn launcher_forwards_only_explicit_daemon_endpoints() {
        let endpoint = Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612");
        let explicit = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint: endpoint.clone(),
            source: crate::commands::daemon::DaemonEndpointSource::Explicit,
        };
        let runtime_record = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint,
            source: crate::commands::daemon::DaemonEndpointSource::RuntimeRecord,
        };

        assert!(forwarded_editor_daemon_endpoint(Some(&explicit)).is_some());
        assert!(forwarded_editor_daemon_endpoint(Some(&runtime_record)).is_none());
    }

    #[test]
    fn editor_start_scope_excludes_runtime_host() {
        use az_proto_runtime::RUNTIME_HOST_SERVICE_NAME;

        let service_names = required_editor_service_names();

        assert_eq!(
            service_names,
            vec![
                PROJECT_HOST_SERVICE_NAME.to_string(),
                ASSET_PROCESSOR_SERVICE_NAME.to_string(),
                ASSET_WORKER_SERVICE_NAME.to_string(),
            ]
        );
        assert!(
            !service_names
                .iter()
                .any(|service_name| service_name == RUNTIME_HOST_SERVICE_NAME)
        );
    }

    #[test]
    fn editor_service_session_resolution_prefers_daemon_supervisor_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let project_id = write_test_project_manifest(temp.path(), "local.editor_daemon_select");
        let mut manifest = write_session(
            temp.path(),
            &project_id,
            "daemon-main",
            SessionState::Active,
        );
        let mut descriptor = az_service_catalog::session_supervisor_service_descriptor(
            manifest.id.0,
            test_run(1),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        manifest.upsert_service_descriptor(&descriptor, 1).unwrap();
        write_session_manifest(&manifest);
        let supervisor = az_session::start_session_supervisor_rpc_server_with_manager(
            az_session::SessionManager::with_data_home(
                temp.path(),
                az_filesystem::AzothDataHome::new(temp.path().join("azoth-home")),
            )
            .unwrap(),
            descriptor.endpoint.clone(),
            &manifest.slug,
        )
        .unwrap();
        descriptor.endpoint = supervisor.endpoint().clone();
        manifest.upsert_service_descriptor(&descriptor, 2).unwrap();
        write_session_manifest(&manifest);
        let daemon = az_daemon::AzDaemon::with_data_home(az_filesystem::AzothDataHome::new(
            temp.path().join("azoth-home"),
        ))
        .unwrap();
        daemon.register_project_root(temp.path()).unwrap();
        daemon
            .register_session_supervisor(&project_id, &manifest.slug, &descriptor)
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

        let manager = SessionManager::with_data_home(
            temp.path(),
            az_filesystem::AzothDataHome::new(temp.path().join("azoth-home")),
        )
        .unwrap();
        let session = resolve_editor_session_for_services_with_manager(
            temp.path(),
            None,
            &daemon_endpoint,
            &manager,
        )
        .unwrap();

        assert_eq!(session.slug, "daemon-main");
        assert!(session.live_manifest.is_some());
        supervisor.stop();
        daemon_server.stop();
    }

    #[test]
    fn explicit_editor_service_session_resolution_prefers_daemon_supervisor_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let project_id =
            write_test_project_manifest(temp.path(), "local.editor_explicit_daemon_select");
        let (manifest, supervisor, daemon_server) =
            live_supervisor_for_test_session(temp.path(), &project_id, "explicit-main");
        let daemon_endpoint = crate::commands::daemon::OptionalDaemonEndpoint {
            endpoint: daemon_server.endpoint().clone(),
            source: crate::commands::daemon::DaemonEndpointSource::Explicit,
        };

        let manager = SessionManager::with_data_home(
            temp.path(),
            az_filesystem::AzothDataHome::new(temp.path().join("azoth-home")),
        )
        .unwrap();
        let session = resolve_editor_session_for_services_with_manager(
            temp.path(),
            Some("explicit-main"),
            &daemon_endpoint,
            &manager,
        )
        .unwrap();

        assert_eq!(session.slug, manifest.slug);
        assert!(session.live_manifest.is_some());
        supervisor.stop();
        daemon_server.stop();
    }

    #[test]
    fn editor_service_session_resolution_falls_back_to_local_bootstrap_without_live_daemon_session()
    {
        let temp = tempfile::tempdir().unwrap();
        let project_id = write_test_project_manifest(temp.path(), "local.editor_local_bootstrap");
        write_session(temp.path(), &project_id, "main", SessionState::Active);
        let daemon = az_daemon::AzDaemon::with_data_home(az_filesystem::AzothDataHome::new(
            temp.path().join("azoth-home"),
        ))
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

        let manager = SessionManager::with_data_home(
            temp.path(),
            az_filesystem::AzothDataHome::new(temp.path().join("azoth-home")),
        )
        .unwrap();
        let session = resolve_editor_session_for_services_with_manager(
            temp.path(),
            None,
            &daemon_endpoint,
            &manager,
        )
        .unwrap();

        assert_eq!(session.slug, "main");
        assert!(session.live_manifest.is_none());
        daemon_server.stop();
    }

    #[test]
    fn editor_service_session_resolution_checks_daemon_before_local_bootstrap() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/editor.rs"
        ))
        .expect("read editor command source");
        let resolver_start = source
            .find("fn resolve_editor_session_for_services_with_manager(")
            .expect("find resolver");
        let resolver_end = source[resolver_start..]
            .find("\nfn ensure_live_editor_services_from_resolution(")
            .map(|offset| resolver_start + offset)
            .expect("find next helper");
        let resolver_source = &source[resolver_start..resolver_end];
        let requested_lookup = resolver_source
            .find("requested_session_manifest_through_daemon")
            .expect("find requested daemon lookup");
        let requested_local_fallback = resolver_source
            .find("ensure_editor_session_with_manager(manager, Some(session))")
            .expect("find requested local fallback");
        let daemon_lookup = resolver_source
            .find("active_session_manifest_through_daemon")
            .expect("find daemon lookup");
        let local_fallback = resolver_source
            .find("ensure_editor_session_with_manager(manager, None)")
            .expect("find local fallback");

        assert!(
            requested_lookup < requested_local_fallback,
            "explicit editor service sessions should prefer daemon/session-supervisor discovery before local session bootstrap"
        );
        assert!(
            daemon_lookup < local_fallback,
            "editor service readiness should prefer daemon/session-supervisor discovery before local session bootstrap"
        );
    }

    #[test]
    fn editor_service_plan_needs_prepare_when_required_services_are_missing() {
        let manifest = active_test_manifest();

        assert!(editor_service_plan_needs_prepare(&manifest));
    }

    #[test]
    fn editor_service_plan_reuses_existing_planned_processes() {
        let mut manifest = active_test_manifest();
        add_required_editor_service_processes(&mut manifest, ServiceProcessState::Planned);

        assert!(!editor_service_plan_needs_prepare(&manifest));
    }

    #[test]
    fn editor_service_plan_reuses_running_processes() {
        let mut manifest = active_test_manifest();
        add_required_editor_service_processes(&mut manifest, ServiceProcessState::Running);

        assert!(!editor_service_plan_needs_prepare(&manifest));
    }

    #[test]
    fn editor_service_plan_replans_when_descriptor_endpoint_changes() {
        let mut manifest = active_test_manifest();
        add_required_editor_service_processes(&mut manifest, ServiceProcessState::Running);
        let replanned_project_host =
            project_host_service_descriptor(test_run(2), Endpoint::in_process("project-host:next"));
        manifest.upsert_service_descriptor(&replanned_project_host, 2);

        assert!(editor_service_plan_needs_prepare(&manifest));
        let (service, state) = first_unready_editor_service(&manifest).unwrap();
        assert_eq!(service.name, PROJECT_HOST_SERVICE_NAME);
        assert_eq!(state, EditorServiceReadiness::EndpointMismatch);
    }

    #[test]
    fn editor_service_plan_reuses_clean_exited_processes() {
        let mut manifest = active_test_manifest();
        add_required_editor_service_processes(&mut manifest, ServiceProcessState::Exited);

        assert!(!editor_service_plan_needs_prepare(&manifest));
        let (_, state) = first_unready_editor_service(&manifest).unwrap();
        assert_eq!(state, EditorServiceReadiness::Exited);
        assert!(!state.is_terminal());
    }

    #[test]
    fn editor_service_plan_replans_failed_processes() {
        let mut manifest = active_test_manifest();
        add_required_editor_service_processes(&mut manifest, ServiceProcessState::Failed);

        assert!(editor_service_plan_needs_prepare(&manifest));
    }

    #[test]
    fn editor_service_readiness_reports_missing_process_records() {
        let mut manifest = active_test_manifest();
        let project_host =
            project_host_service_descriptor(test_run(1), Endpoint::in_process("project-host:main"));
        let asset_processor = asset_processor_service_descriptor(
            test_run(1),
            Endpoint::in_process("asset-processor:main"),
        );
        manifest.upsert_service_descriptor(&project_host, 1);
        manifest.upsert_service_descriptor(&asset_processor, 1);

        let (service, state) = first_unready_editor_service(&manifest).unwrap();

        assert_eq!(service.name, PROJECT_HOST_SERVICE_NAME);
        assert_eq!(state, EditorServiceReadiness::MissingProcess);
    }

    #[test]
    fn editor_service_readiness_waits_for_planned_processes() {
        let mut manifest = active_test_manifest();
        add_required_editor_service_processes(&mut manifest, ServiceProcessState::Planned);

        let (service, state) = first_unready_editor_service(&manifest).unwrap();

        assert_eq!(service.name, PROJECT_HOST_SERVICE_NAME);
        assert_eq!(state, EditorServiceReadiness::Planned);
        assert!(!state.is_terminal());
    }

    #[test]
    fn editor_service_readiness_accepts_running_required_services() {
        let mut manifest = active_test_manifest();
        add_required_editor_service_processes(&mut manifest, ServiceProcessState::Running);

        assert!(first_unready_editor_service(&manifest).is_none());
    }

    #[test]
    fn live_editor_service_readiness_accepts_running_required_services() {
        let mut status = active_proto_status();
        add_required_proto_editor_services(&mut status, ProtoServiceProcessState::Running);

        assert!(first_unready_editor_service_status(&status).is_none());
    }

    #[test]
    fn live_editor_service_plan_check_uses_supervisor_status_shape() {
        let mut status = active_proto_status();

        assert!(editor_service_status_needs_prepare(&status));

        add_required_proto_editor_services(&mut status, ProtoServiceProcessState::Running);
        assert!(!editor_service_status_needs_prepare(&status));

        status.manifest.services[0].run = test_run(2);
        assert!(!editor_service_status_needs_prepare(&status));

        let mut failed = active_proto_status();
        add_required_proto_editor_services(&mut failed, ProtoServiceProcessState::Failed);
        assert!(editor_service_status_needs_prepare(&failed));

        let mut exited = active_proto_status();
        add_required_proto_editor_services(&mut exited, ProtoServiceProcessState::Exited);
        assert!(!editor_service_status_needs_prepare(&exited));
    }

    #[test]
    fn live_editor_service_readiness_ignores_observational_run_change() {
        let mut status = active_proto_status();
        add_required_proto_editor_services(&mut status, ProtoServiceProcessState::Running);
        status.manifest.services[0].run = test_run(2);

        assert!(first_unready_editor_service_status(&status).is_none());
    }

    #[test]
    fn editor_launcher_uses_terminal_start_result_without_status_polling() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/editor.rs"
        ))
        .expect("read editor command source");
        let source = production_source_without_cfg_test_modules(&source);

        assert!(!source.contains("EDITOR_SERVICE_RUNNING_POLL_MS"));
        assert!(!source.contains("fn wait_for_editor_services_running"));
        assert!(source.contains("first_unready_editor_service_status(&result.status)"));
    }

    #[test]
    fn editor_service_plan_check_prefers_live_supervisor_status() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/editor.rs"
        ))
        .expect("read editor command source");
        let plan_start = source
            .find("fn editor_service_plan_needs_prepare_for_session(")
            .expect("find plan function");
        let plan_end = source[plan_start..]
            .find("\nfn editor_service_plan_needs_prepare(")
            .map(|offset| plan_start + offset)
            .expect("find local plan helper");
        let plan_source = &source[plan_start..plan_end];
        let live_status = plan_source
            .find("live_session_status_through_daemon")
            .expect("find live status lookup");
        let local_fallback = plan_source
            .find("editor_service_plan_needs_prepare(manifest)")
            .expect("find local plan fallback");

        assert!(
            live_status < local_fallback,
            "editor service plan checks should prefer live supervisor status before local manifest fallback"
        );
        assert!(plan_source.contains("editor_service_status_needs_prepare"));
    }

    #[test]
    fn editor_service_live_path_checks_supervisor_before_local_session_read() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/editor.rs"
        ))
        .expect("read editor command source");
        let ensure_start = source
            .find("fn ensure_editor_session_services(")
            .expect("find ensure-services function");
        let ensure_end = source[ensure_start..]
            .find("\nfn resolve_editor_session_for_services(")
            .map(|offset| ensure_start + offset)
            .expect("find resolver");
        let ensure_source = &source[ensure_start..ensure_end];
        let live_fast_path = ensure_source
            .find("ensure_live_editor_services_from_resolution")
            .expect("find live manifest service path");
        let local_read = ensure_source
            .find("SessionManager::new(project_path)")
            .expect("find local manager read");

        assert!(
            live_fast_path < local_read,
            "editor ensure-services should check live supervisor status before reading local session metadata"
        );

        let helper_start = source
            .find("fn ensure_live_editor_services_from_resolution(")
            .expect("find live service helper");
        let helper_end = source[helper_start..]
            .find("\nfn ensure_editor_session_with_manager(")
            .map(|offset| helper_start + offset)
            .expect("find next helper");
        let helper_source = &source[helper_start..helper_end];

        assert!(helper_source.contains("live_session_status_for_proto_manifest_through_daemon"));
        assert!(helper_source.contains("start_services_for_proto_manifest_through_daemon"));
        assert!(helper_source.contains("first_unready_editor_service_status(&result.status)"));
    }

    #[test]
    fn inactive_editor_session_is_rejected_before_service_orchestration() {
        let mut manifest = active_test_manifest();
        manifest.preserve_failure(2);

        assert!(matches!(
            ensure_editor_session_active(&manifest),
            Err(CliError::SessionNotActive { session, state })
                if session == "main" && state == "failed-preserved"
        ));
    }

    fn active_test_manifest() -> SessionManifest {
        let mut manifest = SessionManifest::new(
            SessionId::new(),
            "local.editor_test".to_string(),
            "main".to_string(),
            PathBuf::from("projects/example"),
            PathBuf::from("projects/example/.azoth/workspaces/main"),
            PathBuf::from("projects/example/.azoth/sessions/main"),
            1,
        );
        manifest.activate(1);
        manifest
    }

    fn write_test_project_manifest(root: &Path, id: &str) -> String {
        let manifest = az_project::ProjectManifest::new(id, "Editor Command Test", "0.1.0");
        az_project::write_project_manifest(root, &manifest).unwrap();
        az_project::refresh_project_lock(root).unwrap();
        id.to_string()
    }

    fn write_session(
        root: &Path,
        project_id: &str,
        slug: &str,
        state: SessionState,
    ) -> SessionManifest {
        let session_id = SessionId::new();
        let run_dir = az_session::SessionManager::with_data_home(
            root,
            az_filesystem::AzothDataHome::new(root.join("azoth-home")),
        )
        .unwrap()
        .sessions_dir()
        .join(session_id.to_string());
        std::fs::create_dir_all(&run_dir).unwrap();
        let mut manifest = SessionManifest::new(
            session_id,
            project_id.to_string(),
            slug.to_string(),
            root.to_path_buf(),
            root.to_path_buf(),
            run_dir,
            0,
        );
        manifest.state = state;
        write_session_manifest(&manifest);
        manifest
    }

    fn write_session_manifest(manifest: &SessionManifest) {
        std::fs::write(
            manifest.run_dir.join(SESSION_MANIFEST_FILE),
            toml::to_string(manifest).unwrap(),
        )
        .unwrap();
    }

    fn live_supervisor_for_test_session(
        root: &Path,
        project_id: &str,
        slug: &str,
    ) -> (
        SessionManifest,
        az_session::SessionSupervisorRpcServer,
        az_daemon::AzDaemonRpcServer,
    ) {
        let mut manifest = write_session(root, project_id, slug, SessionState::Active);
        let mut descriptor = az_service_catalog::session_supervisor_service_descriptor(
            manifest.id.0,
            test_run(1),
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        );
        manifest.upsert_service_descriptor(&descriptor, 1).unwrap();
        write_session_manifest(&manifest);
        let supervisor = az_session::start_session_supervisor_rpc_server_with_manager(
            az_session::SessionManager::with_data_home(
                root,
                az_filesystem::AzothDataHome::new(root.join("azoth-home")),
            )
            .unwrap(),
            descriptor.endpoint.clone(),
            &manifest.slug,
        )
        .unwrap();
        descriptor.endpoint = supervisor.endpoint().clone();
        manifest.upsert_service_descriptor(&descriptor, 2).unwrap();
        write_session_manifest(&manifest);
        let daemon = az_daemon::AzDaemon::with_data_home(az_filesystem::AzothDataHome::new(
            root.join("azoth-home"),
        ))
        .unwrap();
        daemon.register_project_root(root).unwrap();
        daemon
            .register_session_supervisor(project_id, &manifest.slug, &descriptor)
            .unwrap();
        let daemon_server = az_daemon::start_az_daemon_rpc_server_with_daemon(
            daemon,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0"),
        )
        .unwrap();

        (manifest, supervisor, daemon_server)
    }

    fn active_proto_status() -> ProtoSessionWorkspaceStatus {
        ProtoSessionWorkspaceStatus {
            manifest: ProtoSessionManifest::new(
                uuid::Uuid::from_u128(1),
                "local.editor_test",
                "main",
                "projects/example",
                "projects/example/.azoth/workspaces/main",
                "projects/example/.azoth/sessions/main",
                ProtoSessionState::Active,
            ),
            failure_reason: None,
        }
    }

    fn add_required_proto_editor_services(
        status: &mut ProtoSessionWorkspaceStatus,
        state: ProtoServiceProcessState,
    ) {
        let project_host = az_proto_core::ServiceDescriptor::new(
            az_proto_core::ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            ServiceRole::ProjectHost,
            Endpoint::in_process("project-host:main"),
        );
        let asset_processor = az_proto_core::ServiceDescriptor::new(
            az_proto_core::ServiceId::new(ASSET_PROCESSOR_NAMESPACE, ASSET_PROCESSOR_SERVICE_NAME),
            ServiceRole::AssetProcessor,
            Endpoint::in_process("asset-processor:main"),
        );
        let asset_worker = az_proto_core::ServiceDescriptor::new(
            az_proto_core::ServiceId::new(
                ASSET_WORKER_SERVICE_NAMESPACE,
                ASSET_WORKER_SERVICE_NAME,
            ),
            ServiceRole::Worker,
            Endpoint::in_process("asset-worker:main"),
        );
        status.manifest.services.push(project_host.clone());
        status.manifest.services.push(asset_processor.clone());
        status.manifest.services.push(asset_worker.clone());
        status.manifest.processes.push(test_proto_process(
            PROJECT_HOST_SERVICE_NAME,
            ServiceRole::ProjectHost,
            &project_host.endpoint,
            state,
        ));
        status.manifest.processes.push(test_proto_process(
            ASSET_PROCESSOR_SERVICE_NAME,
            ServiceRole::AssetProcessor,
            &asset_processor.endpoint,
            state,
        ));
        status.manifest.processes.push(test_proto_process(
            ASSET_WORKER_SERVICE_NAME,
            ServiceRole::Worker,
            &asset_worker.endpoint,
            state,
        ));
    }

    fn add_required_editor_service_processes(
        manifest: &mut SessionManifest,
        state: ServiceProcessState,
    ) {
        let project_host =
            project_host_service_descriptor(test_run(1), Endpoint::in_process("project-host:main"));
        let asset_processor = asset_processor_service_descriptor(
            test_run(1),
            Endpoint::in_process("asset-processor:main"),
        );
        let asset_worker =
            asset_worker_service_descriptor(test_run(1), Endpoint::in_process("asset-worker:main"));
        manifest.upsert_service_descriptor(&project_host, 1);
        manifest.upsert_service_descriptor(&asset_processor, 1);
        manifest.upsert_service_descriptor(&asset_worker, 1);
        manifest.processes.push(test_process(
            PROJECT_HOST_SERVICE_NAME,
            SupervisedServiceRole::ProjectHost,
            &project_host.endpoint,
            state,
        ));
        manifest.processes.push(test_process(
            ASSET_PROCESSOR_SERVICE_NAME,
            SupervisedServiceRole::AssetProcessor,
            &asset_processor.endpoint,
            state,
        ));
        manifest.processes.push(test_process(
            ASSET_WORKER_SERVICE_NAME,
            SupervisedServiceRole::Worker,
            &asset_worker.endpoint,
            state,
        ));
    }

    fn test_proto_process(
        service_name: &str,
        role: ServiceRole,
        endpoint: &Endpoint,
        state: ProtoServiceProcessState,
    ) -> ProtoServiceProcessRecord {
        ProtoServiceProcessRecord {
            owner_id: "local.editor_test".to_string(),
            owner_root: "projects/example".to_string(),
            service_name: service_name.to_string(),
            role,
            run: test_run(1),
            previous_run: None,
            endpoint: endpoint.clone(),
            program: "az-test-service".to_string(),
            program_artifact: None,
            cwd: "projects/example".to_string(),
            args: Vec::new(),
            stdout_log: "service.stdout.log".to_string(),
            stderr_log: "service.stderr.log".to_string(),
            structured_log: "service.capnp.log".to_string(),
            state,
            pid: (state == ProtoServiceProcessState::Running).then_some(42),
            process_start_time: (state == ProtoServiceProcessState::Running).then_some(9_001),
            exit_code: (state == ProtoServiceProcessState::Failed).then_some(1),
            failure: (state == ProtoServiceProcessState::Failed).then_some("failed".to_string()),
            planned_unix_ms: 1,
            updated_unix_ms: 2,
            started_unix_ms: matches!(
                state,
                ProtoServiceProcessState::Starting | ProtoServiceProcessState::Running
            )
            .then_some(2),
            exited_unix_ms: matches!(
                state,
                ProtoServiceProcessState::Exited | ProtoServiceProcessState::Failed
            )
            .then_some(3),
        }
    }

    fn test_process(
        service_name: &str,
        role: SupervisedServiceRole,
        endpoint: &Endpoint,
        state: ServiceProcessState,
    ) -> ServiceProcessRecord {
        let mut process = ServiceProcessRecord::planned(
            service_name,
            role,
            test_run(1),
            endpoint,
            "az-test-service",
            PathBuf::from("projects/example"),
            Vec::new(),
            PathBuf::from("service.stdout.log"),
            PathBuf::from("service.stderr.log"),
            PathBuf::from("service.capnp.log"),
            None,
            1,
        );
        match state {
            ServiceProcessState::Planned => {}
            ServiceProcessState::Starting => process.mark_starting(2),
            ServiceProcessState::Running => process
                .mark_running(
                    ProcessIdentity {
                        process_id: 42,
                        process_start_time: 9_001,
                    },
                    2,
                )
                .unwrap(),
            ServiceProcessState::Exited => process.mark_exited(None, None, 3),
            ServiceProcessState::Failed => process.mark_exited(Some(1), Some("failed".into()), 3),
        }
        process
    }
}
