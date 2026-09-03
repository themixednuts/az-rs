use std::io::{Read, Write};

use super::super::error::TerrainRegionAssetFormatError;

pub(in crate::engine_asset) fn checked_u32(
    count: usize,
    what: &'static str,
) -> Result<u32, TerrainRegionAssetFormatError> {
    u32::try_from(count).map_err(|_| TerrainRegionAssetFormatError::TooManyItems { what, count })
}

pub(in crate::engine_asset) fn write_u32(
    writer: &mut impl Write,
    value: u32,
) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

pub(in crate::engine_asset) fn write_u64(
    writer: &mut impl Write,
    value: u64,
) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

pub(in crate::engine_asset) fn write_u8(
    writer: &mut impl Write,
    value: u8,
) -> Result<(), std::io::Error> {
    writer.write_all(&[value])
}

pub(in crate::engine_asset) fn write_i32(
    writer: &mut impl Write,
    value: i32,
) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

pub(in crate::engine_asset) fn write_f32(
    writer: &mut impl Write,
    value: f32,
) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

pub(in crate::engine_asset) fn read_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

pub(in crate::engine_asset) fn read_u64(reader: &mut impl Read) -> Result<u64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

pub(in crate::engine_asset) fn read_u8(reader: &mut impl Read) -> Result<u8, std::io::Error> {
    let mut bytes = [0u8; 1];
    reader.read_exact(&mut bytes)?;
    Ok(bytes[0])
}

pub(in crate::engine_asset) fn read_i32(reader: &mut impl Read) -> Result<i32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

pub(in crate::engine_asset) fn read_f32(reader: &mut impl Read) -> Result<f32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}
