//! The engine's `types` contribution: AZ type registrations and component
//! lowerings, composed by every host role.
//!
//! A `register(ctx)` surface is contribution *content*, not a contribution —
//! identity attaches here, at the manifested bundle that calls it
//! (asset-contract ticket 014, D5). The member crates below keep their
//! `register` functions exactly as they were; what this crate adds is the one
//! thing they never had, which is a caller with a name.
//!
//! Membership is decided by linkage, not by taste: this bundle is composed by
//! all twelve target roles, so every crate it names must be one a `tool` or a
//! `named-service` target can link. That is why the Bevy-world types — the
//! ones behind `engine_lowerings()` and `engine_prefab_type_registry()` — are
//! not here: their role-coherent home is a narrower bundle, and deciding it is
//! the gem sweep's prefab-type pass (ticket 014, F5).

use az_gem_contract::prelude::*;

/// Sealing is privacy: the generated `types_contribution` is the only way in.
struct Types;

#[contribution]
impl Contribution for Types {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        az_core::rtti::register(ctx);
        az_transform::register(ctx);
    }
}
