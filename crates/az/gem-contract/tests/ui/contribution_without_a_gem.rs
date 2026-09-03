//! Identity has one source. A crate that is not inside a gem resolves against
//! the engine that hosts it — every crate in this tree is below the engine's
//! `engine.toml` — and the engine declares no contribution for it, so the
//! zero-argument attribute names the stanza that would, instead of inventing
//! an id from the crate name (ADR 0032, asset-contract 014 D2).
//!
//! The `../../..` in the expected message is trybuild's own scratch crate
//! counting its way back up to the repository root, which is where the engine
//! manifest sits.

use az_gem_contract::prelude::*;

struct RuntimeServer;

#[contribution]
impl Contribution for RuntimeServer {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        let _ = ctx.role();
    }
}

fn main() {
    let _ = RuntimeServer;
}
