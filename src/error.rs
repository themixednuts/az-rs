use std::path::PathBuf;
use thiserror::Error;

pub type CliResult<T> = Result<T, CliError>;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Session(Box<az_session::SessionError>),

    #[error(transparent)]
    ServiceProcess(#[from] az_service_supervision::ServiceProcessError),

    #[error(transparent)]
    DaemonEndpointRecord(Box<az_endpoint_discovery::DaemonEndpointRecordError>),

    #[error(transparent)]
    DaemonProjectRegistry(Box<az_proto_daemon::DaemonProjectRegistryError>),

    #[error(transparent)]
    RpcTransport(#[from] az_rpc::AzRpcTransportError),

    #[error(transparent)]
    ServiceProtocol(#[from] capnp::Error),

    #[error(transparent)]
    SideChannelCapability(#[from] az_proto_core::SideChannelCapabilityError),

    #[error(transparent)]
    Observability(#[from] az_observability::ObservedLogFileError),

    #[error(transparent)]
    ProjectManifest(Box<az_project::ProjectManifestError>),

    #[error(transparent)]
    IdsGeneration(Box<az_project::ids_generation::IdsGenerationError>),

    #[error("generated gem ids crates are stale: {crates}")]
    StaleGemIds { crates: String },

    #[error(transparent)]
    ProjectBuildSelectorPreflight(Box<az_project::ProjectBuildSelectorPreflightError>),

    #[error(transparent)]
    RuntimeFileStaging(Box<az_project::RuntimeFileStagingError>),

    #[error(transparent)]
    DataHome(Box<az_filesystem::DataHomeError>),

    #[error(transparent)]
    HostTool(#[from] az_filesystem::HostToolError),

    #[error(transparent)]
    ProjectScaffold(Box<az_project_scaffold::ScaffoldError>),

    #[error(transparent)]
    SourceControl(#[from] az_source_control::SourceControlError),

    #[error(transparent)]
    Secret(Box<az_secrets::SecretError>),

    #[error("invalid secret reference: {message}")]
    InvalidSecretReference { message: String },

    #[error(transparent)]
    PackageManifest(Box<az_asset::PackageManifestError>),

    #[error(transparent)]
    PackagePayload(Box<az_asset::PackagePayloadError>),

    #[error(transparent)]
    AssetCatalog(#[from] az_asset::AssetCatalogError),

    #[error("unsupported config key `{0}`")]
    UnsupportedConfigKey(String),

    #[error("failed to parse config manifest {path}: {message}")]
    ConfigParse { path: PathBuf, message: String },

    #[error("session `{session}` is {state}; editor attach requires an active session")]
    SessionNotActive { session: String, state: String },

    #[error("session `{session}` is missing required service `{service}`")]
    MissingSessionService { session: String, service: String },

    #[error(
        "session `{}` service `{}` is {}; editor attach requires it to be running",
        .0.session,
        .0.service,
        .0.state
    )]
    SessionServiceNotRunning(Box<SessionServiceNotRunningDetails>),

    #[error(
        "session `{}` service `{}` did not grant `{}` capability `{}`",
        .0.session,
        .0.service,
        .0.audience,
        .0.permissions
    )]
    MissingServiceCapability(Box<MissingServiceCapabilityDetails>),

    #[error("expected {expected} descriptor for {operation}, got {actual}")]
    UnexpectedServiceDescriptor {
        operation: &'static str,
        expected: String,
        actual: String,
    },

    #[error("{operation} received invalid {service} descriptor: {reason}")]
    InvalidServiceDescriptor {
        operation: &'static str,
        service: String,
        reason: String,
    },

    #[error(
        "no active session is available for {operation}; pass --session <name> or create one with `azoth session create <name>`"
    )]
    NoActiveSession { operation: &'static str },

    #[error(
        "multiple active sessions are available for {operation}; pass --session <name>: {sessions:?}"
    )]
    AmbiguousActiveSessions {
        operation: &'static str,
        sessions: Vec<String>,
    },

    #[error("{operation} discovered session `{session}` from the wrong project: {reason}")]
    SessionDiscoveryMismatch {
        operation: &'static str,
        session: String,
        reason: String,
    },

    #[error("azd endpoint is required for {operation}; start azd or pass --daemon-endpoint")]
    MissingDaemonEndpoint { operation: &'static str },

    #[error("azd returned invalid data for {operation}: {reason}")]
    DaemonAuthorityMismatch {
        operation: &'static str,
        reason: String,
    },

    #[error("session-supervisor returned invalid data for {operation}: {reason}")]
    SessionSupervisorAuthorityMismatch {
        operation: &'static str,
        reason: String,
    },

    #[error("asset-processor returned invalid data for {operation}: {reason}")]
    AssetProcessorAuthorityMismatch {
        operation: &'static str,
        reason: String,
    },

    #[error(
        "asset processing failed for session `{session}` platform `{platform}`: {failed} failed job(s)"
    )]
    AssetProcessingFailed {
        session: String,
        platform: String,
        failed: u32,
    },

    #[error(
        "asset processing for session `{session}` platform `{platform}` did not reach idle within {timeout_ms}ms"
    )]
    AssetProcessingWaitTimedOut {
        session: String,
        platform: String,
        timeout_ms: u64,
    },

    #[error("runtime-host returned invalid data for {operation}: {reason}")]
    RuntimeHostAuthorityMismatch {
        operation: &'static str,
        reason: String,
    },

    #[error("azd did not publish a reachable endpoint within {timeout_ms}ms; log={log_path}")]
    DaemonStartTimedOut { timeout_ms: u64, log_path: PathBuf },

    #[error("{operation} timed out after {timeout_ms}ms while contacting azd endpoint {endpoint}")]
    DaemonRpcTimedOut {
        operation: &'static str,
        endpoint: String,
        timeout_ms: u64,
    },

    #[error(
        "session `{session}` orphan cleanup could not safely stop every recorded service: {failures:?}"
    )]
    SessionOrphanCleanupFailed {
        session: String,
        failures: Vec<String>,
    },

    #[error(
        "session `{}` has no recorded process for service `{}`{}",
        .0.session,
        .0.service,
        .0.run
    )]
    MissingServiceProcess(Box<MissingServiceProcessDetails>),

    #[error("session exec requires a command after `--`")]
    MissingSessionExecCommand,

    #[error("invalid service plan: {message}")]
    InvalidServicePlan { message: String },

    #[error(
        "azoth editor cannot use daemon endpoint kind {kind:?}; use platform IPC or explicit TCP debug endpoints"
    )]
    UnsupportedEditorDaemonEndpoint { kind: az_proto_core::EndpointKind },

    #[error(
        "{operation} cannot use endpoint kind {kind:?}; use platform IPC or explicit TCP debug endpoints"
    )]
    UnsupportedEndpointKind {
        operation: &'static str,
        kind: az_proto_core::EndpointKind,
    },

    #[error("failed to read structured service log `{path}`: {message}")]
    StructuredServiceLog { path: PathBuf, message: String },

    #[error(
        "refusing to read session `{}` service `{}` log `{}` outside session run directory `{}`",
        .0.session,
        .0.service,
        .0.path.display(),
        .0.run_dir.display()
    )]
    InvalidServiceLogPath(Box<InvalidServiceLogPathDetails>),

    #[error("invalid asset status page: {message}")]
    InvalidAssetStatusPage { message: String },

    #[error(
        "runtime launch requires DB-owned asset source roots for session `{session}` asset view {workspace_id}; asset-processor returned none"
    )]
    MissingRuntimeAssetSourceRoots { session: String, workspace_id: i64 },

    #[error("invalid authored edit: {message}")]
    InvalidAuthoredEdit { message: String },

    #[error("invalid argument: {message}")]
    InvalidArgument { message: String },

    #[error(
        "command failed in {}: {} {:?}; status={:?}",
        .0.cwd.display(),
        .0.program,
        .0.args,
        .0.status
    )]
    CommandFailed(Box<CommandFailedDetails>),
}

/// Payload of [`CliError::SessionServiceNotRunning`], boxed to keep `CliError` small.
#[derive(Debug)]
pub struct SessionServiceNotRunningDetails {
    pub session: String,
    pub service: String,
    pub state: String,
}

/// Payload of [`CliError::MissingServiceCapability`], boxed to keep `CliError` small.
#[derive(Debug)]
pub struct MissingServiceCapabilityDetails {
    pub session: String,
    pub service: String,
    pub audience: String,
    pub permissions: String,
}

/// Payload of [`CliError::MissingServiceProcess`], boxed to keep `CliError` small.
#[derive(Debug)]
pub struct MissingServiceProcessDetails {
    pub session: String,
    pub service: String,
    pub run: String,
}

/// Payload of [`CliError::InvalidServiceLogPath`], boxed to keep `CliError` small.
#[derive(Debug)]
pub struct InvalidServiceLogPathDetails {
    pub session: String,
    pub service: String,
    pub path: PathBuf,
    pub run_dir: PathBuf,
}

/// Payload of [`CliError::CommandFailed`], boxed to keep `CliError` small.
#[derive(Debug)]
pub struct CommandFailedDetails {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub status: Option<i32>,
}

/// Preserves `?` ergonomics for the source errors whose payloads are boxed inside
/// [`CliError`]: `#[from]` on a boxed field would only yield `From<Box<E>>`.
macro_rules! boxed_source_conversions {
    ($($source:ty => $variant:ident),* $(,)?) => {
        $(
            impl From<$source> for CliError {
                fn from(source: $source) -> Self {
                    Self::$variant(Box::new(source))
                }
            }
        )*
    };
}

boxed_source_conversions! {
    az_session::SessionError => Session,
    az_endpoint_discovery::DaemonEndpointRecordError => DaemonEndpointRecord,
    az_proto_daemon::DaemonProjectRegistryError => DaemonProjectRegistry,
    az_project::ProjectManifestError => ProjectManifest,
    az_project::ids_generation::IdsGenerationError => IdsGeneration,
    az_project::ProjectBuildSelectorPreflightError => ProjectBuildSelectorPreflight,
    az_project::RuntimeFileStagingError => RuntimeFileStaging,
    az_filesystem::DataHomeError => DataHome,
    az_project_scaffold::ScaffoldError => ProjectScaffold,
    az_secrets::SecretError => Secret,
    az_asset::PackageManifestError => PackageManifest,
    az_asset::PackagePayloadError => PackagePayload,
}
