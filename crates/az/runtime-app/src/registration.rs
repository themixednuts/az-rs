//! What a contribution gives a runtime app: deferred app configuration and
//! typed launch arguments.
//!
//! Both arrive through the composing host's registrars, so a runtime app runs
//! exactly what its own composition registered. Neither carries a role: the
//! composition *is* the role closure — generation only builds a target's glue
//! from role-matching contributions — so a second role filter here would be a
//! table beside the one fact, and the fail-open `roles.is_empty()` wildcard
//! that came with it is gone.

use std::any::{Any, TypeId, type_name};
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::path::PathBuf;

use az_asset::PackagePayloadKind;
use az_framework::asset::AssetCatalogPlugin;
use az_gem_contract::{HostsWorld, Registries, RegistryEntry, Unconditional};
use bevy::prelude::App;
use clap::{ArgMatches, Args, Command, FromArgMatches};

use crate::{RuntimeAppContext, RuntimeAppError};

/// Configures the composed `App` once the engine owns it.
///
/// This runs after the engine's own plugins are installed and after the merged
/// command line is parsed, which is why it exists at all: composition happens
/// before a process has arguments, so anything a contribution derives from the
/// command line has to be deferred to here.
///
/// There is exactly one phase. The two-phase builder this replaced let a
/// pre-default read observe nothing a post-default write had done yet, a hundred
/// lines and two crates apart — the mechanism behind a shipped server defect.
/// Engine-owned launch configuration is settled from [`Launch`] before any of
/// these run, so nothing needs a phase before the plugins any more.
pub type RuntimeAppConfigurer =
    fn(&mut App, &RuntimeAppContext, &RuntimeLaunchSettings) -> Result<(), RuntimeAppError>;

/// One contribution's deferred configuration of a runtime app.
///
/// The name is the compose key: it is what a failing configuration is reported
/// under, and it orders same-priority contributions in diagnostics.
#[derive(Clone, Copy)]
pub struct RuntimeAppContributionRegistration {
    name: &'static str,
    priority: i32,
    configure: RuntimeAppConfigurer,
}

impl RuntimeAppContributionRegistration {
    /// Contribute app configuration at default priority.
    #[must_use]
    pub const fn new(name: &'static str, configure: RuntimeAppConfigurer) -> Self {
        Self {
            name,
            priority: 0,
            configure,
        }
    }

    /// Override composition precedence. Lower priorities configure first.
    ///
    /// This is real domain precedence: a base runtime installs the plugins a
    /// role layer then configures, so the order is the answer, not a
    /// presentation detail. Ties fall to composition order — the resolved lock
    /// closure — and nothing else.
    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub const fn priority(&self) -> i32 {
        self.priority
    }

    /// # Errors
    ///
    /// Whatever the contribution reports while configuring the app.
    pub fn configure(
        &self,
        app: &mut App,
        context: &RuntimeAppContext,
        settings: &RuntimeLaunchSettings,
    ) -> Result<(), RuntimeAppError> {
        (self.configure)(app, context, settings)
    }
}

impl RegistryEntry for RuntimeAppContributionRegistration {
    /// `HostsWorld`: this hands out `&mut App`, so the capability that says a
    /// host owns a world is the floor for registering into it. A host without
    /// one cannot accept the registration at all, rather than accepting it and
    /// dropping it.
    type Requires = HostsWorld;
    type Key = &'static str;

    fn registry_name() -> &'static str {
        "runtime-app-contribution"
    }

    fn key(&self) -> &'static str {
        self.name
    }
}

/// App contributions this host composed, in configuration order.
///
/// Priority ascending, then composition order. The sort is stable, so the
/// second key is the resolved closure order the composition already carries.
#[must_use]
pub fn contributions(registries: &Registries) -> Vec<&RuntimeAppContributionRegistration> {
    let mut contributions = registries
        .get::<RuntimeAppContributionRegistration>()
        .map(|registry| registry.entries().collect::<Vec<_>>())
        .unwrap_or_default();
    contributions.sort_by_key(|contribution| contribution.priority());
    contributions
}

/// Type-erased values parsed once from the runtime's merged command line.
///
/// Each contribution owns its argument type and registers it through
/// [`RuntimeLaunchSettingsRegistration`]; the runtime builds one Clap command
/// for the whole composition, so a unified target validates its complete
/// argument surface in one pass instead of through nested parsers.
#[derive(Default)]
pub struct RuntimeLaunchSettings {
    values: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl RuntimeLaunchSettings {
    #[must_use]
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref())
    }

    /// # Errors
    ///
    /// [`RuntimeAppError::MissingRuntimeLaunchSettings`] when `T` was never
    /// registered into this composition, so the merged command never parsed it.
    pub fn require<T: Any + Send + Sync>(&self) -> Result<&T, RuntimeAppError> {
        self.get::<T>()
            .ok_or(RuntimeAppError::MissingRuntimeLaunchSettings {
                type_name: type_name::<T>(),
            })
    }
}

type RuntimeLaunchArgsAugmenter = fn(Command) -> Command;
type RuntimeLaunchArgsParser = fn(&ArgMatches) -> Result<Box<dyn Any + Send + Sync>, clap::Error>;
type RuntimeLaunchArgsTypeId = fn() -> TypeId;

fn augment_runtime_launch_args<T: Args>(command: Command) -> Command {
    T::augment_args(command)
}

fn parse_runtime_launch_args<T>(
    matches: &ArgMatches,
) -> Result<Box<dyn Any + Send + Sync>, clap::Error>
where
    T: FromArgMatches + Send + Sync + 'static,
{
    T::from_arg_matches(matches).map(|settings| Box::new(settings) as Box<dyn Any + Send + Sync>)
}

const fn runtime_launch_args_type_id<T: 'static>() -> TypeId {
    TypeId::of::<T>()
}

/// One typed argument group contributed to a runtime's merged command line.
#[derive(Clone, Copy)]
pub struct RuntimeLaunchSettingsRegistration {
    name: &'static str,
    priority: i32,
    augment: RuntimeLaunchArgsAugmenter,
    parse: RuntimeLaunchArgsParser,
    type_id: RuntimeLaunchArgsTypeId,
}

impl RuntimeLaunchSettingsRegistration {
    /// Contribute an argument group at default priority.
    #[must_use]
    pub const fn new<T>(name: &'static str) -> Self
    where
        T: Args + FromArgMatches + Send + Sync + 'static,
    {
        Self {
            name,
            priority: 0,
            augment: augment_runtime_launch_args::<T>,
            parse: parse_runtime_launch_args::<T>,
            type_id: runtime_launch_args_type_id::<T>,
        }
    }

    /// Override composition precedence. Lower priorities compose first.
    ///
    /// This is real domain precedence: argument groups reach one Clap command
    /// in this order, so it decides `--help` layout and which group owns a
    /// shared short flag. Ties fall to composition order and nothing else.
    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub const fn priority(&self) -> i32 {
        self.priority
    }
}

impl RegistryEntry for RuntimeLaunchSettingsRegistration {
    /// Parsing a command line needs nothing of the host: the floor lives on
    /// the app contribution that consumes the parsed value, not here.
    type Requires = Unconditional;
    type Key = &'static str;

    fn registry_name() -> &'static str {
        "runtime-launch-settings"
    }

    fn key(&self) -> &'static str {
        self.name
    }
}

/// Argument groups this host composed, in command-building order.
///
/// Priority ascending, then composition order, on the same terms as
/// [`contributions`].
#[must_use]
pub fn launch_settings(registries: &Registries) -> Vec<&RuntimeLaunchSettingsRegistration> {
    let mut registrations = registries
        .get::<RuntimeLaunchSettingsRegistration>()
        .map(|registry| registry.entries().collect::<Vec<_>>())
        .unwrap_or_default();
    registrations.sort_by_key(|registration| registration.priority());
    registrations
}

/// Every package container the engine can read products through.
///
/// The names come from [`PackagePayloadKind::container_name`], so what
/// `--container` accepts and what the reader selects cannot disagree about
/// spelling.
const CONTAINERS: [PackagePayloadKind; 3] = [
    PackagePayloadKind::Loose,
    PackagePayloadKind::AzPack,
    PackagePayloadKind::Pak,
];

/// Resolve a session-reported container name to the kind that reads it.
fn container(value: &str) -> Result<PackagePayloadKind, String> {
    PackagePayloadKind::from_container_name(value).ok_or_else(|| {
        format!(
            "unknown package container `{value}`; the engine reads {}",
            CONTAINERS
                .map(PackagePayloadKind::container_name)
                .join(", ")
        )
    })
}

/// The engine's own launch arguments.
///
/// Logging, the runtime asset root, and the catalog mounted over it are
/// engine-owned — the engine installs the log plugin, resolves the product
/// cache, and mounts the catalog — so the flags that set them are engine-owned
/// too, and are parsed before any contribution runs. This is what replaced the
/// builder phase in which contributions used to reach back and rewrite launch
/// configuration.
#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct Launch {
    /// Runtime asset root (a processed `Cache/<platform>` directory).
    #[arg(long, value_name = "DIR", global = true)]
    pub assets: Option<PathBuf>,
    /// Package container the runtime assets ship in.
    #[arg(
        long,
        value_name = "NAME",
        value_parser = container,
        requires = "payload",
        global = true
    )]
    pub container: Option<PackagePayloadKind>,
    /// Package payload the container reads products from.
    #[arg(long, value_name = "FILE", requires = "container", global = true)]
    pub payload: Option<PathBuf>,
    /// Raise log verbosity; repeatable.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
    /// Log errors only.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
    /// ANSI color policy for log output.
    #[arg(long, value_enum, default_value_t = crate::RuntimeLogColor::Auto, global = true)]
    pub color: crate::RuntimeLogColor,
}

impl Launch {
    /// The log level these flags select, over the launcher's own default.
    #[must_use]
    pub const fn level(&self, default: bevy::log::Level) -> bevy::log::Level {
        if self.quiet {
            return bevy::log::Level::ERROR;
        }
        match self.verbose {
            0 => default,
            1 => bevy::log::Level::DEBUG,
            _ => bevy::log::Level::TRACE,
        }
    }

    /// The catalog this launch mounts over `asset_root`.
    ///
    /// A launcher that says nothing gets the root read as the engine finds it:
    /// the native catalog with an auto-detected payload backend. A session that
    /// already reported a built package names it instead, and `--container` and
    /// `--payload` are admitted only as the pair
    /// [`AssetCatalogPlugin::native_package`] takes — the packaged form is
    /// either fully reported or absent, never half of one.
    #[must_use]
    pub fn catalog(&self, asset_root: impl Into<PathBuf>) -> AssetCatalogPlugin {
        match (self.container, self.payload.as_ref()) {
            (Some(container), Some(payload)) => {
                AssetCatalogPlugin::native_package(asset_root, container, payload)
            }
            _ => AssetCatalogPlugin::new(asset_root),
        }
    }
}

/// Parse the whole composition's command line in one pass.
///
/// # Errors
///
/// [`RuntimeAppError::DuplicateRuntimeLaunchSettingsType`] when two argument
/// groups carry one type, so a `require` could not say which it meant, or
/// [`RuntimeAppError::RuntimeLaunchArguments`] when the command line does not
/// satisfy the merged command.
pub fn parse<I, T>(
    name: &str,
    registries: &Registries,
    args: I,
) -> Result<RuntimeLaunchSettings, RuntimeAppError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let registrations = launch_settings(registries);
    let mut types = BTreeSet::new();
    let mut command = Launch::augment_args(
        Command::new("azoth-runtime")
            .bin_name(name.to_owned())
            .version(env!("CARGO_PKG_VERSION"))
            .about("Launch an AZoth-generated project runtime")
            .after_help(
                "Environment:\n  RUST_LOG  Layer tracing directives over -v/--verbose or -q/--quiet.\n  NO_COLOR  Disable automatic color output.",
            ),
    );
    for registration in &registrations {
        if !types.insert((registration.type_id)()) {
            return Err(RuntimeAppError::DuplicateRuntimeLaunchSettingsType {
                name: registration.name(),
            });
        }
        command = (registration.augment)(command);
    }

    let matches = command
        .try_get_matches_from(args)
        .map_err(RuntimeAppError::RuntimeLaunchArguments)?;
    let mut settings = RuntimeLaunchSettings::default();
    settings.values.insert(
        TypeId::of::<Launch>(),
        parse_runtime_launch_args::<Launch>(&matches)
            .map_err(RuntimeAppError::RuntimeLaunchArguments)?,
    );
    for registration in registrations {
        settings.values.insert(
            (registration.type_id)(),
            (registration.parse)(&matches).map_err(RuntimeAppError::RuntimeLaunchArguments)?,
        );
    }
    Ok(settings)
}
