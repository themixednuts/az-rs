use bevy::prelude::*;

use crate::heightmap::math::{ExactF32, floor_index};

mod builder;
mod bundle;

pub use builder::TerrainChunkBuilder;
pub use bundle::TerrainChunkBundle;

/// Terrain chunk component.
///
/// Represents a chunk of terrain with heightmap data and LOD levels.
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
pub struct TerrainChunk {
    /// Chunk position in world space.
    pub position: Vec3,

    /// Chunk size in world units.
    pub size: f32,

    /// Resolution in vertices per side.
    pub resolution: usize,

    /// Current LOD level.
    pub current_lod: u8,

    /// Maximum LOD levels.
    pub max_lod_levels: u8,

    /// Distance thresholds for LOD transitions.
    pub lod_distances: Vec<f32>,
}

impl Default for TerrainChunk {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            size: 1024.0,
            resolution: 129,
            current_lod: 0,
            max_lod_levels: 4,
            lod_distances: vec![256.0, 512.0, 1024.0, 2048.0],
        }
    }
}

impl TerrainChunk {
    /// Create a new terrain chunk.
    #[must_use]
    pub const fn new(position: Vec3, size: f32, resolution: usize) -> Self {
        Self {
            position,
            size,
            resolution,
            current_lod: 0,
            max_lod_levels: 4,
            lod_distances: Vec::new(),
        }
    }

    /// Create a terrain chunk builder.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use az_gem_legacy_terrain::TerrainChunk;
    /// use bevy::math::Vec3;
    ///
    /// let chunk = TerrainChunk::builder(Vec3::ZERO, 1024.0, 129)
    ///     .with_lod_levels(6)
    ///     .with_lod_distances(vec![128.0, 256.0, 512.0, 1024.0, 2048.0, 4096.0])
    ///     .build();
    /// ```
    #[must_use]
    pub fn builder(position: Vec3, size: f32, resolution: usize) -> TerrainChunkBuilder {
        TerrainChunkBuilder::new(position, size, resolution)
    }

    /// Get vertex spacing in meters.
    #[must_use]
    pub fn vertex_spacing(&self) -> f32 {
        self.size / (self.resolution - 1).exact_f32()
    }

    /// Convert an X/Z world position to heightmap coordinates.
    #[must_use]
    pub fn world_to_heightmap(&self, world_pos: Vec3) -> Option<(usize, usize)> {
        let local = world_pos - self.position;
        let cells = (self.resolution - 1).exact_f32();
        let x = floor_index(local.x / self.size * cells);
        let z = floor_index(local.z / self.size * cells);

        if x < self.resolution && z < self.resolution {
            Some((x, z))
        } else {
            None
        }
    }
}
