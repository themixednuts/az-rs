//! Terrain region assets and surface data.

mod asset;
mod components;
mod heightmap;
mod provider;
mod surface;
mod type_ids;

use bevy::prelude::*;

use crate::engine_asset;
use crate::water::{SerializableWaterQuadtree, WaterNodeData, WaterNodeFlags};

pub use asset::TerrainRegionAsset;
pub use components::{TerrainSectorNode, TerrainWaterSurface};
pub use heightmap::RegionHeightmap;
pub use provider::TerrainDataProvider;
pub use surface::{
    RegionMaterialDataAsset, SerializableMacroMaterialParams, TerrainMaterialLayerData,
    TerrainSurfaceMap, TerrainSurfacePalette, TerrainSurfaceWeight, TileMaterialData,
    WorldMaterialDataAsset,
};
pub use type_ids::*;

pub fn register_region_components(app: &mut App) {
    app.register_type::<RegionHeightmap>()
        .register_type::<SerializableWaterQuadtree>()
        .register_type::<WaterNodeData>()
        .register_type::<WaterNodeFlags>()
        .register_type::<TerrainSurfaceWeight>()
        .register_type::<TerrainSurfaceMap>()
        .register_type::<TerrainSurfacePalette>()
        .register_type::<TerrainMaterialLayerData>()
        .register_type::<SerializableMacroMaterialParams>()
        .register_type::<RegionMaterialDataAsset>()
        .register_type::<TileMaterialData>()
        .register_type::<WorldMaterialDataAsset>()
        .register_type::<TerrainRegionAsset>()
        .register_type::<engine_asset::TerrainWorldManifest>()
        .register_type::<TerrainSectorNode>()
        .register_type::<TerrainWaterSurface>()
        .init_asset::<TerrainRegionAsset>()
        .init_asset::<engine_asset::TerrainWorldManifest>()
        .init_asset::<RegionMaterialDataAsset>()
        .init_asset::<WorldMaterialDataAsset>()
        .init_asset_loader::<engine_asset::TerrainRegionAssetLoader>()
        .init_asset_loader::<engine_asset::TerrainWorldManifestLoader>();
}
