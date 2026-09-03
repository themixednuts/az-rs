//! Terrain region ECS component data.

use bevy::math::Vec3A;
use bevy::math::bounding::Aabb3d;
use bevy::prelude::*;

use crate::world::TerrainRegionId;

/// Terrain sector state.
#[derive(Component, Debug, Clone, PartialEq, Reflect)]
#[reflect(Component)]
pub struct TerrainSectorNode {
    pub region: TerrainRegionId,
    pub bounds: Aabb3d,
    pub lod_level: u8,
    pub neighbor_lods: [u8; 4],
    pub min_height: f32,
    pub max_height: f32,
    pub mesh_dirty: bool,
    pub material_dirty: bool,
}

impl Default for TerrainSectorNode {
    fn default() -> Self {
        Self {
            region: TerrainRegionId::default(),
            bounds: Aabb3d::from_min_max(Vec3A::ZERO, Vec3A::ZERO),
            lod_level: 0,
            neighbor_lods: [0; 4],
            min_height: 0.0,
            max_height: 0.0,
            mesh_dirty: true,
            material_dirty: true,
        }
    }
}

/// Terrain water mesh spawned for a loaded region.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct TerrainWaterSurface {
    pub region: TerrainRegionId,
}
