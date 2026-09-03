use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use std::io::{self, IsTerminal as _};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

mod commands;
mod error;

use commands::build::BuildAssetOutput;

const DEFAULT_SESSION_SERVICE_START_TIMEOUT_MS: u64 = 60 * 60 * 1_000;

use az_observability::{
    ObservedLogContext, ServiceObservabilityConfig, install_service_observability,
};
use az_proto_core::{ServiceId, ServiceRole};
use error::CliResult;

/// `AZoth` Project Manager
///
/// Create, manage, and build game projects using the `AZoth` Engine.
#[derive(Parser)]
#[command(
    name = "azoth",
    version,
    about = "Create and manage AZoth game projects",
    long_about = None,
    after_help = "Terms: a gem is a reusable plugin crate; azd is the user-level project and session daemon; Lore is the source-control service; a workspace is an existing source-control checkout; a session is a supervision scope attached to one workspace; a projection is a generated runtime view.\n\nEnvironment:\n  RUST_LOG  Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR  Disable automatic color output."
)]
struct Cli {
    /// Output encoding. JSON uses the stable `azoth.output.v1` envelope.
    #[arg(long, value_enum, default_value_t = CliOutputFormat::Text, global = true)]
    format: CliOutputFormat,

    /// Increase default log verbosity (`-v` info, `-vv` debug, `-vvv` trace).
    #[arg(short, long, action = ArgAction::Count, global = true)]
    verbose: u8,

    /// Restrict default diagnostics to errors. `RUST_LOG` can add directives.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// When to colorize diagnostic output.
    #[arg(long, value_enum, default_value_t = ColorArg::Auto, global = true)]
    color: ColorArg,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum CliOutputFormat {
    /// Human-readable command output.
    #[default]
    Text,
    /// Machine-readable output with every rendered line represented as a string.
    Json,
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

#[derive(Subcommand)]
enum Commands {
    /// Create a new game project
    New {
        /// Project name
        name: String,

        /// Project directory (defaults to ./<name>)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Lore server URL used to create and checkpoint the project repository
        #[arg(long, value_name = "URL")]
        lore_url: Option<String>,

        /// Durable project topology
        #[arg(long, value_enum, default_value_t = ProjectTopologyArg::SinglePlayer)]
        topology: ProjectTopologyArg,
    },

    /// Inspect or change the project's durable topology scaffold
    Topology {
        #[command(subcommand)]
        action: TopologyAction,
    },

    /// Inspect or regenerate disposable topology-derived Cargo targets
    Targets {
        #[command(subcommand)]
        action: TargetsAction,
    },

    /// Initialize Azoth project workflow in an existing directory
    Init {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Project name (defaults to the directory name)
        #[arg(short, long)]
        name: Option<String>,

        /// Lore server URL used to create and checkpoint the project repository
        #[arg(long, value_name = "URL")]
        lore_url: Option<String>,
    },

    /// Create and register project/gem plugin crates
    Gem {
        #[command(subcommand)]
        action: GemAction,
    },

    /// Project configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Provision machine-local secrets for generated project services
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },

    /// Refresh or verify the generated project lock file
    Lock {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Verify the committed lock without rewriting it
        #[arg(long)]
        check: bool,
    },

    /// Synchronize the selected engine and project-local derived state
    Engine {
        #[command(subcommand)]
        action: EngineAction,
    },

    /// Runtime supervision sessions attached to an existing workspace
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },

    /// Connect to the user-level azd daemon
    Daemon {
        /// Daemon endpoint transport kind
        #[arg(long, value_enum)]
        endpoint_kind: Option<EndpointKindArg>,

        /// Explicit daemon endpoint address
        #[arg(long)]
        endpoint: Option<String>,

        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Project information and status
    Info {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Project-scoped asset-registry maintenance
    AssetDb {
        #[command(subcommand)]
        action: AssetDbAction,
    },

    /// Remove regenerable Azoth data without touching Cargo artifacts or authored source
    Clean {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Include the durable asset database; with no scope flags, all scopes are cleaned
        #[arg(long)]
        assetdb: bool,

        /// Include cooked product caches; with no scope flags, all scopes are cleaned
        #[arg(long)]
        products: bool,

        /// Limit --products to one platform cache
        #[arg(long, value_name = "PLATFORM", requires = "products")]
        platform: Option<String>,

        /// Include generated graph, editor, and project-host derived state
        #[arg(long)]
        derived: bool,

        /// Include per-project endpoint records and sockets
        #[arg(long)]
        endpoints: bool,

        /// Include ephemeral session records; source-control workspaces always survive
        #[arg(long)]
        sessions: bool,
    },

    /// Build the project
    Build {
        /// Package/build profile (for example pc-dev or pc-release)
        #[arg(long, default_value = "pc-dev")]
        profile: String,

        /// Runtime asset output produced after asset processing
        #[arg(long, value_enum, default_value_t = BuildAssetOutput::Package)]
        asset_output: BuildAssetOutput,

        /// Generated target or package selector; repeat to build multiple packages
        #[arg(short = 'p', long = "package")]
        packages: Vec<String>,

        /// Session to use for package catalog reads
        #[arg(long)]
        session: Option<String>,

        /// Rust target triple; non-Windows hosts use cargo-xwin for Windows MSVC targets
        #[arg(short, long, value_name = "TRIPLE")]
        target: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// azd endpoint transport kind for daemon-backed build planning
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed build planning
        #[arg(long)]
        daemon_endpoint: Option<String>,
    },

    /// Launch the project runtime through the session service workflow
    Run {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Session workspace to launch from; inferred when exactly one active session exists
        #[arg(short, long)]
        session: Option<String>,

        /// Runtime instance id
        #[arg(long, default_value = "play-preview")]
        runtime_id: String,

        /// Runtime role
        #[arg(long, value_enum, default_value_t = RuntimeRoleArg::PlayPreview)]
        role: RuntimeRoleArg,

        /// Include unsaved project-host journal state in the launch snapshot
        #[arg(long)]
        include_unsaved_journal: bool,

        /// Runtime launch profile
        #[arg(long, default_value = "play")]
        launch_profile: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,
    },

    /// Open the editor
    Editor {
        /// Project directory to open; omit to show the project launcher
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Session workspace to attach to; requires --project
        #[arg(short, long)]
        session: Option<String>,

        /// Do not create/prepare/start the session service workflow before launching
        #[arg(long = "no-ensure-services", default_value_t = true, action = clap::ArgAction::SetFalse)]
        ensure_services: bool,

        /// azd endpoint transport kind for daemon-backed editor attach
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed editor attach
        #[arg(long)]
        daemon_endpoint: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum EndpointKindArg {
    InProcess,
    WindowsNamedPipe,
    UnixDomainSocket,
    Tcp,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RuntimeRoleArg {
    EditorWorld,
    PlayPreview,
    ServerPreview,
    Validation,
    Thumbnail,
    Bake,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProjectTopologyArg {
    SinglePlayer,
    MultiplayerClientServer,
    MultiplayerPeerToPeer,
}

impl From<ProjectTopologyArg> for az_project::ProjectTopologyKind {
    fn from(value: ProjectTopologyArg) -> Self {
        match value {
            ProjectTopologyArg::SinglePlayer => Self::SinglePlayer,
            ProjectTopologyArg::MultiplayerClientServer => Self::MultiplayerClientServer,
            ProjectTopologyArg::MultiplayerPeerToPeer => Self::MultiplayerPeerToPeer,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ServiceLogStreamArg {
    Stdout,
    Stderr,
    Structured,
}

#[derive(Subcommand)]
enum GemAction {
    /// Create a path-backed gem crate and optionally register it with a project
    New {
        /// Gem display/package name
        name: String,

        /// Project directory whose azoth.toml should reference the gem
        #[arg(long = "project", alias = "project-path", value_name = "DIR")]
        project_path: Option<PathBuf>,

        /// Gem directory relative to the project path; defaults to gems/<name>
        #[arg(long)]
        path: Option<PathBuf>,

        /// Stable gem id for azoth.toml/gem.toml; defaults to local.<name>
        #[arg(long)]
        id: Option<String>,

        /// Cargo package name for the gem crate; defaults to <name>
        #[arg(long)]
        package: Option<String>,

        /// Create the gem without adding it to azoth.toml or project Cargo linkage
        #[arg(long = "no-register", default_value_t = true, action = clap::ArgAction::SetFalse)]
        register: bool,

        /// ADR 0026 auth entry-point capability to scaffold (`provider` or
        /// `session-authority`); may be repeated, but only one is supported at
        /// a time. Omit for a generic gem.
        #[arg(long = "capability")]
        capabilities: Vec<String>,
    },

    /// Register an existing path-backed gem crate with a project
    Register {
        /// Project directory whose azoth.toml should reference the gem
        #[arg(long = "project", alias = "project-path", value_name = "DIR")]
        project_path: Option<PathBuf>,

        /// Existing gem directory relative to the project path
        path: PathBuf,

        /// Expected stable gem id; defaults to the id in gem.toml
        #[arg(long)]
        id: Option<String>,

        /// Expected Cargo package name; defaults to [package].name in Cargo.toml
        #[arg(long)]
        package: Option<String>,
    },

    /// Enable an engine-registered gem for a project
    Enable {
        /// Stable engine gem id from engine.toml
        id: String,

        /// Gem capability to enable; may be repeated
        #[arg(long = "capability")]
        capabilities: Vec<String>,

        /// Project directory whose azoth.toml should enable the gem
        #[arg(long = "project", alias = "project-path", value_name = "DIR")]
        project_path: Option<PathBuf>,
    },

    /// List project-registered gems resolved through azoth.toml/gem.toml
    List {
        /// Project directory whose azoth.toml declares gems
        #[arg(long = "project", alias = "project-path", value_name = "DIR")]
        project_path: Option<PathBuf>,

        /// Include disabled gem declarations without resolving them
        #[arg(long)]
        all: bool,
    },

    /// Validate enabled project gem declarations and manifests
    Validate {
        /// Project directory whose azoth.toml declares gems
        #[arg(long = "project", alias = "project-path", value_name = "DIR")]
        project_path: Option<PathBuf>,
    },

    /// Generate committed per-gem id-constant crates from gem.toml
    Ids {
        /// Project directory whose azoth.toml declares path-backed gems
        #[arg(long = "project", alias = "project-path", value_name = "DIR")]
        project_path: Option<PathBuf>,

        /// Generate ids crates for the engine and its catalog gems instead of
        /// project gems
        #[arg(long)]
        engine: bool,

        /// Verify freshness without writing; exit nonzero when any crate is stale
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum TopologyAction {
    /// Add destination role packages and persist the selected topology
    Set {
        /// Stable topology ID
        #[arg(value_enum)]
        topology: ProjectTopologyArg,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Delete inactive role packages after safety checks and confirmation
    Prune {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Skip the interactive confirmation (modified/referenced packages still fail)
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum TargetsAction {
    /// List the generated target matrix and fingerprint freshness
    List {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Force regeneration of the generated target workspace
    Regenerate {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Launch the foreground azd process
    Serve {
        /// Project root to register on startup; may be repeated
        #[arg(long = "project", value_name = "DIR")]
        projects: Vec<PathBuf>,

        /// Durable project registry TOML path
        #[arg(long = "project-registry")]
        project_registry: Option<PathBuf>,
    },

    /// Launch azd in the background and wait for its endpoint record
    Start {
        /// Project root to register on startup; may be repeated
        #[arg(long = "project", value_name = "DIR")]
        projects: Vec<PathBuf>,

        /// Durable project registry TOML path
        #[arg(long = "project-registry")]
        project_registry: Option<PathBuf>,

        /// Milliseconds to wait for azd to publish a reachable endpoint
        #[arg(
            long,
            default_value_t = commands::daemon::DEFAULT_DAEMON_START_TIMEOUT_MS
        )]
        timeout_ms: u64,
    },

    /// Ask the running azd process to shut down gracefully
    Stop {
        /// Operator-visible shutdown reason
        #[arg(long)]
        reason: Option<String>,

        /// Project root whose project-scoped azd endpoint should be stopped; may be repeated
        #[arg(long = "project", value_name = "DIR")]
        projects: Vec<PathBuf>,
    },

    /// Register a project root with a running azd process
    RegisterProject {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Forget a project registration; the project itself is untouched
    ForgetProject {
        /// Project id as shown by `azoth daemon list-projects`
        project_id: String,
    },

    /// List projects registered with azd
    ListProjects,

    /// Show the published azd endpoint record or an explicitly resolved endpoint
    Status,

    /// Resolve one registered project by id
    ResolveProject {
        /// Project id from azoth.toml
        project_id: String,
    },

    /// Inspect session supervisors registered with azd
    Supervisors {
        #[command(subcommand)]
        action: DaemonSupervisorsAction,
    },

    /// Plan project build and service commands through azd
    Plan {
        #[command(subcommand)]
        action: DaemonPlanAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Get a configuration value
    Get {
        /// Configuration key (for example, project.name or paths.assets)
        key: String,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Set a configuration value
    Set {
        /// Configuration key
        key: String,

        /// Configuration value
        value: String,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// List all configuration values
    List {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SecretAction {
    /// Atomically provision or rotate a typed project secret
    Set {
        /// Typed reference retained by project configuration (secret://...)
        reference: String,

        /// Read secret bytes from this file
        #[arg(long, value_name = "FILE")]
        from_file: PathBuf,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Print the machine-local path for a typed project secret
    Path {
        /// Typed secret reference (secret://...)
        reference: String,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum EngineAction {
    /// Force regeneration of the project-local engine patch table
    Sync {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AssetDbAction {
    /// Complete one-time project asset storage migration before service startup
    Migrate {
        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Compact bounded operational asset history with the processor stopped
    Compact {
        /// Days of superseded completed-attempt history to retain
        #[arg(long, default_value_t = 30)]
        finished_attempt_retention_days: i64,

        /// Days of closed asset-path history to retain
        #[arg(long, default_value_t = 180)]
        closed_path_retention_days: i64,

        /// Report the maintenance plan without changing the database
        #[arg(long)]
        dry_run: bool,

        /// Reclaim free database pages after compaction
        #[arg(long)]
        vacuum: bool,

        /// Rebuild database indexes after compaction
        #[arg(long)]
        reindex: bool,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Report or reclaim stale workspace views with the processor stopped
    GcViews {
        /// Explicitly report stale views without changing registry rows (the default)
        #[arg(long, conflicts_with = "apply")]
        dry_run: bool,

        /// Apply the reported bounded reclamation plan
        #[arg(long, conflicts_with = "dry_run")]
        apply: bool,

        /// Maximum rows removed from each dependency tier per transaction
        #[arg(long, default_value_t = NonZeroU32::new(1_024).unwrap())]
        batch_size: NonZeroU32,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// Create a supervision scope over the existing project workspace
    Create {
        /// Session name
        name: String,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// List active sessions
    List {
        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Open a session in the editor
    Open {
        /// Session name
        name: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Run a command inside the workspace referenced by a validated session
    #[command(trailing_var_arg = true)]
    Exec {
        /// Session name
        name: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Command and arguments to run after `--`
        #[arg(
            required = true,
            trailing_var_arg = true,
            allow_hyphen_values = true,
            value_name = "COMMAND"
        )]
        command: Vec<String>,
    },

    /// Show session status
    Status {
        /// Session name
        name: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Validate that a session manifest still points at the expected Lore workspace
    Validate {
        /// Session name
        name: String,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Retire a session supervision scope
    Retire {
        /// Session name
        name: String,

        /// Do not ask the session supervisor to stop live services before retiring
        #[arg(long = "no-stop-services", default_value_t = true, action = clap::ArgAction::SetFalse)]
        stop_services: bool,

        /// Milliseconds to wait for live services to record clean exits
        #[arg(long, default_value_t = 30_000)]
        service_shutdown_timeout_ms: u64,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Stop every session supervisor running this CLI's own `az-sessiond` image
    Sweep {
        /// Shutdown reason recorded by each swept session supervisor
        #[arg(long)]
        reason: Option<String>,

        /// Milliseconds to wait for swept supervisors to exit
        #[arg(long, default_value_t = 30_000)]
        service_shutdown_timeout_ms: u64,
    },

    /// Manage session project services (planning, supervision, lifecycle, registration)
    Services {
        #[command(subcommand)]
        action: SessionServicesAction,
    },

    /// Inspect and edit project-host source documents for a session
    Document {
        #[command(subcommand)]
        action: SessionDocumentAction,
    },

    /// Inspect session asset-processor state, products, and job attempts
    Asset {
        #[command(subcommand)]
        action: SessionAssetAction,
    },

    /// Launch and inspect runtime projections for a session
    Runtime {
        #[command(subcommand)]
        action: SessionRuntimeAction,
    },

    /// Clear a failed-preserved session after the cause has been repaired
    Recover {
        /// Session name
        name: String,

        /// Clear recovery even if project-host still has a failed process record
        #[arg(long)]
        force: bool,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

/// Session service lifecycle subcommands.
///
/// Flattened into the parent subcommand list alongside [`SessionServicesReportAction`], so the CLI surface is a
/// single flat set of subcommands.
#[derive(Subcommand)]
enum SessionServicesAction {
    /// Plan project services and register their session endpoints
    Prepare {
        /// Session name
        name: String,

        /// Endpoint transport kind (defaults to named pipes on Windows, Unix sockets elsewhere)
        #[arg(long, value_enum)]
        kind: Option<EndpointKindArg>,

        /// Allow replanning services for a failed-preserved session during recovery
        #[arg(long)]
        recover: bool,

        /// Optional OTLP endpoint forwarded into planned project service processes
        #[arg(long)]
        otlp_endpoint: Option<String>,

        /// azd endpoint transport kind for daemon-backed service planning
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed service planning
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Only prepare the named service and its required dependencies; repeat for multiple services
        #[arg(long = "service")]
        services: Vec<String>,
    },

    /// Start planned project services and monitor them in the foreground
    Supervise {
        /// Session name
        name: String,

        /// Session-supervisor endpoint transport kind
        #[arg(long, value_enum)]
        session_supervisor_kind: Option<EndpointKindArg>,

        /// Explicit session-supervisor endpoint address
        #[arg(long)]
        session_supervisor_endpoint: Option<String>,

        /// Optional OTLP endpoint forwarded into az-sessiond
        #[arg(long)]
        otlp_endpoint: Option<String>,

        /// azd endpoint transport kind for publishing session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for publishing session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Start planned project services in the background
    Start {
        /// Session name
        name: String,

        /// Session-supervisor endpoint transport kind
        #[arg(long, value_enum)]
        session_supervisor_kind: Option<EndpointKindArg>,

        /// Explicit session-supervisor endpoint address
        #[arg(long)]
        session_supervisor_endpoint: Option<String>,

        /// Optional OTLP endpoint forwarded into az-sessiond
        #[arg(long)]
        otlp_endpoint: Option<String>,

        /// Milliseconds to wait for the session-supervisor RPC to answer
        #[arg(long, default_value_t = DEFAULT_SESSION_SERVICE_START_TIMEOUT_MS)]
        timeout_ms: u64,

        /// azd endpoint transport kind for publishing session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for publishing session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,

        /// Only start the named planned service; repeat to start multiple services
        #[arg(long = "service")]
        services: Vec<String>,
    },

    /// Request graceful shutdown of session-owned project services
    Stop {
        /// Session name
        name: String,

        /// Shutdown reason recorded by the session supervisor
        #[arg(long)]
        reason: Option<String>,

        /// Return after sending the shutdown request instead of waiting for recorded exits
        #[arg(long = "no-wait", default_value_t = true, action = clap::ArgAction::SetFalse)]
        wait: bool,

        /// Milliseconds to wait for live services to record clean exits
        #[arg(long, default_value_t = 30_000)]
        service_shutdown_timeout_ms: u64,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
    #[command(flatten)]
    Report(SessionServicesReportAction),
}

/// Session service inspection subcommands.
#[derive(Subcommand)]
enum SessionServicesReportAction {
    /// Inspect registered services, process state, and service logs for a session
    Status {
        /// Session name
        name: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Print a recorded stdout, stderr, or structured log for one session-owned service process
    Log {
        /// Session name
        name: String,

        /// Service process name, e.g. project-host or asset-processor
        service: String,

        /// Specific retained service run to read
        #[arg(long)]
        run: Option<uuid::Uuid>,

        /// Log stream to print
        #[arg(long, value_enum, default_value_t = ServiceLogStreamArg::Stderr)]
        stream: ServiceLogStreamArg,

        /// Number of trailing lines to print
        #[arg(long, default_value_t = 200)]
        tail: usize,

        /// Print the whole log instead of only the trailing window
        #[arg(long)]
        all: bool,

        /// Keep the command open and stream new log lines after printing the selected tail
        #[arg(long)]
        follow: bool,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Register a runtime-host endpoint for a session
    #[command(hide = true)]
    RegisterRuntimeHost {
        /// Session name
        name: String,

        /// Endpoint address, e.g. a named pipe, socket path, or TCP address
        endpoint: String,

        /// Endpoint transport kind
        #[arg(long, value_enum)]
        kind: EndpointKindArg,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

/// Authored document read/write subcommands.
///
/// Flattened into the parent subcommand list alongside [`SessionDocumentHistoryAction`], so the CLI surface is a
/// single flat set of subcommands.
#[derive(Subcommand)]
enum SessionDocumentAction {
    /// Save an open source session through project-host
    Save {
        /// Session name
        name: String,

        /// Project-relative source path, e.g. prefabs/door.prefab.ron
        document: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Open a project-host source session
    Load {
        /// Session name
        name: String,

        /// Project-relative source path, e.g. prefabs/door.prefab.ron
        document: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Print a project-host prefab source snapshot
    Snapshot {
        /// Session name
        name: String,

        /// Project-relative prefab source path, e.g. prefabs/door.prefab.ron
        document: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Print project-host source-session status
    Status {
        /// Session name
        name: String,

        /// Project-relative source path, e.g. prefabs/door.prefab.ron
        document: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
    #[command(flatten)]
    Report(SessionDocumentHistoryAction),
}

/// Authored document history subcommands.
#[derive(Subcommand)]
enum SessionDocumentHistoryAction {
    /// Undo the latest edit in a project-host source session
    Undo {
        /// Session name
        name: String,

        /// Project-relative source path, e.g. prefabs/door.prefab.ron
        document: String,

        /// Expected source revision; defaults to project-host source-session status revision
        #[arg(long)]
        expected_revision: Option<u64>,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Redo the latest undo in a project-host source session
    Redo {
        /// Session name
        name: String,

        /// Project-relative source path, e.g. prefabs/door.prefab.ron
        document: String,

        /// Expected source revision; defaults to project-host source-session status revision
        #[arg(long)]
        expected_revision: Option<u64>,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Create a DB-authored file-backed source through the asset-processor workflow
    Create {
        /// Session name
        name: String,

        /// Asset-db relative source path, e.g. achievementdata/achievementdatatable.ron
        source_path: String,

        /// Source schema type registered by asset-processor
        #[arg(long)]
        schema_type: String,

        /// Import bytes from this local file instead of using a registered default template
        #[arg(long = "from")]
        from: Option<PathBuf>,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

/// Asset subcommands that drive processing.
///
/// Flattened into the parent subcommand list alongside [`SessionAssetReportAction`], so the CLI surface is a
/// single flat set of subcommands.
#[derive(Subcommand)]
enum SessionAssetAction {
    /// Process all project asset sources and publish the native runtime catalog
    Process {
        /// Session name
        name: String,

        /// Asset platform to process
        #[arg(long, default_value = "pc")]
        platform: String,

        /// Reuse already-built asset services instead of rebuilding generated targets
        #[arg(long)]
        skip_build: bool,

        /// azd endpoint transport kind
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Import one or more existing authored source files and enqueue their asset work
    Record {
        /// Session name
        name: String,

        /// Asset-db relative source paths
        #[arg(required = true)]
        source_paths: Vec<String>,

        /// Registered source schema type shared by the recorded files
        #[arg(long)]
        schema_type: String,

        /// Registered source-root key
        #[arg(long, default_value = "project:source-root")]
        source_root: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Force-reprocess one or more current source assets
    Reprocess {
        /// Session name
        name: String,

        /// Asset-db relative source paths
        #[arg(required = true)]
        source_paths: Vec<String>,

        /// Registered source-root key
        #[arg(long, default_value = "project:source-root")]
        source_root: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Reconcile registered authored sources without draining the processing queue
    Reconcile {
        /// Session name
        name: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
    #[command(flatten)]
    Report(SessionAssetReportAction),
}

/// Asset subcommands that report state.
#[derive(Subcommand)]
enum SessionAssetReportAction {
    /// Read a page of entries from the attached workspace
    Status {
        /// Session name
        name: String,

        /// Last workspace asset entry id from a previous page
        #[arg(long)]
        after: Option<i64>,

        /// Maximum entries to return
        #[arg(long, default_value_t = 50)]
        limit: u32,

        /// Follow cursor pages until all matching workspace assets are printed
        #[arg(long)]
        all: bool,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Read aggregate asset processing queue state for a session/platform
    ProcessingStatus {
        /// Session name
        name: String,

        /// Asset platform to query
        #[arg(long, default_value = "pc")]
        platform: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Read the asset-processor service health and active operation
    Health {
        /// Session name
        name: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Read the attached workspace snapshot for a session
    Snapshot {
        /// Session name
        name: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Read the registered asset builders for the project asset processor
    Builders {
        /// Session name
        name: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
    #[command(flatten)]
    Catalog(SessionAssetCatalogAction),
}

/// Asset subcommands that read the published product catalog and job records.
#[derive(Subcommand)]
enum SessionAssetCatalogAction {
    /// Read catalog products for a session/platform
    Products {
        /// Session name
        name: String,

        /// Asset platform to query
        #[arg(long, default_value = "pc")]
        platform: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Force-publish runtime assetcatalog.bin into Cache/<platform>
    PublishCatalog {
        /// Session name
        name: String,

        /// Asset platform whose catalog products become the catalog
        #[arg(long, default_value = "pc")]
        platform: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Inspect one asset-processor job and its selected attempt, products, and dependencies
    Job {
        /// Session name
        name: String,

        /// Job id to inspect
        #[arg(long, conflicts_with = "attempt_id")]
        job_id: Option<i64>,

        /// Attempt id to inspect
        #[arg(long, conflicts_with = "job_id")]
        attempt_id: Option<i64>,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Read the registered runtime projections for a session runtime-host
    Projections {
        /// Session name
        name: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SessionRuntimeAction {
    /// Launch a runtime projection through project-host and runtime-host RPC
    Launch {
        /// Session name
        name: String,

        /// Runtime instance id
        #[arg(long, default_value = "editor-world")]
        runtime_id: String,

        /// Runtime role
        #[arg(long, value_enum, default_value_t = RuntimeRoleArg::EditorWorld)]
        role: RuntimeRoleArg,

        /// Include unsaved project-host journal state in the launch snapshot
        #[arg(long)]
        include_unsaved_journal: bool,

        /// Runtime launch profile
        #[arg(long, default_value = "editor")]
        launch_profile: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Query a runtime projection through runtime-host RPC
    Status {
        /// Session name
        name: String,

        /// Runtime instance id
        #[arg(long, default_value = "editor-world")]
        runtime_id: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Read the latest runtime viewport frame metadata through runtime-host RPC
    ViewportFrame {
        /// Session name
        name: String,

        /// Runtime instance id
        #[arg(long, default_value = "editor-world")]
        runtime_id: String,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },

    /// Stop a runtime projection through runtime-host RPC
    Stop {
        /// Session name
        name: String,

        /// Runtime instance id
        #[arg(long, default_value = "editor-world")]
        runtime_id: String,

        /// Preserve runtime state for inspection
        #[arg(long)]
        preserve: bool,

        /// azd endpoint transport kind for daemon-backed session-supervisor discovery
        #[arg(long, value_enum)]
        daemon_endpoint_kind: Option<EndpointKindArg>,

        /// Explicit azd endpoint address for daemon-backed session-supervisor discovery
        #[arg(long)]
        daemon_endpoint: Option<String>,

        /// Project directory (defaults to current directory)
        #[arg(long = "project", alias = "path", value_name = "DIR")]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum DaemonSupervisorsAction {
    /// List session supervisors registered under one project
    List {
        /// Project id from azoth.toml
        project_id: String,
    },

    /// Resolve one session supervisor registered under one project
    Resolve {
        /// Project id from azoth.toml
        project_id: String,

        /// Session slug/name
        session: String,
    },

    /// Remove a session supervisor descriptor from azd if it still matches
    Unregister {
        /// Project id from azoth.toml
        project_id: String,

        /// Session slug/name
        session: String,
    },

    /// Remove unreachable session supervisor descriptors from azd
    Prune {
        /// Project id from azoth.toml
        project_id: String,
    },
}

#[derive(Subcommand)]
enum DaemonPlanAction {
    /// Plan project Cargo build commands through azd
    Build {
        /// Project id from azoth.toml
        project_id: String,

        /// Package/build profile
        #[arg(long, default_value = "pc-dev")]
        profile: String,

        /// Generated target or package selector; repeat to plan multiple packages
        #[arg(short = 'p', long = "package")]
        packages: Vec<String>,

        /// Target platform/triple
        #[arg(short, long)]
        target: Option<String>,
    },

    /// Plan project service build and launch commands through azd
    Services {
        /// Project id from azoth.toml
        project_id: String,

        /// Session slug/name
        session: String,

        /// Endpoint kind to assign to planned project services
        #[arg(long, value_enum)]
        service_endpoint_kind: Option<EndpointKindArg>,
    },
}

fn run_json_child() -> CliResult<()> {
    let args = std::env::args_os().collect::<Vec<_>>();
    let Some(executable) = args.first() else {
        return Ok(());
    };
    let output = ProcessCommand::new(executable)
        .args(&args[1..])
        .env("AZOTH_JSON_CHILD", "1")
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    let document = serde_json::json!({
        "schema": "azoth.output.v1",
        "command": args[1..]
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        "success": output.status.success(),
        "exit_code": output.status.code(),
        "lines": stdout.lines().collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&document).expect("serialize azoth output envelope")
    );

    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
    Ok(())
}

fn main() -> CliResult<()> {
    let cli = Cli::parse();
    install_service_observability(
        ServiceObservabilityConfig::new(ObservedLogContext::new(
            ServiceId::new("azoth", "cli"),
            ServiceRole::Unknown,
            uuid::Uuid::now_v7(),
        ))
        .with_default_directives(default_log_directives(cli.verbose, cli.quiet))
        .with_ansi(cli.color.stderr_ansi()),
    )?;

    if cli.format == CliOutputFormat::Json && std::env::var_os("AZOTH_JSON_CHILD").is_none() {
        return run_json_child();
    }

    dispatch(cli.command)
}

fn dispatch(command: Commands) -> CliResult<()> {
    match command {
        Commands::New {
            name,
            path,
            lore_url,
            topology,
        } => commands::new::execute(name, path, lore_url, topology.into()),

        Commands::Topology { action } => dispatch_topology(action),

        Commands::Targets { action } => dispatch_targets(action),

        Commands::Init {
            path,
            name,
            lore_url,
        } => commands::init::execute(path, name, lore_url),

        Commands::Gem { action } => dispatch_gem(action),

        Commands::Config { action } => dispatch_config(action),

        Commands::Secret { action } => dispatch_secret(action),

        Commands::Lock { path, check } => commands::config::lock(path, check),

        Commands::Engine { action } => dispatch_engine(action),

        Commands::Session { action } => dispatch_session(action),

        Commands::Daemon {
            endpoint_kind,
            endpoint,
            action,
        } => dispatch_daemon(endpoint_kind, endpoint, action),

        Commands::Info { path } => commands::info::execute(path),
        Commands::AssetDb { action } => dispatch_assetdb(action),

        Commands::Clean {
            path,
            assetdb,
            products,
            platform,
            derived,
            endpoints,
            sessions,
        } => commands::clean::execute(&commands::clean::CleanOptions {
            path,
            scopes: commands::clean::CleanScopes::from_flags([
                assetdb, products, derived, endpoints, sessions,
            ]),
            platform,
        }),

        Commands::Build {
            profile,
            asset_output,
            packages,
            session,
            target,
            path,
            daemon_endpoint_kind,
            daemon_endpoint,
        } => commands::build::execute(commands::build::BuildOptions {
            profile,
            asset_output,
            package_selectors: packages,
            session,
            target,
            path,
            daemon_endpoint_kind: daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
        }),

        Commands::Run {
            path,
            session,
            runtime_id,
            role,
            include_unsaved_journal,
            launch_profile,
            daemon_endpoint_kind,
            daemon_endpoint,
        } => commands::run::execute(
            session,
            &runtime_id,
            role.into(),
            include_unsaved_journal,
            &launch_profile,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),

        Commands::Editor {
            path,
            session,
            ensure_services,
            daemon_endpoint_kind,
            daemon_endpoint,
        } => commands::editor::execute(
            path,
            session,
            ensure_services,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
        ),
    }
}

fn dispatch_topology(action: TopologyAction) -> CliResult<()> {
    match action {
        TopologyAction::Set { topology, path } => commands::topology::set(topology.into(), path),
        TopologyAction::Prune { path, force } => commands::topology::prune(path, force),
    }
}

fn dispatch_targets(action: TargetsAction) -> CliResult<()> {
    match action {
        TargetsAction::List { path } => commands::targets::list(path),
        TargetsAction::Regenerate { path } => commands::targets::regenerate(path),
    }
}

fn dispatch_config(action: ConfigAction) -> CliResult<()> {
    match action {
        ConfigAction::Get { key, path } => commands::config::get(&key, path),
        ConfigAction::Set { key, value, path } => commands::config::set(&key, &value, path),
        ConfigAction::List { path } => commands::config::list(path),
    }
}

fn dispatch_secret(action: SecretAction) -> CliResult<()> {
    match action {
        SecretAction::Set {
            reference,
            from_file,
            path,
        } => commands::secret::set(&reference, &from_file, path),
        SecretAction::Path { reference, path } => commands::secret::path(&reference, path),
    }
}

fn dispatch_engine(action: EngineAction) -> CliResult<()> {
    match action {
        EngineAction::Sync { path } => commands::engine::sync(path),
    }
}

fn dispatch_assetdb(action: AssetDbAction) -> CliResult<()> {
    match action {
        AssetDbAction::Migrate { path } => commands::assetdb::migrate(path),
        AssetDbAction::Compact {
            finished_attempt_retention_days,
            closed_path_retention_days,
            dry_run,
            vacuum,
            reindex,
            path,
        } => commands::assetdb::maintain(
            path,
            &commands::assetdb::AssetDbMaintenanceAction::Compact {
                finished_attempt_retention_days,
                closed_path_retention_days,
                dry_run,
                vacuum,
                reindex,
            },
        ),
        AssetDbAction::GcViews {
            dry_run,
            apply,
            batch_size,
            path,
        } => commands::assetdb::maintain(
            path,
            &commands::assetdb::AssetDbMaintenanceAction::GcViews {
                dry_run,
                apply,
                batch_size: batch_size.get(),
            },
        ),
    }
}

fn dispatch_gem(action: GemAction) -> CliResult<()> {
    match action {
        GemAction::New {
            name,
            project_path,
            path,
            id,
            package,
            register,
            capabilities,
        } => commands::gem::new_gem(
            project_path,
            name,
            path,
            id,
            package,
            register,
            &capabilities,
        ),
        GemAction::Register {
            project_path,
            path,
            id,
            package,
        } => commands::gem::register(project_path, path, id, package),
        GemAction::Enable {
            id,
            capabilities,
            project_path,
        } => commands::gem::enable_engine(project_path, id, capabilities),
        GemAction::List { project_path, all } => commands::gem::list(project_path, all),
        GemAction::Validate { project_path } => commands::gem::validate(project_path),
        GemAction::Ids {
            project_path,
            engine,
            check,
        } => commands::gem::generate_ids(project_path, engine, check),
    }
}

fn dispatch_session(action: SessionAction) -> CliResult<()> {
    match action {
        SessionAction::Create { name, path } => commands::session::create(name, path),
        SessionAction::List {
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::list(daemon_endpoint_kind.map(Into::into), daemon_endpoint, path),
        SessionAction::Open {
            name,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::open(
            &name,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAction::Exec {
            name,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
            command,
        } => commands::session::exec(
            &name,
            command,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAction::Status {
            name,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::status(
            &name,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAction::Validate { name, path } => commands::session::validate(&name, path),
        SessionAction::Retire {
            name,
            stop_services,
            service_shutdown_timeout_ms,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::retire(
            &name,
            stop_services,
            service_shutdown_timeout_ms,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAction::Sweep {
            reason,
            service_shutdown_timeout_ms,
        } => commands::session::sweep(reason, service_shutdown_timeout_ms),
        SessionAction::Services { action } => dispatch_session_services(action),
        SessionAction::Document { action } => dispatch_session_document(action),
        SessionAction::Asset { action } => dispatch_session_asset(action),
        SessionAction::Runtime { action } => dispatch_session_runtime(action),
        SessionAction::Recover {
            name,
            force,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::recover(
            &name,
            force,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
    }
}

fn dispatch_session_services(action: SessionServicesAction) -> CliResult<()> {
    match action {
        SessionServicesAction::Prepare {
            name,
            kind,
            recover,
            otlp_endpoint,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
            services,
        } => commands::session::prepare_services(commands::session::PrepareServicesOptions {
            name,
            kind: kind.map(Into::into),
            recover,
            otlp_endpoint,
            daemon_endpoint_kind: daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
            service_names: services,
        }),
        SessionServicesAction::Supervise {
            name,
            session_supervisor_kind,
            session_supervisor_endpoint,
            otlp_endpoint,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::supervise_services(
            &name,
            session_supervisor_kind.map(Into::into),
            session_supervisor_endpoint.as_deref(),
            otlp_endpoint.as_deref(),
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionServicesAction::Start {
            name,
            session_supervisor_kind,
            session_supervisor_endpoint,
            otlp_endpoint,
            timeout_ms,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
            services,
        } => commands::session::start_services(commands::session::StartServicesOptions {
            name,
            session_supervisor_kind: session_supervisor_kind.map(Into::into),
            session_supervisor_endpoint,
            otlp_endpoint,
            timeout_ms,
            daemon_endpoint_kind: daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
            service_names: services,
        }),
        SessionServicesAction::Stop {
            name,
            reason,
            wait,
            service_shutdown_timeout_ms,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::stop_services(
            &name,
            reason,
            wait,
            service_shutdown_timeout_ms,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionServicesAction::Report(action) => dispatch_session_services_report(action),
    }
}

fn dispatch_session_services_report(action: SessionServicesReportAction) -> CliResult<()> {
    match action {
        SessionServicesReportAction::Status {
            name,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::services(
            &name,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionServicesReportAction::Log {
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
        } => commands::session::service_log(commands::session::ServiceLogOptions {
            name,
            service,
            run,
            stream,
            tail,
            all,
            follow,
            daemon_endpoint_kind: daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        }),
        SessionServicesReportAction::RegisterRuntimeHost {
            name,
            endpoint,
            kind,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::register_runtime_host(
            &name,
            endpoint,
            kind.into(),
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
    }
}

fn dispatch_session_document(action: SessionDocumentAction) -> CliResult<()> {
    match action {
        SessionDocumentAction::Save {
            name,
            document,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::save_document(
            &name,
            &document,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionDocumentAction::Load {
            name,
            document,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::document_load(
            &name,
            &document,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionDocumentAction::Snapshot {
            name,
            document,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::document_snapshot(
            &name,
            &document,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionDocumentAction::Status {
            name,
            document,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::document_status(
            &name,
            &document,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionDocumentAction::Report(action) => dispatch_session_document_history(action),
    }
}

fn dispatch_session_document_history(action: SessionDocumentHistoryAction) -> CliResult<()> {
    match action {
        SessionDocumentHistoryAction::Undo {
            name,
            document,
            expected_revision,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::undo_document(
            &name,
            &document,
            expected_revision,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionDocumentHistoryAction::Redo {
            name,
            document,
            expected_revision,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::redo_document(
            &name,
            &document,
            expected_revision,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionDocumentHistoryAction::Create {
            name,
            source_path,
            schema_type,
            from,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::create_source_file(
            &name,
            &source_path,
            &schema_type,
            from,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
    }
}

fn dispatch_session_asset(action: SessionAssetAction) -> CliResult<()> {
    match action {
        SessionAssetAction::Process {
            name,
            platform,
            skip_build,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::process_assets(
            &name,
            &platform,
            skip_build,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint.as_deref(),
            path,
        ),
        SessionAssetAction::Record {
            name,
            source_paths,
            schema_type,
            source_root,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::record_source_assets(
            &name,
            &source_paths,
            &schema_type,
            &source_root,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetAction::Reprocess {
            name,
            source_paths,
            source_root,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::force_reprocess_assets(
            &name,
            &source_paths,
            &source_root,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetAction::Reconcile {
            name,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::reconcile_asset_sources(
            &name,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetAction::Report(action) => dispatch_session_asset_report(action),
    }
}

fn dispatch_session_asset_report(action: SessionAssetReportAction) -> CliResult<()> {
    match action {
        SessionAssetReportAction::Status {
            name,
            after,
            limit,
            all,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::workspace_entries(
            &name,
            after,
            limit,
            all,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetReportAction::ProcessingStatus {
            name,
            platform,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::asset_processing_status(
            &name,
            &platform,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetReportAction::Health {
            name,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::asset_health(
            &name,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetReportAction::Snapshot {
            name,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::workspace_snapshot(
            &name,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetReportAction::Builders {
            name,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::asset_builders(
            &name,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetReportAction::Catalog(action) => dispatch_session_asset_catalog(action),
    }
}

fn dispatch_session_asset_catalog(action: SessionAssetCatalogAction) -> CliResult<()> {
    match action {
        SessionAssetCatalogAction::Products {
            name,
            platform,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::catalog_products(
            &name,
            &platform,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetCatalogAction::PublishCatalog {
            name,
            platform,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::publish_asset_catalog(
            &name,
            &platform,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetCatalogAction::Job {
            name,
            job_id,
            attempt_id,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::inspect_job(
            &name,
            job_id,
            attempt_id,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionAssetCatalogAction::Projections {
            name,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::runtime_projections(
            &name,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
    }
}

fn dispatch_session_runtime(action: SessionRuntimeAction) -> CliResult<()> {
    match action {
        SessionRuntimeAction::Launch {
            name,
            runtime_id,
            role,
            include_unsaved_journal,
            launch_profile,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::launch_runtime(
            &name,
            &runtime_id,
            role.into(),
            include_unsaved_journal,
            &launch_profile,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionRuntimeAction::Status {
            name,
            runtime_id,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::runtime_status(
            &name,
            &runtime_id,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionRuntimeAction::ViewportFrame {
            name,
            runtime_id,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::runtime_viewport_frame(
            &name,
            &runtime_id,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
        SessionRuntimeAction::Stop {
            name,
            runtime_id,
            preserve,
            daemon_endpoint_kind,
            daemon_endpoint,
            path,
        } => commands::session::stop_runtime(
            &name,
            &runtime_id,
            preserve,
            daemon_endpoint_kind.map(Into::into),
            daemon_endpoint,
            path,
        ),
    }
}

fn dispatch_daemon(
    endpoint_kind: Option<EndpointKindArg>,
    endpoint: Option<String>,
    action: DaemonAction,
) -> CliResult<()> {
    match action {
        DaemonAction::Serve {
            projects,
            project_registry,
        } => commands::daemon::serve(
            &projects,
            project_registry.as_deref(),
            endpoint_kind.map(Into::into),
            endpoint.as_deref(),
        ),
        DaemonAction::Start {
            projects,
            project_registry,
            timeout_ms,
        } => commands::daemon::start(
            &projects,
            project_registry.as_deref(),
            timeout_ms,
            endpoint_kind.map(Into::into),
            endpoint.as_deref(),
        ),
        DaemonAction::Stop { reason, projects } => {
            commands::daemon::stop(reason, projects, endpoint_kind.map(Into::into), endpoint)
        }
        DaemonAction::RegisterProject { path } => {
            commands::daemon::register_project(path, endpoint_kind.map(Into::into), endpoint)
        }
        DaemonAction::ForgetProject { project_id } => commands::daemon::forget_project(&project_id),
        DaemonAction::ListProjects => {
            commands::daemon::list_projects(endpoint_kind.map(Into::into), endpoint)
        }
        DaemonAction::Status => commands::daemon::status(endpoint_kind.map(Into::into), endpoint),
        DaemonAction::ResolveProject { project_id } => {
            commands::daemon::resolve_project(project_id, endpoint_kind.map(Into::into), endpoint)
        }
        DaemonAction::Supervisors { action } => {
            dispatch_daemon_supervisors(action, endpoint_kind, endpoint)
        }
        DaemonAction::Plan { action } => dispatch_daemon_plan(action, endpoint_kind, endpoint),
    }
}

fn dispatch_daemon_supervisors(
    action: DaemonSupervisorsAction,
    endpoint_kind: Option<EndpointKindArg>,
    endpoint: Option<String>,
) -> CliResult<()> {
    match action {
        DaemonSupervisorsAction::List { project_id } => commands::daemon::list_session_supervisors(
            project_id,
            endpoint_kind.map(Into::into),
            endpoint,
        ),
        DaemonSupervisorsAction::Resolve {
            project_id,
            session,
        } => commands::daemon::resolve_session_supervisor(
            project_id,
            session,
            endpoint_kind.map(Into::into),
            endpoint,
        ),
        DaemonSupervisorsAction::Unregister {
            project_id,
            session,
        } => commands::daemon::unregister_session_supervisor(
            project_id,
            session,
            endpoint_kind.map(Into::into),
            endpoint,
        ),
        DaemonSupervisorsAction::Prune { project_id } => {
            commands::daemon::prune_session_supervisors(
                &project_id,
                endpoint_kind.map(Into::into),
                endpoint,
            )
        }
    }
}

fn dispatch_daemon_plan(
    action: DaemonPlanAction,
    endpoint_kind: Option<EndpointKindArg>,
    endpoint: Option<String>,
) -> CliResult<()> {
    match action {
        DaemonPlanAction::Build {
            project_id,
            profile,
            packages,
            target,
        } => commands::daemon::plan_build(
            project_id,
            profile,
            packages,
            target,
            endpoint_kind.map(Into::into),
            endpoint,
        ),
        DaemonPlanAction::Services {
            project_id,
            session,
            service_endpoint_kind,
        } => commands::daemon::plan_services(
            project_id,
            session,
            service_endpoint_kind.map(Into::into),
            endpoint_kind.map(Into::into),
            endpoint,
        ),
    }
}

impl From<EndpointKindArg> for az_proto_core::EndpointKind {
    fn from(value: EndpointKindArg) -> Self {
        match value {
            EndpointKindArg::InProcess => Self::InProcess,
            EndpointKindArg::WindowsNamedPipe => Self::WindowsNamedPipe,
            EndpointKindArg::UnixDomainSocket => Self::UnixDomainSocket,
            EndpointKindArg::Tcp => Self::Tcp,
        }
    }
}

impl From<RuntimeRoleArg> for az_proto_runtime::RuntimeRole {
    fn from(value: RuntimeRoleArg) -> Self {
        match value {
            RuntimeRoleArg::EditorWorld => Self::EditorWorld,
            RuntimeRoleArg::PlayPreview => Self::PlayPreview,
            RuntimeRoleArg::ServerPreview => Self::ServerPreview,
            RuntimeRoleArg::Validation => Self::Validation,
            RuntimeRoleArg::Thumbnail => Self::Thumbnail,
            RuntimeRoleArg::Bake => Self::Bake,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BuildAssetOutput, Cli, Commands, DaemonAction, DaemonPlanAction, SessionAction,
        SessionAssetAction, SessionServicesAction,
    };
    use clap::Parser as _;
    use std::collections::BTreeSet;

    use az_architecture_guard::{
        DependencyBoundary, PROJECT_FORMAT_EXAMPLE_OR_WORKSPACE_PROJECT_PATH_SEGMENTS,
        PROJECT_FORMAT_OR_EXAMPLE_PATH_SEGMENTS, PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
        forbidden_production_dependencies, non_allowlisted_production_dependencies,
        production_source_without_cfg_test_modules,
    };

    const FORBIDDEN_ROOT_DEPENDENCIES: &[&str] = &[
        "az-asset-processor",
        "az-asset-worker",
        "az-assetdb",
        "az-daemon",
        "az-editor",
        "az-editor-inspector",
        "az-editor-ui",
        "az-engine",
        "az-project-host",
        "az-runtime-host",
        "az-sessiond",
        "gridmate",
        "lmbr-central",
        "lyshine",
        "project-plugin",
    ];
    const ROOT_CLI_GEM_CONTRACT_DEPENDENCIES: &[&str] = &["az-gem-auth"];
    const ROOT_CLI_GEM_CONTRACT_PATHS: &[&str] = &["gems/auth"];
    const FORBIDDEN_PROTOCOL_DEPENDENCY_PATH_NAMES: &[&str] = &[
        "az-asset",
        "az-asset-builder",
        "az-asset-processor",
        "az-asset-worker",
        "az-assetdb",
        "az-daemon",
        "az-editor",
        "az-editor-inspector",
        "az-editor-ui",
        "az-engine",
        "az-framework",
        "az-observability",
        "az-project",
        "az-project-host",
        "az-rpc",
        "az-runtime-host",
        "az-session",
        "az-sessiond",
        "bevy",
        "gpui",
        "gridmate",
        "lmbr-central",
        "lyshine",
        "project-plugin",
    ];

    #[test]
    fn build_cli_accepts_repeatable_package_selectors() {
        let cli = Cli::try_parse_from([
            "azoth",
            "build",
            "-p",
            "server",
            "--package",
            "azoth-target-server",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Commands::Build { packages, .. }
                if packages == ["server", "azoth-target-server"]
        ));
    }

    #[test]
    fn build_cli_accepts_cache_asset_output() {
        let cli = Cli::try_parse_from(["azoth", "build", "--asset-output", "cache"]).unwrap();

        assert!(matches!(
            cli.command,
            Commands::Build {
                asset_output: BuildAssetOutput::Cache,
                ..
            }
        ));
    }

    #[test]
    fn build_cli_accepts_package_asset_output() {
        let cli = Cli::try_parse_from(["azoth", "build", "--asset-output", "package"]).unwrap();

        assert!(matches!(
            cli.command,
            Commands::Build {
                asset_output: BuildAssetOutput::Package,
                ..
            }
        ));
    }

    #[test]
    fn build_cli_defaults_to_package_asset_output() {
        let cli = Cli::try_parse_from(["azoth", "build"]).unwrap();

        assert!(matches!(
            cli.command,
            Commands::Build {
                asset_output: BuildAssetOutput::Package,
                ..
            }
        ));
    }

    #[test]
    fn daemon_plan_build_cli_accepts_repeatable_package_selectors() {
        let cli = Cli::try_parse_from([
            "azoth",
            "daemon",
            "plan",
            "build",
            "local.example",
            "-p",
            "server",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Commands::Daemon {
                action: DaemonAction::Plan {
                    action: DaemonPlanAction::Build { packages, .. }
                },
                ..
            } if packages == ["server"]
        ));
    }

    #[test]
    fn session_services_prepare_cli_accepts_repeatable_service_selection() {
        let cli = Cli::try_parse_from([
            "azoth",
            "session",
            "services",
            "prepare",
            "editor",
            "--service",
            "asset-worker",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Commands::Session {
                action: SessionAction::Services {
                    action: SessionServicesAction::Prepare { services, .. }
                }
            } if services == ["asset-worker"]
        ));
    }

    #[test]
    fn session_asset_process_cli_uses_processing_terminology() {
        let cli = Cli::try_parse_from([
            "azoth",
            "session",
            "asset",
            "process",
            "editor",
            "--platform",
            "pc",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Commands::Session {
                action: SessionAction::Asset {
                    action: SessionAssetAction::Process { name, platform, .. }
                }
            } if name == "editor" && platform == "pc"
        ));
    }

    #[test]
    fn workspace_default_members_are_azoth_editor_engine_surface() {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest = toml::from_str::<toml::Value>(&manifest).unwrap();
        let default_members = manifest["workspace"]["default-members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let expected = BTreeSet::from([
            ".",
            "crates/az/daemon",
            "crates/az/sessiond",
            "crates/editor/core",
            "crates/az/engine",
            "crates/az/graph-layout",
            "crates/az/project-host",
            "crates/az/runtime-app",
            "crates/az/runtime-host",
            "crates/az/asset-processor",
            "crates/az/asset-processor-host",
        ]);

        assert_eq!(default_members, expected);
        assert!(
            default_members
                .iter()
                .all(|member| { !workspace_default_member_is_project_specific(member) })
        );
    }

    #[test]
    fn workspace_default_member_guard_rejects_project_specific_surfaces() {
        for member in [
            "gems/lyshine",
            "formats/legacy/cry-chunk-assets",
            "formats/compat/az-pak",
            "projects/sample/crates/plugin",
            "formats/legacy/project-data",
            "projects/sample/examples/client",
        ] {
            assert!(
                workspace_default_member_is_project_specific(member),
                "`{member}` must stay opt-in, not part of root default-members"
            );
        }
    }

    #[test]
    fn workspace_members_follow_source_tree_taxonomy() {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest = toml::from_str::<toml::Value>(&manifest).unwrap();
        let members = manifest["workspace"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<BTreeSet<_>>();

        let retired = members
            .iter()
            .copied()
            .filter(|member| workspace_member_uses_retired_source_region(member))
            .collect::<Vec<_>>();
        assert!(
            retired.is_empty(),
            "workspace members use retired flat source-tree regions: {retired:?}"
        );

        let outside_taxonomy = members
            .iter()
            .copied()
            .filter(|member| !workspace_member_uses_accepted_source_region(member))
            .collect::<Vec<_>>();
        assert!(
            outside_taxonomy.is_empty(),
            "workspace members must live under the Azoth engine/editor/project taxonomy: {outside_taxonomy:?}"
        );
    }

    #[test]
    fn workspace_members_are_grouped_by_source_tree_taxonomy() {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest = toml::from_str::<toml::Value>(&manifest).unwrap();
        let members = manifest["workspace"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().replace('\\', "/"))
            .collect::<Vec<_>>();

        let out_of_order = members
            .windows(2)
            .filter(|pair| {
                workspace_member_source_region_rank(&pair[0])
                    > workspace_member_source_region_rank(&pair[1])
            })
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect::<Vec<_>>();

        assert!(
            out_of_order.is_empty(),
            "workspace members must be grouped by source-tree taxonomy: {out_of_order:?}"
        );
    }

    #[test]
    fn lumberyard_o3de_analogues_have_stable_azoth_regions() {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest = toml::from_str::<toml::Value>(&manifest).unwrap();
        let members = manifest["workspace"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().replace('\\', "/"))
            .collect::<BTreeSet<_>>();

        for (package, expected_path) in [
            ("az-core", "crates/az/core"),
            ("az-framework", "crates/az/framework"),
            ("az-engine", "crates/az/engine"),
            ("gridmate", "crates/az/gridmate"),
            ("gridmate-bevy", "crates/az/gridmate-bevy"),
            ("az-asset-builder", "crates/az/asset-builder"),
            ("az-asset-processor", "crates/az/asset-processor"),
            ("az-asset-worker", "crates/az/asset-worker"),
            ("az-project-host", "crates/az/project-host"),
            ("az-runtime-host", "crates/az/runtime-host"),
            ("az-editor", "crates/editor/core"),
            ("az-editor-ui", "crates/editor/ui"),
            ("az-editor-inspector", "crates/editor/inspector"),
        ] {
            assert!(
                members.contains(expected_path),
                "`{package}` must stay at `{expected_path}` to preserve the engine/editor source-tree taxonomy"
            );
            assert_eq!(
                workspace_member_package_name(expected_path),
                package,
                "`{expected_path}` must remain the `{package}` package"
            );
        }
    }

    #[test]
    fn engine_manifest_registers_every_top_level_engine_gem() {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine_graph = az_project::resolve_engine_graph_at(workspace_root).unwrap();
        let registered = engine_graph
            .gems
            .iter()
            .map(|gem| gem.declaration.path.to_string_lossy().replace('\\', "/"))
            .collect::<BTreeSet<_>>();

        let mut expected = BTreeSet::new();
        for entry in std::fs::read_dir(workspace_root.join("gems")).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_dir() {
                continue;
            }

            let root = entry.path();
            if !root.join("Cargo.toml").exists() {
                continue;
            }

            let directory = entry.file_name().to_string_lossy().replace('\\', "/");
            let gem_path = format!("gems/{directory}");
            expected.insert(gem_path);
        }

        assert_eq!(
            registered, expected,
            "engine.toml must be the engine gem registry for top-level gems/* plugin directories"
        );
    }

    #[test]
    fn steam_locator_is_reusable_integration_not_project_tooling() {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest = toml::from_str::<toml::Value>(&manifest).unwrap();
        let members = manifest["workspace"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().replace('\\', "/"))
            .collect::<BTreeSet<_>>();
        let steam_locator_path = manifest["workspace"]["dependencies"]["steam-locator"]["path"]
            .as_str()
            .unwrap()
            .replace('\\', "/");

        assert_eq!(
            steam_locator_path, "crates/integrations/steam/locator",
            "shared Steam app discovery must live under crates/integrations, not project tooling"
        );
        assert!(members.contains("crates/integrations/steam/locator"));
        assert!(!members.contains("projects/sample/tools/steam-locator"));
    }

    #[test]
    fn az_asset_stays_native_catalog_package_not_closed_product_registry() {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let asset_src = workspace_root.join("crates/az/asset/src");

        for retired_module in ["product.rs", "slice_meta.rs", "transform_profile.rs"] {
            assert!(
                !asset_src.join(retired_module).exists(),
                "az-asset must not own retired compatibility/product registry module `{retired_module}`"
            );
        }

        for entry in std::fs::read_dir(&asset_src).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for forbidden in ["ProductAssetKind", "ProductAssetKindSpec"] {
                assert!(
                    !source.contains(forbidden),
                    "az-asset must expose open registries/catalogs, not closed product kind table `{forbidden}` in {}",
                    path.display()
                );
            }
        }
    }

    fn workspace_member_uses_accepted_source_region(member: &str) -> bool {
        member == "."
            || member.starts_with("crates/az/")
            || member.starts_with("crates/editor/")
            || member.starts_with("crates/engine/")
            || member.starts_with("crates/formats/")
            || member.starts_with("crates/import/")
            || member.starts_with("crates/integrations/")
            || member.starts_with("gems/")
            || member.starts_with("formats/")
            || member.starts_with("projects/")
    }

    fn workspace_member_source_region_rank(member: &str) -> usize {
        if member == "." {
            0
        } else if member.starts_with("crates/az/") {
            1
        } else if member.starts_with("crates/editor/") {
            2
        } else if member.starts_with("crates/engine/") {
            3
        } else if member.starts_with("crates/formats/") {
            4
        } else if member.starts_with("crates/import/") {
            5
        } else if member.starts_with("crates/integrations/") {
            6
        } else if member.starts_with("formats/") {
            7
        } else if member.starts_with("gems/") {
            8
        } else if member.starts_with("projects/") {
            9
        } else {
            usize::MAX
        }
    }

    fn workspace_member_uses_retired_source_region(member: &str) -> bool {
        member.starts_with("crates/az-")
            || member.starts_with("crates/editor-")
            || member.starts_with("crates/tools/")
            || member.starts_with("crates/gems/")
            || member.starts_with("crates/plugins")
            || member.starts_with("crates/types")
            || member.starts_with("examples/")
    }

    fn workspace_member_package_name(member: &str) -> String {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(member)
            .join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest = toml::from_str::<toml::Value>(&manifest).unwrap();
        manifest["package"]["name"].as_str().unwrap().to_string()
    }

    #[test]
    fn azoth_cli_stays_a_launcher_not_an_editor_or_project_service_binary() {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest = toml::from_str::<toml::Value>(&manifest).unwrap();
        let violations = forbidden_root_production_dependencies(&manifest);

        assert!(
            violations.is_empty(),
            "azoth CLI must launch editor/session/service processes instead of linking them directly: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn azoth_cli_dependency_guard_rejects_workspace_aliases() {
        let manifest = toml::from_str::<toml::Value>(
            r#"
            [workspace.dependencies]
            daemon-api = { package = "az-daemon", path = "crates/az/daemon" }
            project-api = { package = "project-plugin", path = "projects/sample/crates/plugin" }

            [dependencies]
            daemon-api = { workspace = true }
            project-api = { workspace = true }
            "#,
        )
        .unwrap();

        assert_eq!(
            forbidden_root_production_dependencies(&manifest),
            vec![
                "dependencies.daemon-api(workspace package az-daemon)",
                "dependencies.project-api(workspace package project-plugin)",
            ]
        );
    }

    #[test]
    fn azoth_cli_dependency_guard_allows_only_provider_neutral_auth_contract() {
        let manifest = toml::from_str::<toml::Value>(
            r#"
            [workspace.dependencies]
            az-gem-auth = { path = "gems/auth" }
            az-gem-auth-steam = { path = "gems/auth-steam" }

            [dependencies]
            az-gem-auth = { workspace = true }
            az-gem-auth-steam = { workspace = true }
            "#,
        )
        .unwrap();

        assert_eq!(
            forbidden_root_production_dependencies(&manifest),
            vec!["dependencies.az-gem-auth-steam"]
        );
    }

    #[test]
    fn azoth_cli_dependency_guard_rejects_workspace_path_only_aliases() {
        let manifest = toml::from_str::<toml::Value>(
            r#"
            [workspace.dependencies]
            terrain-api = { path = "gems/legacy-terrain" }
            legacy-format = { path = "formats/legacy/cry-chunk-assets" }
            compatible-pak = { path = "formats/compat/az-pak" }
            project-tool = { path = "projects/sample/tools/cap" }

            [dependencies]
            terrain-api = { workspace = true }
            legacy-format = { workspace = true }
            compatible-pak = { workspace = true }
            project-tool = { workspace = true }
            "#,
        )
        .unwrap();

        assert_eq!(
            forbidden_root_production_dependencies(&manifest),
            vec![
                "dependencies.compatible-pak(workspace path formats/compat/az-pak)",
                "dependencies.legacy-format(workspace path formats/legacy/cry-chunk-assets)",
                "dependencies.project-tool(workspace path projects/sample/tools/cap)",
                "dependencies.terrain-api(workspace path gems/legacy-terrain)",
            ]
        );
    }

    #[test]
    fn azoth_cli_dependency_guard_rejects_target_specific_dependencies() {
        let manifest = toml::from_str::<toml::Value>(
            r#"
            [target.'cfg(windows)'.dependencies]
            service-core = { package = "az-project-host", path = "crates/az/project-host" }
            terrain-api = { path = "gems/legacy-terrain" }

            [target.'cfg(unix)'.build-dependencies]
            project-api = { package = "project-plugin", path = "projects/sample/crates/plugin" }
            legacy-format = { path = "formats/legacy/cry-chunk-assets" }
            compatible-pak = { path = "formats/compat/az-pak" }
            "#,
        )
        .unwrap();

        assert_eq!(
            forbidden_root_production_dependencies(&manifest),
            vec![
                "target.cfg(unix).build-dependencies.compatible-pak(path formats/compat/az-pak)",
                "target.cfg(unix).build-dependencies.legacy-format(path formats/legacy/cry-chunk-assets)",
                "target.cfg(unix).build-dependencies.project-api(package project-plugin)",
                "target.cfg(windows).dependencies.service-core(package az-project-host)",
                "target.cfg(windows).dependencies.terrain-api(path gems/legacy-terrain)",
            ]
        );
    }

    /// Production dependencies each protocol/transport crate is allowed to declare.
    /// The guard below asserts nothing outside these lists reaches those crates.
    fn protocol_and_transport_dependency_allowlists()
    -> [(&'static str, &'static str, &'static [&'static str]); 8] {
        [
            (
                "az-proto-core",
                "crates/az/proto/core/Cargo.toml",
                &[
                    "blake3",
                    "capnp",
                    "capnp-rpc",
                    "capnpc",
                    "thiserror",
                    "uuid",
                ][..],
            ),
            (
                "az-proto-daemon",
                "crates/az/proto/daemon/Cargo.toml",
                &[
                    "az-proto-core",
                    "az-proto-session",
                    "capnp",
                    "capnpc",
                    "serde",
                    "thiserror",
                    "toml",
                ][..],
            ),
            (
                "az-proto-observability",
                "crates/az/proto/observability/Cargo.toml",
                &["az-proto-core", "capnp", "capnpc", "tracing", "uuid"][..],
            ),
            (
                "az-proto-project",
                "crates/az/proto/project/Cargo.toml",
                &[
                    // The project-host ABI surface is defined in terms of engine
                    // reflection envelopes (`az-core`), node-graph value types
                    // (`az-node-graph`), and blake3 projection hashes; these are
                    // the wire shapes themselves, not service/runtime logic.
                    "az-core",
                    "az-node-graph",
                    "az-proto-core",
                    "az-proto-runtime",
                    "blake3",
                    "capnp",
                    "capnpc",
                    "thiserror",
                    "uuid",
                ][..],
            ),
            (
                "az-proto-asset",
                "crates/az/proto/asset/Cargo.toml",
                &["az-proto-core", "capnp", "capnpc", "thiserror", "uuid"][..],
            ),
            (
                "az-proto-runtime",
                "crates/az/proto/runtime/Cargo.toml",
                &["az-proto-core", "capnp", "capnpc", "thiserror", "uuid"][..],
            ),
            (
                "az-proto-session",
                "crates/az/proto/session/Cargo.toml",
                &[
                    "az-proto-asset",
                    "az-proto-core",
                    "az-proto-project",
                    "az-proto-runtime",
                    "capnp",
                    "capnpc",
                    "uuid",
                ][..],
            ),
            (
                "az-rpc",
                "crates/az/rpc/Cargo.toml",
                &[
                    "az-proto-core",
                    "capnp",
                    "capnp-rpc",
                    // Unix listener endpoint leases prevent stale-socket
                    // recovery from racing another transport owner.
                    "fs2",
                    "futures",
                    "thiserror",
                    "tokio",
                    "tokio-util",
                    "tracing",
                    // Named-pipe owner-only security descriptors (ADR 0031 C4)
                    // are transport-layer concerns.
                    "windows",
                ][..],
            ),
        ]
    }

    #[test]
    fn protocol_and_rpc_crates_stay_abi_and_transport_only() {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_manifest =
            std::fs::read_to_string(workspace_root.join("Cargo.toml")).unwrap();
        let workspace_manifest = toml::from_str::<toml::Value>(&workspace_manifest).unwrap();

        for (label, manifest_path, allowed_dependencies) in
            protocol_and_transport_dependency_allowlists()
        {
            let manifest_path = workspace_root.join(manifest_path);
            let manifest = std::fs::read_to_string(&manifest_path)
                .unwrap_or_else(|error| panic!("read {}: {error}", manifest_path.display()));
            let manifest = toml::from_str::<toml::Value>(&manifest)
                .unwrap_or_else(|error| panic!("parse {}: {error}", manifest_path.display()));
            let violations = production_dependencies_outside_allowlist(
                &manifest,
                &workspace_manifest,
                allowed_dependencies,
            );

            assert!(
                violations.is_empty(),
                "{label} must remain a protocol/transport boundary crate; unexpected production deps: {}",
                violations.join(", ")
            );
        }
    }

    #[test]
    fn protocol_dependency_guard_rejects_service_aliases_targets_and_path_only_refs() {
        let manifest = toml::from_str::<toml::Value>(
            r#"
            [dependencies]
            editor = { package = "az-editor-ui", path = "crates/editor/ui" }
            capnp = { path = "crates/editor/core" }
            serde-json = "1"

            [target.'cfg(windows)'.dependencies]
            runtime = { workspace = true }
            legacy = { path = "formats/legacy/cry-chunk-assets" }

            [target.'cfg(unix)'.build-dependencies]
            capnpc = { path = "gems/legacy-tools" }
            compatible-pak = { path = "formats/compat/az-pak" }
            "#,
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<toml::Value>(
            r#"
            [workspace.dependencies]
            runtime = { package = "az-runtime-host", path = "crates/az/runtime-host" }
            "#,
        )
        .unwrap();

        assert_eq!(
            production_dependencies_outside_allowlist(
                &manifest,
                &workspace_manifest,
                &["capnp", "capnpc"]
            ),
            vec![
                "dependencies.capnp(path crates/editor/core)",
                "dependencies.editor(package az-editor-ui)",
                "dependencies.serde-json",
                "target.cfg(unix).build-dependencies.capnpc(path gems/legacy-tools)",
                "target.cfg(unix).build-dependencies.compatible-pak(path formats/compat/az-pak)",
                "target.cfg(windows).dependencies.legacy(path formats/legacy/cry-chunk-assets)",
                "target.cfg(windows).dependencies.runtime(workspace package az-runtime-host)",
            ]
        );
    }

    #[test]
    fn universal_process_entrypoints_use_shared_observability_installer() {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for (label, path, expected_installer) in [
            (
                "azoth",
                workspace_root.join("src/main.rs"),
                "install_service_observability",
            ),
            (
                "azd",
                workspace_root.join("crates/az/daemon/src/main.rs"),
                "install_service_observability",
            ),
            (
                "az-sessiond",
                workspace_root.join("crates/az/sessiond/src/main.rs"),
                "install_service_observability",
            ),
            (
                // The editor binary is an 11-line entry; its startup sequence and
                // the observability install live in the library so the GUI test
                // harness never has to load on a server image.
                "az-editor",
                workspace_root.join("crates/editor/core/src/startup.rs"),
                "install_process_observability",
            ),
        ] {
            let source = std::fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!("read {label} entrypoint {}: {error}", path.display())
            });
            let production_source = source.split("#[cfg(test)]").next().unwrap_or(&source);

            assert!(
                production_source.contains(expected_installer),
                "{label} must install tracing through the shared observability boundary"
            );
            assert!(
                !production_source.contains("tracing_subscriber::fmt(")
                    && !production_source.contains("tracing_subscriber::registry("),
                "{label} must not install an ad hoc tracing subscriber"
            );
            if label == "az-sessiond" {
                assert!(
                    production_source.contains("with_otlp_endpoint(otlp_endpoint)"),
                    "az-sessiond must forward its explicit --otlp-endpoint flag into the shared observability installer"
                );
            }
        }
    }

    #[test]
    fn azoth_cli_launcher_commands_do_not_embed_service_hosts() {
        const FORBIDDEN_SERVICE_HOST_SNIPPETS: &[&str] = &[
            "AssetDb::open(",
            "AssetDb::open_in_memory(",
            "AzDaemon::new(",
            "AzDaemonRpc::",
            "ProjectHostRpc::",
            "ProjectHost::new(",
            "AssetProcessorRpc::",
            "RuntimeHostRpc::",
            "RuntimeHost::new(",
            "SessionSupervisorRpc::",
            "start_az_daemon_rpc_server",
            "start_az_daemon_rpc_server_with_daemon",
            "start_project_host_rpc_server",
            "start_asset_processor_rpc_server",
            "start_runtime_host_rpc_server",
            "start_session_supervisor_rpc_server",
        ];
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for (label, source_path, required_snippets, forbidden_snippets) in [
            (
                "daemon command",
                workspace_root.join("src/commands/daemon.rs"),
                &["daemon_launch_command(", "Command::new(&command.program)"][..],
                FORBIDDEN_SERVICE_HOST_SNIPPETS,
            ),
            (
                "session command",
                workspace_root.join("src/commands/session.rs"),
                &["sessiond_launch_command(", "Command::new(&command.program)"][..],
                FORBIDDEN_SERVICE_HOST_SNIPPETS,
            ),
            (
                "build command",
                workspace_root.join("src/commands/build.rs"),
                &[
                    "plan_build_commands_through_daemon(",
                    "Command::new(&command.program)",
                ][..],
                FORBIDDEN_SERVICE_HOST_SNIPPETS,
            ),
            (
                "run command",
                workspace_root.join("src/commands/run.rs"),
                &[
                    "active_session_slug_through_daemon(",
                    "crate::commands::session::launch_runtime(",
                ][..],
                FORBIDDEN_SERVICE_HOST_SNIPPETS,
            ),
            (
                "editor command",
                workspace_root.join("src/commands/editor.rs"),
                &[
                    "crate::commands::daemon::start_for_editor(",
                    "requested_session_manifest_through_daemon(",
                    "active_session_manifest_through_daemon(",
                    "start_services_for_proto_manifest_through_daemon(",
                    "crate::commands::session::start_services(",
                    "Command::new(&command.program)",
                ][..],
                FORBIDDEN_SERVICE_HOST_SNIPPETS,
            ),
        ] {
            let source = std::fs::read_to_string(&source_path)
                .unwrap_or_else(|error| panic!("read {}: {error}", source_path.display()));
            let production_source = production_source_without_cfg_test_modules(&source);

            for required in required_snippets {
                assert!(
                    production_source.contains(required),
                    "{label} must keep process-launch path `{required}`"
                );
            }
            for forbidden in forbidden_snippets {
                assert!(
                    !production_source.contains(forbidden),
                    "{label} must not embed service-host API `{forbidden}` in production code"
                );
            }
        }
    }

    fn forbidden_root_production_dependencies(manifest: &toml::Value) -> Vec<String> {
        forbidden_production_dependencies(
            manifest,
            manifest,
            DependencyBoundary::new(
                FORBIDDEN_ROOT_DEPENDENCIES,
                PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
                PROJECT_FORMAT_EXAMPLE_OR_WORKSPACE_PROJECT_PATH_SEGMENTS,
            )
            .with_exempt_names(ROOT_CLI_GEM_CONTRACT_DEPENDENCIES)
            .with_exempt_paths(ROOT_CLI_GEM_CONTRACT_PATHS),
        )
    }

    fn production_dependencies_outside_allowlist(
        manifest: &toml::Value,
        workspace_manifest: &toml::Value,
        allowed_dependencies: &[&str],
    ) -> Vec<String> {
        non_allowlisted_production_dependencies(
            manifest,
            workspace_manifest,
            allowed_dependencies,
            DependencyBoundary::new(
                FORBIDDEN_PROTOCOL_DEPENDENCY_PATH_NAMES,
                PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
                PROJECT_FORMAT_OR_EXAMPLE_PATH_SEGMENTS,
            ),
        )
    }

    fn workspace_default_member_is_project_specific(member: &str) -> bool {
        let member = member.replace('\\', "/");
        member.starts_with("projects/")
            || member.starts_with("gems/")
            || member.contains("/gems/")
            || member.starts_with("formats/legacy/")
            || member.contains("/formats/legacy/")
            || member.starts_with("formats/compat/")
            || member.contains("/formats/compat/")
            || member.starts_with("examples/")
    }
}
