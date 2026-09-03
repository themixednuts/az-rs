//! Identity is minted once in the gem manifest and consumed as a const from
//! the generated ids crate: the attribute refuses to re-mint an id literal
//! at the contribution's own call site (ADR 0032).

use az_gem_contract::prelude::*;

struct RuntimeServer;

#[contribution(
    gem = "azoth.vegetation",
    id = "runtime",
    roles = [Server],
    caps = [HostsWorld],
)]
impl Contribution for RuntimeServer {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        let _ = ctx.app();
    }
}

fn main() {
    let _ = RuntimeServer;
}
