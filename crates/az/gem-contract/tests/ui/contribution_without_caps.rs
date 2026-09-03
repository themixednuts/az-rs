//! The capability floor is declared, never inferred from absence: the
//! attribute requires `caps`, and a contribution with no floor writes
//! `caps = []`.

use az_gem_contract::prelude::*;

mod ids {
    use az_gem_contract::{ContributionId, GemId};

    pub const GEM: GemId = GemId::new("azoth.vegetation");
    pub const RUNTIME: ContributionId = ContributionId::new("runtime");
}

struct RuntimeServer;

#[contribution(
    gem = ids::GEM,
    id = ids::RUNTIME,
    roles = [Server, Unified],
)]
impl Contribution for RuntimeServer {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        let _ = ctx.role();
    }
}

fn main() {
    let _ = RuntimeServer;
}
