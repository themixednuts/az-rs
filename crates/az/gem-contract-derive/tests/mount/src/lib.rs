//! Fixture gem for the manifest forms of `#[contribution]`.
//!
//! `gem.toml` sits beside this crate's `Cargo.toml`, exactly as a real gem's
//! does. The whole hand-written registration surface below is a type and a
//! `register` body: no id, no const path, no roles, no caps, no entry-item
//! name. This package implements two of the gem's contributions, so each block
//! names which one it is with a bare token — including the fold from the
//! hyphenated `runtime-host`. `Keyed` is the runtime contribution spelled the
//! explicit way, so a test can hold the two descriptors up against each other.

use az_gem_contract::prelude::*;

/// Stands in for the gem's generated ids crate, which is what a *cross-gem*
/// reference consumes. The keyed twin below is the only thing in this crate
/// that needs it.
pub mod ids {
    use az_gem_contract::{ContributionId, GemId};

    pub const GEM: GemId = GemId::new("azoth.mount");
    pub const RUNTIME: ContributionId = ContributionId::new("runtime");
}

pub struct Component(pub &'static str);

impl RegistryEntry for Component {
    type Key = &'static str;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "component"
    }

    fn key(&self) -> &'static str {
        self.0
    }
}

pub struct Tick(pub &'static str);

impl RegistryEntry for Tick {
    type Key = &'static str;
    type Requires = HasAuthority;

    fn registry_name() -> &'static str {
        "authority-system"
    }

    fn key(&self) -> &'static str {
        self.0
    }
}

/// One registration body, so the two entry items differ only in how they came
/// by their identity.
fn mount<D: Provides<HasAuthority> + Provides<HostsWorld>>(ctx: &mut GemContext<'_, D>) {
    ctx.registrar::<Component>()
        .register(Component("MountComponent"));
    ctx.registrar::<Tick>()
        .register(Tick("mount_authority_tick"));
    ctx.registrar::<ClockDefinition>()
        .register(ClockDefinition {
            name: ClockName::new("simulation"),
            rate_hz: Some(60.0),
        });
    ctx.when::<HasClient>(|client| {
        client
            .registrar::<Component>()
            .register(Component("MountProjection"));
    });
}

/// Sealing is privacy: nothing outside this crate can name the type, and the
/// generated `runtime_contribution` is the only way to reach it.
struct Runtime;

#[contribution(runtime)]
impl Contribution for Runtime {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        mount(ctx);
    }
}

/// The gem's second contribution in the same crate, and the reason the blocks
/// name themselves: `runtime-host` folds to the token below and to the entry
/// item `runtime_host_contribution`, neither of which anyone writes twice.
struct Host;

#[contribution(runtime_host)]
impl Contribution for Host {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<Component>()
            .register(Component("MountHostPanel"));
    }
}

/// The same contribution, spelled the explicit way.
pub struct Keyed;

#[contribution(
    gem = ids::GEM,
    id = ids::RUNTIME,
    roles = [Server, Unified],
    caps = [HasAuthority, HostsWorld],
)]
impl Contribution for Keyed {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        mount(ctx);
    }
}
