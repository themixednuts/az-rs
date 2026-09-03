//! The engine's `assets` contribution: the asset types the engine owns.
//!
//! Composed by the runtime roles and by the three pipeline roles, because an
//! asset type is one fact both halves need — the pipeline to classify a
//! source into it, the runtime to resolve a product out of it. The two `tool`
//! and `named-service` roles name no assets at all, which is the whole reason
//! this is not folded into `types`.
//!
//! `az_core::asset::register` and `az_core::rtti::register` are two surfaces on
//! one crate and they land in two bundles; that is the granularity the ruling
//! asks for. Identity attaches to the bundle, not to the crate.
//!
//! `az_animation::assets` is the same shape seen from the other side. Most
//! engine crates already split the runtime-safe declaration from the builder
//! that fills it — `az-mesh` beside `az-mesh-builder`, `az-scene` beside
//! `az-prefab-builder`, `az-terrain-runtime` beside `az-terrain-builder` — but
//! az-animation declares both in one crate, so the split is by surface rather
//! than by crate. It has to be one or the other: a runtime that cannot name the
//! asset type it is loading has the coverage hole the deleted link-time anchor
//! used to hide by force-linking the whole crate into every binary.

use az_gem_contract::prelude::*;

/// Sealing is privacy: the generated `assets_contribution` is the only way in.
struct Assets;

#[contribution]
impl Contribution for Assets {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        az_animation::assets(ctx);
        az_core::asset::register(ctx);
        az_mesh::register(ctx);
        az_scene::register(ctx);
        az_terrain_runtime::register(ctx);
    }
}
