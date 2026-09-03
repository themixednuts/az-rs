use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::core::TerrainRegionAsset;
use crate::heightmap::math::{ExactF32, mesh_index};

impl TerrainRegionAsset {
    #[must_use]
    pub fn water_region_world_size(&self) -> Option<f32> {
        let water = self.water_quadtree.as_ref()?;
        if water.region_size > 0 {
            Some(water.region_size.exact_f32())
        } else {
            Some(self.region_world_size())
        }
    }

    #[must_use]
    pub fn water_surface_mesh(&self, surface_offset: f32) -> Option<Mesh> {
        let water = self.water_quadtree.as_ref()?;
        let region_size = self.water_region_world_size()?;
        let center = Vec2::new(
            region_size.mul_add(0.5, self.origin.x),
            region_size.mul_add(0.5, self.origin.y),
        );
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();

        for tile in water.surface_tiles() {
            let Some(min) = tile.min(self.origin, region_size) else {
                continue;
            };
            let Some(size) = tile.size(region_size) else {
                continue;
            };
            if size <= 0.0 {
                continue;
            }

            let max = min + Vec2::splat(size);
            let y = tile.node.height + surface_offset;
            let base = mesh_index(positions.len());

            positions.extend_from_slice(&[
                [min.x - center.x, y, min.y - center.y],
                [max.x - center.x, y, min.y - center.y],
                [min.x - center.x, y, max.y - center.y],
                [max.x - center.x, y, max.y - center.y],
            ]);
            normals.extend_from_slice(&[Vec3::Y.to_array(); 4]);
            uvs.extend_from_slice(&[
                [
                    (min.x - self.origin.x) / region_size,
                    (min.y - self.origin.y) / region_size,
                ],
                [
                    (max.x - self.origin.x) / region_size,
                    (min.y - self.origin.y) / region_size,
                ],
                [
                    (min.x - self.origin.x) / region_size,
                    (max.y - self.origin.y) / region_size,
                ],
                [
                    (max.x - self.origin.x) / region_size,
                    (max.y - self.origin.y) / region_size,
                ],
            ]);
            indices.extend_from_slice(&[base, base + 2, base + 1, base + 1, base + 2, base + 3]);
        }

        if positions.is_empty() {
            return None;
        }

        Some(
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(Indices::U32(indices)),
        )
    }
}
