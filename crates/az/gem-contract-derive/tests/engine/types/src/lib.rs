//! Second contribution crate of the fixture engine, nested one directory
//! below the `engine.toml` that declares it.
//!
//! Two things fall out of the manifest that the author never writes, and they
//! are the same two the nested *gem* crate gets: the empty capability floor of
//! a `caps`-less stanza, and the fold from the hyphenated id `prefab-types` to
//! the entry item `prefab_types_contribution`.

use az_gem_contract::prelude::*;

pub struct Kind(pub &'static str);

impl RegistryEntry for Kind {
    type Key = &'static str;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "prefab-type"
    }

    fn key(&self) -> &'static str {
        self.0
    }
}

struct Types;

#[contribution]
impl Contribution for Types {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<Kind>().register(Kind("MountTransform"));
    }
}
