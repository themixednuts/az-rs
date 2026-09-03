//! Terrain region and water rendering systems.

use az_gem_lmbr_central::{
    MaterialAsset, apply_material_asset_bindings as apply_terrain_material_asset_bindings,
};
use bevy::asset::AssetEvent;
use bevy::prelude::*;

mod config;
pub mod material;
mod resources;
mod systems;

use crate::chunk::TerrainSet;

pub use config::{TerrainMeshResolution, TerrainRegionRenderConfig, TerrainWaterRenderConfig};
use resources::{LoadedTerrainWorldManifests, RenderedTerrainRegions};
use systems::{process_loaded_terrain_world_manifests, spawn_loaded_terrain_regions};

pub fn register_render_components(app: &mut App) {
    app.register_type::<TerrainMeshResolution>()
        .register_type::<TerrainRegionRenderConfig>()
        .register_type::<TerrainWaterRenderConfig>()
        .init_resource::<TerrainRegionRenderConfig>()
        .init_resource::<TerrainWaterRenderConfig>()
        .init_resource::<RenderedTerrainRegions>()
        .init_resource::<LoadedTerrainWorldManifests>()
        .add_message::<AssetEvent<MaterialAsset>>()
        .add_systems(
            Update,
            (
                process_loaded_terrain_world_manifests
                    .in_set(TerrainSet::MeshGeneration)
                    .before(spawn_loaded_terrain_regions),
                spawn_loaded_terrain_regions.in_set(TerrainSet::MeshGeneration),
                apply_terrain_material_asset_bindings.in_set(TerrainSet::MeshGeneration),
            ),
        );
}
