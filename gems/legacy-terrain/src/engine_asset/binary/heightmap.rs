use std::io::{Read, Write};

use crate::RegionHeightmap;

use super::super::error::TerrainRegionAssetFormatError;
use super::primitives::{checked_u32, read_f32, read_u32, write_f32, write_u32};

pub(in crate::engine_asset) fn write_heightmap(
    writer: &mut impl Write,
    heightmap: &RegionHeightmap,
) -> Result<(), TerrainRegionAssetFormatError> {
    write_u32(
        writer,
        checked_u32(heightmap.resolution, "heightmap resolution")?,
    )?;
    write_f32(writer, heightmap.cell_size)?;
    write_f32(writer, heightmap.height_scale)?;
    write_f32(writer, heightmap.min_height)?;
    write_f32(writer, heightmap.max_height)?;
    write_u32(
        writer,
        checked_u32(heightmap.samples.len(), "height samples")?,
    )?;
    for sample in &heightmap.samples {
        writer.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

pub(in crate::engine_asset) fn read_heightmap(
    reader: &mut impl Read,
) -> Result<RegionHeightmap, TerrainRegionAssetFormatError> {
    let resolution = read_u32(reader)? as usize;
    let cell_size = read_f32(reader)?;
    let height_scale = read_f32(reader)?;
    let min_height = read_f32(reader)?;
    let max_height = read_f32(reader)?;
    let sample_count = read_u32(reader)? as usize;
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

    let byte_count =
        sample_count
            .checked_mul(2)
            .ok_or(TerrainRegionAssetFormatError::InvalidData(
                "height sample bytes overflow",
            ))?;
    let mut sample_bytes = vec![0u8; byte_count];
    reader.read_exact(&mut sample_bytes)?;
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
