//! The attribute-declared floor is the same fact `declare_caps!` writes: a
//! client registrar call inside a contribution that declared only
//! `HostsWorld` fails at the gem author's own call site (E0277, ADR 0041).

use az_gem_contract::prelude::*;

mod ids {
    use az_gem_contract::{ContributionId, GemId};

    pub const GEM: GemId = GemId::new("azoth.vegetation");
    pub const RUNTIME: ContributionId = ContributionId::new("runtime");
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

struct RuntimeServer;

#[contribution(
    gem = ids::GEM,
    id = ids::RUNTIME,
    roles = [Server, Unified],
    caps = [HostsWorld],
)]
impl Contribution for RuntimeServer {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<ClientSystemReg>()
            .register(ClientSystemReg("oops_client_projection"));
    }
}

fn main() {
    let _ = RuntimeServer;
}
