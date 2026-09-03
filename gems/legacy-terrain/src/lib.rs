//! `LegacyTerrain` Gem component data and mesh generation.
//!
//! Lumberyard reference: `dev/Gems/LegacyTerrain/Code/Source/LegacyTerrainLevelComponent.cpp`
//! and `dev/Gems/LegacyTerrain/Code/Source/terrain.h`.

mod chunk;
pub mod engine_asset;
pub mod heightmap;
pub mod lod;
pub mod materials;
mod region;
mod render;
pub mod water;
mod world;

use az_gem_contract::{Contribution, GemContext, contribution};
use bevy::prelude::*;

// The `register_*` helpers arrive through the public glob re-exports below;
// importing them privately as well would shadow those and make the public
// names unreachable.

pub use chunk::*;
pub use heightmap::{Heightmap, TerrainHeight};
pub use lod::{TerrainLod, TerrainLodConfig};
pub use materials::{TerrainMaterial, TerrainMaterialLayer};
pub use region::*;
pub use render::*;
pub use water::{
    SerializableWaterQuadtree, WATER_NODE_EMPTY_HEIGHT, WaterNodeAddress, WaterNodeData,
    WaterNodeFlags, WaterSurfaceTile,
};
pub use world::*;

/// Register `LegacyTerrain` component types and systems.
pub struct LegacyTerrainPlugin;

impl Plugin for LegacyTerrainPlugin {
    fn build(&self, app: &mut App) {
        register_world_components(app);
        register_region_components(app);
        register_chunk_components(app);
        register_render_components(app);

        info!("LegacyTerrainPlugin initialized");
    }
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// `LegacyTerrain` reads its heightmaps, materials, and water quadtrees straight
/// out of a level's terrain region files; nothing here is an engine asset type,
/// a build rule, or a prefab type, so this gem owns no registry entry. An empty
/// `register` is the honest shape of that, and the compose-seam test holds it
/// to empty so a registration added later has to be declared here.
struct Package;

#[contribution]
impl Contribution for Package {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

#[cfg(test)]
mod tests;
