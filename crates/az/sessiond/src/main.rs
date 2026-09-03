use std::io::{self, IsTerminal as _};
use std::path::PathBuf;

use az_endpoint_discovery::{
    default_daemon_endpoint, default_daemon_endpoint_kind, read_daemon_endpoint_record,
    remove_daemon_endpoint_record,
};
use az_observability::{ServiceObservabilityConfig, install_service_observability};
use az_proto_core::{Endpoint, EndpointKind, ProtocolVersion};
use az_proto_daemon::daemon_capnp;
use az_rpc::AzRpcTransportError;
use az_session::{SessionManager, sessiond_structured_log_path};
use az_sessiond::{
    ServiceStartup, SessionDaemonConfig, install_shutdown_signal_channel, run_session_daemon,
    sessiond_observed_log_context,
};
use clap::{ArgAction, Parser, ValueEnum};
use thiserror::Error;
use tokio::runtime::Builder;
use tokio::task::LocalSet;
use tracing::warn;

// The bools are the CLI's own shape: clap derives one `bool` field per flag,
// so folding them into enums would rename or remove operator-facing flags.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Parser)]
#[command(
    name = "az-sessiond",
    version,
    about = "Run the Azoth per-workspace session supervisor daemon",
    after_help = "Environment:\n  RUST_LOG  Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR  Disable automatic color output."
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

    /// Project root containing azoth.toml; session state resolves through the Azoth data home
    #[arg(long = "project", value_name = "DIR")]
    project: PathBuf,

    /// Session slug/name to supervise
    #[arg(long)]
    session: String,

    /// Maximum time a spawned service may take to publish and pass its readiness boundary
    #[arg(long, default_value_t = 300_000)]
    service_ready_timeout_ms: u64,

    /// Only monitor already-started children owned by this process
    #[arg(long)]
    no_start: bool,

    /// Keep running even if all supervised children exit
    #[arg(long)]
    keep_alive: bool,

    /// Only start the named planned service on daemon startup; repeat to start multiple services
    #[arg(long = "start-service")]
    start_services: Vec<String>,

    /// Session-supervisor endpoint transport kind
    #[arg(long, value_enum)]
    endpoint_kind: Option<EndpointKindArg>,

    /// Explicit session-supervisor endpoint address
    #[arg(long)]
    endpoint: Option<String>,

    /// Do not publish this az-sessiond process as the session-supervisor service
    #[arg(long)]
    no_publish_session_supervisor: bool,

    /// azd endpoint transport kind for publishing session-supervisor discovery
    #[arg(long, value_enum)]
    daemon_endpoint_kind: Option<EndpointKindArg>,

    /// Explicit azd endpoint address for publishing session-supervisor discovery
    #[arg(long)]
    daemon_endpoint: Option<String>,

    /// Do not register this session-supervisor with azd, even if a runtime azd endpoint record exists
    #[arg(
        long,
        conflicts_with_all = ["daemon_endpoint_kind", "daemon_endpoint"]
    )]
    no_daemon_registration: bool,

    /// Optional OTLP endpoint for exporting tracing spans
    #[arg(long)]
    otlp_endpoint: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EndpointKindArg {
    // Mirrors `EndpointKind` exhaustively so `public_endpoint_kind` stays a
    // total conversion that rejects in-process explicitly. `#[value(skip)]`
    // keeps clap from ever offering it, which is exactly why it is never
    // constructed.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "kept so the EndpointKind mirror stays exhaustive and in-process is rejected by name rather than by omission; only the rejection tests construct it"
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
enum SessiondCliError {
    #[error(
        "{operation} cannot use endpoint kind {kind:?}; use platform IPC or explicit TCP debug endpoints"
    )]
    UnsupportedEndpointKind {
        operation: &'static str,
        kind: EndpointKind,
    },

    // Boxed: `DaemonEndpointRecordError` alone is at least 128 bytes, which
    // would make every `Result<_, SessiondCliError>` in this binary oversized.
    #[error(transparent)]
    DaemonEndpointRecord(Box<az_endpoint_discovery::DaemonEndpointRecordError>),

    #[error(transparent)]
    DaemonRpcTransport(#[from] AzRpcTransportError),

    #[error("azd unavailable until restarted: protocol preflight failed: {reason}")]
    DaemonUnavailableUntilRestart { reason: String },

    #[error(
        "--no-daemon-registration cannot be combined with --daemon-endpoint-kind or --daemon-endpoint"
    )]
    ConflictingDaemonRegistrationArgs,
}

// `#[from]` on the boxed field would give `From<Box<_>>`; callers reach this
// variant through `?` on the unboxed error.
impl From<az_endpoint_discovery::DaemonEndpointRecordError> for SessiondCliError {
    fn from(error: az_endpoint_discovery::DaemonEndpointRecordError) -> Self {
        Self::DaemonEndpointRecord(Box::new(error))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let otlp_endpoint = args.otlp_endpoint.clone();
    let mut config = SessionDaemonConfig::new(args.project.clone(), args.session.clone());
    let structured_log_path = install_logging(
        &args.project,
        &args.session,
        config.run,
        otlp_endpoint.clone(),
        default_log_directives(args.verbose, args.quiet),
        args.color.stderr_ansi(),
    )?;
    config.service_ready_timeout = std::time::Duration::from_millis(args.service_ready_timeout_ms);
    config.startup = if args.no_start {
        ServiceStartup::None
    } else {
        ServiceStartup::Named(args.start_services)
    };
    config.exit_when_services_exit = !args.keep_alive;
    config.publish_session_supervisor = !args.no_publish_session_supervisor;
    config.child_otlp_endpoint = otlp_endpoint;
    if let Some(kind) = args.endpoint_kind {
        config.session_supervisor_endpoint_kind =
            public_endpoint_kind(kind, "az-sessiond session-supervisor endpoint")?;
    }
    if let Some(endpoint) = args.endpoint {
        config.session_supervisor_endpoint = Some(Endpoint::new(
            config.session_supervisor_endpoint_kind,
            endpoint,
        ));
    }
    config.daemon_endpoint = session_daemon_endpoint(
        args.daemon_endpoint_kind,
        args.daemon_endpoint,
        args.no_daemon_registration,
    )?;
    let shutdown_signals = install_shutdown_signal_channel()?;
    config.shutdown_events = Some(shutdown_signals.receiver());
    tracing::info!(
        structured_log = %structured_log_path.display(),
        "az-sessiond structured observability log initialized"
    );

    let report = run_session_daemon(&config)?;
    drop(shutdown_signals);
    println!(
        "az-sessiond stopped: started={}, skipped={}, exited={}, stopped={}, shutdown_requested={}",
        report.started, report.skipped, report.exited, report.stopped, report.shutdown_requested
    );
    if let Some(endpoint) = report.session_supervisor_endpoint {
        println!(
            "session-supervisor: {:?} {}",
            endpoint.kind, endpoint.address
        );
    }
    Ok(())
}

fn install_logging(
    project_root: &PathBuf,
    session: &str,
    run: uuid::Uuid,
    otlp_endpoint: Option<String>,
    default_directives: &str,
    ansi: bool,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let manager = SessionManager::new(project_root)?;
    let manifest = manager.session(session)?;
    let context = sessiond_observed_log_context(&manifest, run);
    let path = sessiond_structured_log_path(&manifest);
    install_service_observability(
        ServiceObservabilityConfig::new(context)
            .with_structured_log(path.clone())
            .with_otlp_endpoint(otlp_endpoint)
            .with_default_directives(default_directives)
            .with_ansi(ansi),
    )?;

    Ok(path)
}

fn optional_daemon_endpoint(
    kind: Option<EndpointKindArg>,
    endpoint: Option<String>,
) -> Result<Option<ResolvedDaemonEndpoint>, SessiondCliError> {
    if kind.is_none() && endpoint.is_none() {
        return read_daemon_endpoint_record()?
            .map(|record| {
                record
                    .require_protocol_version(ProtocolVersion::CURRENT)
                    .map_err(|error| SessiondCliError::DaemonUnavailableUntilRestart {
                        reason: error.to_string(),
                    })?;
                Ok(ResolvedDaemonEndpoint {
                    endpoint: record.endpoint,
                    source: DaemonEndpointSource::RuntimeRecord,
                })
            })
            .transpose();
    }

    let kind = kind
        .map(|kind| public_endpoint_kind(kind, "az-sessiond daemon endpoint"))
        .transpose()?
        .unwrap_or_else(default_daemon_endpoint_kind);
    Ok(Some(ResolvedDaemonEndpoint {
        endpoint: match endpoint {
            Some(address) => Endpoint::new(kind, address),
            None => default_daemon_endpoint(kind)?,
        },
        source: DaemonEndpointSource::Explicit,
    }))
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

fn session_daemon_endpoint(
    kind: Option<EndpointKindArg>,
    endpoint: Option<String>,
    disabled: bool,
) -> Result<Option<Endpoint>, SessiondCliError> {
    if disabled {
        if kind.is_some() || endpoint.is_some() {
            return Err(SessiondCliError::ConflictingDaemonRegistrationArgs);
        }
        return Ok(None);
    }

    let Some(resolved) = optional_daemon_endpoint(kind, endpoint)? else {
        return Ok(None);
    };

    if resolved.source == DaemonEndpointSource::RuntimeRecord {
        match probe_daemon_endpoint(&resolved.endpoint) {
            Ok(()) => {}
            Err(error) if is_daemon_connection_failure(&error) => {
                warn!(
                    error = %error,
                    "azd runtime endpoint record was stale; removing record and supervising session without azd registration"
                );
                remove_daemon_endpoint_record()?;
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        }
    }

    Ok(Some(resolved.endpoint))
}

fn probe_daemon_endpoint(endpoint: &Endpoint) -> Result<(), AzRpcTransportError> {
    let runtime = Builder::new_current_thread().enable_io().build()?;
    let local = LocalSet::new();
    let endpoint = endpoint.clone();
    local.block_on(&runtime, async move {
        az_rpc::connect_twoparty_bootstrap::<daemon_capnp::az_daemon::Client>(&endpoint)
            .await
            .map(|_| ())
    })
}

const fn is_daemon_connection_failure(error: &AzRpcTransportError) -> bool {
    matches!(
        error,
        AzRpcTransportError::Io(_) | AzRpcTransportError::Capnp(_)
    )
}

const fn public_endpoint_kind(
    value: EndpointKindArg,
    operation: &'static str,
) -> Result<EndpointKind, SessiondCliError> {
    let kind = match value {
        EndpointKindArg::InProcess => EndpointKind::InProcess,
        EndpointKindArg::WindowsNamedPipe => EndpointKind::WindowsNamedPipe,
        EndpointKindArg::UnixDomainSocket => EndpointKind::UnixDomainSocket,
        EndpointKindArg::Tcp => EndpointKind::Tcp,
    };
    if matches!(kind, EndpointKind::InProcess) {
        return Err(SessiondCliError::UnsupportedEndpointKind { operation, kind });
    }

    Ok(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_endpoint_kind_without_address_uses_default_endpoint() {
        let endpoint = optional_daemon_endpoint(Some(EndpointKindArg::Tcp), None)
            .unwrap()
            .unwrap();

        assert_eq!(endpoint.source, DaemonEndpointSource::Explicit);
        assert_eq!(
            endpoint.endpoint,
            Endpoint::new(EndpointKind::Tcp, "127.0.0.1:37612")
        );
    }

    #[test]
    fn public_endpoint_kind_rejects_in_process() {
        let error = public_endpoint_kind(
            EndpointKindArg::InProcess,
            "az-sessiond session-supervisor endpoint",
        )
        .unwrap_err();

        assert!(matches!(
            error,
            SessiondCliError::UnsupportedEndpointKind {
                operation: "az-sessiond session-supervisor endpoint",
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn daemon_endpoint_kind_rejects_in_process() {
        let error = optional_daemon_endpoint(
            Some(EndpointKindArg::InProcess),
            Some("azd:test".to_string()),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            SessiondCliError::UnsupportedEndpointKind {
                operation: "az-sessiond daemon endpoint",
                kind: EndpointKind::InProcess
            }
        ));
    }

    #[test]
    fn no_daemon_registration_skips_runtime_record_lookup() {
        let endpoint = session_daemon_endpoint(None, None, true).unwrap();

        assert_eq!(endpoint, None);
    }

    #[test]
    fn no_daemon_registration_rejects_explicit_daemon_endpoint() {
        let error = session_daemon_endpoint(
            Some(EndpointKindArg::Tcp),
            Some("127.0.0.1:37123".to_string()),
            true,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            SessiondCliError::ConflictingDaemonRegistrationArgs
        ));
    }
}
