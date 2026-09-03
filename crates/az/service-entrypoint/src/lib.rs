//! Shared launch contract helpers for generated project service binaries.
//!
//! Project service entrypoints stay project-owned so they can force-link
//! project/gem inventory, but argv parsing, role validation, observability,
//! and readiness serialization are engine contracts.

use std::io::{self, IsTerminal as _};
use std::num::NonZeroUsize;
use std::num::TryFromIntError;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

use az_observability::{ObservedLogContext, ServiceObservabilityConfig};
pub use az_proto_core::{Endpoint, EndpointKind, ServiceId, ServiceRole};
use az_service_supervision::{ServiceEndpointKind, ServiceReadyRecord, SupervisedServiceRole};
use az_source_control::{LoreCli, SourceControlProvider as _};
use clap::{ArgAction, Args, Parser, ValueEnum};
use thiserror::Error;
use uuid::Uuid;

mod lifecycle;

pub use lifecycle::{
    ServiceLifecycleControlError, ServiceLifecycleControlServer, ServiceLifetime,
    ServiceLifetimeError, ServiceTermination, ServiceTerminationError, ServiceTerminationFailure,
    ServiceTerminationWait, start_service_lifecycle_control,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceArgs {
    pub log_directives: String,
    pub ansi: bool,
    pub endpoint: Endpoint,
    pub project_root: PathBuf,
    pub project_id: String,
    pub owner_id: String,
    pub owner_root: PathBuf,
    pub scope: ServiceScope,
    pub workspace_root: PathBuf,
    pub branch: String,
    pub asset_db: Option<PathBuf>,
    pub side_channel_root: Option<PathBuf>,
    pub staging_root: Option<PathBuf>,
    pub cache_root: Option<PathBuf>,
    pub asset_processor_endpoint: Option<Endpoint>,
    pub worker_count: Option<NonZeroUsize>,
    pub service: String,
    pub run: Uuid,
    pub ready_file: PathBuf,
    pub capability_grants: PathBuf,
    pub lifecycle_capability_grants: PathBuf,
    pub structured_log: PathBuf,
    pub otlp_endpoint: Option<String>,
    /// Endpoint on which this process serves the `ObservabilityControl`
    /// interface so the editor can dial its per-channel log verbosity live.
    /// Optional: absent on older launch plans (the control server is skipped).
    pub observability_endpoint: Option<Endpoint>,
    /// Brokered capability grant set authorizing observability-control callers.
    pub observability_capability_grants: Option<PathBuf>,
    /// Endpoint for the generic authenticated service lifetime control.
    pub lifecycle_endpoint: Endpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceScope {
    Project,
    Session { slug: String, id: uuid::Uuid },
}

#[derive(Debug, Error)]
pub enum ServiceEntryError {
    #[error("{message}")]
    InvalidArgument { message: String },

    #[error("workspace discovery failed: {0}")]
    WorkspaceDiscovery(#[from] az_source_control::SourceControlError),
}

impl ServiceEntryError {
    fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct CommonServiceCli {
    /// Increase default log verbosity (`-v` info, `-vv` debug, `-vvv` trace).
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,
    /// Restrict default diagnostics to errors. `RUST_LOG` can add directives.
    #[arg(short, long, conflicts_with = "verbose")]
    quiet: bool,
    /// When to colorize diagnostic output.
    #[arg(long, value_enum, default_value_t = CliColor::Auto)]
    color: CliColor,
    /// Transport used by this service's RPC listener.
    #[arg(long)]
    endpoint_kind: CliEndpointKind,
    /// Address for this service's RPC listener.
    #[arg(long)]
    endpoint: String,
    /// Absolute project directory containing `azoth.toml`.
    #[arg(long = "project", value_name = "DIR")]
    project: PathBuf,
    /// Stable Azoth project identifier.
    #[arg(long)]
    project_id: String,
    /// Stable identifier for the process that owns this service.
    #[arg(long)]
    owner_id: String,
    /// Absolute directory owned by the service owner.
    #[arg(long)]
    owner_root: PathBuf,
    /// Absolute root of the existing workspace served by this process.
    #[arg(long)]
    workspace_root: PathBuf,
    /// Source-control branch; discovered from Lore when omitted.
    #[arg(long)]
    branch: Option<String>,
    /// Planned service name.
    #[arg(long)]
    service: String,
    /// `UUIDv7` launch label used only for attribution and log selection.
    #[arg(long)]
    run: Uuid,
    /// Absolute file written after the service passes readiness checks.
    #[arg(long)]
    ready_file: PathBuf,
    /// Absolute path to the role-interface capability-grant document.
    #[arg(long)]
    capability_grants: PathBuf,
    /// Absolute capability-grant document for lifecycle control.
    #[arg(long)]
    lifecycle_capability_grants: PathBuf,
    /// Absolute path for structured JSON logs.
    #[arg(long)]
    structured_log: PathBuf,
    /// Optional OpenTelemetry gRPC collector endpoint.
    #[arg(long)]
    otlp_endpoint: Option<String>,
    /// Transport for the optional observability control endpoint.
    #[arg(long, requires = "observability_endpoint")]
    observability_endpoint_kind: Option<CliEndpointKind>,
    /// Optional observability control address.
    #[arg(
        long,
        requires_all = [
            "observability_endpoint_kind",
            "observability_capability_grants"
        ]
    )]
    observability_endpoint: Option<String>,
    /// Absolute capability-grant document for observability control.
    #[arg(long, requires = "observability_endpoint")]
    observability_capability_grants: Option<PathBuf>,
    /// Transport for the mandatory lifecycle-control endpoint.
    #[arg(long)]
    lifecycle_endpoint_kind: CliEndpointKind,
    /// Address for the mandatory lifecycle-control endpoint.
    #[arg(long)]
    lifecycle_endpoint: String,
}

#[derive(Debug, Clone, Parser)]
#[command(
    version,
    about = "Run the Azoth project-host service",
    disable_help_subcommand = true,
    after_help = "Environment:\n  RUST_LOG  Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR  Disable automatic color output."
)]
pub struct ProjectHostCli {
    #[command(flatten)]
    common: CommonServiceCli,
    /// Absolute root for project-host side-channel payloads.
    #[arg(long)]
    side_channel_root: PathBuf,
    /// Transport used to connect to the project Asset Processor.
    #[arg(long)]
    asset_processor_endpoint_kind: CliEndpointKind,
    /// Address of the project Asset Processor.
    #[arg(long)]
    asset_processor_endpoint: String,
}

impl ProjectHostCli {
    /// Convert parsed `project-host` flags into validated [`ServiceArgs`].
    ///
    /// # Errors
    ///
    /// Returns [`ServiceEntryError::InvalidArgument`] if the common flags do
    /// not validate for this role — an empty or nil identity field, a
    /// malformed endpoint, or a role/scope mismatch — or
    /// [`ServiceEntryError::WorkspaceDiscovery`] if the workspace backing the
    /// launch cannot be resolved.
    pub fn into_service_args(self) -> Result<ServiceArgs, ServiceEntryError> {
        service_args_from_cli(
            ServiceRole::ProjectHost,
            self.common,
            ServiceScope::Project,
            RoleServiceArgs {
                side_channel_root: Some(self.side_channel_root),
                asset_processor_endpoint: Some(Endpoint::new(
                    self.asset_processor_endpoint_kind.into(),
                    self.asset_processor_endpoint,
                )),
                ..RoleServiceArgs::default()
            },
        )
    }
}

#[derive(Debug, Clone, Parser)]
#[command(
    version,
    about = "Run the Azoth asset-processor service",
    disable_help_subcommand = true,
    after_help = "Environment:\n  RUST_LOG  Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR  Disable automatic color output."
)]
pub struct AssetProcessorCli {
    #[command(flatten)]
    common: CommonServiceCli,
    /// Absolute path to the session asset database.
    #[arg(long)]
    asset_db: PathBuf,
}

impl AssetProcessorCli {
    /// Convert parsed `asset-processor` flags into validated [`ServiceArgs`].
    ///
    /// # Errors
    ///
    /// Returns [`ServiceEntryError::InvalidArgument`] if the common flags do
    /// not validate for this role — an empty or nil identity field, a
    /// malformed endpoint, or a role/scope mismatch — or
    /// [`ServiceEntryError::WorkspaceDiscovery`] if the workspace backing the
    /// launch cannot be resolved.
    pub fn into_service_args(self) -> Result<ServiceArgs, ServiceEntryError> {
        service_args_from_cli(
            ServiceRole::AssetProcessor,
            self.common,
            ServiceScope::Project,
            RoleServiceArgs {
                asset_db: Some(self.asset_db),
                ..RoleServiceArgs::default()
            },
        )
    }
}

#[derive(Debug, Clone, Parser)]
#[command(
    version,
    about = "Run one project-linked Azoth asset worker",
    disable_help_subcommand = true,
    after_help = "Environment:\n  RUST_LOG  Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR  Disable automatic color output."
)]
pub struct AssetWorkerCli {
    #[command(flatten)]
    common: CommonServiceCli,
    /// Absolute scratch directory for in-progress products.
    #[arg(long)]
    staging_root: PathBuf,
    /// Absolute processed-product cache root.
    #[arg(long)]
    cache_root: PathBuf,
    /// Transport used to connect to the session asset processor.
    #[arg(long)]
    asset_processor_endpoint_kind: CliEndpointKind,
    /// Address of the session asset processor.
    #[arg(long)]
    asset_processor_endpoint: String,
    /// Maximum number of asset jobs processed concurrently by this worker.
    #[arg(long, default_value_t = default_worker_count())]
    workers: NonZeroUsize,
}

fn default_worker_count() -> NonZeroUsize {
    std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
}

impl AssetWorkerCli {
    /// Convert parsed `asset-worker` flags into validated [`ServiceArgs`].
    ///
    /// # Errors
    ///
    /// Returns [`ServiceEntryError::InvalidArgument`] if the common flags do
    /// not validate for this role — an empty or nil identity field, a
    /// malformed endpoint, or a role/scope mismatch — or
    /// [`ServiceEntryError::WorkspaceDiscovery`] if the workspace backing the
    /// launch cannot be resolved.
    pub fn into_service_args(self) -> Result<ServiceArgs, ServiceEntryError> {
        service_args_from_cli(
            ServiceRole::Worker,
            self.common,
            ServiceScope::Project,
            RoleServiceArgs {
                staging_root: Some(self.staging_root),
                cache_root: Some(self.cache_root),
                asset_processor_endpoint: Some(Endpoint::new(
                    self.asset_processor_endpoint_kind.into(),
                    self.asset_processor_endpoint,
                )),
                worker_count: Some(self.workers),
                ..RoleServiceArgs::default()
            },
        )
    }
}

#[derive(Debug, Clone, Parser)]
#[command(
    version,
    about = "Run the Azoth runtime-host service",
    disable_help_subcommand = true,
    after_help = "Environment:\n  RUST_LOG  Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR  Disable automatic color output."
)]
pub struct RuntimeHostCli {
    #[command(flatten)]
    common: CommonServiceCli,
    /// Human-readable supervision session name.
    #[arg(long)]
    session: String,
    /// UUID of the supervision session.
    #[arg(long)]
    session_id: uuid::Uuid,
    /// Absolute root for runtime-host side-channel payloads.
    #[arg(long)]
    side_channel_root: PathBuf,
}

impl RuntimeHostCli {
    /// Convert parsed `runtime-host` flags into validated [`ServiceArgs`].
    ///
    /// # Errors
    ///
    /// Returns [`ServiceEntryError::InvalidArgument`] if the common flags do
    /// not validate for this role — an empty or nil identity field, a
    /// malformed endpoint, or a role/scope mismatch — or
    /// [`ServiceEntryError::WorkspaceDiscovery`] if the workspace backing the
    /// launch cannot be resolved.
    pub fn into_service_args(self) -> Result<ServiceArgs, ServiceEntryError> {
        service_args_from_cli(
            ServiceRole::RuntimeHost,
            self.common,
            ServiceScope::Session {
                slug: self.session,
                id: self.session_id,
            },
            RoleServiceArgs {
                side_channel_root: Some(self.side_channel_root),
                ..RoleServiceArgs::default()
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum CliEndpointKind {
    /// Windows named-pipe IPC.
    WindowsNamedPipe,
    /// Unix-domain-socket IPC.
    UnixDomainSocket,
    /// Explicit TCP, intended for diagnostics and remote development.
    Tcp,
    /// Test-only in-process transport; never accepted at a process boundary.
    #[value(skip)]
    InProcess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum CliColor {
    /// Colorize diagnostics only on a terminal when `NO_COLOR` is unset.
    #[default]
    Auto,
    /// Always colorize diagnostic output.
    Always,
    /// Never colorize diagnostic output.
    Never,
}

impl CliColor {
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

impl From<CliEndpointKind> for EndpointKind {
    fn from(value: CliEndpointKind) -> Self {
        match value {
            CliEndpointKind::WindowsNamedPipe => Self::WindowsNamedPipe,
            CliEndpointKind::UnixDomainSocket => Self::UnixDomainSocket,
            CliEndpointKind::Tcp => Self::Tcp,
            CliEndpointKind::InProcess => Self::InProcess,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct RoleServiceArgs {
    asset_db: Option<PathBuf>,
    side_channel_root: Option<PathBuf>,
    staging_root: Option<PathBuf>,
    cache_root: Option<PathBuf>,
    asset_processor_endpoint: Option<Endpoint>,
    worker_count: Option<NonZeroUsize>,
}

fn service_args_from_cli(
    service_role: ServiceRole,
    common: CommonServiceCli,
    scope: ServiceScope,
    role_args: RoleServiceArgs,
) -> Result<ServiceArgs, ServiceEntryError> {
    let branch = match common.branch.as_deref().map(str::trim) {
        Some(branch) if !branch.is_empty() => branch.to_string(),
        Some(_) => return Err(invalid("--branch cannot be empty")),
        None => LoreCli
            .current_branch(&common.workspace_root)?
            .filter(|branch| !branch.trim().is_empty())
            .ok_or_else(|| {
                invalid(format!(
                    "workspace `{}` does not report a current Lore branch",
                    common.workspace_root.display()
                ))
            })?,
    };
    let observability_endpoint = match (
        common.observability_endpoint_kind,
        common.observability_endpoint,
    ) {
        (Some(kind), Some(address)) => Some(Endpoint::new(kind.into(), address)),
        (None, None) => None,
        _ => {
            return Err(invalid(
                "--observability-endpoint and --observability-endpoint-kind must be provided together",
            ));
        }
    };
    let parsed = ServiceArgs {
        log_directives: default_log_directives(common.verbose, common.quiet).to_string(),
        ansi: common.color.stderr_ansi(),
        endpoint: Endpoint::new(common.endpoint_kind.into(), common.endpoint),
        project_root: common.project,
        project_id: common.project_id,
        owner_id: common.owner_id,
        owner_root: common.owner_root,
        scope,
        workspace_root: common.workspace_root,
        branch,
        asset_db: role_args.asset_db,
        side_channel_root: role_args.side_channel_root,
        staging_root: role_args.staging_root,
        cache_root: role_args.cache_root,
        asset_processor_endpoint: role_args.asset_processor_endpoint,
        worker_count: role_args.worker_count,
        service: common.service,
        run: common.run,
        ready_file: common.ready_file,
        capability_grants: common.capability_grants,
        lifecycle_capability_grants: common.lifecycle_capability_grants,
        structured_log: common.structured_log,
        otlp_endpoint: common.otlp_endpoint,
        observability_endpoint,
        observability_capability_grants: common.observability_capability_grants,
        lifecycle_endpoint: Endpoint::new(
            common.lifecycle_endpoint_kind.into(),
            common.lifecycle_endpoint,
        ),
    };
    validate_service_args(&parsed)?;
    validate_service_role_args(service_role, &parsed)?;
    Ok(parsed)
}

#[cfg(test)]
fn clap_error(error: &clap::Error) -> ServiceEntryError {
    invalid(error.to_string())
}

/// Check the launch arguments every service role shares.
///
/// # Errors
///
/// Returns [`ServiceEntryError::InvalidArgument`] if either endpoint is
/// malformed, if the project id, owner id, branch, service name, or session
/// slug is empty, if a session scope carries the nil UUID, or if `--run` is not
/// a `UUIDv7`.
pub fn validate_service_args(args: &ServiceArgs) -> Result<(), ServiceEntryError> {
    validate_endpoint("--endpoint", &args.endpoint)?;
    validate_endpoint("--lifecycle-endpoint", &args.lifecycle_endpoint)?;
    validate_non_empty("project id", &args.project_id)?;
    validate_non_empty("owner id", &args.owner_id)?;
    if let ServiceScope::Session { slug, id } = &args.scope {
        validate_non_empty("session slug", slug)?;
        if id.is_nil() {
            return Err(invalid("session id cannot be the nil UUID"));
        }
    }
    validate_non_empty("branch", &args.branch)?;
    validate_non_empty("service name", &args.service)?;
    if args.run.get_version_num() != 7 {
        return Err(invalid("--run must be a UUIDv7 value"));
    }
    validate_absolute_path("--project", &args.project_root)?;
    validate_absolute_path("--owner-root", &args.owner_root)?;
    validate_absolute_path("--workspace-root", &args.workspace_root)?;
    validate_absolute_path("--ready-file", &args.ready_file)?;
    validate_absolute_path("--capability-grants", &args.capability_grants)?;
    validate_absolute_path(
        "--lifecycle-capability-grants",
        &args.lifecycle_capability_grants,
    )?;
    validate_absolute_path("--structured-log", &args.structured_log)?;
    if let Some(asset_db) = &args.asset_db {
        validate_absolute_path("--asset-db", asset_db)?;
    }
    if let Some(side_channel_root) = &args.side_channel_root {
        validate_absolute_path("--side-channel-root", side_channel_root)?;
    }
    if let Some(staging_root) = &args.staging_root {
        validate_absolute_path("--staging-root", staging_root)?;
    }
    if let Some(cache_root) = &args.cache_root {
        validate_absolute_path("--cache-root", cache_root)?;
    }
    if let Some(asset_processor_endpoint) = &args.asset_processor_endpoint {
        validate_endpoint("--asset-processor-endpoint", asset_processor_endpoint)?;
    }
    match (
        &args.observability_endpoint,
        &args.observability_capability_grants,
    ) {
        (Some(endpoint), Some(grants)) => {
            validate_endpoint("--observability-endpoint", endpoint)?;
            validate_absolute_path("--observability-capability-grants", grants)?;
        }
        (None, None) => {}
        _ => {
            return Err(invalid(
                "--observability-endpoint requires --observability-capability-grants (and vice versa)",
            ));
        }
    }
    Ok(())
}

/// Check the launch arguments that only some roles accept.
///
/// # Errors
///
/// Returns [`ServiceEntryError::InvalidArgument`] if `service_role` does not
/// match the argument scope (runtime-host requires session scope, every other
/// role requires project scope), if `--workers` is present for a role other
/// than asset-worker or missing for asset-worker, or if a role-specific path or
/// endpoint flag is missing or supplied where it is not accepted.
pub fn validate_service_role_args(
    service_role: ServiceRole,
    args: &ServiceArgs,
) -> Result<(), ServiceEntryError> {
    validate_role_scope(service_role, args)?;
    validate_role_worker_count(service_role, args)?;
    validate_role_paths_and_endpoints(service_role, args)
}

/// Only runtime-host is session-scoped; every other generated service role is
/// project-scoped.
fn validate_role_scope(
    service_role: ServiceRole,
    args: &ServiceArgs,
) -> Result<(), ServiceEntryError> {
    match (service_role, &args.scope) {
        (ServiceRole::RuntimeHost, ServiceScope::Session { .. })
        | (
            ServiceRole::ProjectHost | ServiceRole::AssetProcessor | ServiceRole::Worker,
            ServiceScope::Project,
        ) => {}
        (ServiceRole::RuntimeHost, ServiceScope::Project) => {
            return Err(invalid(
                "RuntimeHost requires session-scoped launch arguments",
            ));
        }
        (_, ServiceScope::Session { .. }) => {
            return Err(invalid(format!(
                "{service_role:?} requires project-scoped launch arguments"
            )));
        }
        (_, ServiceScope::Project) => {}
    }
    Ok(())
}

/// `--workers` is required by asset-worker and accepted by nothing else.
fn validate_role_worker_count(
    service_role: ServiceRole,
    args: &ServiceArgs,
) -> Result<(), ServiceEntryError> {
    match (service_role, args.worker_count) {
        // Must precede the catch-all `(_, None)` below, which would otherwise
        // swallow the missing-`--workers` refusal.
        (ServiceRole::Worker, None) => {
            return Err(invalid("--workers is required for asset-worker"));
        }
        (ServiceRole::Worker, Some(_)) | (_, None) => {}
        (role, Some(_)) => {
            return Err(invalid(format!(
                "--workers is valid only for Worker, not {role:?}"
            )));
        }
    }
    Ok(())
}

/// Each role names exactly the paths and endpoints it owns, and refuses the
/// rest, so a launch plan cannot hand a service state belonging to another.
fn validate_role_paths_and_endpoints(
    service_role: ServiceRole,
    args: &ServiceArgs,
) -> Result<(), ServiceEntryError> {
    match service_role {
        ServiceRole::ProjectHost => {
            reject_role_path("--asset-db", args.asset_db.as_ref(), "project-host")?;
            require_role_path(
                "--side-channel-root",
                args.side_channel_root.as_ref(),
                "project-host",
            )?;
            reject_role_path("--staging-root", args.staging_root.as_ref(), "project-host")?;
            reject_role_path("--cache-root", args.cache_root.as_ref(), "project-host")?;
            require_role_endpoint(
                "--asset-processor-endpoint",
                args.asset_processor_endpoint.as_ref(),
                "project-host",
            )?;
        }
        ServiceRole::AssetProcessor => {
            require_role_path("--asset-db", args.asset_db.as_ref(), "asset-processor")?;
            reject_role_path(
                "--side-channel-root",
                args.side_channel_root.as_ref(),
                "asset-processor",
            )?;
            reject_role_path(
                "--staging-root",
                args.staging_root.as_ref(),
                "asset-processor",
            )?;
            reject_role_path("--cache-root", args.cache_root.as_ref(), "asset-processor")?;
            reject_role_endpoint(
                "--asset-processor-endpoint",
                args.asset_processor_endpoint.as_ref(),
                "asset-processor",
            )?;
        }
        ServiceRole::Worker => {
            reject_role_path("--asset-db", args.asset_db.as_ref(), "asset-worker")?;
            reject_role_path(
                "--side-channel-root",
                args.side_channel_root.as_ref(),
                "asset-worker",
            )?;
            require_role_path("--staging-root", args.staging_root.as_ref(), "asset-worker")?;
            require_role_path("--cache-root", args.cache_root.as_ref(), "asset-worker")?;
            require_role_endpoint(
                "--asset-processor-endpoint",
                args.asset_processor_endpoint.as_ref(),
                "asset-worker",
            )?;
        }
        ServiceRole::RuntimeHost => {
            reject_role_path("--asset-db", args.asset_db.as_ref(), "runtime-host")?;
            require_role_path(
                "--side-channel-root",
                args.side_channel_root.as_ref(),
                "runtime-host",
            )?;
            reject_role_path("--staging-root", args.staging_root.as_ref(), "runtime-host")?;
            reject_role_path("--cache-root", args.cache_root.as_ref(), "runtime-host")?;
            reject_role_endpoint(
                "--asset-processor-endpoint",
                args.asset_processor_endpoint.as_ref(),
                "runtime-host",
            )?;
        }
        other => {
            return Err(invalid(format!(
                "unsupported generated project service role {other:?}"
            )));
        }
    }
    Ok(())
}

fn require_role_path(
    flag: &'static str,
    value: Option<&PathBuf>,
    role: &'static str,
) -> Result<(), ServiceEntryError> {
    if value.is_none() {
        return Err(invalid(format!("{flag} is required for {role}")));
    }
    Ok(())
}

fn reject_role_path(
    flag: &'static str,
    value: Option<&PathBuf>,
    role: &'static str,
) -> Result<(), ServiceEntryError> {
    if value.is_some() {
        return Err(invalid(format!("{flag} is not accepted by {role}")));
    }
    Ok(())
}

fn require_role_endpoint(
    flag: &'static str,
    value: Option<&Endpoint>,
    role: &'static str,
) -> Result<(), ServiceEntryError> {
    if value.is_none() {
        return Err(invalid(format!("{flag} is required for {role}")));
    }
    Ok(())
}

fn reject_role_endpoint(
    flag: &'static str,
    value: Option<&Endpoint>,
    role: &'static str,
) -> Result<(), ServiceEntryError> {
    if value.is_some() {
        return Err(invalid(format!("{flag} is not accepted by {role}")));
    }
    Ok(())
}

/// Check that one endpoint is addressable for its transport.
///
/// # Errors
///
/// Returns [`ServiceEntryError::InvalidArgument`] if the address is blank, or
/// if a TCP endpoint's address does not parse as a socket address.
pub fn validate_endpoint(label: &str, endpoint: &Endpoint) -> Result<(), ServiceEntryError> {
    if endpoint.address.trim().is_empty() {
        return Err(invalid(format!("{label} address cannot be empty")));
    }
    match endpoint.kind {
        EndpointKind::WindowsNamedPipe | EndpointKind::UnixDomainSocket => Ok(()),
        EndpointKind::Tcp => endpoint
            .address
            .parse::<std::net::SocketAddr>()
            .map(|_| ())
            .map_err(|_| {
                invalid(format!(
                    "{label} TCP address must be host:port: {}",
                    endpoint.address
                ))
            }),
        EndpointKind::InProcess => Err(invalid(format!(
            "{label} in-process endpoints are test-only; generated project services must bind IPC or TCP"
        ))),
    }
}

fn validate_non_empty(label: &str, value: &str) -> Result<(), ServiceEntryError> {
    if value.trim().is_empty() {
        return Err(invalid(format!("{label} cannot be empty")));
    }
    Ok(())
}

fn validate_absolute_path(label: &str, path: &Path) -> Result<(), ServiceEntryError> {
    if path.as_os_str().is_empty() {
        return Err(invalid(format!("{label} path cannot be empty")));
    }
    if !path.is_absolute() {
        return Err(invalid(format!(
            "{label} path must be absolute: {}",
            path.display()
        )));
    }
    Ok(())
}

/// Install this process's tracing subscriber and structured log sink.
///
/// # Errors
///
/// Returns [`az_observability::ObservedLogFileError`] if the structured log
/// file named by `--structured-log` cannot be created or opened.
pub fn install_observability(
    args: &ServiceArgs,
    role: ServiceRole,
) -> Result<(), az_observability::ObservedLogFileError> {
    let mut context =
        ObservedLogContext::new(ServiceId::new("azoth", &args.service), role, args.run);
    if let ServiceScope::Session { slug, .. } = &args.scope {
        context = context.with_session_slug(slug.clone());
    }
    az_observability::install_service_observability(
        ServiceObservabilityConfig::new(context)
            .with_structured_log(args.structured_log.clone())
            .with_otlp_endpoint(args.otlp_endpoint.clone())
            .with_default_directives(args.log_directives.clone())
            .with_ansi(args.ansi),
    )
}

/// Start this process's `ObservabilityControl` server if the launch plan
/// configured one.
///
/// Control is configured by `--observability-endpoint` together with
/// `--observability-capability-grants`. The returned server must be kept alive
/// for the process lifetime (dropping it stops the listener); `None` means
/// control was not configured for this launch.
///
/// # Errors
///
/// Returns an error if the grant file cannot be read or decoded, or if the
/// control endpoint cannot be bound.
pub fn start_observability_control_server(
    args: &ServiceArgs,
    role: ServiceRole,
) -> Result<
    Option<az_observability_control::ObservabilityControlRpcServer>,
    az_observability_control::ObservabilityControlRpcTransportError,
> {
    let (Some(endpoint), Some(grants)) = (
        &args.observability_endpoint,
        &args.observability_capability_grants,
    ) else {
        return Ok(None);
    };
    let scope = match args.scope {
        ServiceScope::Project => az_observability_control::ObservabilityControlScope::Project,
        ServiceScope::Session { id, .. } => {
            az_observability_control::ObservabilityControlScope::Session(id)
        }
    };
    let server = az_observability_control::start_observability_control_rpc_server_with_capability_grant_file(
        endpoint.clone(),
        scope,
        ServiceId::new("azoth", &args.service),
        role,
        args.run,
        grants,
    )?;
    Ok(Some(server))
}

/// Parse a CLI endpoint-kind string into a transport.
///
/// # Errors
///
/// Returns [`ServiceEntryError::InvalidArgument`] if `value` names no known
/// kind, or names `in-process`, which is test-only and cannot be published by a
/// generated service.
pub fn parse_endpoint_kind(value: &str) -> Result<EndpointKind, ServiceEntryError> {
    let kind = CliEndpointKind::from_str(value, true)
        .map_err(|_| invalid(format!("unsupported endpoint kind `{value}`")))?;
    match kind {
        CliEndpointKind::InProcess => Err(invalid(
            "`in-process` endpoints are test-only; generated project services must bind IPC or TCP",
        )),
        _ => Ok(kind.into()),
    }
}

#[derive(Debug, Error)]
pub enum ReadyFileError {
    #[error("system time error: {0}")]
    SystemTime(#[from] SystemTimeError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("service role {0:?} cannot publish a supervision readiness record")]
    UnsupportedRole(ServiceRole),
}

/// Publish this service's supervision readiness record.
///
/// # Errors
///
/// Returns any error [`write_ready_file_with_observability`] returns.
pub fn write_ready_file(
    args: &ServiceArgs,
    role: ServiceRole,
    endpoint: &Endpoint,
) -> Result<(), ReadyFileError> {
    write_ready_file_with_observability(args, role, endpoint, None)
}

/// Like [`write_ready_file`] but also records the bound observability-control
/// endpoint.
///
/// This lets the session manager — and through it the editor — discover the
/// real address even when the control server bound TCP `:0`.
///
/// # Errors
///
/// Returns [`ReadyFileError::SystemTime`] if the clock is before the Unix
/// epoch, [`ReadyFileError::UnsupportedRole`] if `role` publishes no
/// supervision record, [`ReadyFileError::TomlSerialize`] if the record cannot
/// be encoded, or [`ReadyFileError::Io`] if the ready file's directory cannot
/// be created or the file cannot be written.
pub fn write_ready_file_with_observability(
    args: &ServiceArgs,
    role: ServiceRole,
    endpoint: &Endpoint,
    observability_endpoint: Option<&Endpoint>,
) -> Result<(), ReadyFileError> {
    let ready_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let role =
        SupervisedServiceRole::from_proto(role).ok_or(ReadyFileError::UnsupportedRole(role))?;
    let mut record = ServiceReadyRecord::new(
        &args.service,
        role,
        args.run,
        endpoint,
        Some(std::process::id()),
        ready_unix_ms,
    );
    record.observability_endpoint_kind =
        observability_endpoint.map(|endpoint| ServiceEndpointKind::from_proto(endpoint.kind));
    record.observability_endpoint_address =
        observability_endpoint.map(|endpoint| endpoint.address.clone());
    record.lifecycle_endpoint_kind = Some(ServiceEndpointKind::from_proto(
        args.lifecycle_endpoint.kind,
    ));
    record.lifecycle_endpoint_address = Some(args.lifecycle_endpoint.address.clone());
    if let Some(parent) = args.ready_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(&record)?;
    std::fs::write(&args.ready_file, text)?;
    Ok(())
}

/// The CLI spelling of one endpoint kind.
///
/// # Panics
///
/// Panics on [`EndpointKind::InProcess`], which is test-only and has no CLI
/// spelling; generated services must publish IPC or TCP.
#[must_use]
pub fn endpoint_kind_arg(kind: EndpointKind) -> &'static str {
    match kind {
        EndpointKind::WindowsNamedPipe => "windows-named-pipe",
        EndpointKind::UnixDomainSocket => "unix-domain-socket",
        EndpointKind::Tcp => "tcp",
        EndpointKind::InProcess => {
            panic!("in-process endpoints are test-only; generated services must publish IPC or TCP")
        }
    }
}

#[derive(Debug, Error)]
pub enum ServiceTimeError {
    #[error("system time error: {0}")]
    SystemTime(#[from] SystemTimeError),

    #[error("timestamp is out of i64 range: {0}")]
    OutOfRange(#[from] TryFromIntError),
}

/// The current Unix time in milliseconds, as the signed value the record
/// schema stores.
///
/// # Errors
///
/// Returns [`ServiceTimeError::SystemTime`] if the clock is before the Unix
/// epoch, or [`ServiceTimeError::OutOfRange`] if the millisecond count does not
/// fit in an `i64`.
pub fn now_unix_ms_i64() -> Result<i64, ServiceTimeError> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(i64::try_from(millis)?)
}

fn invalid(message: impl Into<String>) -> ServiceEntryError {
    ServiceEntryError::invalid(message)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    fn abs(name: &str) -> String {
        std::env::temp_dir()
            .join("az-service-entrypoint-tests")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }

    fn base_args() -> Vec<String> {
        vec![
            "--endpoint".to_string(),
            "127.0.0.1:9229".to_string(),
            "--lifecycle-endpoint-kind".to_string(),
            "tcp".to_string(),
            "--lifecycle-endpoint".to_string(),
            "127.0.0.1:9230".to_string(),
            "--endpoint-kind".to_string(),
            "tcp".to_string(),
            "--run".to_string(),
            Uuid::now_v7().to_string(),
            "--project".to_string(),
            abs("project"),
            "--project-id".to_string(),
            "local.test".to_string(),
            "--owner-id".to_string(),
            "project:local.test".to_string(),
            "--owner-root".to_string(),
            abs("owner"),
            "--ready-file".to_string(),
            abs("ready/service.toml"),
            "--capability-grants".to_string(),
            abs("grants/service.capnp.packed"),
            "--lifecycle-capability-grants".to_string(),
            abs("grants/service.lifecycle.capnp.packed"),
            "--structured-log".to_string(),
            abs("logs/service.azlog"),
            "--workspace-root".to_string(),
            abs("workspace"),
            "--branch".to_string(),
            "main".to_string(),
            "--service".to_string(),
            "project-host".to_string(),
        ]
    }

    fn with_pair(mut args: Vec<String>, flag: &str, value: impl Into<String>) -> Vec<String> {
        args.push(flag.to_string());
        args.push(value.into());
        args
    }

    fn project_host_args() -> Vec<String> {
        with_pair(
            with_pair(base_args(), "--asset-processor-endpoint-kind", "tcp"),
            "--asset-processor-endpoint",
            "127.0.0.1:9330",
        )
    }

    fn argv(args: Vec<String>) -> Vec<String> {
        std::iter::once("azoth-project-service".to_string())
            .chain(args)
            .collect()
    }

    fn parse_project_host_args(args: Vec<String>) -> Result<ServiceArgs, ServiceEntryError> {
        ProjectHostCli::try_parse_from(argv(args))
            .map_err(|error| clap_error(&error))?
            .into_service_args()
    }

    fn parse_asset_worker_args(args: Vec<String>) -> Result<ServiceArgs, ServiceEntryError> {
        AssetWorkerCli::try_parse_from(argv(args))
            .map_err(|error| clap_error(&error))?
            .into_service_args()
    }

    #[test]
    fn project_service_requires_a_distinct_lifecycle_capability_grant_document() {
        let mut args = with_pair(
            project_host_args(),
            "--side-channel-root",
            abs("side-channel/project-host"),
        );
        let flag = args
            .iter()
            .position(|arg| arg == "--lifecycle-capability-grants")
            .unwrap();
        args.drain(flag..=flag + 1);

        let error = parse_project_host_args(args).unwrap_err();

        assert!(error.to_string().contains("--lifecycle-capability-grants"));
    }

    #[test]
    fn project_host_accepts_only_side_channel_role_context() {
        let args = with_pair(
            project_host_args(),
            "--side-channel-root",
            abs("side-channel/project-host"),
        );

        let parsed = parse_project_host_args(args).expect("project-host args");

        assert_eq!(parsed.project_id, "local.test");
        assert!(parsed.asset_db.is_none());
        assert!(parsed.side_channel_root.is_some());
        assert!(parsed.cache_root.is_none());
    }

    #[test]
    fn project_host_rejects_worker_cache_root() {
        let args = with_pair(
            with_pair(
                project_host_args(),
                "--side-channel-root",
                abs("side-channel/project-host"),
            ),
            "--cache-root",
            abs("Cache/pc"),
        );

        let error = parse_project_host_args(args).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unexpected argument '--cache-root'")
        );
    }

    #[test]
    fn worker_requires_processor_endpoint_and_cache_roots() {
        let args = with_pair(
            with_pair(
                with_pair(base_args(), "--staging-root", abs("staging/worker")),
                "--cache-root",
                abs("Cache/pc"),
            ),
            "--asset-processor-endpoint-kind",
            "tcp",
        );
        let args = with_pair(args, "--asset-processor-endpoint", "127.0.0.1:9330");

        let parsed = parse_asset_worker_args(args).expect("worker args");

        assert_eq!(
            parsed
                .asset_processor_endpoint
                .as_ref()
                .map(|endpoint| endpoint.address.as_str()),
            Some("127.0.0.1:9330")
        );
        assert!(parsed.staging_root.is_some());
        assert!(parsed.cache_root.is_some());
        assert!(parsed.asset_db.is_none());
        assert_eq!(parsed.worker_count, Some(default_worker_count()));
    }

    #[test]
    fn worker_count_is_an_explicit_runtime_setting() {
        let args = with_pair(
            with_pair(
                with_pair(base_args(), "--staging-root", abs("staging/worker")),
                "--cache-root",
                abs("Cache/pc"),
            ),
            "--asset-processor-endpoint-kind",
            "tcp",
        );
        let args = with_pair(args, "--asset-processor-endpoint", "127.0.0.1:9330");
        let args = with_pair(args, "--workers", "17");

        let parsed = parse_asset_worker_args(args).expect("worker args");

        assert_eq!(parsed.worker_count.map(NonZeroUsize::get), Some(17));
    }

    #[test]
    fn worker_rejects_missing_processor_endpoint() {
        let args = with_pair(
            with_pair(base_args(), "--staging-root", abs("staging/worker")),
            "--cache-root",
            abs("Cache/pc"),
        );

        let error = parse_asset_worker_args(args).unwrap_err();

        assert!(error.to_string().contains("--asset-processor-endpoint"));
    }

    #[test]
    fn parser_rejects_unknown_arguments_through_clap() {
        let mut args = with_pair(
            project_host_args(),
            "--side-channel-root",
            abs("side-channel/project-host"),
        );
        args.push("--unsupported-project-service-flag".to_string());

        let error = parse_project_host_args(args).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unexpected argument '--unsupported-project-service-flag'")
        );
    }

    #[test]
    fn parser_rejects_unknown_endpoint_kind_through_clap() {
        let mut args = with_pair(
            with_pair(
                project_host_args(),
                "--asset-db",
                abs("assetdb/assetdb.sqlite"),
            ),
            "--side-channel-root",
            abs("side-channel/project-host"),
        );
        set_flag(&mut args, "--endpoint-kind", "local-only");

        let error = parse_project_host_args(args).unwrap_err();

        assert!(
            error.to_string().contains("invalid value 'local-only'"),
            "{error}"
        );
    }

    #[test]
    fn endpoints_reject_in_process_and_invalid_tcp() {
        let error = validate_endpoint("--endpoint", &Endpoint::in_process("test")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("in-process endpoints are test-only")
        );

        let error =
            validate_endpoint("--endpoint", &Endpoint::new(EndpointKind::Tcp, "abc")).unwrap_err();
        assert!(error.to_string().contains("TCP address must be host:port"));
    }

    #[test]
    fn ready_file_serializes_supervisor_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ready = temp.path().join("ready/service.toml");
        let mut args = with_pair(
            project_host_args(),
            "--side-channel-root",
            abs("side-channel/project-host"),
        );
        set_flag(&mut args, "--ready-file", ready.to_string_lossy());
        let parsed = parse_project_host_args(args).expect("project-host args");

        write_ready_file(&parsed, ServiceRole::ProjectHost, &parsed.endpoint).expect("write ready");

        let text = std::fs::read_to_string(&ready).expect("read ready");
        assert!(text.contains("schema_version = 1"));
        assert!(text.contains("service_name = \"project-host\""));
        assert!(text.contains("role = \"project-host\""));
        assert!(Path::new(&ready).exists());
    }

    fn set_flag(args: &mut [String], flag: &str, value: impl AsRef<str>) {
        let index = args.iter().position(|arg| arg == flag).expect("flag");
        args[index + 1] = value.as_ref().to_string();
    }
}
