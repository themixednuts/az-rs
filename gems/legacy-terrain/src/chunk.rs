//! Terrain chunk components and mesh generation systems.

mod component;
mod set;
mod systems;

use bevy::prelude::*;

use crate::heightmap::Heightmap;
use crate::lod::TerrainLod;

pub use component::{TerrainChunk, TerrainChunkBuilder, TerrainChunkBundle};
pub use set::TerrainSet;
pub use systems::{generate_terrain_meshes, update_terrain_lod};

pub fn register_chunk_components(app: &mut App) {
    app.register_type::<TerrainChunk>()
        .register_type::<Heightmap>()
        .register_type::<TerrainLod>()
        .configure_sets(
            Update,
            (
                TerrainSet::Lod,
                TerrainSet::MeshGeneration.after(TerrainSet::Lod),
            ),
        )
        .add_systems(
            Update,
            (
                update_terrain_lod.in_set(TerrainSet::Lod),
                generate_terrain_meshes.in_set(TerrainSet::MeshGeneration),
            ),
        );
}
