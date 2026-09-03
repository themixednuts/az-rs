//! `LmbrCentral`'s `builders` contribution: the gem's source families, product
//! formats, asset types and build rules.
//!
//! A `register(ctx)` surface is contribution *content*, not a contribution —
//! identity attaches at the manifested bundle that calls it (asset-contract
//! ticket 014, D5). `lmbr-central-assets` keeps its `register` exactly as it
//! was; what this crate adds is a caller with a name.
//!
//! It is a crate of its own rather than a second block inside
//! `az-gem-lmbr-central` because `lmbr-central-assets` depends on that gem: a
//! block there could not name it without a dependency cycle. It is not part of
//! `az-engine-builders` for the mirror-image reason — naming it there would
//! make the asset-processor host link gem code, which ticket 014's D7 says
//! stays untrue.

use az_gem_contract::prelude::*;

/// Sealing is privacy: the generated `builders_contribution` is the only way
/// in.
struct Builders;

#[contribution]
impl Contribution for Builders {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        lmbr_central_assets::register(ctx);
    }
}
