//! A client-only registration inside a server-declared contribution must
//! fail at the gem author's own call site (E0277, ADR 0041).

use az_gem_contract::{GemContext, HasClient, RegistryEntry, declare_caps};

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

declare_caps!(ServerGemCaps: HasAuthority, HostsWorld);

fn build(ctx: &mut GemContext<'_, ServerGemCaps>) {
    ctx.registrar::<ClientSystemReg>()
        .register(ClientSystemReg("oops_client_projection"));
}

fn main() {
    let _ = build;
}
