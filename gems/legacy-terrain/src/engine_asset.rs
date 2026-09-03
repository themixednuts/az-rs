//! Engine terrain-region asset format.

mod bevy_region_reader;
mod binary;
mod constants;
mod error;
mod inspection;
mod loader;
mod manifest;
mod region;

pub use constants::{TERRAIN_REGION_ASSET_EXTENSIONS, TERRAIN_WORLD_MANIFEST_EXTENSIONS};
pub use error::{TerrainRegionAssetFormatError, TerrainWorldManifestFormatError};
pub use inspection::{
    TerrainRegionAssetFileInspection, TerrainRegionAssetInspection,
    inspect_terrain_region_asset_file,
};
pub use loader::{TerrainRegionAssetLoader, TerrainWorldManifestLoader};
pub use manifest::{
    TerrainRegionProduct, TerrainWorldManifest, TerrainWorldManifestProduct,
    TerrainWorldManifestRegion, build_terrain_world_manifests, read_terrain_world_manifest,
    read_terrain_world_manifest_from_reader, terrain_region_product_from_path,
    terrain_world_manifest_engine_path, write_terrain_world_manifest,
};
pub use region::{
    read_terrain_region_asset, read_terrain_region_asset_from_reader, write_terrain_region_asset,
};

#[cfg(test)]
use crate::{
    RegionHeightmap, SerializableWaterQuadtree, TerrainMaterialLayerData, TerrainRegionAsset,
    TerrainRegionId, WaterNodeData,
};
#[cfg(test)]
use bevy::prelude::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TerrainSurfaceWeight, WaterNodeFlags};

    #[test]
    fn terrain_region_asset_round_trips_engine_binary() {
        let heightmap = RegionHeightmap::from_samples(vec![1, 2, 3, 4], 2, 1.0, 2.0, -4.0).unwrap();
        let asset = TerrainRegionAsset {
            region: TerrainRegionId::new(2, 3),
            origin: Vec2::new(4096.0, 6144.0),
            heightmap,
            water_quadtree: Some(SerializableWaterQuadtree {
                region_size: 2048,
                quadtree_nodes: vec![WaterNodeData {
                    height: 10.0,
                    floor_height: 2.0,
                    flags: WaterNodeFlags::from_bits(5),
                }],
            }),
            surface_resolution: 1,
            surface_weights: vec![TerrainSurfaceWeight {
                ids: [1, 2, 127],
                weights: [128, 127, 0],
            }],
            material_layers: vec![TerrainMaterialLayerData {
                material_path: "materials/terrain/frontend/a.mtl".to_string(),
                splat_map_path: "materials/terrain/frontend/a.surfacemap".to_string(),
                affected_tiles: 0xffff,
                priority: 7,
            }],
            default_material_path: "materials/terrain/frontend/default.mtl".to_string(),
            world_material: None,
        };

        let mut bytes = Vec::new();
        write_terrain_region_asset(&asset, &mut bytes).unwrap();
        let decoded = read_terrain_region_asset(&bytes).unwrap();

        assert_eq!(decoded.region, asset.region);
        assert_eq!(decoded.origin, asset.origin);
        assert_eq!(decoded.heightmap, asset.heightmap);
        assert_eq!(decoded.water_quadtree, asset.water_quadtree);
        assert_eq!(decoded.surface_resolution, asset.surface_resolution);
        assert_eq!(decoded.surface_weights, asset.surface_weights);
        assert_eq!(decoded.material_layers, asset.material_layers);
        assert_eq!(decoded.default_material_path, asset.default_material_path);
        assert!(decoded.world_material.is_none());
    }

    #[test]
    fn terrain_world_manifest_round_trips_binary() {
        let manifest = TerrainWorldManifest {
            world_name: "frontend".to_string(),
            regions: vec![
                TerrainWorldManifestRegion {
                    x: 0,
                    y: 0,
                    path: "terrain/frontend_0_0.terrain-region.bin".to_string(),
                },
                TerrainWorldManifestRegion {
                    x: 1,
                    y: 0,
                    path: "terrain/frontend_1_0.terrain-region.bin".to_string(),
                },
            ],
        };

        let mut bytes = Vec::new();
        write_terrain_world_manifest(&manifest, &mut bytes).unwrap();
        let decoded = read_terrain_world_manifest(&bytes).unwrap();

        assert_eq!(decoded, manifest);
        assert_eq!(decoded.regions[0].region(), TerrainRegionId::new(0, 0));
    }
}
