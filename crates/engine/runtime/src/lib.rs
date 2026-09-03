//! The engine's `runtime` contribution: the Bevy-world component types, the
//! component lowerings that turn authored data into them, the middleware's
//! reflected types, and the replication wire it owns.
//!
//! This is the bundle that closes the *other* unattributed floor. Until it
//! existed, `az_prefab_builder::engine_lowerings()` handed every caller the
//! engine's adapters outside composition — self-described as standing in for
//! "what the linked inventory used to supply for free" — so the one registry
//! whose whole purpose is to make two adapters for one component a compose
//! error could not see half its own entries (asset-contract ticket 014, D5 and
//! F5).
//!
//! Composed by nine of the twelve roles: the six that own a world, the two that
//! build one, and `runtime-host`, which is what the editor's viewport preview
//! is. A lowering is read where an authored component becomes a native one —
//! AZSCENE processing and that preview — while the AZ types and the wire entries
//! are read wherever a host resolves an identity off the wire or out of a
//! product. The middleware's seven reflected types are read alongside the
//! lowerings, by the same AZSCENE analysis that resolves a `FacetedComponent`
//! out of a prefab document.
//!
//! Three roles are left out and each for its own reason. `tool` and
//! `named-service` name no world and no assets, and leaving them out is what
//! keeps them free of Bevy. `project-host` is the interesting one: it is
//! guarded as a generic project service adapter that may not depend on Bevy or
//! gridmate at all, it applies az-transform, az-render and az-framework straight
//! to Bevy's `AppTypeRegistry` under D7's carve-out, and it lowers nothing. It
//! does read a composed `Registry<PrefabType>` — but it cannot name az-facet to
//! link it, so the middleware's types reach its registry the way every reflected
//! dependency does: `PrefabType::of` applies the type it names *and the types
//! that type reflects through*, so a composed component carrying a
//! `FacetedComponent` field brings the base with it. Adding this bundle to that
//! role would put gridmate in a service adapter's link set to say directly what
//! composition already says.
//!
//! The capability floor is empty on purpose. Every registry this bundle writes
//! to is `Unconditional`, and nothing here asks for the `App`; declaring
//! `HostsWorld` would be a floor no entry needs whose only effect would be to
//! exclude the four App-less roles that consume this bundle's content.

use az_gem_contract::prelude::*;
use az_prefab::PrefabType;

/// Sealing is privacy: the generated `runtime_contribution` is the only way in.
struct Runtime;

#[contribution]
impl Contribution for Runtime {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        az_facet::register(ctx);
        az_framework::register(ctx);
        gridmate::registration::register(ctx);
        // az-facet's seven reflected types are spelled here rather than folded
        // into `az_facet::register` so the asymmetry with this bundle's other
        // members is stated where it is decided: az-framework's reflected type
        // is applied directly by `engine_prefab_type_registry`, under D7's
        // carve-out for the crates every prefab host links unconditionally, and
        // az-facet is not one of those.
        ctx.registrar::<PrefabType>()
            .register_many(az_facet::prefab_types());
    }
}
