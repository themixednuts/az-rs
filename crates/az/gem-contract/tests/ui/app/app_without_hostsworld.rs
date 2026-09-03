//! The unsanctioned bypass: a contribution that did not declare `HostsWorld`
//! cannot reach `&mut App` at all.

use az_gem_contract::{GemContext, declare_caps};

declare_caps!(WorkerGemCaps:);

fn build(ctx: &mut GemContext<'_, WorkerGemCaps>) {
    let _ = ctx.app();
}

fn main() {
    let _ = build;
}
