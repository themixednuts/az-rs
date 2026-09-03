use std::io::{Read, Write};

use crate::{SerializableWaterQuadtree, WaterNodeData, WaterNodeFlags};

use super::super::error::TerrainRegionAssetFormatError;
use super::primitives::{checked_u32, read_i32, read_u32, write_f32, write_i32, write_u32};

pub(in crate::engine_asset) fn write_water_quadtree(
    writer: &mut impl Write,
    water: Option<&SerializableWaterQuadtree>,
) -> Result<(), TerrainRegionAssetFormatError> {
    match water {
        Some(water) => {
            writer.write_all(&[1])?;
            write_i32(writer, water.region_size)?;
            write_u32(
                writer,
                checked_u32(water.quadtree_nodes.len(), "water nodes")?,
            )?;
            for node in &water.quadtree_nodes {
                write_f32(writer, node.height)?;
                write_f32(writer, node.floor_height)?;
                write_u32(writer, node.flags.bits())?;
            }
        }
        None => writer.write_all(&[0])?,
    }
    Ok(())
}

pub(in crate::engine_asset) fn read_water_quadtree(
    reader: &mut impl Read,
) -> Result<Option<SerializableWaterQuadtree>, TerrainRegionAssetFormatError> {
    let mut present = [0u8; 1];
    reader.read_exact(&mut present)?;
    if present[0] == 0 {
        return Ok(None);
    }

    let region_size = read_i32(reader)?;
    let node_count = read_u32(reader)? as usize;
    let byte_count =
        node_count
            .checked_mul(12)
            .ok_or(TerrainRegionAssetFormatError::InvalidData(
                "water node bytes overflow",
            ))?;
    let mut node_bytes = vec![0u8; byte_count];
    reader.read_exact(&mut node_bytes)?;
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
