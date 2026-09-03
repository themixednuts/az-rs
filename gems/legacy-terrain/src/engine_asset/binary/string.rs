use std::io::{Read, Write};

use super::super::error::TerrainRegionAssetFormatError;
use super::primitives::{checked_u32, read_u32, write_u32};

pub(in crate::engine_asset) fn write_string(
    writer: &mut impl Write,
    value: &str,
) -> Result<(), TerrainRegionAssetFormatError> {
    write_u32(writer, checked_u32(value.len(), "string bytes")?)?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

pub(in crate::engine_asset) fn read_string(
    reader: &mut impl Read,
) -> Result<String, TerrainRegionAssetFormatError> {
    let len = read_u32(reader)? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}
