use std::io::{self, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use az_daemon::{
    AzDaemon, default_daemon_endpoint, default_daemon_endpoint_kind, project_daemon_endpoint,
    start_az_daemon_rpc_server_with_daemon_and_shutdown, write_daemon_endpoint_record,
    write_project_daemon_endpoint_record,
};
use az_observability::{
    ObservedLogContext, ServiceObservabilityConfig, install_service_observability,
};
use az_proto_core::{Capability, Endpoint, EndpointKind, ServiceId, ServiceRole};
use az_proto_daemon::{
    DAEMON_AUDIENCE, DAEMON_LEASE_PERMISSION, ProcessIdentity as ProtoProcessIdentity,
    TouchEditorLeaseRequest, editor_process_lease_id,
};
use az_service_supervision::ProcessIdentity;
use clap::{ArgAction, Parser, ValueEnum};
use thiserror::Error;
use tracing::info;

#[derive(Debug, Parser)]
#[command(name = "azd")]
#[command(version)]
#[command(about = "Azoth user-level project and session discovery daemon")]
#[command(
    after_help = "Environment:\n  RUST_LOG                       Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR                       Disable automatic color output.\n  AZOTH_DAEMON_PROJECT_REGISTRY  Default for --project-registry.\n  OTEL_EXPORTER_OTLP_ENDPOINT    Default for --otlp-endpoint."
)]
struct Args {
    /// Increase default log verbosity (`-v` info, `-vv` debug, `-vvv` trace).
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Restrict default diagnostics to errors. `RUST_LOG` can add directives.
    #[arg(short, long, conflicts_with = "verbose")]
    quiet: bool,

    /// When to colorize diagnostic output.
    #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
    color: ColorArg,

    /// Daemon endpoint transport kind
    #[arg(long, value_enum)]
    endpoint_kind: Option<EndpointKindArg>,

    /// Explicit daemon endpoint address
    #[arg(long)]
    endpoint: Option<String>,

    /// Project root to register on startup; may be repeated
    #[arg(long = "project", value_name = "DIR")]
    projects: Vec<PathBuf>,

    /// Durable project registry TOML path
    #[arg(long = "project-registry", env = "AZOTH_DAEMON_PROJECT_REGISTRY")]
    project_registry: Option<PathBuf>,

    /// OpenTelemetry OTLP gRPC endpoint for traces.
    #[arg(long, env = "OTEL_EXPORTER_OTLP_ENDPOINT", value_name = "URL")]
    otlp_endpoint: Option<String>,

    /// Editor process identity (`PID:START_TOKEN`) that owns this sidecar lease; may be repeated
    #[arg(long = "editor-owner-process", value_name = "PID:START_TOKEN")]
    editor_owner_processes: Vec<EditorOwnerProcessArg>,

    /// Exit this sidecar once no registered editor owner process is alive
    #[arg(long)]
    shutdown_when_editor_leases_gone: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EndpointKindArg {
    // Mirrors `EndpointKind` exhaustively so `public_endpoint_kind` stays a
    // total conversion that rejects in-process by name. `#[value(skip)]` keeps
    // clap from ever offering it, so outside the tests nothing constructs it --
    // and the tests do, which is why the expectation is gated to non-test
    // builds rather than stated unconditionally.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "kept so the EndpointKind mirror stays exhaustive; only `daemon_endpoint_kind_rejects_in_process` constructs it"
        )
    )]
    #[value(skip)]
    InProcess,
    WindowsNamedPipe,
    UnixDomainSocket,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum ColorArg {
    /// Colorize diagnostics only on a terminal when `NO_COLOR` is unset.
    #[default]
    Auto,
    /// Always colorize diagnostic output.
    Always,
    /// Never colorize diagnostic output.
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EditorOwnerProcessArg(ProcessIdentity);

impl FromStr for EditorOwnerProcessArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (process_id, process_start_time) = value
            .split_once(':')
            .ok_or_else(|| "expected PID:START_TOKEN".to_string())?;
        let process_id = process_id
            .parse::<u32>()
            .map_err(|error| format!("invalid editor owner PID: {error}"))?;
        let process_start_time = process_start_time
            .parse::<u64>()
            .map_err(|error| format!("invalid editor owner start token: {error}"))?;
        if process_id == 0 || process_start_time == 0 {
            return Err("editor owner process identity values must be non-zero".to_string());
        }
        Ok(Self(ProcessIdentity {
            process_id,
            process_start_time,
        }))
    }
}

impl ColorArg {
    fn stderr_ansi(self) -> bool {
        match self {
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && io::stderr().is_terminal(),
            Self::Always => true,
            Self::Never => false,
        }
    }
}

const fn default_log_directives(verbosity: u8, quiet: bool) -> &'static str {
    match (quiet, verbosity) {
        (true, _) => "error",
        (false, 0) => "warn",
        (false, 1) => "info",
        (false, 2) => "debug",
        (false, _) => "trace",
    }
}

#[derive(Debug, Error)]
enum AzdCliError {
    #[error(
        "{operation} cannot use endpoint kind {kind:?}; use platform IPC or explicit TCP debug endpoints"
    )]
    UnsupportedEndpointKind {
        operation: &'static str,
        kind: EndpointKind,
    },

    #[error(transparent)]
    DaemonRpcTransport(#[from] az_daemon::DaemonRpcTransportError),

    #[error("editor owner process identity must be complete")]
    InvalidEditorOwnerProcess,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let run = uuid::Uuid::now_v7();
    install_service_observability(
        ServiceObservabilityConfig::new(ObservedLogContext::new(
            ServiceId::new("azoth", "azd"),
            ServiceRole::Daemon,
            run,
        ))
        .with_default_directives(default_log_directives(args.verbose, args.quiet))
        .with_otlp_endpoint(args.otlp_endpoint.as_deref())
        .with_ansi(args.color.stderr_ansi()),
    )?;
    // The endpoint is isolated per project root (path-keyed) so several projects
    // can run their own azd without colliding on a machine-global pipe/port.
    // `azd` started without a project root keeps the global endpoint.
    let primary_root = args.projects.first().cloned();
    let endpoint = daemon_endpoint(args.endpoint_kind, args.endpoint, primary_root.as_deref())?;
    let daemon = match args.project_registry {
        Some(path) => AzDaemon::with_project_registry_path(path)?,
        None => AzDaemon::with_default_project_registry()?,
    };
    for root in &args.projects {
        let project = daemon.register_project_root(root)?;
        info!(
            project_id = %project.project_id,
            root = %project.root,
            "azd startup project registered"
        );
    }
    for owner_process in &args.editor_owner_processes {
        let result = daemon.touch_editor_lease(&editor_owner_lease_request(owner_process.0)?)?;
        info!(
            owner_process_id = owner_process.0.process_id,
            owner_process_start_time = owner_process.0.process_start_time,
            active_lease_count = result.active_lease_count,
            "azd startup editor owner lease registered"
        );
    }

    let shutdown = az_work::CancellationToken::new();
    az_work::install_signal_handlers(&shutdown)?;
    let server = start_az_daemon_rpc_server_with_daemon_and_shutdown(
        daemon.clone(),
        endpoint,
        shutdown.clone(),
        run,
    )?;
    let endpoint_record = match primary_root.as_deref() {
        Some(root) => write_project_daemon_endpoint_record(root, server.endpoint())?,
        None => write_daemon_endpoint_record(server.endpoint())?,
    };
    println!(
        "azd: {:?} {}",
        server.endpoint().kind,
        server.endpoint().address
    );

    daemon.wait_for_shutdown(&shutdown, args.shutdown_when_editor_leases_gone)?;

    let report = daemon.stop_registered_session_supervisors("azd shutdown");
    if report.stopped > 0 || report.failed > 0 {
        info!(
            stopped = report.stopped,
            failed = report.failed,
            "azd requested session-supervisor shutdown before exit"
        );
    }
    drop(endpoint_record);
    server.stop();
    Ok(())
}

fn editor_owner_lease_request(
    owner_process: ProcessIdentity,
) -> Result<TouchEditorLeaseRequest, AzdCliError> {
    if owner_process.process_id == 0 || owner_process.process_start_time == 0 {
        return Err(AzdCliError::InvalidEditorOwnerProcess);
    }
    let owner_process = ProtoProcessIdentity {
        process_id: owner_process.process_id,
        process_start_time: owner_process.process_start_time,
    };
    Ok(TouchEditorLeaseRequest {
        capability: Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
            .with_audience(DAEMON_AUDIENCE)
            .with_permissions([DAEMON_LEASE_PERMISSION]),
        lease_id: editor_process_lease_id(owner_process),
        owner_process,
        purpose: "editor sidecar owner".to_string(),
    })
}

fn daemon_endpoint(
    kind: Option<EndpointKindArg>,
    endpoint: Option<String>,
    project_root: Option<&Path>,
) -> Result<Endpoint, AzdCliError> {
    let kind = kind
        .map(|kind| public_endpoint_kind(kind, "azd endpoint"))
        .transpose()?
        .unwrap_or_else(default_daemon_endpoint_kind);
    validate_public_endpoint_kind(kind, "azd endpoint")?;

    Ok(match (endpoint, project_root) {
        (Some(address), _) => Endpoint::new(kind, address),
        (None, Some(root)) => project_daemon_endpoint(kind, root)?,
        (None, None) => default_daemon_endpoint(kind)?,
    })
}

fn public_endpoint_kind(
    value: EndpointKindArg,
    operation: &'static str,
) -> Result<EndpointKind, AzdCliError> {
    let kind = match value {
        EndpointKindArg::InProcess => EndpointKind::InProcess,
        EndpointKindArg::WindowsNamedPipe => EndpointKind::WindowsNamedPipe,
        EndpointKindArg::UnixDomainSocket => EndpointKind::UnixDomainSocket,
        EndpointKindArg::Tcp => EndpointKind::Tcp,
    };
    validate_public_endpoint_kind(kind, operation)?;
    Ok(kind)
}

const fn validate_public_endpoint_kind(
    kind: EndpointKind,
    operation: &'static str,
) -> Result<(), AzdCliError> {
    if matches!(kind, EndpointKind::InProcess) {
        return Err(AzdCliError::UnsupportedEndpointKind { operation, kind });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_endpoint_kind_without_address_uses_default_endpoint() {
        let endpoint = daemon_endpoint(Some(EndpointKindArg::Tcp), None, None).unwrap();

        assert_eq!(
            endpoint,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612")
        );
    }

    #[test]
    fn daemon_endpoint_kind_rejects_in_process() {
        let error = daemon_endpoint(
            Some(EndpointKindArg::InProcess),
            Some("azd:test".to_string()),
            None,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AzdCliError::UnsupportedEndpointKind {
                operation: "azd endpoint",
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn daemon_endpoint_kind_with_address_keeps_explicit_address() {
        let endpoint = daemon_endpoint(
            Some(EndpointKindArg::Tcp),
            Some("127.0.0.1:39000".to_string()),
            None,
        )
        .unwrap();

        assert_eq!(
            endpoint,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:39000")
        );
    }

    #[test]
    fn daemon_endpoint_without_address_uses_project_scoped_endpoint_when_root_is_present() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path();
        let endpoint =
            daemon_endpoint(Some(EndpointKindArg::Tcp), None, Some(project_root)).unwrap();

        assert_eq!(endpoint.kind, EndpointKind::Tcp);
        assert_eq!(endpoint.address, "127.0.0.1:0");
    }

    #[test]
    fn editor_owner_process_arg_requires_the_complete_identity() {
        assert_eq!(
            "42:9001".parse::<EditorOwnerProcessArg>().unwrap(),
            EditorOwnerProcessArg(ProcessIdentity {
                process_id: 42,
                process_start_time: 9_001,
            })
        );
        assert!("42".parse::<EditorOwnerProcessArg>().is_err());
        assert!("42:0".parse::<EditorOwnerProcessArg>().is_err());
    }
}
