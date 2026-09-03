//! Engine animation gem.
//!
//! Generic animation source schemas, build rules, and product formats live in
//! `az-animation`, and the engine's `builders` bundle composes that crate's
//! `register` for the three pipeline roles. What this gem adds is the Bevy
//! runtime that plays the results.

mod bevy_runtime;
#[path = "blend_space_asset.rs"]
mod blend_space_runtime;
mod character_definition;
mod character_runtime;
#[path = "events.rs"]
mod event_runtime;
mod motion_runtime;

use az_gem_contract::{Contribution, GemContext, contribution};

pub use az_animation::*;
pub use bevy_runtime::*;
pub use blend_space_runtime::*;
pub use character_definition::*;
pub use character_runtime::*;
pub use event_runtime::*;
pub use motion_runtime::*;

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// `az_animation::register` is not called here: the engine's `builders` bundle
/// claims it, and this gem's roles overlap that bundle at `project-host`, so a
/// second caller would be a duplicate key rather than a second registration.
/// What is left is the Bevy character, motion, blend-space, and event runtime,
/// none of which is a registry entry — an empty `register` is the honest shape
/// of that, and the compose-seam test holds it to empty so a registration added
/// later has to be declared here.
struct Package;

#[contribution]
impl Contribution for Package {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}
