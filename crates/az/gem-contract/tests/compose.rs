//! Static-seam composition: the spike-010 host matrix ported onto the
//! production contract crate.

use az_gem_contract::{
    CapabilityId, ClockDefinition, ClockName, ClockUse, ComposeError, Composer, Contribution,
    ContributionDescriptor, ContributionId, GemContext, GemId, GemTargetRole, HasAuthority,
    HasClient, HostCapability, ProductActivation, RegistryEntry, Unconditional, declare_caps,
};

/// Nothing selected: what a host composes with when the project raised no
/// product capability for the instance.
const NOTHING: ProductActivation = ProductActivation { enabled: &[] };

struct ComponentReg(&'static str);

impl RegistryEntry for ComponentReg {
    type Key = &'static str;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "component"
    }

    fn key(&self) -> &'static str {
        self.0
    }
}

struct ClientSystemReg(&'static str);

impl RegistryEntry for ClientSystemReg {
    type Key = &'static str;
    type Requires = HasClient;

    fn registry_name() -> &'static str {
        "client-system"
    }

    fn key(&self) -> &'static str {
        self.0
    }
}

struct AuthoritySystemReg(&'static str);

impl RegistryEntry for AuthoritySystemReg {
    type Key = &'static str;
    type Requires = HasAuthority;

    fn registry_name() -> &'static str {
        "authority-system"
    }

    fn key(&self) -> &'static str {
        self.0
    }
}

declare_caps!(AlphaCaps:);

/// Engine-flavoured contribution: shared registration only, composes into
/// every role including non-App hosts.
struct AlphaContribution;

impl Contribution for AlphaContribution {
    type Caps = AlphaCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("azoth.alpha"),
            contribution: ContributionId::new("runtime"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, AlphaCaps>) {
        let mut components = ctx.registrar::<ComponentReg>();
        components.register(ComponentReg("TransformComponent"));
        components.register(ComponentReg("NameComponent"));
    }
}

declare_caps!(BetaCaps: HasAuthority, HostsWorld);

/// Project-flavoured mixed contribution: authoritative floor, client behavior
/// through optional narrowing, one sanctioned raw-App touch.
struct BetaContribution;

impl Contribution for BetaContribution {
    type Caps = BetaCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("sample.beta"),
            contribution: ContributionId::new("runtime"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, BetaCaps>) {
        ctx.registrar::<ComponentReg>()
            .register(ComponentReg("MountComponent"));
        ctx.registrar::<AuthoritySystemReg>()
            .register(AuthoritySystemReg("mount_authority_tick"));
        ctx.registrar::<ClockDefinition>()
            .register(ClockDefinition {
                name: ClockName::new("simulation"),
                rate_hz: Some(60.0),
            });
        ctx.registrar::<ClockUse>().register(ClockUse {
            clock: ClockName::new("simulation"),
            system: "mount_authority_tick",
        });

        // The sanctioned hatch: compiles because BetaCaps declares
        // HostsWorld; lands in the compose report's raw-access section.
        // Declaring HostsWorld is feature-free; only cashing it out for a
        // Bevy value belongs to the `app` feature.
        #[cfg(feature = "app")]
        let _app = ctx.app();

        // No plugin split: client behavior stays in the same contribution
        // behind optional narrowing; declined visibly on authority-only hosts.
        ctx.when::<HasClient>(|client| {
            client
                .registrar::<ClientSystemReg>()
                .register(ClientSystemReg("mount_client_projection"));
        });
    }
}

fn compose(role: GemTargetRole) -> Composer {
    let mut composer = Composer::new(role);
    composer
        .add(AlphaContribution, NOTHING)
        .expect("alpha requires nothing");
    let _ = composer.add(BetaContribution, NOTHING);
    composer
}

#[test]
fn alpha_composes_into_every_role_including_non_app_hosts() {
    for role in [
        GemTargetRole::Server,
        GemTargetRole::Client,
        GemTargetRole::Unified,
        GemTargetRole::AssetWorker,
    ] {
        let composer = compose(role);
        let report = composer.finalize().expect("composition is valid");
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.instance.gem.as_str() == "azoth.alpha"),
            "alpha must compose into `{role}`"
        );
    }
}

#[test]
fn beta_is_refused_by_containment_on_client_and_worker() {
    let server = compose(GemTargetRole::Server).finalize().unwrap();
    let unified = compose(GemTargetRole::Unified).finalize().unwrap();
    let client = compose(GemTargetRole::Client).finalize().unwrap();
    let worker = compose(GemTargetRole::AssetWorker).finalize().unwrap();

    assert!(server.refusals.is_empty());
    assert!(unified.refusals.is_empty());

    assert_eq!(client.refusals.len(), 1);
    let refusal = client.refusals[0].to_string();
    assert!(refusal.contains("sample.beta"));
    assert!(refusal.contains("HasAuthority"));

    assert_eq!(worker.refusals.len(), 1);
}

/// The report's shape is feature-independent: the raw-access section exists
/// either way, and without the `app` feature there is no hatch to fill it.
#[test]
fn raw_app_access_is_reported_wherever_beta_composed() {
    let server = compose(GemTargetRole::Server).finalize().unwrap();
    let unified = compose(GemTargetRole::Unified).finalize().unwrap();

    for report in [&server, &unified] {
        #[cfg(feature = "app")]
        {
            assert_eq!(report.raw_app_access.len(), 1);
            assert_eq!(report.raw_app_access[0].gem.as_str(), "sample.beta");
        }
        #[cfg(not(feature = "app"))]
        assert!(report.raw_app_access.is_empty());
    }
}

#[test]
fn narrowing_runs_on_unified_and_is_declined_visibly_on_server() {
    let unified = compose(GemTargetRole::Unified).finalize().unwrap();
    assert!(
        unified
            .entries
            .iter()
            .any(|entry| entry.key == "mount_client_projection")
    );
    assert!(unified.declined_narrowings.is_empty());

    let server = compose(GemTargetRole::Server).finalize().unwrap();
    assert!(
        !server
            .entries
            .iter()
            .any(|entry| entry.key == "mount_client_projection")
    );
    assert_eq!(server.declined_narrowings.len(), 1);
    assert_eq!(
        server.declined_narrowings[0].capability,
        HostCapability::HasClient
    );
}

#[test]
fn duplicate_keys_fail_composition_naming_both_culprits() {
    declare_caps!(DupCaps:);

    struct DuplicateContribution;

    impl Contribution for DuplicateContribution {
        type Caps = DupCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("sample.dup-demo"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, DupCaps>) {
            ctx.registrar::<ComponentReg>()
                .register(ComponentReg("TransformComponent"));
        }
    }

    let mut composer = Composer::new(GemTargetRole::Server);
    composer.add(AlphaContribution, NOTHING).unwrap();
    composer.add(DuplicateContribution, NOTHING).unwrap();

    let error = composer.finalize().unwrap_err();
    let ComposeError::Duplicate {
        registry,
        key,
        first,
        second,
    } = error
    else {
        panic!("expected duplicate error, got {error}");
    };
    assert_eq!(registry, "component");
    assert_eq!(key, "TransformComponent");
    assert_eq!(first.gem.as_str(), "azoth.alpha");
    assert_eq!(second.gem.as_str(), "sample.dup-demo");
}

#[test]
fn removal_is_bulk_and_re_add_bumps_the_generation() {
    let mut composer = Composer::new(GemTargetRole::Server);
    composer.add(AlphaContribution, NOTHING).unwrap();
    composer.add(BetaContribution, NOTHING).unwrap();

    let removed = composer.remove(GemId::new("sample.beta"), ContributionId::new("runtime"));
    assert_eq!(removed, 4, "beta owns exactly four registry entries");

    let report = composer.finalize().unwrap();
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.instance.gem.as_str() != "sample.beta")
    );
    assert!(report.raw_app_access.is_empty());

    let instance = composer.add(BetaContribution, NOTHING).unwrap();
    assert_eq!(instance.generation, 1, "re-added instance is generation 1");
}

/// Activation is composition data, not identity: the same contribution
/// registers differently under different selections, and the selection reaches
/// it through the context rather than through the entry item, exactly as its
/// attribution does.
#[test]
fn the_composition_supplies_the_activation_the_contribution_reads() {
    declare_caps!(OverlayCaps:);

    struct OverlayContribution;

    impl Contribution for OverlayContribution {
        type Caps = OverlayCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("acme.overlay"),
                contribution: ContributionId::new("overlay"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, OverlayCaps>) {
            if ctx.activation().contains(CapabilityId::new("anti-cheat")) {
                ctx.registrar::<ComponentReg>()
                    .register(ComponentReg("AntiCheatComponent"));
            }
            ctx.registrar::<ComponentReg>()
                .register(ComponentReg("OverlayComponent"));
        }
    }

    const ANTI_CHEAT: ProductActivation = ProductActivation {
        enabled: &[CapabilityId::new("anti-cheat")],
    };

    let keys = |activation| {
        let mut composer = Composer::new(GemTargetRole::Client);
        composer
            .add(OverlayContribution, activation)
            .expect("an empty floor composes");
        composer
            .finalize()
            .expect("composition is valid")
            .entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect::<Vec<_>>()
    };

    assert_eq!(keys(ANTI_CHEAT), ["AntiCheatComponent", "OverlayComponent"]);
    assert_eq!(keys(NOTHING), ["OverlayComponent"]);
}

#[test]
fn an_unresolvable_clock_is_a_composition_error() {
    declare_caps!(ClockCaps:);

    struct UndefinedClockContribution;

    impl Contribution for UndefinedClockContribution {
        type Caps = ClockCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("sample.clockless"),
                contribution: ContributionId::new("runtime"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, ClockCaps>) {
            ctx.registrar::<ClockUse>().register(ClockUse {
                clock: ClockName::new("input-framing"),
                system: "sample_input",
            });
        }
    }

    let mut composer = Composer::new(GemTargetRole::Server);
    composer.add(UndefinedClockContribution, NOTHING).unwrap();

    let error = composer.finalize().unwrap_err();
    let ComposeError::UnresolvableClock { clock, .. } = error else {
        panic!("expected unresolvable clock, got {error}");
    };
    assert_eq!(clock.as_str(), "input-framing");
}
