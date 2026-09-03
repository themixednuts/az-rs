use bevy::prelude::*;

use crate::chunk::{TerrainChunk, TerrainChunkBundle};
use crate::heightmap::Heightmap;
use crate::lod::TerrainLod;

use super::core::TerrainRegionAsset;

impl TerrainRegionAsset {
    /// Build a renderable chunk bundle by resampling this region's heightmap.
    ///
    /// # Errors
    ///
    /// Returns any error [`Heightmap::from_region_heightmap_resampled`]
    /// returns: the region heightmap or `resolution` being zero, or the
    /// resampled height data not matching `resolution`.
    pub fn render_bundle_with_resolution(
        &self,
        resolution: usize,
    ) -> Result<TerrainChunkBundle, &'static str> {
        let heightmap = Heightmap::from_region_heightmap_resampled(&self.heightmap, resolution)?;
        let size = self.region_world_size();
        let center = Vec3::new(
            size.mul_add(0.5, self.origin.x),
            0.0,
            size.mul_add(0.5, self.origin.y),
        );

        Ok(TerrainChunkBundle {
            chunk: TerrainChunk::builder(center, size, resolution).build(),
            heightmap,
            lod: TerrainLod::default(),
            transform: Transform::from_translation(center),
        })
    }
}
