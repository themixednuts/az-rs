//! Second contribution crate of the fixture gem, nested one directory below
//! the `gem.toml` that declares it.
//!
//! Two things fall out of the manifest here that the author never writes: the
//! empty capability floor of a `caps`-less stanza, and the fold from the
//! hyphenated id `prefab-types` to the entry item `prefab_types_contribution`.

use az_gem_contract::prelude::*;

pub struct Prefab(pub &'static str);

impl RegistryEntry for Prefab {
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
        ctx.registrar::<Prefab>().register(Prefab("MountComponent"));
    }
}
