use bevy::prelude::*;

use super::TerrainChunk;

/// Builder for [`TerrainChunk`].
pub struct TerrainChunkBuilder {
    position: Vec3,
    size: f32,
    resolution: usize,
    max_lod_levels: u8,
    lod_distances: Vec<f32>,
}

impl TerrainChunkBuilder {
    pub(super) fn new(position: Vec3, size: f32, resolution: usize) -> Self {
        Self {
            position,
            size,
            resolution,
            max_lod_levels: 4,
            lod_distances: vec![128.0, 256.0, 512.0, 1024.0],
        }
    }

    /// Set the maximum LOD levels.
    #[must_use]
    pub const fn with_lod_levels(mut self, levels: u8) -> Self {
        self.max_lod_levels = levels;
        self
    }

    /// Set custom LOD distance thresholds.
    #[must_use]
    pub fn with_lod_distances(mut self, distances: Vec<f32>) -> Self {
        self.lod_distances = distances;
        self
    }

    /// Build the terrain chunk.
    #[must_use]
    pub fn build(self) -> TerrainChunk {
        TerrainChunk {
            position: self.position,
            size: self.size,
            resolution: self.resolution,
            current_lod: 0,
            max_lod_levels: self.max_lod_levels,
            lod_distances: self.lod_distances,
        }
    }
}
