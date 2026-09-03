//! Roads and Rivers Gem component data and Bevy integration.
//!
//! Lumberyard's `RoadComponent` and `RiverComponent` build Cry render
//! nodes from a shared `SplineGeometry` strip. In Rust, we keep the
//! reflected component names and field intent, but use Bevy primitives
//! and meshes for the rendered representation.

mod components;
mod geometry;
mod render;

use az_gem_contract::{Contribution, GemContext, contribution};
use az_prefab::PrefabType;
use bevy::prelude::*;

// The `register_*` helpers arrive through the public glob re-exports below;
// importing them privately as well would shadow those and make the public
// names unreachable.

pub use components::*;
pub use geometry::*;
pub use render::*;

/// Register roads/rivers component data and systems.
pub struct RoadsAndRiversPlugin;

impl Plugin for RoadsAndRiversPlugin {
    fn build(&self, app: &mut App) {
        register_geometry_components(app);
        register_road_river_components(app);
        register_render_components(app);
    }
}

/// The Bevy-native Prefab component types this crate owns.
#[must_use]
pub fn prefab_types() -> [PrefabType; 1] {
    [PrefabType::of::<RoadComponent>()]
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// Roads and rivers are spline geometry turned into Bevy meshes at runtime; the
/// reflected half is the pipeline-only bundle below, and nothing else here is a
/// registry entry. An empty `register` is the honest shape of that, and the
/// compose-seam test holds it to empty so a registration added later has to be
/// declared here.
struct Package;

#[contribution(package)]
impl Contribution for Package {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

/// Sealing is privacy: the generated `prefab_types_contribution` is the only
/// way in.
///
/// Reflected prefab types are read by the two hosts that resolve prefab
/// documents without booting the runtime plugin graph — `project-host` through
/// its type registry, `asset-worker` through AZSCENE analysis.
struct Prefabs;

#[contribution(prefab_types)]
impl Contribution for Prefabs {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<PrefabType>().register_many(prefab_types());
    }
}
