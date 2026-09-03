use std::io::{Read, Write};

use crate::TerrainMaterialLayerData;

use super::super::error::TerrainRegionAssetFormatError;
use super::primitives::{checked_u32, read_u8, read_u32, read_u64, write_u8, write_u32, write_u64};
use super::string::{read_string, write_string};

pub(in crate::engine_asset) fn write_material_layers(
    writer: &mut impl Write,
    layers: &[TerrainMaterialLayerData],
) -> Result<(), TerrainRegionAssetFormatError> {
    write_u32(writer, checked_u32(layers.len(), "material layers")?)?;
    for layer in layers {
        write_string(writer, &layer.material_path)?;
        write_string(writer, &layer.splat_map_path)?;
        write_u64(writer, layer.affected_tiles)?;
        write_u8(writer, layer.priority)?;
    }
    Ok(())
}

pub(in crate::engine_asset) fn read_material_layers(
    reader: &mut impl Read,
) -> Result<Vec<TerrainMaterialLayerData>, TerrainRegionAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut layers = Vec::with_capacity(count);
    for _ in 0..count {
        let material_path = read_string(reader)?;
        let splat_map_path = read_string(reader)?;
        let affected_tiles = read_u64(reader)?;
        let priority = read_u8(reader)?;
        layers.push(TerrainMaterialLayerData {
            material_path,
            splat_map_path,
            affected_tiles,
            priority,
        });
    }
    Ok(layers)
}
