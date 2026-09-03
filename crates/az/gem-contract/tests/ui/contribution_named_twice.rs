//! The bare token names *this* block's contribution, and a block is one
//! contribution: two tokens would be two identities sharing one type, one
//! capability floor, and one `register` body (ADR 0032).

use az_gem_contract::prelude::*;

struct RuntimeServer;

#[contribution(runtime, tooling)]
impl Contribution for RuntimeServer {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        let _ = ctx.role();
    }
}

fn main() {
    let _ = RuntimeServer;
}
