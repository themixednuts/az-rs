//! Startup sequence for the standalone `az-editor` binary.
//!
//! `src/bin/az-editor.rs` is only `fn main`; everything it does before the
//! first window lives here. The binary's own test harness cannot load on a
//! Windows server image — linking the GUI entry pulls in the GPUI Windows
//! platform, whose import table names a `DirectComposition` entry those images
//! do not export — so the library harness owns these tests instead.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use az_endpoint_discovery::{
    DaemonEndpointRecordError, default_daemon_endpoint_kind, project_daemon_endpoint,
    remove_project_daemon_endpoint_record,
};
use az_proto_core::{Endpoint, EndpointKind};
use tracing::warn;

use crate::cli::{
    EditorCli, UiPresentPolicyArg, ViewportPresentPolicyArg, default_log_directives,
    validate_unbound_launcher_args,
};
use crate::daemon_bootstrap::{is_daemon_transport_failure, probe_daemon_endpoint};
use crate::{EditorApp, EditorError, EditorResult, ensure_daemon_endpoint_for_project};

/// Parse this process's arguments and start the editor, reporting a startup
/// failure on stderr.
///
/// The returned code is the binary's exit status: `main` hands it straight
/// back so nothing between the failure report and process exit can change it.
#[must_use]
pub fn run_from_env() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            print_startup_failure(&failure);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<EditorStartupFailure>> {
    let args = EditorCli::parse(std::env::args().skip(1))?;
    let EditorCli {
        verbose,
        quiet,
        color,
        viewport_diagnostic,
        viewport_present_policy,
        ui_present_policy,
        trace_ui,
        theme_dir,
        project_root,
        asset_processor_project_root,
        session,
        daemon_endpoint_kind,
        daemon_endpoint,
    } = args;
    apply_editor_process_options(
        viewport_diagnostic,
        viewport_present_policy,
        ui_present_policy,
        trace_ui,
        theme_dir.as_deref(),
    );
    crate::logging::install_process_observability(
        default_log_directives(verbose, quiet),
        color.stderr_ansi(),
    )
    .map_err(EditorError::from)?;
    let _app = if let Some(project_root) = asset_processor_project_root {
        let daemon_endpoint =
            optional_daemon_endpoint(daemon_endpoint_kind, daemon_endpoint, &project_root)?;
        match open_asset_processor_session(
            project_root.clone(),
            session.as_deref(),
            daemon_endpoint,
        ) {
            Ok(app) => app,
            Err(error) => {
                let recovery = attach_recovery_report(&error, &project_root, session.as_deref());
                return Err(Box::new(EditorStartupFailure { error, recovery }));
            }
        }
    } else if let Some(project_root) = project_root {
        let daemon_endpoint =
            optional_daemon_endpoint(daemon_endpoint_kind, daemon_endpoint, &project_root)?;
        match open_project_session(project_root.clone(), session.as_deref(), daemon_endpoint) {
            Ok(app) => app,
            Err(error) => {
                let recovery = attach_recovery_report(&error, &project_root, session.as_deref());
                return Err(Box::new(EditorStartupFailure { error, recovery }));
            }
        }
    } else {
        validate_unbound_launcher_args(
            session.as_deref(),
            daemon_endpoint_kind,
            daemon_endpoint.as_deref(),
        )?;
        EditorApp::new()?
    };

    Ok(())
}

fn apply_editor_process_options(
    viewport_diagnostic: bool,
    viewport_present_policy: ViewportPresentPolicyArg,
    ui_present_policy: UiPresentPolicyArg,
    trace_ui: bool,
    theme_dir: Option<&Path>,
) {
    // SAFETY: `run` calls this before observability, GPUI, or any other editor
    // subsystem can create threads or read these process-scoped settings.
    unsafe {
        std::env::set_var(
            "AZOTH_EDITOR_VIEWPORT_DIAGNOSTIC",
            if viewport_diagnostic { "1" } else { "0" },
        );
        std::env::set_var(
            "AZOTH_EDITOR_VIEWPORT_PRESENT_POLICY",
            viewport_present_policy.as_str(),
        );
        std::env::set_var("AZOTH_EDITOR_UI_PRESENT_POLICY", ui_present_policy.as_str());
        std::env::set_var("AZOTH_EDITOR_TRACE_UI", if trace_ui { "1" } else { "0" });
        if let Some(theme_dir) = theme_dir {
            std::env::set_var("AZOTH_EDITOR_THEME_DIR", theme_dir);
        }
    }
}

#[derive(Debug)]
struct EditorStartupFailure {
    error: EditorError,
    recovery: Option<AttachRecoveryReport>,
}

impl From<EditorError> for EditorStartupFailure {
    fn from(error: EditorError) -> Self {
        Self {
            error,
            recovery: None,
        }
    }
}

// `run` returns the failure boxed, so `?` on an `EditorError` needs this hop.
// The startup failure is large enough that carrying it inline made every
// `Result` in `run` pay for the one path that fails.
impl From<EditorError> for Box<EditorStartupFailure> {
    fn from(error: EditorError) -> Self {
        Self::new(EditorStartupFailure::from(error))
    }
}

fn print_startup_failure(failure: &EditorStartupFailure) {
    eprintln!("az-editor failed: {}", failure.error);
    if let Some(recovery) = &failure.recovery {
        eprintln!();
        eprint!("{recovery}");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttachRecoveryReport {
    summary: String,
    commands: Vec<AttachRecoveryCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttachRecoveryCommand {
    label: &'static str,
    command: String,
}

impl AttachRecoveryReport {
    fn new(summary: impl Into<String>, commands: Vec<AttachRecoveryCommand>) -> Self {
        Self {
            summary: summary.into(),
            commands,
        }
    }
}

impl AttachRecoveryCommand {
    const fn new(label: &'static str, command: String) -> Self {
        Self { label, command }
    }
}

impl fmt::Display for AttachRecoveryReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "editor attach recovery:")?;
        writeln!(f, "  {}", self.summary)?;
        if !self.commands.is_empty() {
            writeln!(f, "  Suggested commands:")?;
            for command in &self.commands {
                writeln!(f, "    {}: {}", command.label, command.command)?;
            }
        }
        Ok(())
    }
}

fn attach_recovery_report(
    error: &EditorError,
    project_root: &Path,
    requested_session: Option<&str>,
) -> Option<AttachRecoveryReport> {
    let project_arg = command_path_arg(project_root);
    session_attach_recovery(error, &project_arg, requested_session)
        .or_else(|| service_attach_recovery(error, &project_arg))
}

/// Recovery for the three "which session" failures: none active, more than one
/// active, or the resolved session not being in an attachable state.
fn session_attach_recovery(
    error: &EditorError,
    project_arg: &str,
    requested_session: Option<&str>,
) -> Option<AttachRecoveryReport> {
    match error {
        EditorError::NoActiveEditorSession => {
            let session = requested_session.unwrap_or("main");
            Some(AttachRecoveryReport::new(
                "No active editor session was found. Create or activate a session, then start its project services.",
                vec![
                    AttachRecoveryCommand::new(
                        "create session",
                        format!("azoth session create {session} --project {project_arg}"),
                    ),
                    AttachRecoveryCommand::new(
                        "start daemon",
                        format!("azoth daemon start --project {project_arg}"),
                    ),
                    AttachRecoveryCommand::new(
                        "prepare services",
                        format!("azoth session services prepare {session} --project {project_arg}"),
                    ),
                    AttachRecoveryCommand::new(
                        "start services",
                        format!("azoth session services start {session} --project {project_arg}"),
                    ),
                ],
            ))
        }
        EditorError::AmbiguousEditorSession => Some(AttachRecoveryReport::new(
            "More than one active session is available. Inspect sessions and reopen the editor with an explicit session name.",
            vec![
                AttachRecoveryCommand::new(
                    "inspect sessions",
                    format!("azoth session list --project {project_arg}"),
                ),
                AttachRecoveryCommand::new(
                    "open explicit session",
                    format!("azoth editor --project {project_arg} --session <session>"),
                ),
            ],
        )),
        EditorError::SessionNotActive { session, state } => {
            let mut commands = vec![AttachRecoveryCommand::new(
                "inspect services",
                format!("azoth session services status {session} --project {project_arg}"),
            )];
            if state == "failed-preserved" {
                commands.extend([
                    AttachRecoveryCommand::new(
                        "recover session",
                        format!("azoth session recover {session} --project {project_arg}"),
                    ),
                    AttachRecoveryCommand::new(
                        "prepare services",
                        format!(
                            "azoth session services prepare {session} --recover --project {project_arg}"
                        ),
                    ),
                    AttachRecoveryCommand::new(
                        "start services",
                        format!("azoth session services start {session} --project {project_arg}"),
                    ),
                ]);
            }
            Some(AttachRecoveryReport::new(
                format!(
                    "Session `{session}` is {state}; the editor can only attach to an active session."
                ),
                commands,
            ))
        }
        _ => None,
    }
}

/// Recovery for failures after the session itself resolves: a project service
/// missing or not running, or azd being unreachable for discovery.
fn service_attach_recovery(error: &EditorError, project_arg: &str) -> Option<AttachRecoveryReport> {
    match error {
        EditorError::MissingSessionService { session, service } => Some(AttachRecoveryReport::new(
            format!(
                "Session `{session}` is missing required service `{service}`. Refresh the service plan and start the session services."
            ),
            vec![
                AttachRecoveryCommand::new(
                    "start daemon",
                    format!("azoth daemon start --project {project_arg}"),
                ),
                AttachRecoveryCommand::new(
                    "prepare services",
                    format!("azoth session services prepare {session} --project {project_arg}"),
                ),
                AttachRecoveryCommand::new(
                    "start services",
                    format!("azoth session services start {session} --project {project_arg}"),
                ),
                AttachRecoveryCommand::new(
                    "inspect services",
                    format!("azoth session services status {session} --project {project_arg}"),
                ),
            ],
        )),
        EditorError::SessionServiceNotRunning {
            session,
            service,
            run,
            state,
        } => {
            let service_name = service_name_from_label(service);
            let mut commands = vec![
                AttachRecoveryCommand::new(
                    "inspect services",
                    format!("azoth session services status {session} --project {project_arg}"),
                ),
                AttachRecoveryCommand::new(
                    "read service log",
                    format!(
                        "azoth session services log {session} {service_name} --run {run} --project {project_arg}"
                    ),
                ),
            ];
            if state == "missing process record" || state == "planned" {
                commands.push(AttachRecoveryCommand::new(
                    "start services",
                    format!("azoth session services start {session} --project {project_arg}"),
                ));
            } else {
                commands.extend([
                    AttachRecoveryCommand::new(
                        "prepare services",
                        format!("azoth session services prepare {session} --project {project_arg}"),
                    ),
                    AttachRecoveryCommand::new(
                        "start services",
                        format!("azoth session services start {session} --project {project_arg}"),
                    ),
                ]);
            }
            Some(AttachRecoveryReport::new(
                format!(
                    "Session `{session}` service `{service}` run {run} is {state}. The editor requires project-host and asset-processor to be running."
                ),
                commands,
            ))
        }
        EditorError::ServiceDiscovery(message)
            if message.contains("requires a reachable azd endpoint")
                || message.contains("azd runtime endpoint record was stale") =>
        {
            Some(AttachRecoveryReport::new(
                "The editor could not reach azd for session discovery. Start azd with this project registered, then retry the editor.",
                vec![AttachRecoveryCommand::new(
                    "start daemon",
                    format!("azoth daemon start --project {project_arg}"),
                )],
            ))
        }
        _ => None,
    }
}

fn command_path_arg(path: &Path) -> String {
    let value = path.to_string_lossy();
    if value.chars().any(char::is_whitespace) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.into_owned()
    }
}

fn service_name_from_label(service: &str) -> &str {
    service
        .rsplit_once('/')
        .map_or(service, |(_, service_name)| service_name)
}

fn optional_daemon_endpoint(
    kind: Option<EndpointKind>,
    endpoint: Option<String>,
    project_root: &Path,
) -> EditorResult<Option<ResolvedDaemonEndpoint>> {
    if kind.is_none() && endpoint.is_none() {
        // No explicit endpoint: read-or-spawn this project's own azd daemon
        // (path-keyed, isolated from other projects) and use that endpoint.
        let endpoint = ensure_daemon_endpoint_for_project(project_root)?;
        return Ok(Some(ResolvedDaemonEndpoint {
            endpoint,
            source: DaemonEndpointSource::RuntimeRecord,
        }));
    }

    let kind = kind.unwrap_or_else(default_daemon_endpoint_kind);
    Ok(Some(ResolvedDaemonEndpoint {
        endpoint: match endpoint {
            Some(address) => Endpoint::new(kind, address),
            None => project_daemon_endpoint(kind, project_root)
                .map_err(|error| daemon_endpoint_error(&error))?,
        },
        source: DaemonEndpointSource::Explicit,
    }))
}

fn daemon_endpoint_error(error: &DaemonEndpointRecordError) -> EditorError {
    EditorError::InvalidArgument(format!("failed to resolve azd endpoint: {error}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedDaemonEndpoint {
    endpoint: Endpoint,
    source: DaemonEndpointSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonEndpointSource {
    Explicit,
    RuntimeRecord,
}

/// Verify the endpoint the editor will attach through, leaving only the GPUI
/// shell creation to the caller.
///
/// The split is what keeps this file's tests out of the GUI link: every
/// pre-window failure is decided here, and only `open_project_session` names
/// the shell constructor.
fn verify_project_attach_endpoint(
    project_root: &Path,
    daemon_endpoint: Option<ResolvedDaemonEndpoint>,
    missing_endpoint_message: &str,
) -> EditorResult<Endpoint> {
    let Some(daemon_endpoint) = daemon_endpoint else {
        return Err(EditorError::ServiceDiscovery(
            missing_endpoint_message.to_string(),
        ));
    };
    let ResolvedDaemonEndpoint { endpoint, source } = daemon_endpoint;

    // A runtime-record endpoint was already probed by bootstrap; this rechecks
    // liveness right before the session opens. It runs the one shared bounded
    // probe (`daemon_bootstrap`), so a peer that completes the handshake and
    // then goes quiet fails inside the deadline instead of hanging the CLI.
    if source == DaemonEndpointSource::RuntimeRecord {
        match probe_daemon_endpoint(&endpoint) {
            Ok(()) => {}
            Err(error) if is_daemon_transport_failure(&error) => {
                warn!(
                    error = %error,
                    "azd runtime endpoint record was stale; removing record"
                );
                remove_project_daemon_endpoint_record(project_root)
                    .map_err(|error| daemon_endpoint_error(&error))?;
                return Err(EditorError::ServiceDiscovery(format!(
                    "azd runtime endpoint record was stale; start azd or pass --daemon-endpoint: {error}"
                )));
            }
            Err(error) => return Err(error),
        }
    }

    Ok(endpoint)
}

fn open_project_session(
    project_root: PathBuf,
    session: Option<&str>,
    daemon_endpoint: Option<ResolvedDaemonEndpoint>,
) -> EditorResult<EditorApp> {
    let endpoint = verify_project_attach_endpoint(
        &project_root,
        daemon_endpoint,
        "project-bound az-editor attach requires a reachable azd endpoint; start azd or pass --daemon-endpoint",
    )?;

    EditorApp::open_project_session_via_daemon(project_root, session.unwrap_or("main"), endpoint)
}

fn open_asset_processor_session(
    project_root: PathBuf,
    session: Option<&str>,
    daemon_endpoint: Option<ResolvedDaemonEndpoint>,
) -> EditorResult<EditorApp> {
    let endpoint = verify_project_attach_endpoint(
        &project_root,
        daemon_endpoint,
        "project-bound az-editor asset-processor shell requires a reachable azd endpoint; start azd or pass --daemon-endpoint",
    )?;

    EditorApp::open_asset_processor_session_via_daemon(
        project_root,
        session.unwrap_or("main"),
        endpoint,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_endpoint_kind_without_address_uses_default_endpoint() {
        let endpoint =
            optional_daemon_endpoint(Some(EndpointKind::Tcp), None, Path::new("projects/example"))
                .unwrap()
                .unwrap();

        assert_eq!(endpoint.source, DaemonEndpointSource::Explicit);
        assert_eq!(
            endpoint.endpoint,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:0")
        );
    }

    #[test]
    fn project_attach_without_daemon_endpoint_fails_before_app_fallback() {
        let error = verify_project_attach_endpoint(
            Path::new("projects/example"),
            None,
            "project-bound az-editor attach requires a reachable azd endpoint; start azd or pass --daemon-endpoint",
        )
        .unwrap_err();

        assert!(matches!(error, EditorError::ServiceDiscovery(message)
            if message.contains("requires a reachable azd endpoint")));
    }

    #[test]
    fn missing_service_recovery_prepares_and_starts_services() {
        let report = attach_recovery_report(
            &EditorError::MissingSessionService {
                session: "lighting".to_string(),
                service: "azoth/project-host".to_string(),
            },
            Path::new("projects/example"),
            Some("lighting"),
        )
        .unwrap();
        let text = report.to_string();

        assert!(text.contains("azoth daemon start --project projects/example"));
        assert!(
            text.contains("azoth session services prepare lighting --project projects/example")
        );
        assert!(text.contains("azoth session services start lighting --project projects/example"));
        assert!(text.contains("azoth session services status lighting --project projects/example"));
    }

    #[test]
    fn stopped_service_recovery_points_to_run_log_and_restart() {
        let run = uuid::Uuid::from_bytes([3; 16]);
        let report = attach_recovery_report(
            &EditorError::SessionServiceNotRunning {
                session: "lighting".to_string(),
                service: "azoth/asset-processor".to_string(),
                run,
                state: "failed".to_string(),
            },
            Path::new("projects/example"),
            Some("lighting"),
        )
        .unwrap();
        let text = report.to_string();

        assert!(text.contains(
            &format!("azoth session services log lighting asset-processor --run {run} --project projects/example")
        ));
        assert!(
            text.contains("azoth session services prepare lighting --project projects/example")
        );
        assert!(text.contains("azoth session services start lighting --project projects/example"));
    }

    #[test]
    fn planned_service_recovery_starts_existing_plan_without_replanning() {
        let run = uuid::Uuid::from_bytes([2; 16]);
        let report = attach_recovery_report(
            &EditorError::SessionServiceNotRunning {
                session: "lighting".to_string(),
                service: "azoth/project-host".to_string(),
                run,
                state: "planned".to_string(),
            },
            Path::new("projects/example"),
            Some("lighting"),
        )
        .unwrap();
        let text = report.to_string();

        assert!(text.contains(
            &format!("azoth session services log lighting project-host --run {run} --project projects/example")
        ));
        assert!(text.contains("azoth session services start lighting --project projects/example"));
        assert!(
            !text.contains("azoth session services prepare lighting --project projects/example")
        );
    }

    #[test]
    fn failed_preserved_session_recovery_includes_recover_flow() {
        let report = attach_recovery_report(
            &EditorError::SessionNotActive {
                session: "lighting".to_string(),
                state: "failed-preserved".to_string(),
            },
            Path::new("projects/example"),
            Some("lighting"),
        )
        .unwrap();
        let text = report.to_string();

        assert!(text.contains("azoth session recover lighting --project projects/example"));
        assert!(text.contains(
            "azoth session services prepare lighting --recover --project projects/example"
        ));
        assert!(text.contains("azoth session services start lighting --project projects/example"));
    }

    #[test]
    fn recovery_commands_quote_project_paths_with_spaces() {
        let report = attach_recovery_report(
            &EditorError::NoActiveEditorSession,
            Path::new("projects/example game"),
            None,
        )
        .unwrap();
        let text = report.to_string();

        assert!(text.contains("azoth session create main --project \"projects/example game\""));
        assert!(text.contains("azoth daemon start --project \"projects/example game\""));
    }
}
