//! Terrain-region binary assets.

use std::io::{Cursor, Read, Write};

use bevy::prelude::*;

use crate::{TerrainRegionAsset, TerrainRegionId};

use super::binary::{
    checked_u32, read_f32, read_heightmap, read_i32, read_material_layers, read_string,
    read_surface_weights, read_u32, read_water_quadtree, write_f32, write_heightmap, write_i32,
    write_material_layers, write_string, write_surface_weights, write_u32, write_water_quadtree,
};
use super::constants::{MAGIC, VERSION};
use super::error::TerrainRegionAssetFormatError;

/// Write an engine terrain-region asset.
///
/// # Errors
///
/// Returns [`TerrainRegionAssetFormatError::TooManyItems`] if the surface
/// resolution, or any heightmap, water, surface-weight or material-layer
/// count, exceeds `u32`, or [`TerrainRegionAssetFormatError::Io`] if `writer`
/// fails.
pub fn write_terrain_region_asset(
    asset: &TerrainRegionAsset,
    mut writer: impl Write,
) -> Result<(), TerrainRegionAssetFormatError> {
    writer.write_all(MAGIC)?;
    write_u32(&mut writer, VERSION)?;
    write_i32(&mut writer, asset.region.x)?;
    write_i32(&mut writer, asset.region.y)?;
    write_f32(&mut writer, asset.origin.x)?;
    write_f32(&mut writer, asset.origin.y)?;

    write_heightmap(&mut writer, &asset.heightmap)?;
    write_water_quadtree(&mut writer, asset.water_quadtree.as_ref())?;
    write_u32(
        &mut writer,
        checked_u32(asset.surface_resolution, "surface resolution")?,
    )?;
    write_surface_weights(&mut writer, &asset.surface_weights)?;
    write_material_layers(&mut writer, &asset.material_layers)?;
    write_string(&mut writer, &asset.default_material_path)?;

    Ok(())
}

/// Read an engine terrain-region asset.
///
/// # Errors
///
/// Returns any error [`read_terrain_region_asset_from_reader`] returns.
pub fn read_terrain_region_asset(
    bytes: &[u8],
) -> Result<TerrainRegionAsset, TerrainRegionAssetFormatError> {
    read_terrain_region_asset_from_reader(Cursor::new(bytes))
}

/// Read an engine terrain-region asset from a stream.
///
/// # Errors
///
/// Returns [`TerrainRegionAssetFormatError::BadMagic`] if the stream does not
/// start with the region magic, [`TerrainRegionAssetFormatError::UnsupportedVersion`]
/// for any version other than the current one,
/// [`TerrainRegionAssetFormatError::InvalidData`] if a heightmap, water or
/// surface record is internally inconsistent,
/// [`TerrainRegionAssetFormatError::Utf8`] if a stored path is not UTF-8, or
/// [`TerrainRegionAssetFormatError::Io`] if `reader` ends early or fails.
pub fn read_terrain_region_asset_from_reader(
    mut reader: impl Read,
) -> Result<TerrainRegionAsset, TerrainRegionAssetFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(TerrainRegionAssetFormatError::BadMagic { found: magic });
    }

    let version = read_u32(&mut reader)?;
    if version != VERSION {
        return Err(TerrainRegionAssetFormatError::UnsupportedVersion(version));
    }

    let region = TerrainRegionId::new(read_i32(&mut reader)?, read_i32(&mut reader)?);
    let origin = Vec2::new(read_f32(&mut reader)?, read_f32(&mut reader)?);
    let heightmap = read_heightmap(&mut reader)?;
    let water_quadtree = read_water_quadtree(&mut reader)?;
    let surface_resolution = read_u32(&mut reader)? as usize;
    let surface_weights = read_surface_weights(&mut reader)?;
    let material_layers = read_material_layers(&mut reader)?;
    let default_material_path = read_string(&mut reader)?;

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
