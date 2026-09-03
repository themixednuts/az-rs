//! A bare token and the four keys are two answers to the same question: the
//! token sends the attribute to `gem.toml` for roles and caps, the keys spell
//! them at the call site. Taking both would let a call site disagree with the
//! manifest, so the attribute takes one or the other (ADR 0032).

use az_gem_contract::prelude::*;

mod ids {
    use az_gem_contract::{ContributionId, GemId};

    pub const GEM: GemId = GemId::new("azoth.vegetation");
    pub const RUNTIME: ContributionId = ContributionId::new("runtime");
}

struct RuntimeServer;

#[contribution(
    runtime,
    gem = ids::GEM,
    id = ids::RUNTIME,
    roles = [Server],
    caps = [],
)]
impl Contribution for RuntimeServer {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        let _ = ctx.role();
    }
}

fn main() {
    let _ = RuntimeServer;
}
