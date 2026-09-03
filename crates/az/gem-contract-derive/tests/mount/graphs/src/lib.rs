//! Third contribution crate of the fixture gem: the one that compiles a
//! generated product in.
//!
//! The whole hand-written surface is still a type and a `register` body, and
//! the body says nothing about graphs. `gem.toml` declares
//! `products = ["graphs"]`, the build script writes the projection into
//! `OUT_DIR`, and `#[contribution]` writes the call that registers it — first,
//! ahead of the body below. Deleting the declaration would not break this
//! file; it would stop the graph composing, which is why the declaration and
//! the build script check each other from both sides.

use az_gem_contract::prelude::*;

pub struct Marker(pub &'static str);

impl RegistryEntry for Marker {
    type Key = &'static str;
    type Requires = Unconditional;

    fn registry_name() -> &'static str {
        "marker"
    }

    fn key(&self) -> &'static str {
        self.0
    }
}

struct Graphs;

#[contribution]
impl Contribution for Graphs {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<Marker>().register(Marker("MountGraphsRan"));
    }
}
