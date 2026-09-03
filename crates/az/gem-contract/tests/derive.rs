//! A derive-declared contribution and a hand-written one are the same
//! composition: same descriptor, same capability floor, same compose report
//! in every role.

use az_gem_contract::prelude::*;
use az_gem_contract::{CapSet, CapsDecl, ComposeReport, Composer, ProductActivation};

/// Nothing selected: these contributions are the same either way, so the
/// composition passes the empty selection.
const NOTHING: ProductActivation = ProductActivation { enabled: &[] };

/// Stands in for the gem's generated consts-only ids crate: identity is
/// minted once and referenced, never re-spelled at a call site.
mod ids {
    use az_gem_contract::GemId;

    pub const GEM: GemId = GemId::new("azoth.mount");

    pub mod contrib {
        use az_gem_contract::ContributionId;

        pub const RUNTIME: ContributionId = ContributionId::new("runtime");
        pub const TOOLING: ContributionId = ContributionId::new("tooling");
    }
}

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

/// One registration body, so the two entry items differ only in how they
/// declare identity, roles, and the capability floor.
fn register_mount<D: Provides<HasAuthority> + Provides<HostsWorld>>(ctx: &mut GemContext<'_, D>) {
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

    // Declaring `HostsWorld` is feature-free; only cashing the declaration out
    // for a Bevy value belongs to the `app` feature.
    #[cfg(feature = "app")]
    let _app = ctx.app();

    ctx.when::<HasClient>(|client| {
        client
            .registrar::<ClientSystemReg>()
            .register(ClientSystemReg("mount_client_projection"));
    });
}

declare_caps!(HandWrittenCaps: HasAuthority, HostsWorld);

struct HandWritten;

impl Contribution for HandWritten {
    type Caps = HandWrittenCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: ids::GEM,
            contribution: ids::contrib::RUNTIME,
            roles: &[GemTargetRole::Server, GemTargetRole::Unified],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, HandWrittenCaps>) {
        register_mount(ctx);
    }
}

struct Derived;

#[contribution(
    gem = ids::GEM,
    id = ids::contrib::RUNTIME,
    roles = [Server, Unified],
    caps = [HasAuthority, HostsWorld],
)]
impl Contribution for Derived {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        register_mount(ctx);
    }
}

struct DerivedTooling;

#[contribution(
    gem = ids::GEM,
    id = ids::contrib::TOOLING,
    roles = [Tool],
    caps = [],
)]
impl Contribution for DerivedTooling {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<ComponentReg>()
            .register(ComponentReg("ToolComponent"));
    }

    fn ready(&self) -> bool {
        false
    }
}

const ROLES: [GemTargetRole; 5] = [
    GemTargetRole::Server,
    GemTargetRole::Client,
    GemTargetRole::Unified,
    GemTargetRole::HeadlessServer,
    GemTargetRole::AssetWorker,
];

fn compose<C: Contribution>(role: GemTargetRole, contribution: C) -> ComposeReport {
    let mut composer = Composer::new(role);
    let _ = composer.add(contribution, NOTHING);
    composer.finalize().expect("composition is valid")
}

#[test]
fn the_descriptor_is_the_same_pure_data() {
    assert_eq!(HandWritten.descriptor(), Derived.descriptor());
    assert_eq!(Derived.descriptor().gem, ids::GEM);
    assert!(Derived.descriptor().applies_to_role(GemTargetRole::Unified));
    assert!(!Derived.descriptor().applies_to_role(GemTargetRole::Client));
}

#[test]
fn the_generated_caps_declaration_carries_the_same_floor() {
    let floor = CapSet::EMPTY
        .with(HostCapability::HasAuthority)
        .with(HostCapability::HostsWorld);
    assert_eq!(DerivedCaps::REQUIRED, floor);
    assert_eq!(DerivedCaps::REQUIRED, HandWrittenCaps::REQUIRED);
}

#[test]
fn both_entry_items_compose_identically_in_every_role() {
    for role in ROLES {
        let hand = compose(role, HandWritten);
        let derived = compose(role, Derived);
        assert_eq!(hand, derived, "`{role}` composes the same either way");
        assert_eq!(hand.to_string(), derived.to_string());
    }
}

#[test]
fn an_empty_floor_composes_into_an_app_less_host_with_the_authors_lifecycle() {
    assert_eq!(DerivedToolingCaps::REQUIRED, CapSet::EMPTY);

    let mut composer = Composer::new(GemTargetRole::Tool);
    composer
        .add(DerivedTooling, NOTHING)
        .expect("an empty floor is contained by every host");
    assert!(!composer.all_ready(), "the author's `ready` still answers");

    let report = composer.finalize().expect("composition is valid");
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].key, "ToolComponent");
    assert_eq!(
        report.entries[0].instance.contribution,
        ids::contrib::TOOLING
    );
}

#[test]
fn the_derive_declared_floor_is_enforced_by_the_witness() {
    let server = compose(GemTargetRole::Server, Derived);
    assert!(server.refusals.is_empty());
    assert_eq!(server.entries.len(), 4);
    // The report's shape does not move with the feature: the section exists
    // either way, and without the hatch there is nothing to put in it.
    #[cfg(feature = "app")]
    assert_eq!(server.raw_app_access.len(), 1);
    #[cfg(not(feature = "app"))]
    assert!(server.raw_app_access.is_empty());
    assert_eq!(server.declined_narrowings.len(), 1);

    let client = compose(GemTargetRole::Client, Derived);
    assert_eq!(client.refusals.len(), 1);
    assert!(client.entries.is_empty());
    let refusal = client.refusals[0].to_string();
    assert!(refusal.contains("azoth.mount"));
    assert!(refusal.contains("HasAuthority"));
}
