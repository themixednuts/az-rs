//! Terrain data lookup traits.

use bevy::math::bounding::Aabb3d;
use bevy::math::{Vec3, Vec3A};

use super::asset::TerrainRegionAsset;
use super::surface::TerrainSurfaceWeight;

/// Terrain data lookup interface.
pub trait TerrainDataProvider {
    fn bounds(&self) -> Aabb3d;
    fn cell_size(&self) -> f32;
    fn height_at_world(&self, x: f32, z: f32) -> Option<f32>;
    fn normal_at_world(&self, x: f32, z: f32) -> Option<Vec3>;
    fn surface_weight_at(&self, x: usize, y: usize) -> Option<TerrainSurfaceWeight>;
    fn surface_weight_at_world(&self, x: f32, z: f32) -> Option<TerrainSurfaceWeight>;
}

impl TerrainDataProvider for TerrainRegionAsset {
    fn bounds(&self) -> Aabb3d {
        let width = self.region_world_size();
        Aabb3d::from_min_max(
            Vec3A::new(self.origin.x, self.heightmap.min_height, self.origin.y),
            Vec3A::new(
                self.origin.x + width,
                self.heightmap.max_height,
                self.origin.y + width,
            ),
        )
    }

    fn cell_size(&self) -> f32 {
        self.heightmap.cell_size
    }

    fn height_at_world(&self, x: f32, z: f32) -> Option<f32> {
        self.height_at_world(x, z)
    }

    fn normal_at_world(&self, x: f32, z: f32) -> Option<Vec3> {
        self.normal_at_world(x, z)
    }

    fn surface_weight_at(&self, x: usize, y: usize) -> Option<TerrainSurfaceWeight> {
        self.surface_weight_at(x, y)
    }

    fn surface_weight_at_world(&self, x: f32, z: f32) -> Option<TerrainSurfaceWeight> {
        self.surface_weight_at_world(x, z)
    }
}
