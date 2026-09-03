use bevy::prelude::*;

use super::super::surface::{TerrainSurfaceMap, TerrainSurfaceWeight};
use super::core::TerrainRegionAsset;
use crate::heightmap::math::{ExactF32, clamped_index};

impl TerrainRegionAsset {
    #[must_use]
    pub fn contains_world_position(&self, x: f32, z: f32) -> bool {
        let size = self.region_world_size();
        x >= self.origin.x
            && z >= self.origin.y
            && x < self.origin.x + size
            && z < self.origin.y + size
    }

    #[must_use]
    pub fn height_at_world(&self, x: f32, z: f32) -> Option<f32> {
        let local_x = (x - self.origin.x) / self.heightmap.cell_size;
        let local_y = (z - self.origin.y) / self.heightmap.cell_size;
        self.heightmap.bilinear_height(local_x, local_y)
    }

    #[must_use]
    pub fn normal_at_world(&self, x: f32, z: f32) -> Option<Vec3> {
        let center = self.height_at_world(x, z)?;
        let spacing = self.heightmap.cell_size;
        if spacing <= 0.0 {
            return None;
        }

        let left = self.height_at_world(x - spacing, z).unwrap_or(center);
        let right = self.height_at_world(x + spacing, z).unwrap_or(center);
        let down = self.height_at_world(x, z - spacing).unwrap_or(center);
        let up = self.height_at_world(x, z + spacing).unwrap_or(center);

        let dx = Vec3::new(2.0 * spacing, right - left, 0.0);
        let dz = Vec3::new(0.0, up - down, 2.0 * spacing);
        let normal = dz.cross(dx).normalize_or_zero();
        (normal != Vec3::ZERO).then_some(normal)
    }

    #[must_use]
    pub fn surface_weight_at(&self, x: usize, y: usize) -> Option<TerrainSurfaceWeight> {
        if self.surface_resolution == 0
            || x >= self.surface_resolution
            || y >= self.surface_resolution
        {
            return None;
        }
        self.surface_weights
            .get(y * self.surface_resolution + x)
            .copied()
    }

    #[must_use]
    pub fn surface_weight_at_world(&self, x: f32, z: f32) -> Option<TerrainSurfaceWeight> {
        if self.surface_resolution == 0 {
            return None;
        }
        let size = self.region_world_size();
        if size <= 0.0 || !self.contains_world_position(x, z) {
            return None;
        }

        let surface_resolution = self.surface_resolution.exact_f32();
        let last = self.surface_resolution - 1;
        let surface_x = clamped_index(((x - self.origin.x) / size) * surface_resolution, last);
        let surface_z = clamped_index(((z - self.origin.y) / size) * surface_resolution, last);
        self.surface_weight_at(surface_x, surface_z)
    }

    #[must_use]
    pub fn surface_map(&self) -> Option<TerrainSurfaceMap> {
        TerrainSurfaceMap::new(self.surface_resolution, self.surface_weights.clone()).ok()
    }
}
