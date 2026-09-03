//! Terrain world metadata and level component data.

use bevy::math::bounding::Aabb3d;
use bevy::math::{Vec3, Vec3A};
use bevy::prelude::*;
use std::collections::HashMap;

use crate::heightmap::math::ExactF32;
use crate::region::{TerrainRegionAsset, TerrainSurfaceWeight};

/// Lumberyard `LegacyTerrain::LegacyTerrainLevelConfig` type UUID.
pub const LEGACY_TERRAIN_LEVEL_CONFIG_TYPE_ID: &str = "65950218-99C8-4DF5-A949-6642A8C69444";

/// Lumberyard `LegacyTerrain::LegacyTerrainLevelComponent` component UUID.
pub const LEGACY_TERRAIN_LEVEL_COMPONENT_TYPE_ID: &str = "9BA33CA7-07DF-409F-A240-5A7AA67EFFE8";

/// Lumberyard `LegacyTerrain::LegacyTerrainModule` type UUID.
pub const LEGACY_TERRAIN_MODULE_TYPE_ID: &str = "4774487F-71DC-4CA4-943C-A31CE05D0616";

pub const DEFAULT_TERRAIN_UNIT_SIZE_METERS: i32 = 2;
pub const DEFAULT_TERRAIN_SIZE_METERS: i32 = 1024;
pub const DEFAULT_TERRAIN_SECTOR_SIZE_METERS: i32 = 64;
pub const DEFAULT_TERRAIN_SECTORS_TABLE_SIZE: i32 = 16;
pub const TERRAIN_NODE_CHUNK_VERSION: i32 = 8;

pub const TERRAIN_INFO_COMPONENT_TYPE_ID: &str = "2041EB3F-62B8-491A-914E-4DAF1F1D70A2";
pub const TERRAIN_INFO_SERVER_FACET_TYPE_ID: &str = "4FB54D01-E8CE-45C2-A4ED-1059401C6168";

/// Terrain level component configuration.
///
/// Lumberyard reference: `dev/Gems/LegacyTerrain/Code/Source/LegacyTerrainLevelComponent.h:25`.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct LegacyTerrainLevelConfig;

/// Terrain metadata read from compiled octree data.
///
/// Lumberyard reference: `dev/Code/CryEngine/CryCommon/I3DEngine.h:726`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct LegacyTerrainInfo {
    pub heightmap_size_in_units: i32,
    pub unit_size_in_meters: i32,
    pub sector_size_in_meters: i32,
    pub sectors_table_size_in_sectors: i32,
    pub heightmap_z_ratio: f32,
    pub ocean_water_level: f32,
}

impl Default for LegacyTerrainInfo {
    fn default() -> Self {
        Self {
            heightmap_size_in_units: DEFAULT_TERRAIN_SIZE_METERS / DEFAULT_TERRAIN_UNIT_SIZE_METERS,
            unit_size_in_meters: DEFAULT_TERRAIN_UNIT_SIZE_METERS,
            sector_size_in_meters: DEFAULT_TERRAIN_SECTOR_SIZE_METERS,
            sectors_table_size_in_sectors: DEFAULT_TERRAIN_SECTORS_TABLE_SIZE,
            heightmap_z_ratio: 0.0,
            ocean_water_level: 0.0,
        }
    }
}

impl LegacyTerrainInfo {
    #[must_use]
    pub const fn terrain_size_in_meters(&self) -> i32 {
        self.heightmap_size_in_units * self.unit_size_in_meters
    }

    #[must_use]
    pub const fn sector_size_in_units(&self) -> i32 {
        self.sector_size_in_meters / self.unit_size_in_meters
    }
}

/// Region coordinate in legacy terrain data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
pub struct TerrainRegionId {
    pub x: i32,
    pub y: i32,
}

impl TerrainRegionId {
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Loaded terrain region index.
#[derive(Debug, Clone, Resource, Reflect)]
pub struct TerrainWorld {
    pub world_name: String,
    pub terrain_info: LegacyTerrainInfo,
    pub bounds: Aabb3d,
    pub loaded_regions: HashMap<TerrainRegionId, Handle<TerrainRegionAsset>>,
}

impl Default for TerrainWorld {
    fn default() -> Self {
        let terrain_info = LegacyTerrainInfo::default();
        let size = terrain_info.terrain_size_in_meters().exact_f32();
        Self {
            world_name: String::new(),
            terrain_info,
            bounds: Aabb3d::from_min_max(Vec3A::ZERO, Vec3A::new(size, 0.0, size)),
            loaded_regions: HashMap::new(),
        }
    }
}

impl TerrainWorld {
    #[must_use]
    pub fn loaded_region_at_world<'a>(
        &self,
        x: f32,
        z: f32,
        terrain_assets: &'a Assets<TerrainRegionAsset>,
    ) -> Option<&'a TerrainRegionAsset> {
        self.loaded_regions
            .values()
            .filter_map(|handle| terrain_assets.get(handle))
            .find(|region| region.contains_world_position(x, z))
    }

    #[must_use]
    pub fn height_at_world(
        &self,
        x: f32,
        z: f32,
        terrain_assets: &Assets<TerrainRegionAsset>,
    ) -> Option<f32> {
        self.loaded_region_at_world(x, z, terrain_assets)?
            .height_at_world(x, z)
    }

    #[must_use]
    pub fn normal_at_world(
        &self,
        x: f32,
        z: f32,
        terrain_assets: &Assets<TerrainRegionAsset>,
    ) -> Option<Vec3> {
        self.loaded_region_at_world(x, z, terrain_assets)?
            .normal_at_world(x, z)
    }

    #[must_use]
    pub fn surface_weight_at_world(
        &self,
        x: f32,
        z: f32,
        terrain_assets: &Assets<TerrainRegionAsset>,
    ) -> Option<TerrainSurfaceWeight> {
        self.loaded_region_at_world(x, z, terrain_assets)?
            .surface_weight_at_world(x, z)
    }
}

/// Level component that owns terrain data for a loaded world.
///
/// Lumberyard reference: `dev/Gems/LegacyTerrain/Code/Source/LegacyTerrainLevelComponent.h:34`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Component)]
pub struct LegacyTerrainLevelComponent {
    pub configuration: LegacyTerrainLevelConfig,
    pub terrain_info: LegacyTerrainInfo,
}

pub fn register_world_components(app: &mut App) {
    app.register_type::<LegacyTerrainLevelConfig>()
        .register_type::<LegacyTerrainInfo>()
        .register_type::<LegacyTerrainLevelComponent>()
        .register_type::<TerrainRegionId>()
        .register_type::<TerrainWorld>()
        .init_resource::<TerrainWorld>();
}
