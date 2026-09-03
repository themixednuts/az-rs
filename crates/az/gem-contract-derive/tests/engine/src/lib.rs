//! Fixture engine for the manifest forms of `#[contribution]`.
//!
//! `engine.toml` sits beside this crate's `Cargo.toml`, exactly as the real
//! engine's does at the repository root. The hand-written surface is a type
//! and a `register` body — no id, no const path, no roles, no caps, no
//! entry-item name — and that is the point: an engine author's authoring
//! experience is a gem author's, because it is the same reader answering.

use az_gem_contract::prelude::*;

pub struct Rule(pub &'static str);

impl RegistryEntry for Rule {
    type Key = &'static str;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "build-rule"
    }

    fn key(&self) -> &'static str {
        self.0
    }
}

/// Sealing is privacy here too: the generated `builders_contribution` is the
/// only way to reach it.
struct Builders;

#[contribution]
impl Contribution for Builders {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<Rule>().register(Rule("mount-source"));
    }
}
