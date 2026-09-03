use bevy::prelude::*;

use crate::heightmap::Heightmap;
use crate::lod::TerrainLod;

use super::TerrainChunk;

/// Bundle for spawning a terrain chunk.
#[derive(Bundle, Default)]
pub struct TerrainChunkBundle {
    pub chunk: TerrainChunk,
    pub heightmap: Heightmap,
    pub lod: TerrainLod,
    pub transform: Transform,
}
