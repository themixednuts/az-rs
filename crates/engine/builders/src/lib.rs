//! The engine's `builders` contribution: build rules, source schemas, product
//! formats, templates and edit codecs.
//!
//! This is the bundle that closes the builder floor. The asset-processor host
//! links no *project or gem* code — that is still the point of the worker
//! split — but engine built-ins are neither, and until this existed the host
//! started with an empty classifier set and failed source-root registration
//! with `BuilderCatalogUnavailable` (asset-contract ticket 014, D7).
//!
//! Composed only by the three pipeline roles, so a `client` target links none
//! of it. That is the linkage duality ADR 0036 asks for, and it is the reason
//! this is a bundle rather than one contribution per crate: the dependency set
//! is the contribution.
//!
//! Three engine-tree format crates are missing from the list on purpose:
//! `lmbr-central-assets`, `lyshine-canvas` and `texture-atlas-assets` each
//! depend on a *gem* (`az-gem-lmbr-central`, `az-gem-lyshine`,
//! `az-gem-texture-atlas`). Naming them here would make the asset-processor
//! host link gem code, which D7 says stays untrue. Their source families join
//! this bundle when the sweep inverts that dependency — the same
//! engine-id-squatting knot ticket 011 §2 already owns.

use az_gem_contract::prelude::*;

/// Sealing is privacy: the generated `builders_contribution` is the only way
/// in.
struct Builders;

#[contribution]
impl Contribution for Builders {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        // Only the pipeline half: az-animation's asset types and product
        // formats are what a runtime resolves, so they are registered by the
        // `assets` bundle at ten roles instead of these three.
        az_animation::builders(ctx);
        az_graph_builder::register(ctx);
        az_material_builder::register(ctx);
        az_mesh_builder::register(ctx);
        az_nv_cloth::register(ctx);
        az_prefab_builder::register(ctx);
        az_terrain_builder::register(ctx);
        az_texture_builder::register(ctx);

        bink::register(ctx);
        az_lua_bytecode::register(ctx);
        cry_character_assets::register(ctx);
        cry_localization::register(ctx);
        cry_system_assets::register(ctx);
        cry_text::register(ctx);
        cry_xml_assets::register(ctx);
    }
}
