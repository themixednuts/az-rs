use std::io::{Read, Write};

use crate::TerrainSurfaceWeight;

use super::super::error::TerrainRegionAssetFormatError;
use super::primitives::{checked_u32, read_u32, write_u32};

pub(in crate::engine_asset) fn write_surface_weights(
    writer: &mut impl Write,
    weights: &[TerrainSurfaceWeight],
) -> Result<(), TerrainRegionAssetFormatError> {
    write_u32(writer, checked_u32(weights.len(), "surface weights")?)?;
    for weight in weights {
        writer.write_all(&weight.ids)?;
        writer.write_all(&weight.weights)?;
    }
    Ok(())
}

pub(in crate::engine_asset) fn read_surface_weights(
    reader: &mut impl Read,
) -> Result<Vec<TerrainSurfaceWeight>, TerrainRegionAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let byte_count = count
        .checked_mul(6)
        .ok_or(TerrainRegionAssetFormatError::InvalidData(
            "surface weight bytes overflow",
        ))?;
    let mut bytes = vec![0u8; byte_count];
    reader.read_exact(&mut bytes)?;
    Ok(bytes
        .chunks_exact(6)
        .map(|bytes| TerrainSurfaceWeight {
            ids: [bytes[0], bytes[1], bytes[2]],
            weights: [bytes[3], bytes[4], bytes[5]],
        })
        .collect())
}
