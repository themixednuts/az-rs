//! Async Bevy reader for terrain-region assets.

use bevy::asset::AsyncReadExt;
use bevy::asset::io::Reader;
use bevy::prelude::*;

use crate::{
    RegionHeightmap, SerializableWaterQuadtree, TerrainMaterialLayerData, TerrainRegionAsset,
    TerrainRegionId, TerrainSurfaceWeight, WaterNodeData, WaterNodeFlags,
};

use super::constants::{MAGIC, VERSION};
use super::error::TerrainRegionAssetFormatError;

pub(super) async fn read_terrain_region_asset_from_bevy_reader(
    reader: &mut dyn Reader,
) -> Result<TerrainRegionAsset, TerrainRegionAssetFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic).await?;
    if &magic != MAGIC {
        return Err(TerrainRegionAssetFormatError::BadMagic { found: magic });
    }

    let version = read_u32(reader).await?;
    if version != VERSION {
        return Err(TerrainRegionAssetFormatError::UnsupportedVersion(version));
    }

    let region = TerrainRegionId::new(read_i32(reader).await?, read_i32(reader).await?);
    let origin = Vec2::new(read_f32(reader).await?, read_f32(reader).await?);
    let heightmap = read_heightmap(reader).await?;
    let water_quadtree = read_water_quadtree(reader).await?;
    let surface_resolution = read_u32(reader).await? as usize;
    let surface_weights = read_surface_weights(reader).await?;
    let material_layers = read_material_layers(reader).await?;
    let default_material_path = read_string(reader).await?;

    Ok(TerrainRegionAsset {
        region,
        origin,
        heightmap,
        water_quadtree,
        surface_resolution,
        surface_weights,
        material_layers,
        default_material_path,
        world_material: None,
    })
}

async fn read_heightmap(
    reader: &mut dyn Reader,
) -> Result<RegionHeightmap, TerrainRegionAssetFormatError> {
    let resolution = read_u32(reader).await? as usize;
    let cell_size = read_f32(reader).await?;
    let height_scale = read_f32(reader).await?;
    let min_height = read_f32(reader).await?;
    let max_height = read_f32(reader).await?;
    let sample_count = read_u32(reader).await? as usize;
    let expected =
        resolution
            .checked_mul(resolution)
            .ok_or(TerrainRegionAssetFormatError::InvalidData(
                "heightmap resolution overflow",
            ))?;
    if sample_count != expected {
        return Err(TerrainRegionAssetFormatError::InvalidData(
            "height sample count does not match resolution",
        ));
    }

    let mut sample_bytes = vec![0u8; byte_len(sample_count, 2, "height sample bytes")?];
    reader.read_exact(&mut sample_bytes).await?;
    let samples = sample_bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();

    Ok(RegionHeightmap {
        resolution,
        cell_size,
        height_scale,
        min_height,
        max_height,
        samples,
    })
}

async fn read_water_quadtree(
    reader: &mut dyn Reader,
) -> Result<Option<SerializableWaterQuadtree>, TerrainRegionAssetFormatError> {
    let mut present = [0u8; 1];
    reader.read_exact(&mut present).await?;
    if present[0] == 0 {
        return Ok(None);
    }

    let region_size = read_i32(reader).await?;
    let node_count = read_u32(reader).await? as usize;
    let mut node_bytes = vec![0u8; byte_len(node_count, 12, "water node bytes")?];
    reader.read_exact(&mut node_bytes).await?;
    let quadtree_nodes = node_bytes
        .chunks_exact(12)
        .map(|bytes| WaterNodeData {
            height: f32::from_le_bytes(bytes[0..4].try_into().expect("four bytes")),
            floor_height: f32::from_le_bytes(bytes[4..8].try_into().expect("four bytes")),
            flags: WaterNodeFlags::from_bits(u32::from_le_bytes(
                bytes[8..12].try_into().expect("four bytes"),
            )),
        })
        .collect();

    Ok(Some(SerializableWaterQuadtree {
        region_size,
        quadtree_nodes,
    }))
}

async fn read_surface_weights(
    reader: &mut dyn Reader,
) -> Result<Vec<TerrainSurfaceWeight>, TerrainRegionAssetFormatError> {
    let count = read_u32(reader).await? as usize;
    let mut bytes = vec![0u8; byte_len(count, 6, "surface weight bytes")?];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes
        .chunks_exact(6)
        .map(|bytes| TerrainSurfaceWeight {
            ids: [bytes[0], bytes[1], bytes[2]],
            weights: [bytes[3], bytes[4], bytes[5]],
        })
        .collect())
}

async fn read_material_layers(
    reader: &mut dyn Reader,
) -> Result<Vec<TerrainMaterialLayerData>, TerrainRegionAssetFormatError> {
    let count = read_u32(reader).await? as usize;
    let mut layers = Vec::with_capacity(count);
    for _ in 0..count {
        let material_path = read_string(reader).await?;
        let splat_map_path = read_string(reader).await?;
        let affected_tiles = read_u64(reader).await?;
        let priority = read_u8(reader).await?;
        layers.push(TerrainMaterialLayerData {
            material_path,
            splat_map_path,
            affected_tiles,
            priority,
        });
    }
    Ok(layers)
}

async fn read_string(reader: &mut dyn Reader) -> Result<String, TerrainRegionAssetFormatError> {
    let len = read_u32(reader).await? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes).await?;
    Ok(String::from_utf8(bytes)?)
}

async fn read_u32(reader: &mut dyn Reader) -> Result<u32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(u32::from_le_bytes(bytes))
}

async fn read_u64(reader: &mut dyn Reader) -> Result<u64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes).await?;
    Ok(u64::from_le_bytes(bytes))
}

async fn read_u8(reader: &mut dyn Reader) -> Result<u8, std::io::Error> {
    let mut bytes = [0u8; 1];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes[0])
}

async fn read_i32(reader: &mut dyn Reader) -> Result<i32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(i32::from_le_bytes(bytes))
}

async fn read_f32(reader: &mut dyn Reader) -> Result<f32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(f32::from_le_bytes(bytes))
}

fn byte_len(
    count: usize,
    stride: usize,
    what: &'static str,
) -> Result<usize, TerrainRegionAssetFormatError> {
    count
        .checked_mul(stride)
        .ok_or(TerrainRegionAssetFormatError::InvalidData(what))
}
