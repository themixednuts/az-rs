//! Compose-seam tests: what a host ends up with after composing a set of
//! contributions, and what the engine then builds from it.

use az_asset::PackagePayloadKind;
use az_framework::asset::{AssetCatalog, AssetCatalogSource, CatalogAssetPayloadBackend};
use az_gem_contract::{
    ClockDefinition, ClockName, ClockUse, ComposeError, Composer, Contribution,
    ContributionDescriptor, ContributionId, GemContext, GemId, GemTargetRole, ProductActivation,
    declare_caps,
};
use bevy::asset::io::{AssetReaderError, AssetSourceId, Reader};
use bevy::ecs::schedule::ScheduleLabel;
use bevy::tasks::block_on;

use super::*;

declare_caps!(WorldCaps: HostsWorld);

const SIMULATION: ClockName = ClockName::new("simulation");
const INPUT_FRAMING: ClockName = ClockName::new("input-framing");

#[derive(Resource, Default, Debug, PartialEq, Eq)]
struct Order(Vec<&'static str>);

// Signature must match the contribution callback
// `RuntimeAppContributionRegistration::new` takes; dropping the `Result`
// would make it unregisterable.
#[allow(clippy::unnecessary_wraps)]
fn base(
    app: &mut App,
    _context: &RuntimeAppContext,
    _settings: &RuntimeLaunchSettings,
) -> Result<(), RuntimeAppError> {
    app.init_resource::<Order>();
    app.world_mut().resource_mut::<Order>().0.push("base");
    Ok(())
}

// Signature must match the contribution callback
// `RuntimeAppContributionRegistration::new` takes; dropping the `Result`
// would make it unregisterable.
#[allow(clippy::unnecessary_wraps)]
fn role(
    app: &mut App,
    _context: &RuntimeAppContext,
    _settings: &RuntimeLaunchSettings,
) -> Result<(), RuntimeAppError> {
    app.world_mut().resource_mut::<Order>().0.push("role");
    Ok(())
}

#[derive(clap::Args, Debug, Clone, PartialEq, Eq)]
struct TestArgs {
    #[arg(long)]
    test_value: String,
}

#[derive(Resource, Debug, PartialEq, Eq)]
struct Parsed(String);

fn from_settings(
    app: &mut App,
    _context: &RuntimeAppContext,
    settings: &RuntimeLaunchSettings,
) -> Result<(), RuntimeAppError> {
    let value = settings.require::<TestArgs>()?.test_value.clone();
    app.insert_resource(Parsed(value));
    Ok(())
}

/// One contribution, registering both families, in an order that is not the
/// order the host will use them in.
struct Runtime;

impl Contribution for Runtime {
    type Caps = WorldCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("azoth.runtime-app-tests"),
            contribution: ContributionId::new("runtime"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, WorldCaps>) {
        ctx.registrar::<RuntimeAppContributionRegistration>()
            .register_many([
                RuntimeAppContributionRegistration::new("az.test.role", role).with_priority(20),
                RuntimeAppContributionRegistration::new("az.test.base", base).with_priority(10),
                RuntimeAppContributionRegistration::new("az.test.settings", from_settings)
                    .with_priority(30),
            ]);
        ctx.registrar::<RuntimeLaunchSettingsRegistration>()
            .register(
                RuntimeLaunchSettingsRegistration::new::<TestArgs>("az.test.args")
                    .with_priority(100),
            );
    }
}

fn composed() -> Composer {
    let mut composer = Composer::new(GemTargetRole::Server);
    composer
        .add(Runtime, ProductActivation::default())
        .expect("a server host provides HostsWorld");
    composer
}

fn config() -> RuntimeAppConfig {
    RuntimeAppConfig::new("test", GemTargetRole::Server, SessionMode::Live)
}

/// Drive a composition the way [`run`] does: take the app the composition
/// owns, then pin what is left, because the built app outlives the call and
/// reads its registries.
fn pinned(mut composer: Composer) -> (App, &'static Composer) {
    let app = composer.take_app().expect("the role owns a world");
    (app, Box::leak(Box::new(composer)))
}

fn built(composer: Composer, args: &[&str]) -> App {
    let (mut app, composer) = pinned(composer);
    build(config(), composer, &mut app, args.iter().copied()).expect("the target builds");
    app
}

/// The v8 generated main's call, pinned as types rather than as a call —
/// running it would block on the app loop. If this stops compiling, every
/// generated ship crate stops compiling with it.
#[test]
fn the_generated_main_calls_this_shape() {
    let entry: fn(RuntimeAppConfig, Composer) -> Result<(), RuntimeAppError> = run;
    let _ = entry;
    let _ = RuntimeAppConfig::new("target", GemTargetRole::Client, SessionMode::Live)
        .with_runtime_project("project", PathBuf::from("project"));
    // The generated `main` returns `Box<dyn Error>`, so `run`'s error has to
    // convert through `?`.
    let _: Box<dyn std::error::Error> = Box::new(RuntimeAppError::WindowedFeatureRequired);
}

#[test]
fn runtime_errors_stay_below_large_result_threshold() {
    assert!(std::mem::size_of::<RuntimeAppError>() < 128);
    assert!(std::mem::size_of::<RuntimeAssetRootError>() < 128);
}

#[test]
fn runtime_asset_errors_preserve_project_manifest_source_chain() {
    let start = PathBuf::from("missing-project");
    let asset_error = RuntimeAssetRootError::Project {
        start: start.clone(),
        source: Box::new(az_project::ProjectManifestError::ProjectRootNotFound { start }),
    };

    let project_error = std::error::Error::source(&asset_error).expect("project manifest source");
    let project_error = project_error
        .downcast_ref::<Box<az_project::ProjectManifestError>>()
        .expect("typed project manifest source");
    assert!(matches!(
        project_error.as_ref(),
        az_project::ProjectManifestError::ProjectRootNotFound { .. }
    ));

    let error = RuntimeAppError::RuntimeAssets(asset_error);
    let project_error = std::error::Error::source(&error).expect("runtime app source");
    let project_error = project_error
        .downcast_ref::<Box<az_project::ProjectManifestError>>()
        .expect("typed project manifest source");
    assert!(matches!(
        project_error.as_ref(),
        az_project::ProjectManifestError::ProjectRootNotFound { .. }
    ));
}

#[test]
fn composed_contributions_configure_in_priority_then_composition_order() {
    let app = built(composed(), &["test", "--test-value", "typed"]);

    assert_eq!(
        app.world().resource::<Order>(),
        &Order(vec!["base", "role"])
    );
    assert_eq!(
        app.world().resource::<Parsed>(),
        &Parsed("typed".to_string())
    );
}

#[test]
fn same_priority_contributions_keep_composition_order() {
    // Signature must match the contribution callback
    // `RuntimeAppContributionRegistration::new` takes; dropping the `Result`
    // would make it unregisterable.
    #[allow(clippy::unnecessary_wraps)]
    fn first(
        app: &mut App,
        _: &RuntimeAppContext,
        _: &RuntimeLaunchSettings,
    ) -> Result<(), RuntimeAppError> {
        app.init_resource::<Order>();
        app.world_mut().resource_mut::<Order>().0.push("first");
        Ok(())
    }

    // Signature must match the contribution callback
    // `RuntimeAppContributionRegistration::new` takes; dropping the `Result`
    // would make it unregisterable.
    #[allow(clippy::unnecessary_wraps)]
    fn second(
        app: &mut App,
        _: &RuntimeAppContext,
        _: &RuntimeLaunchSettings,
    ) -> Result<(), RuntimeAppError> {
        app.world_mut().resource_mut::<Order>().0.push("second");
        Ok(())
    }

    struct Tied;

    impl Contribution for Tied {
        type Caps = WorldCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.runtime-app-tied"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, WorldCaps>) {
            // Registered second-then-first alphabetically on purpose: the
            // tiebreak is the closure order, not the name.
            ctx.registrar::<RuntimeAppContributionRegistration>()
                .register_many([
                    RuntimeAppContributionRegistration::new("az.test.zulu", first),
                    RuntimeAppContributionRegistration::new("az.test.alpha", second),
                ]);
        }
    }

    let mut composer = Composer::new(GemTargetRole::Server);
    composer
        .add(Tied, ProductActivation::default())
        .expect("a server host provides HostsWorld");
    let app = built(composer, &["test"]);

    assert_eq!(
        app.world().resource::<Order>(),
        &Order(vec!["first", "second"])
    );
}

#[test]
fn every_composed_registration_is_attributed_to_its_contribution() {
    let report = composed().finalize().expect("composition is valid");
    let registries = report
        .entries
        .iter()
        .filter(|entry| {
            entry.registry == "runtime-app-contribution"
                || entry.registry == "runtime-launch-settings"
        })
        .collect::<Vec<_>>();

    assert_eq!(registries.len(), 4);
    assert!(registries.iter().all(|entry| {
        entry.instance.gem.as_str() == "azoth.runtime-app-tests"
            && entry.instance.contribution.as_str() == "runtime"
    }));
}

/// What a configurer saw of its own composition while it was configuring.
#[derive(Resource, Debug, PartialEq, Eq)]
struct Seen {
    composed: usize,
    configurers: usize,
}

/// A contribution that needs the composition itself — the shape a wire or
/// scripting plugin uses: read [`Composition`] out of the world being
/// configured and hand its registries on.
// Signature must match the contribution callback
// `RuntimeAppContributionRegistration::new` takes; dropping the `Result`
// would make it unregisterable.
#[allow(clippy::unnecessary_wraps)]
fn read_composition(
    app: &mut App,
    _: &RuntimeAppContext,
    _: &RuntimeLaunchSettings,
) -> Result<(), RuntimeAppError> {
    let composition = app.world().resource::<Composition>();
    let seen = Seen {
        composed: composition.report.composed.len(),
        configurers: composition
            .registries
            .get::<RuntimeAppContributionRegistration>()
            .map_or(0, az_gem_contract::Registry::len),
    };
    app.insert_resource(seen);
    Ok(())
}

#[test]
fn a_contribution_reads_the_composition_while_it_configures() {
    struct Reader;

    impl Contribution for Reader {
        type Caps = WorldCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.runtime-app-reader"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, WorldCaps>) {
            ctx.registrar::<RuntimeAppContributionRegistration>()
                .register(RuntimeAppContributionRegistration::new(
                    "az.test.composition",
                    read_composition,
                ));
        }
    }

    let mut composer = Composer::new(GemTargetRole::Server);
    composer
        .add(Reader, ProductActivation::default())
        .expect("a server host provides HostsWorld");
    let (mut app, composer) = pinned(composer);
    build(config(), composer, &mut app, ["test"]).expect("the target builds");

    // The resource the configurer wrote is the proof of ordering: it could
    // only read a `Composition` that was already in the world when it ran.
    assert_eq!(
        app.world().resource::<Seen>(),
        &Seen {
            composed: 1,
            configurers: 1
        }
    );
    // And what it read is the composition this host pinned, not a copy of it.
    assert!(std::ptr::eq(
        app.world().resource::<Composition>().registries,
        composer.registries()
    ));
}

#[test]
fn two_contributions_under_one_name_fail_composition() {
    struct Rival;

    impl Contribution for Rival {
        type Caps = WorldCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.runtime-app-rival"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, WorldCaps>) {
            ctx.registrar::<RuntimeAppContributionRegistration>()
                .register(RuntimeAppContributionRegistration::new(
                    "az.test.base",
                    role,
                ));
        }
    }

    let mut composer = composed();
    composer
        .add(Rival, ProductActivation::default())
        .expect("a server host provides HostsWorld");

    let error = composer.finalize().expect_err("the name collides");
    let ComposeError::Duplicate {
        registry,
        key,
        first,
        second,
    } = error
    else {
        panic!("expected a duplicate-key failure, got {error}");
    };
    assert_eq!(registry, "runtime-app-contribution");
    assert_eq!(key, "az.test.base");
    assert_eq!(first.gem.as_str(), "azoth.runtime-app-tests");
    assert_eq!(second.gem.as_str(), "azoth.runtime-app-rival");
}

#[test]
fn a_role_without_a_world_refuses_the_app_contribution_floor() {
    let mut composer = Composer::new(GemTargetRole::AssetWorker);
    let refusal = composer
        .add(Runtime, ProductActivation::default())
        .expect_err("an asset worker hosts no world");

    assert_eq!(refusal.required.to_string(), "{HostsWorld}");
    assert_eq!(refusal.provided.to_string(), "{}");
}

#[test]
fn a_role_without_a_world_has_no_runtime_app_to_run() {
    // `Tool` composes nothing here, which used to be its own error at process
    // start; the empty-role case is a generation failure now. What survives is
    // the honest one: the role owns no `App` at all. `run` reports it before it
    // pins anything or reaches a run loop.
    let error = run(
        RuntimeAppConfig::new("test", GemTargetRole::Tool, SessionMode::Live),
        Composer::new(GemTargetRole::Tool),
    )
    .expect_err("a tool host has no world");

    assert!(
        matches!(
            error,
            RuntimeAppError::NoWorld {
                role: GemTargetRole::Tool
            }
        ),
        "{error}"
    );
}

#[test]
fn the_engine_owns_its_own_launch_flags() {
    let app = built(
        composed(),
        &["test", "--test-value", "typed", "-vv", "--color", "never"],
    );
    let context = app.world().resource::<RuntimeAppContext>();

    assert_eq!(context.config().log.level, bevy::log::Level::TRACE);
    assert_eq!(context.config().log.color, RuntimeLogColor::Never);
    assert_eq!(context.session(), SessionMode::Live);
    assert!(context.is_headless());
}

#[test]
fn an_unknown_flag_fails_the_whole_merged_command() {
    let (mut app, composer) = pinned(composed());
    let error = build(config(), composer, &mut app, ["test", "--nonsense"])
        .expect_err("the merged command rejects it");

    assert!(
        matches!(error, RuntimeAppError::RuntimeLaunchArguments(_)),
        "{error}"
    );
}

#[test]
fn one_argument_type_cannot_be_registered_twice() {
    struct Rival;

    impl Contribution for Rival {
        type Caps = WorldCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.runtime-app-rival"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, WorldCaps>) {
            ctx.registrar::<RuntimeLaunchSettingsRegistration>()
                .register(
                    RuntimeLaunchSettingsRegistration::new::<TestArgs>("az.test.rival-args")
                        .with_priority(200),
                );
        }
    }

    let mut composer = composed();
    composer
        .add(Rival, ProductActivation::default())
        .expect("a server host provides HostsWorld");
    let (mut app, composer) = pinned(composer);
    let error = build(config(), composer, &mut app, ["test"])
        .expect_err("one type cannot answer two registrations");

    assert!(
        matches!(
            error,
            RuntimeAppError::DuplicateRuntimeLaunchSettingsType {
                name: "az.test.rival-args"
            }
        ),
        "{error}"
    );
}

/// Two clocks that coexist — the shape `unified` needs: a fixed authority
/// clock and a slower client clock, each with its own schedule. Composed under
/// a headless role because the client stack is behind the `windowed` feature;
/// the clocks are the same either way.
struct Clocks;

impl Contribution for Clocks {
    type Caps = WorldCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("azoth.runtime-app-clocks"),
            contribution: ContributionId::new("runtime"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, WorldCaps>) {
        ctx.registrar::<ClockDefinition>().register_many([
            ClockDefinition {
                name: SIMULATION,
                rate_hz: Some(60.0),
            },
            ClockDefinition {
                name: INPUT_FRAMING,
                rate_hz: Some(30.0),
            },
        ]);
        ctx.registrar::<ClockUse>().register_many([
            ClockUse {
                clock: SIMULATION,
                system: "simulate",
            },
            ClockUse {
                clock: INPUT_FRAMING,
                system: "frame_input",
            },
        ]);
        ctx.app().add_systems(Clock(SIMULATION), simulate);
        ctx.app().add_systems(Clock(INPUT_FRAMING), frame_input);
    }
}

#[derive(Resource, Default, Debug)]
struct Ticks {
    simulation: u32,
    input: u32,
}

fn simulate(mut ticks: ResMut<Ticks>) {
    ticks.simulation += 1;
}

fn frame_input(mut ticks: ResMut<Ticks>) {
    ticks.input += 1;
}

fn composed_clocks() -> Composer {
    let mut composer = Composer::new(GemTargetRole::Server);
    composer
        .add(Clocks, ProductActivation::default())
        .expect("a server host provides HostsWorld");
    composer
}

/// Everything the clock tests build: the clock composition, pinned, with its
/// app built under a server config.
fn built_clocks() -> (App, &'static Composer) {
    let (mut app, composer) = pinned(composed_clocks());
    build(
        RuntimeAppConfig::new("test", GemTargetRole::Server, SessionMode::Live),
        composer,
        &mut app,
        ["test"],
    )
    .expect("the target builds");
    (app, composer)
}

#[test]
fn each_composed_clock_becomes_its_own_schedule() {
    let (app, _) = built_clocks();
    assert_eq!(app.world().resource::<Composition>().report.clocks.len(), 2);

    assert!(app.get_schedule(Clock(SIMULATION)).is_some());
    assert!(app.get_schedule(Clock(INPUT_FRAMING)).is_some());
    // Bevy owns one `FixedUpdate`; a second named clock is a schedule of its
    // own, so neither clock is the other's `Time<Fixed>`.
    assert_ne!(Clock(SIMULATION).intern(), Clock(INPUT_FRAMING).intern());
}

#[test]
fn a_fixed_clock_runs_at_its_own_rate() {
    let (mut app, _) = built_clocks();
    app.init_resource::<Ticks>();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_millis(100),
    ));

    // One second of time in ten frames: the 60 Hz clock owes 60 steps and the
    // 30 Hz clock 30, from the same frames, because each carries its own
    // accumulator rather than sharing one `Time<Fixed>`.
    for _ in 0..10 {
        app.update();
    }

    // Exact step arithmetic is `Pace`'s own test; what this asserts is the
    // wiring: both clocks ticked, from the same frames, and the faster one
    // ticked more — which one shared `Time<Fixed>` could not have produced.
    let ticks = app.world().resource::<Ticks>();
    assert!(ticks.input > 0, "{ticks:?}");
    assert!(ticks.simulation > ticks.input, "{ticks:?}");
}

#[test]
fn a_declared_clock_use_must_be_scheduled_on_that_clock() {
    let (mut app, composer) = built_clocks();

    audit(&mut app, composer.registries()).expect("every declaration matches its placement");
}

#[test]
fn a_system_on_the_wrong_clock_is_a_named_failure() {
    /// Declares `simulate` on the input clock while scheduling it on the
    /// simulation clock — the disagreement the audit exists to catch.
    struct Crossed;

    impl Contribution for Crossed {
        type Caps = WorldCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.runtime-app-crossed"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, WorldCaps>) {
            ctx.registrar::<ClockDefinition>().register_many([
                ClockDefinition {
                    name: SIMULATION,
                    rate_hz: Some(60.0),
                },
                ClockDefinition {
                    name: INPUT_FRAMING,
                    rate_hz: Some(30.0),
                },
            ]);
            ctx.registrar::<ClockUse>().register(ClockUse {
                clock: INPUT_FRAMING,
                system: "simulate",
            });
            ctx.app().add_systems(Clock(SIMULATION), simulate);
        }
    }

    let mut composer = Composer::new(GemTargetRole::Server);
    composer
        .add(Crossed, ProductActivation::default())
        .expect("a server host provides HostsWorld");
    let (mut app, composer) = pinned(composer);
    build(
        RuntimeAppConfig::new("test", GemTargetRole::Server, SessionMode::Live),
        composer,
        &mut app,
        ["test"],
    )
    .expect("the target builds");
    app.init_resource::<Ticks>();

    let error = audit(&mut app, composer.registries()).expect_err("the declaration disagrees");

    assert!(
        matches!(
            error,
            RuntimeAppError::ClockAbsent {
                clock: INPUT_FRAMING,
                system: "simulate"
            }
        ),
        "{error}"
    );
}

#[test]
fn an_unresolvable_clock_fails_composition_rather_than_defaulting() {
    struct Unresolvable;

    impl Contribution for Unresolvable {
        type Caps = WorldCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.runtime-app-unresolvable"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, WorldCaps>) {
            ctx.registrar::<ClockUse>().register(ClockUse {
                clock: SIMULATION,
                system: "simulate",
            });
        }
    }

    let mut composer = Composer::new(GemTargetRole::Server);
    composer
        .add(Unresolvable, ProductActivation::default())
        .expect("a server host provides HostsWorld");

    let error = composer
        .finalize()
        .expect_err("nothing defines the declared clock");
    assert!(
        matches!(error, ComposeError::UnresolvableClock { clock, .. } if clock == SIMULATION),
        "{error}"
    );
}

#[test]
fn resolve_runtime_asset_root_rejects_remote_overrides() {
    let config = config().with_asset_root("https://example.test/Cache/pc");
    let error = resolve_runtime_asset_root(&config).expect_err("remote override");

    assert!(matches!(
        error,
        RuntimeAssetRootError::RemoteOverride { ref path }
            if path == "https://example.test/Cache/pc"
    ));
    let message = error.to_string();
    assert!(message.contains("--assets"), "{message}");
}

#[test]
fn resolve_runtime_asset_root_keeps_explicit_local_override() {
    let path = PathBuf::from("tmp/Cache/pc");
    let resolved = resolve_runtime_asset_root(&config().with_asset_root(path.clone())).unwrap();

    assert_eq!(resolved.path, path);
    assert_eq!(resolved.source, RuntimeAssetRootSource::ExplicitOverride);
}

/// A product the fixture catalog knows about.
const PRODUCT: &str = "fixtures/mounted.bin";
/// A file that exists on disk and in no catalog — what a filesystem-only
/// default source would happily serve.
const STRAY: &str = "stray.bin";

/// A minimal native product cache: one catalogued product, one uncatalogued
/// file beside it, and the `assetcatalog.bin` that names only the first.
fn assets() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("temporary asset root");
    let product = temp.path().join(PRODUCT);
    std::fs::create_dir_all(product.parent().expect("product parent"))
        .expect("create product directory");
    std::fs::write(&product, b"mounted").expect("write catalogued product");
    std::fs::write(temp.path().join(STRAY), b"stray").expect("write uncatalogued file");

    let catalog = az_asset::AssetCatalog::new(vec![az_asset::AssetCatalogEntry::new(
        az_asset::AssetId::new(uuid::Uuid::from_bytes([1; 16]), 1),
        uuid::Uuid::from_bytes([2; 16]),
        "az.test.raw",
        1,
        PRODUCT,
        None,
        7,
        [0; az_asset::PACKAGE_CONTENT_HASH_BYTES],
    )])
    .expect("the fixture catalog validates");
    let mut bytes = Vec::new();
    az_asset::write_asset_catalog(&catalog, &mut bytes).expect("encode the fixture catalog");
    std::fs::write(temp.path().join(az_asset::ASSET_CATALOG_FILE_NAME), bytes)
        .expect("write the fixture catalog");
    temp
}

/// The engine's own flags, as the merged command parses them.
fn flags(args: &[&str]) -> Launch {
    let composer = Composer::new(GemTargetRole::Server);
    parse("test", composer.registries(), args.iter().copied())
        .expect("the merged command parses")
        .require::<Launch>()
        .expect("the engine always parses its own flags")
        .clone()
}

/// What `parse` reports for a command line the engine's own flags reject.
fn rejected(args: &[&str]) -> clap::Error {
    let composer = Composer::new(GemTargetRole::Server);
    let Err(error) = parse("test", composer.registries(), args.iter().copied()) else {
        panic!("the merged command should have rejected {args:?}");
    };
    let RuntimeAppError::RuntimeLaunchArguments(error) = error else {
        panic!("expected an argument failure, got {error}");
    };
    error
}

#[test]
fn a_resolved_asset_root_is_mounted_as_its_catalog() {
    let temp = assets();
    let app = built(
        composed(),
        &[
            "test",
            "--test-value",
            "typed",
            "--assets",
            &temp.path().to_string_lossy(),
        ],
    );

    let catalog = app.world().resource::<AssetCatalog>();
    assert_eq!(catalog.asset_root(), temp.path());
    assert_eq!(catalog.len(), 1);
}

/// The ordering claim itself. Bevy's `init_default_source` keeps whichever
/// default source is already registered and only logs about a late one, so the
/// fact worth asserting is the reader path: the catalog decides what resolves.
/// A file on disk that no catalog entry names is `NotFound` here and would be
/// served if `AssetPlugin` had locked the table first.
#[test]
fn the_catalog_is_mounted_before_the_asset_plugin_locks_the_source_table() {
    let temp = assets();
    let app = built(
        composed(),
        &[
            "test",
            "--test-value",
            "typed",
            "--assets",
            &temp.path().to_string_lossy(),
        ],
    );

    let reader = app
        .world()
        .resource::<AssetServer>()
        .get_source(AssetSourceId::Default)
        .expect("the default asset source is registered")
        .reader();

    let mut bytes = Vec::new();
    let mut product = block_on(reader.read(Path::new(PRODUCT))).expect("catalogued product reads");
    block_on(Reader::read_to_end(&mut product, &mut bytes)).expect("catalogued product streams");
    assert_eq!(bytes, b"mounted");

    let stray = block_on(reader.read(Path::new(STRAY)));
    assert!(
        matches!(stray, Err(AssetReaderError::NotFound(_))),
        "an uncatalogued file on disk must not resolve"
    );
}

#[test]
fn a_target_with_no_asset_root_mounts_no_catalog() {
    let app = built(composed(), &["test", "--test-value", "typed"]);

    assert!(app.world().get_resource::<AssetCatalog>().is_none());
}

#[test]
fn a_reported_package_selects_its_own_payload_backend() {
    let pak = flags(&[
        "test",
        "--container",
        "pak",
        "--payload",
        "package/game.pak",
    ]);
    assert_eq!(pak.container, Some(PackagePayloadKind::Pak));
    assert_eq!(pak.payload.as_deref(), Some(Path::new("package/game.pak")));

    let plugin = pak.catalog("package");
    assert_eq!(plugin.asset_root, PathBuf::from("package"));
    assert_eq!(plugin.catalog_source, AssetCatalogSource::Native);
    assert_eq!(
        plugin.payload_backend,
        CatalogAssetPayloadBackend::Pak {
            payload_path: PathBuf::from("package/game.pak")
        }
    );

    let azpack = flags(&[
        "test",
        "--container",
        "azpack",
        "--payload",
        "package/package.azpack.index",
    ]);
    assert_eq!(
        azpack.catalog("package").payload_backend,
        CatalogAssetPayloadBackend::AzPack
    );
}

#[test]
fn an_unreported_package_reads_the_asset_root_as_the_engine_finds_it() {
    let auto = flags(&["test"]);
    assert_eq!(auto.container, None);
    assert_eq!(auto.payload, None);

    let plugin = auto.catalog("tmp/Cache/pc");
    assert_eq!(plugin.asset_root, PathBuf::from("tmp/Cache/pc"));
    assert_eq!(plugin.catalog_source, AssetCatalogSource::Native);
    assert_eq!(plugin.payload_backend, CatalogAssetPayloadBackend::Auto);
}

#[test]
fn an_unknown_container_names_the_containers_the_engine_reads() {
    let message = rejected(&[
        "test",
        "--container",
        "bundle",
        "--payload",
        "package/game.bundle",
    ])
    .to_string();

    assert!(message.contains("bundle"), "{message}");
    for container in [
        PackagePayloadKind::Loose,
        PackagePayloadKind::AzPack,
        PackagePayloadKind::Pak,
    ] {
        assert!(message.contains(container.container_name()), "{message}");
    }
}

/// The packaged form is a pair, exactly as
/// `AssetCatalogPlugin::native_package` takes it, so half of one never reaches
/// `Launch::catalog` to be silently read as "auto-detect".
#[test]
fn half_a_reported_package_is_an_argument_failure() {
    let container = rejected(&["test", "--container", "pak"]).to_string();
    assert!(container.contains("--payload"), "{container}");

    let payload = rejected(&["test", "--payload", "package/game.pak"]).to_string();
    assert!(payload.contains("--container"), "{payload}");
}

#[cfg(not(feature = "windowed"))]
#[test]
fn a_client_role_requires_the_explicit_windowed_feature() {
    let (mut app, composer) = pinned(Composer::new(GemTargetRole::Client));
    let error = build(
        RuntimeAppConfig::new("test", GemTargetRole::Client, SessionMode::Live),
        composer,
        &mut app,
        ["test"],
    )
    .expect_err("windowed composition should be rejected without its feature");

    assert!(
        matches!(error, RuntimeAppError::WindowedFeatureRequired),
        "{error}"
    );
}

#[test]
fn external_logging_keeps_headless_engine_plugins_without_log_plugin() {
    let (mut app, composer) = pinned(composed());
    build(
        config()
            .with_log(RuntimeLogConfig::default().with_ownership(RuntimeLogOwnership::External)),
        composer,
        &mut app,
        ["test", "--test-value", "typed"],
    )
    .unwrap();

    assert!(!app.is_plugin_added::<LogPlugin>());
    assert!(app.is_plugin_added::<ScheduleRunnerPlugin>());
}
