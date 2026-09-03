//! Engine-owned runtime application lifecycle.
//!
//! A generated ship target is a thin main: it composes its contributions and
//! hands the [`Composer`] to [`run`]. Everything after that is the engine's —
//! validating the composition, the `App` the composition already owns, headless
//! or windowed mode, logging, the asset root and the catalog mounted over it,
//! named-clock schedules, and the blocking run loop.
//!
//! [`run`] pins the composition for the life of the process. That is what lets
//! the world hold `&'static Registries`: the composed entries a system reads at
//! runtime are borrowed from a composition nothing can compose into again,
//! because every mutator on [`Composer`] takes `&mut self` and the pinned
//! handle is shared. One composition per process is the model here; reload owns
//! a different lifecycle.
//!
//! Windowing follows the target role's `HasClient` capability, the same fact the
//! composition is conditioned on. There is no separate mode enum to disagree
//! with it, and no runtime role: the composition carries the
//! [`GemTargetRole`], and [`SessionMode`] is wire and runtime policy that never
//! changes what composed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use az_core::BuildIdentity;
use az_engine::EnginePlugin;
use az_filesystem::{AzothDataHome, DEFAULT_ASSET_PLATFORM};
use az_gem_contract::{
    ComposeError, ComposeReport, Composer, GemTargetRole, HostCapability, Registries, SessionMode,
};
use az_project::{find_project_root, load_project_manifest};
use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use thiserror::Error;
use tracing::{info, instrument};

mod clock;
mod registration;

pub use clock::{Clock, audit};
use registration::parse;
pub use registration::{
    Launch, RuntimeAppConfigurer, RuntimeAppContributionRegistration, RuntimeLaunchSettings,
    RuntimeLaunchSettingsRegistration, contributions, launch_settings,
};

pub type RuntimeLogLayerFactory = fn(&mut App) -> Option<bevy::log::BoxedLayer>;

/// What a launcher tells the engine about the process it is starting.
///
/// Identity and policy only: what composed is the composer's answer, never
/// this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAppConfig {
    pub name: String,
    pub role: GemTargetRole,
    pub session: SessionMode,
    pub tick: Duration,
    pub asset_root: Option<PathBuf>,
    pub runtime_project: Option<RuntimeProjectIdentity>,
    pub project_profile: Option<RuntimeProjectProfile>,
    pub log: RuntimeLogConfig,
}

/// Frame interval a host without a client runs its schedule loop at.
const DEFAULT_TICK: Duration = Duration::from_millis(16);

impl RuntimeAppConfig {
    #[must_use]
    pub fn new(name: impl Into<String>, role: GemTargetRole, session: SessionMode) -> Self {
        Self {
            name: name.into(),
            role,
            session,
            tick: DEFAULT_TICK,
            asset_root: None,
            runtime_project: None,
            project_profile: None,
            log: RuntimeLogConfig::default(),
        }
    }

    #[must_use]
    pub fn with_asset_root(mut self, asset_root: impl Into<PathBuf>) -> Self {
        self.asset_root = Some(asset_root.into());
        self
    }

    #[must_use]
    pub fn with_optional_asset_root(mut self, asset_root: Option<impl Into<PathBuf>>) -> Self {
        self.asset_root = asset_root.map(Into::into);
        self
    }

    /// Supply the identity compiled into a generated development launcher.
    ///
    /// Packaged builds take precedence through executable-adjacent detection;
    /// this identity is the developer-cache fallback.
    #[must_use]
    pub fn with_runtime_project(
        mut self,
        project_name: impl Into<String>,
        project_root: impl Into<PathBuf>,
    ) -> Self {
        self.runtime_project = Some(RuntimeProjectIdentity::new(project_name, project_root));
        self
    }

    #[must_use]
    pub fn with_log(mut self, log: RuntimeLogConfig) -> Self {
        self.log = log;
        self
    }

    #[must_use]
    pub fn with_project_profile(mut self, profile: RuntimeProjectProfile) -> Self {
        self.project_profile = Some(profile);
        self
    }

    /// Frame interval for a host that runs its own schedule loop.
    #[must_use]
    pub const fn with_tick(mut self, tick: Duration) -> Self {
        self.tick = tick;
        self
    }

    /// Whether this role presents to a player.
    ///
    /// The one fact windowing follows. A host with `HasClient` takes the
    /// platform and renderer stack; every other role runs its own loop.
    #[must_use]
    pub const fn is_windowed(&self) -> bool {
        self.role.provided_caps().has(HostCapability::HasClient)
    }

    const fn requires_runtime_assets(&self) -> bool {
        self.runtime_project.is_some() || self.project_profile.is_some()
    }
}

/// Project identity embedded by an Azoth-generated development target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProjectIdentity {
    pub project_name: String,
    pub project_root: PathBuf,
}

impl RuntimeProjectIdentity {
    #[must_use]
    pub fn new(project_name: impl Into<String>, project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_name: project_name.into(),
            project_root: project_root.into(),
        }
    }
}

/// Branch that selected the local runtime asset root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAssetRootSource {
    ExplicitOverride,
    Packaged,
    DeveloperCache,
}

impl RuntimeAssetRootSource {
    const fn label(self) -> &'static str {
        match self {
            Self::ExplicitOverride => "explicit override",
            Self::Packaged => "packaged executable layout",
            Self::DeveloperCache => "developer product cache",
        }
    }
}

/// A runtime asset root selected by the engine-owned resolution chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRuntimeAssetRoot {
    pub path: PathBuf,
    pub source: RuntimeAssetRootSource,
}

/// Resolve the local product cache for one runtime process.
///
/// An explicit root wins. Otherwise, a packaged executable may carry
/// `Cache/<platform>` beside its executable. Development binaries use the
/// project identity embedded by target generation, or discover the nearest
/// `azoth.toml` from the current working directory.
///
/// # Errors
///
/// Returns [`RuntimeAssetRootError::RemoteOverride`] if an explicit
/// `config.asset_root` names an `http(s)` URL; any error
/// [`discover_runtime_project`] returns when `config.runtime_project` is unset
/// and no packaged cache is present; and
/// [`RuntimeAssetRootError::Unavailable`] if neither the executable-adjacent
/// `Cache/<platform>` nor the developer product cache holds an
/// `assetcatalog.bin`.
///
/// # Panics
///
/// Debug builds assert that a resolved project identity carries a non-empty
/// name and an absolute root.
#[instrument(skip(config), fields(target = %config.name, role = %config.role))]
pub fn resolve_runtime_asset_root(
    config: &RuntimeAppConfig,
) -> Result<ResolvedRuntimeAssetRoot, RuntimeAssetRootError> {
    if let Some(path) = config.asset_root.as_ref() {
        reject_remote_runtime_assets_override(path)?;
        let resolved = ResolvedRuntimeAssetRoot {
            path: path.clone(),
            source: RuntimeAssetRootSource::ExplicitOverride,
        };
        info!(
            runtime_assets = %resolved.path.display(),
            source = resolved.source.label(),
            "resolved runtime asset root"
        );
        return Ok(resolved);
    }

    // Package payload containers are still profile-specific. Until installation
    // standardizes that layout, shipped runtimes detect a conventional
    // executable-adjacent `Cache/<platform>` product cache.
    let packaged_candidate = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .map(|directory| directory.join("Cache").join(DEFAULT_ASSET_PLATFORM));
    if let Some(path) = packaged_candidate
        .as_ref()
        .filter(|path| is_runtime_asset_root(path))
    {
        let resolved = ResolvedRuntimeAssetRoot {
            path: path.clone(),
            source: RuntimeAssetRootSource::Packaged,
        };
        info!(
            runtime_assets = %resolved.path.display(),
            source = resolved.source.label(),
            "resolved runtime asset root"
        );
        return Ok(resolved);
    }

    let identity = match &config.runtime_project {
        Some(identity) => identity.clone(),
        None => discover_runtime_project()?,
    };
    debug_assert!(!identity.project_name.trim().is_empty());
    debug_assert!(identity.project_root.is_absolute());

    let developer_cache = AzothDataHome::resolve()
        .project(&identity.project_name, &identity.project_root)
        .default_product_cache_dir();
    if is_runtime_asset_root(&developer_cache) {
        let resolved = ResolvedRuntimeAssetRoot {
            path: developer_cache,
            source: RuntimeAssetRootSource::DeveloperCache,
        };
        info!(
            runtime_assets = %resolved.path.display(),
            source = resolved.source.label(),
            project = %identity.project_name,
            project_root = %identity.project_root.display(),
            "resolved runtime asset root"
        );
        return Ok(resolved);
    }

    let packaged = packaged_candidate.as_ref().map_or_else(
        || "unavailable".to_string(),
        |path| path.display().to_string(),
    );
    Err(RuntimeAssetRootError::Unavailable {
        detail: format!(
            "no native `{ASSET_CATALOG_FILE_NAME}` under packaged cache `{packaged}` or developer cache `{}`",
            developer_cache.display()
        ),
    })
}

const ASSET_CATALOG_FILE_NAME: &str = "assetcatalog.bin";
const RUNTIME_ASSETS_REMEDIATION: &str =
    "pass --assets <Cache/pc> or process the project assets with azoth";

fn is_runtime_asset_root(path: &Path) -> bool {
    path.join(ASSET_CATALOG_FILE_NAME).is_file()
}

fn reject_remote_runtime_assets_override(path: &Path) -> Result<(), RuntimeAssetRootError> {
    let display = path.to_string_lossy();
    let lower = display.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Err(RuntimeAssetRootError::RemoteOverride {
            path: display.trim_end_matches('/').to_string(),
        });
    }
    Ok(())
}

/// Discover the nearest Azoth project by walking up from the current directory
/// to `azoth.toml`. Used by development tools that are not generated targets.
///
/// # Errors
///
/// Returns [`RuntimeAssetRootError::CurrentDirectory`] if the working directory
/// cannot be read, [`RuntimeAssetRootError::Project`] if no `azoth.toml` is
/// found at or above it, or [`RuntimeAssetRootError::ProjectManifest`] if the
/// manifest that was found cannot be loaded.
pub fn discover_runtime_project() -> Result<RuntimeProjectIdentity, RuntimeAssetRootError> {
    let cwd = std::env::current_dir()
        .map_err(|source| RuntimeAssetRootError::CurrentDirectory { source })?;
    let project_root =
        find_project_root(&cwd).map_err(|source| RuntimeAssetRootError::Project {
            start: cwd,
            source: Box::new(source),
        })?;
    let manifest = load_project_manifest(&project_root).map_err(|source| {
        RuntimeAssetRootError::ProjectManifest {
            root: project_root.clone(),
            source: Box::new(source),
        }
    })?;
    Ok(RuntimeProjectIdentity::new(
        manifest.project.name,
        project_root,
    ))
}

#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProjectProfile {
    pub project_id: String,
    pub project_name: String,
    pub target_name: String,
    pub settings_name: String,
    pub build: BuildIdentity,
}

impl RuntimeProjectProfile {
    #[must_use]
    pub fn new(
        project_id: impl Into<String>,
        project_name: impl Into<String>,
        target_name: impl Into<String>,
        settings_name: impl Into<String>,
        build: BuildIdentity,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            project_name: project_name.into(),
            target_name: target_name.into(),
            settings_name: settings_name.into(),
            build,
        }
    }
}

#[derive(Clone)]
pub struct RuntimeLogConfig {
    pub ownership: RuntimeLogOwnership,
    pub level: bevy::log::Level,
    pub filter: Option<String>,
    pub custom_layer: Option<RuntimeLogLayerFactory>,
    pub color: RuntimeLogColor,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RuntimeLogOwnership {
    #[default]
    Engine,
    External,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum RuntimeLogColor {
    /// Use ANSI color only when supported and `NO_COLOR` is unset.
    #[default]
    Auto,
    /// Always emit ANSI color codes.
    Always,
    /// Never emit ANSI color codes.
    Never,
}

impl RuntimeLogConfig {
    #[must_use]
    pub const fn new(level: bevy::log::Level) -> Self {
        Self {
            ownership: RuntimeLogOwnership::Engine,
            level,
            filter: None,
            custom_layer: None,
            color: RuntimeLogColor::Auto,
        }
    }

    #[must_use]
    pub const fn with_ownership(mut self, ownership: RuntimeLogOwnership) -> Self {
        self.ownership = ownership;
        self
    }

    #[must_use]
    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    #[must_use]
    pub const fn with_custom_layer(mut self, custom_layer: RuntimeLogLayerFactory) -> Self {
        self.custom_layer = Some(custom_layer);
        self
    }

    #[must_use]
    pub const fn with_color(mut self, color: RuntimeLogColor) -> Self {
        self.color = color;
        self
    }
}

impl Default for RuntimeLogConfig {
    fn default() -> Self {
        Self::new(bevy::log::Level::INFO)
    }
}

impl std::fmt::Debug for RuntimeLogConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeLogConfig")
            .field("ownership", &self.ownership)
            .field("level", &self.level)
            .field("filter", &self.filter)
            .field("custom_layer", &self.custom_layer.is_some())
            .field("color", &self.color)
            .finish()
    }
}

impl PartialEq for RuntimeLogConfig {
    fn eq(&self, other: &Self) -> bool {
        self.ownership == other.ownership
            && self.level == other.level
            && self.filter == other.filter
            && self.custom_layer.is_some() == other.custom_layer.is_some()
            && self.color == other.color
    }
}

impl Eq for RuntimeLogConfig {}

/// What a contribution can read about the process it is being configured into.
#[derive(Resource, Debug, Clone)]
pub struct RuntimeAppContext {
    config: RuntimeAppConfig,
}

impl RuntimeAppContext {
    #[must_use]
    pub const fn config(&self) -> &RuntimeAppConfig {
        &self.config
    }

    #[must_use]
    pub const fn role(&self) -> GemTargetRole {
        self.config.role
    }

    #[must_use]
    pub const fn session(&self) -> SessionMode {
        self.config.session
    }

    #[must_use]
    pub const fn is_headless(&self) -> bool {
        !self.config.is_windowed()
    }

    #[must_use]
    pub const fn asset_root(&self) -> Option<&PathBuf> {
        self.config.asset_root.as_ref()
    }
}

/// The composition, carried into the world it composed.
///
/// Both halves of what this process composed. The `report` is the
/// operator-visible answer to "which gem registered what", so it belongs where
/// an incident can reach it rather than only in a log line at startup. The
/// `registries` are the composition itself — the composed entries a system
/// reads at runtime, and the one place a plugin that needs them can get them
/// without a process-global.
///
/// `&'static` because [`run`] pins the composition for the life of the process
/// before this resource exists. A contribution reads it during configuration
/// (it is inserted before the configurers run) or from a system through
/// `Res<Composition>`.
#[derive(Resource, Clone)]
pub struct Composition {
    pub report: ComposeReport,
    pub registries: &'static Registries,
}

/// Registries have no shape of their own to print — a composition's readable
/// facts are its report — so this shows the report and says the registries are
/// there.
impl std::fmt::Debug for Composition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Composition")
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

/// Run a composed target under the engine-owned lifecycle.
///
/// The composition already owns the `App` — every contribution registered into
/// it before this was called — so this takes it, pins the composition for the
/// life of the process, installs what the engine owns around the app, lets each
/// composed contribution configure it with the parsed command line in hand, and
/// runs it. Composed clocks become schedules here; unresolvable ones fail at
/// [`Composer::finalize`], which this calls.
///
/// The composer arrives whole rather than as a composer-and-report pair: the
/// report is this composition's own fact, and producing it here is what makes
/// a report that belongs to some *other* composition unrepresentable.
///
/// # Errors
///
/// [`RuntimeAppError::NoWorld`] when the role does not own an `App`,
/// [`RuntimeAppError::Compose`] when the composition does not validate,
/// [`RuntimeAppError::NotReady`] when a composed contribution reports it is
/// not ready to run, [`RuntimeAppError::Exit`] when the app itself exits with
/// an error, or whatever argument parsing, asset-root resolution, or a
/// contribution's own configuration reports.
pub fn run(config: RuntimeAppConfig, mut composer: Composer) -> Result<(), RuntimeAppError> {
    let role = config.role;
    // Taken while the composer is still owned, because everything after this
    // reads the composition through a shared borrow.
    let mut app = composer
        .take_app()
        .ok_or(RuntimeAppError::NoWorld { role })?;
    // The composition outlives the world that reads it: systems hold
    // `&'static Registries` for as long as the process runs. Sharing it is
    // also the seal — `add`, `floor` and `remove` all take `&mut self`, so
    // nothing can compose into a composition the world is already reading.
    let composer: &'static Composer = Box::leak(Box::new(composer));
    build(config, composer, &mut app, std::env::args_os())?;
    if !composer.all_ready() {
        return Err(RuntimeAppError::NotReady { role });
    }
    let exit = app.run();
    // Every composed contribution gets its `finish` whether the app exited
    // cleanly or not; the exit code is reported after, so a failed run reaches
    // the generated `main` as an error instead of a silent success.
    composer.finish();
    match exit {
        AppExit::Success => Ok(()),
        AppExit::Error(code) => Err(RuntimeAppError::Exit { code: code.get() }),
    }
}

/// Everything [`run`] does to the app, up to the run loop.
///
/// Public for the hosts and tests that drive the app themselves — an
/// embedded viewport, a TUI harness, or a test inspecting the composed
/// world (the named-clock [`audit`] needs a built app and no run loop).
/// `args` is the CLI surface [`run`] feeds from `std::env::args_os`;
/// callers own readiness (`Composer::all_ready`) and `finish`. Generated
/// mains call [`run`], never this.
///
/// The composer is `&'static` because the app outlives this call and holds
/// its registries: a caller drives this by taking the app
/// (`Composer::take_app`) and pinning what is left, exactly as [`run`] does.
///
/// # Errors
///
/// The [`run`] error set minus the run loop's own: invalid config, a
/// composition that does not validate, argument parsing, asset-root
/// resolution, or a contribution's configuration failure.
pub fn build<I, T>(
    config: RuntimeAppConfig,
    composer: &'static Composer,
    app: &mut App,
    args: I,
) -> Result<(), RuntimeAppError>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    if config.name.trim().is_empty() {
        return Err(RuntimeAppError::InvalidConfig(
            "runtime app name cannot be empty".to_string(),
        ));
    }
    let report = composer.finalize()?;
    let settings = parse(&config.name, composer.registries(), args)?;
    let context = engine_context(config, &settings)?;

    info!(
        target = %context.config.name,
        role = %context.config.role,
        session = %context.config.session,
        windowed = context.config.is_windowed(),
        composed = report.composed.len(),
        entries = report.entries.len(),
        "running Azoth runtime app"
    );

    if let Some(profile) = context.config.project_profile.clone() {
        app.insert_resource(profile);
    }
    app.insert_resource(context.clone());
    add_engine_owned_plugins(app, &context, settings.require::<Launch>()?)?;
    app.add_plugins(EnginePlugin);
    clock::install(app, &report.clocks)?;

    // Ahead of the configurers, not after them: a contribution that needs the
    // composition — to hand the registries to a plugin of its own, or to read
    // what else composed — configures against a world that already has it.
    app.insert_resource(Composition {
        report,
        registries: composer.registries(),
    });

    for contribution in contributions(composer.registries()) {
        contribution
            .configure(app, &context, &settings)
            .map_err(|source| RuntimeAppError::Contribution {
                name: contribution.name().to_string(),
                source: Box::new(source),
            })?;
    }
    Ok(())
}

/// Settle engine-owned launch configuration, then freeze it as the context.
///
/// The engine's own flags land here, before any plugin exists and before any
/// contribution runs, so nothing downstream has to reach back and rewrite what
/// logging or the asset mount already used.
fn engine_context(
    mut config: RuntimeAppConfig,
    settings: &RuntimeLaunchSettings,
) -> Result<RuntimeAppContext, RuntimeAppError> {
    let launch = settings.require::<Launch>()?;
    config.log.level = launch.level(config.log.level);
    config.log.color = launch.color;
    if let Some(assets) = launch.assets.clone() {
        config.asset_root = Some(assets);
    }
    if config.asset_root.is_none() && config.requires_runtime_assets() {
        config.asset_root = Some(resolve_runtime_asset_root(&config)?.path);
    }
    Ok(RuntimeAppContext { config })
}

fn add_engine_owned_plugins(
    app: &mut App,
    context: &RuntimeAppContext,
    launch: &Launch,
) -> Result<(), RuntimeAppError> {
    // The catalog mounts here, above every branch below, because Bevy locks the
    // asset-source table the first time `AssetPlugin::build` runs and keeps
    // whichever default source is already registered. Mounted after one, the
    // catalog resource would still land and every load would still read the
    // filesystem — the defect this ordering exists to prevent. The condition is
    // the asset root itself: a process the engine resolved no product cache for
    // gets no `AssetPlugin` either, so there is nothing to mount over and no
    // opt-out flag to carry.
    if let Some(asset_root) = &context.config.asset_root {
        app.add_plugins(launch.catalog(asset_root));
    }

    let log_plugin = engine_log_plugin(&context.config.log);
    if context.config.is_windowed() {
        return add_windowed_engine_owned_plugins(app, context, log_plugin);
    }
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(context.config.tick)));
    if let Some(asset_root) = &context.config.asset_root {
        app.add_plugins(bevy::asset::AssetPlugin {
            file_path: asset_root.to_string_lossy().into_owned(),
            watch_for_changes_override: Some(false),
            ..Default::default()
        });
    }
    if let Some(log_plugin) = log_plugin {
        app.add_plugins(log_plugin);
    }
    Ok(())
}

fn engine_log_plugin(config: &RuntimeLogConfig) -> Option<LogPlugin> {
    if config.ownership == RuntimeLogOwnership::External {
        return None;
    }
    let mut plugin = LogPlugin {
        level: config.level,
        ..Default::default()
    };
    if let Some(filter) = &config.filter {
        plugin.filter.clone_from(filter);
    }
    if let Some(custom_layer) = config.custom_layer {
        plugin.custom_layer = custom_layer;
    }
    plugin.fmt_layer = match config.color {
        RuntimeLogColor::Auto => plugin.fmt_layer,
        RuntimeLogColor::Always => runtime_log_fmt_always,
        RuntimeLogColor::Never => runtime_log_fmt_never,
    };
    Some(plugin)
}

// Signature is fixed by bevy's `LogPlugin::fmt_layer` field, which is a
// `fn(&mut App) -> Option<BoxedFmtLayer>`; dropping the `Option` would not
// assign.
#[allow(clippy::unnecessary_wraps)]
fn runtime_log_fmt_always(_: &mut App) -> Option<bevy::log::BoxedFmtLayer> {
    Some(Box::new(
        bevy::log::tracing_subscriber::fmt::Layer::default()
            .with_writer(std::io::stderr)
            .with_ansi(true),
    ))
}

// Signature is fixed by bevy's `LogPlugin::fmt_layer` field, which is a
// `fn(&mut App) -> Option<BoxedFmtLayer>`; dropping the `Option` would not
// assign.
#[allow(clippy::unnecessary_wraps)]
fn runtime_log_fmt_never(_: &mut App) -> Option<bevy::log::BoxedFmtLayer> {
    Some(Box::new(
        bevy::log::tracing_subscriber::fmt::Layer::default()
            .with_writer(std::io::stderr)
            .with_ansi(false),
    ))
}

// Signature must match the `cfg(not(feature = "windowed"))` twin below, which
// genuinely fails with `WindowedFeatureRequired`; the shared call site returns
// this directly, so dropping the `Result` here would break the non-windowed
// build.
#[allow(clippy::unnecessary_wraps)]
#[cfg(feature = "windowed")]
fn add_windowed_engine_owned_plugins(
    app: &mut App,
    context: &RuntimeAppContext,
    log_plugin: Option<LogPlugin>,
) -> Result<(), RuntimeAppError> {
    let mut plugins = DefaultPlugins.build();
    plugins = match log_plugin {
        Some(log_plugin) => plugins.set(log_plugin),
        None => plugins.disable::<LogPlugin>(),
    };
    if let Some(asset_root) = &context.config.asset_root {
        plugins = plugins.set(bevy::asset::AssetPlugin {
            file_path: asset_root.to_string_lossy().into_owned(),
            watch_for_changes_override: Some(true),
            ..Default::default()
        });
    }
    app.add_plugins(plugins);
    Ok(())
}

#[cfg(not(feature = "windowed"))]
fn add_windowed_engine_owned_plugins(
    _: &mut App,
    _: &RuntimeAppContext,
    _: Option<LogPlugin>,
) -> Result<(), RuntimeAppError> {
    Err(RuntimeAppError::WindowedFeatureRequired)
}

#[derive(Debug, Error)]
pub enum RuntimeAppError {
    #[error("invalid runtime app config: {0}")]
    InvalidConfig(String),

    #[error(transparent)]
    RuntimeAssets(#[from] RuntimeAssetRootError),

    /// The composition itself does not validate — a duplicate registry key or
    /// a clock nothing defines. Raised where the engine reads the composition,
    /// so a generated `main` reports it instead of having to ask first.
    #[error(transparent)]
    Compose(#[from] ComposeError),

    #[error("windowed runtime composition requires the `az-runtime-app/windowed` feature")]
    WindowedFeatureRequired,

    #[error("runtime app contribution `{name}` failed: {source}")]
    Contribution {
        name: String,
        #[source]
        source: Box<Self>,
    },

    #[error("host role `{role}` composes no world, so it has no runtime app to run")]
    NoWorld { role: GemTargetRole },

    #[error("host role `{role}` composed a contribution that is not ready to run")]
    NotReady { role: GemTargetRole },

    #[error("runtime app exited with code {code}")]
    Exit { code: u8 },

    #[error("clock `{clock}` declares rate {rate} Hz, which is not a tickable frequency")]
    InvalidClock {
        clock: az_gem_contract::ClockName,
        rate: f64,
    },

    #[error("clock `{clock}` could not build its schedule: {detail}")]
    ClockSchedule {
        clock: az_gem_contract::ClockName,
        detail: String,
    },

    #[error("system `{system}` declares clock `{clock}` but runs on no schedule for it")]
    ClockAbsent {
        clock: az_gem_contract::ClockName,
        system: &'static str,
    },

    #[error("system `{system}` declares clock `{clock}` but also runs on clock `{stray}`")]
    ClockStray {
        clock: az_gem_contract::ClockName,
        system: &'static str,
        stray: az_gem_contract::ClockName,
    },

    #[error(
        "generated runtime launch settings type was registered more than once (second registration `{name}`)"
    )]
    DuplicateRuntimeLaunchSettingsType { name: &'static str },

    #[error("invalid generated runtime launch arguments: {0}")]
    RuntimeLaunchArguments(#[source] clap::Error),

    #[error("runtime contribution requires unparsed launch settings `{type_name}`")]
    MissingRuntimeLaunchSettings { type_name: &'static str },
}

/// Failure to find a usable local runtime product cache.
#[derive(Debug, Error)]
pub enum RuntimeAssetRootError {
    #[error("could not determine the current directory: {source}; {RUNTIME_ASSETS_REMEDIATION}")]
    CurrentDirectory {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "could not discover an Azoth project from {start}: {source}; {RUNTIME_ASSETS_REMEDIATION}"
    )]
    Project {
        start: PathBuf,
        #[source]
        source: Box<az_project::ProjectManifestError>,
    },

    #[error(
        "could not load the Azoth project manifest at {root}: {source}; {RUNTIME_ASSETS_REMEDIATION}"
    )]
    ProjectManifest {
        root: PathBuf,
        #[source]
        source: Box<az_project::ProjectManifestError>,
    },

    #[error(
        "remote runtime assets path `{path}` is not supported yet; pass a local --assets <Cache/pc> or run azoth build"
    )]
    RemoteOverride { path: String },

    #[error("runtime assets unavailable: {detail}; {RUNTIME_ASSETS_REMEDIATION}")]
    Unavailable { detail: String },
}

#[cfg(test)]
mod tests;
