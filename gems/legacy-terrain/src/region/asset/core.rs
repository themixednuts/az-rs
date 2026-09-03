use bevy::prelude::*;

use crate::heightmap::math::ExactF32;
use crate::water::SerializableWaterQuadtree;
use crate::world::TerrainRegionId;

use super::super::heightmap::RegionHeightmap;
use super::super::surface::{
    TerrainMaterialLayerData, TerrainSurfaceWeight, WorldMaterialDataAsset,
};

/// Engine terrain data for one terrain region.
#[derive(Asset, Debug, Clone, Default, PartialEq, Reflect)]
pub struct TerrainRegionAsset {
    pub region: TerrainRegionId,
    pub origin: Vec2,
    pub heightmap: RegionHeightmap,
    pub water_quadtree: Option<SerializableWaterQuadtree>,
    pub surface_resolution: usize,
    pub surface_weights: Vec<TerrainSurfaceWeight>,
    pub material_layers: Vec<TerrainMaterialLayerData>,
    pub default_material_path: String,
    pub world_material: Option<Handle<WorldMaterialDataAsset>>,
}

impl TerrainRegionAsset {
    #[must_use]
    pub fn from_region_heightmap(
        region: TerrainRegionId,
        region_size: f32,
        heightmap: RegionHeightmap,
    ) -> Self {
        Self {
            region,
            origin: Vec2::new(
                region.x.exact_f32() * region_size,
                region.y.exact_f32() * region_size,
            ),
            heightmap,
            ..Default::default()
        }
    }

    #[must_use]
    pub fn region_world_size(&self) -> f32 {
        self.heightmap.resolution.exact_f32() * self.heightmap.cell_size
    }

    #[must_use]
    pub fn render_material_path(&self) -> Option<&str> {
        non_empty_path(&self.default_material_path).or_else(|| {
            self.material_layers
                .iter()
                .find_map(|layer| non_empty_path(&layer.material_path))
        })
    }
}

fn non_empty_path(path: &str) -> Option<&str> {
    let path = path.trim();
    (!path.is_empty()).then_some(path)
}
