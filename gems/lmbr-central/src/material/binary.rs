//! Native binary material asset formats.

use std::io::{Cursor, Read, Write};

use bevy::asset::AsyncReadExt;
use bevy::asset::io::Reader;
use bevy::color::{LinearRgba, Srgba};

use super::definition::{
    MaterialAsset, MaterialDefinition, MaterialOverrideAsset, MaterialOverrideParam,
    MaterialOverrideParamBlock, MaterialOverrideSubTarget, MaterialOverrideSwitch,
    MaterialOverrideTarget, MaterialOverrideValueKind,
};
use super::format::MaterialAssetFormatError;
use super::texture::{
    MaterialPublicParam, MaterialTextureFilter, MaterialTextureMap, MaterialTextureReference,
    MaterialTextureType,
};

const MATERIAL_MAGIC: &[u8; 8] = b"AZMATRL\0";
const MATERIAL_OVERRIDE_MAGIC: &[u8; 8] = b"AZMTLOV\0";
const MATERIAL_VERSION: u32 = 2;
const MATERIAL_OVERRIDE_VERSION: u32 = 1;

pub(super) fn write_material_asset(
    asset: &MaterialAsset,
    mut writer: impl Write,
) -> Result<(), MaterialAssetFormatError> {
    writer.write_all(MATERIAL_MAGIC)?;
    write_u32(&mut writer, MATERIAL_VERSION)?;
    write_string(&mut writer, &asset.source_path)?;
    write_material_definition(&mut writer, &asset.root)?;
    write_material_definitions(&mut writer, &asset.sub_materials)?;
    Ok(())
}

pub(super) fn read_material_asset(bytes: &[u8]) -> Result<MaterialAsset, MaterialAssetFormatError> {
    read_material_asset_from_reader(Cursor::new(bytes))
}

pub(super) fn read_material_asset_from_reader(
    mut reader: impl Read,
) -> Result<MaterialAsset, MaterialAssetFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != MATERIAL_MAGIC {
        return Err(MaterialAssetFormatError::BadMagic { found: magic });
    }
    let version = read_u32(&mut reader)?;
    if version != MATERIAL_VERSION {
        return Err(MaterialAssetFormatError::UnsupportedVersion {
            version,
            expected: MATERIAL_VERSION,
        });
    }
    Ok(MaterialAsset {
        source_path: read_string(&mut reader)?,
        root: read_material_definition(&mut reader)?,
        sub_materials: read_material_definitions(&mut reader)?,
    })
}

pub(super) async fn read_material_asset_from_bevy_reader(
    reader: &mut dyn Reader,
) -> Result<MaterialAsset, MaterialAssetFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic).await?;
    if &magic != MATERIAL_MAGIC {
        return Err(MaterialAssetFormatError::BadMagic { found: magic });
    }
    let version = read_async_u32(reader).await?;
    if version != MATERIAL_VERSION {
        return Err(MaterialAssetFormatError::UnsupportedVersion {
            version,
            expected: MATERIAL_VERSION,
        });
    }
    Ok(MaterialAsset {
        source_path: read_async_string(reader).await?,
        root: read_async_material_definition(reader).await?,
        sub_materials: read_async_material_definitions(reader).await?,
    })
}

pub(super) fn write_material_override_asset(
    asset: &MaterialOverrideAsset,
    mut writer: impl Write,
) -> Result<(), MaterialAssetFormatError> {
    writer.write_all(MATERIAL_OVERRIDE_MAGIC)?;
    write_u32(&mut writer, MATERIAL_OVERRIDE_VERSION)?;
    write_string(&mut writer, &asset.source_path)?;
    write_option_string(&mut writer, asset.max_trigger_distance.as_deref())?;
    write_override_targets(&mut writer, &asset.materials)?;
    write_public_params(&mut writer, &asset.extra_attributes)?;
    Ok(())
}

pub(super) fn read_material_override_asset(
    bytes: &[u8],
) -> Result<MaterialOverrideAsset, MaterialAssetFormatError> {
    read_material_override_asset_from_reader(Cursor::new(bytes))
}

pub(super) fn read_material_override_asset_from_reader(
    mut reader: impl Read,
) -> Result<MaterialOverrideAsset, MaterialAssetFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != MATERIAL_OVERRIDE_MAGIC {
        return Err(MaterialAssetFormatError::BadMagic { found: magic });
    }
    let version = read_u32(&mut reader)?;
    if version != MATERIAL_OVERRIDE_VERSION {
        return Err(MaterialAssetFormatError::UnsupportedVersion {
            version,
            expected: MATERIAL_OVERRIDE_VERSION,
        });
    }
    Ok(MaterialOverrideAsset {
        source_path: read_string(&mut reader)?,
        max_trigger_distance: read_option_string(&mut reader)?,
        materials: read_override_targets(&mut reader)?,
        extra_attributes: read_public_params(&mut reader)?,
    })
}

pub(super) async fn read_material_override_asset_from_bevy_reader(
    reader: &mut dyn Reader,
) -> Result<MaterialOverrideAsset, MaterialAssetFormatError> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic).await?;
    if &magic != MATERIAL_OVERRIDE_MAGIC {
        return Err(MaterialAssetFormatError::BadMagic { found: magic });
    }
    let version = read_async_u32(reader).await?;
    if version != MATERIAL_OVERRIDE_VERSION {
        return Err(MaterialAssetFormatError::UnsupportedVersion {
            version,
            expected: MATERIAL_OVERRIDE_VERSION,
        });
    }
    Ok(MaterialOverrideAsset {
        source_path: read_async_string(reader).await?,
        max_trigger_distance: read_async_option_string(reader).await?,
        materials: read_async_override_targets(reader).await?,
        extra_attributes: read_async_public_params(reader).await?,
    })
}

fn write_material_definitions(
    writer: &mut impl Write,
    values: &[MaterialDefinition],
) -> Result<(), MaterialAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "material definitions")?)?;
    for value in values {
        write_material_definition(writer, value)?;
    }
    Ok(())
}

fn read_material_definitions(
    reader: &mut impl Read,
) -> Result<Vec<MaterialDefinition>, MaterialAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_material_definition(reader)?);
    }
    Ok(values)
}

async fn read_async_material_definitions(
    reader: &mut dyn Reader,
) -> Result<Vec<MaterialDefinition>, MaterialAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_async_material_definition(reader).await?);
    }
    Ok(values)
}

fn write_material_definition(
    writer: &mut impl Write,
    value: &MaterialDefinition,
) -> Result<(), MaterialAssetFormatError> {
    write_option_string(writer, value.name.as_deref())?;
    write_option_string(writer, value.shader.as_deref())?;
    write_option_string(writer, value.surface_type.as_deref())?;
    write_option_color(writer, value.diffuse)?;
    write_option_color(writer, value.specular)?;
    write_option_color(writer, value.emissive)?;
    write_option_linear_rgba(writer, value.emittance)?;
    write_f32(writer, value.opacity)?;
    write_f32(writer, value.shininess)?;
    write_option_f32(writer, value.alpha_test)?;
    write_option_string(writer, value.gen_mask.as_deref())?;
    write_option_string(writer, value.string_gen_mask.as_deref())?;
    write_option_u64(writer, value.material_flags)?;
    write_option_f32(writer, value.cloak_amount)?;
    write_texture_references(writer, &value.textures)?;
    write_public_params(writer, &value.public_params)?;
    write_public_params(writer, &value.extra_attributes)?;
    Ok(())
}

fn read_material_definition(
    reader: &mut impl Read,
) -> Result<MaterialDefinition, MaterialAssetFormatError> {
    Ok(MaterialDefinition {
        name: read_option_string(reader)?,
        shader: read_option_string(reader)?,
        surface_type: read_option_string(reader)?,
        diffuse: read_option_color(reader)?,
        specular: read_option_color(reader)?,
        emissive: read_option_color(reader)?,
        emittance: read_option_linear_rgba(reader)?,
        opacity: read_f32(reader)?,
        shininess: read_f32(reader)?,
        alpha_test: read_option_f32(reader)?,
        gen_mask: read_option_string(reader)?,
        string_gen_mask: read_option_string(reader)?,
        material_flags: read_option_u64(reader)?,
        cloak_amount: read_option_f32(reader)?,
        textures: read_texture_references(reader)?,
        public_params: read_public_params(reader)?,
        extra_attributes: read_public_params(reader)?,
    })
}

async fn read_async_material_definition(
    reader: &mut dyn Reader,
) -> Result<MaterialDefinition, MaterialAssetFormatError> {
    Ok(MaterialDefinition {
        name: read_async_option_string(reader).await?,
        shader: read_async_option_string(reader).await?,
        surface_type: read_async_option_string(reader).await?,
        diffuse: read_async_option_color(reader).await?,
        specular: read_async_option_color(reader).await?,
        emissive: read_async_option_color(reader).await?,
        emittance: read_async_option_linear_rgba(reader).await?,
        opacity: read_async_f32(reader).await?,
        shininess: read_async_f32(reader).await?,
        alpha_test: read_async_option_f32(reader).await?,
        gen_mask: read_async_option_string(reader).await?,
        string_gen_mask: read_async_option_string(reader).await?,
        material_flags: read_async_option_u64(reader).await?,
        cloak_amount: read_async_option_f32(reader).await?,
        textures: read_async_texture_references(reader).await?,
        public_params: read_async_public_params(reader).await?,
        extra_attributes: read_async_public_params(reader).await?,
    })
}

fn write_texture_references(
    writer: &mut impl Write,
    values: &[MaterialTextureReference],
) -> Result<(), MaterialAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "material textures")?)?;
    for value in values {
        write_string(writer, value.map.native_name().as_ref())?;
        write_option_string(writer, value.image_asset_path.as_deref())?;
        write_option_string(writer, value.asset_id.as_deref())?;
        write_option_i32(
            writer,
            value.filter.map(MaterialTextureFilter::native_value),
        )?;
        write_bool(writer, value.is_tile_u)?;
        write_bool(writer, value.is_tile_v)?;
        write_option_i32(
            writer,
            value.texture_type.map(MaterialTextureType::native_value),
        )?;
        write_public_params(writer, &value.texture_modifier)?;
    }
    Ok(())
}

fn read_texture_references(
    reader: &mut impl Read,
) -> Result<Vec<MaterialTextureReference>, MaterialAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let map = MaterialTextureMap::from_native_name(&read_string(reader)?);
        let image_asset_path = read_option_string(reader)?;
        let asset_id = read_option_string(reader)?;
        let filter = read_option_i32(reader)?
            .map(|value| {
                MaterialTextureFilter::from_native_value(value).ok_or(
                    MaterialAssetFormatError::InvalidData("material texture filter"),
                )
            })
            .transpose()?;
        let is_tile_u = read_bool(reader)?;
        let is_tile_v = read_bool(reader)?;
        let texture_type = read_option_i32(reader)?
            .map(|value| {
                MaterialTextureType::from_native_value(value).ok_or(
                    MaterialAssetFormatError::InvalidData("material texture type"),
                )
            })
            .transpose()?;
        let texture_modifier = read_public_params(reader)?;
        values.push(MaterialTextureReference {
            map,
            image_asset_path,
            asset_id,
            filter,
            is_tile_u,
            is_tile_v,
            texture_type,
            texture_modifier,
        });
    }
    Ok(values)
}

async fn read_async_texture_references(
    reader: &mut dyn Reader,
) -> Result<Vec<MaterialTextureReference>, MaterialAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let map = MaterialTextureMap::from_native_name(&read_async_string(reader).await?);
        let image_asset_path = read_async_option_string(reader).await?;
        let asset_id = read_async_option_string(reader).await?;
        let filter = read_async_option_i32(reader)
            .await?
            .map(|value| {
                MaterialTextureFilter::from_native_value(value).ok_or(
                    MaterialAssetFormatError::InvalidData("material texture filter"),
                )
            })
            .transpose()?;
        let is_tile_u = read_async_bool(reader).await?;
        let is_tile_v = read_async_bool(reader).await?;
        let texture_type = read_async_option_i32(reader)
            .await?
            .map(|value| {
                MaterialTextureType::from_native_value(value).ok_or(
                    MaterialAssetFormatError::InvalidData("material texture type"),
                )
            })
            .transpose()?;
        let texture_modifier = read_async_public_params(reader).await?;
        values.push(MaterialTextureReference {
            map,
            image_asset_path,
            asset_id,
            filter,
            is_tile_u,
            is_tile_v,
            texture_type,
            texture_modifier,
        });
    }
    Ok(values)
}

fn write_public_params(
    writer: &mut impl Write,
    values: &[MaterialPublicParam],
) -> Result<(), MaterialAssetFormatError> {
    write_u32(writer, checked_u32(values.len(), "material public params")?)?;
    for value in values {
        write_string(writer, &value.name)?;
        write_string(writer, &value.value)?;
    }
    Ok(())
}

fn read_public_params(
    reader: &mut impl Read,
) -> Result<Vec<MaterialPublicParam>, MaterialAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialPublicParam {
            name: read_string(reader)?,
            value: read_string(reader)?,
        });
    }
    Ok(values)
}

async fn read_async_public_params(
    reader: &mut dyn Reader,
) -> Result<Vec<MaterialPublicParam>, MaterialAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialPublicParam {
            name: read_async_string(reader).await?,
            value: read_async_string(reader).await?,
        });
    }
    Ok(values)
}

fn write_override_targets(
    writer: &mut impl Write,
    values: &[MaterialOverrideTarget],
) -> Result<(), MaterialAssetFormatError> {
    write_u32(
        writer,
        checked_u32(values.len(), "material override targets")?,
    )?;
    for value in values {
        write_option_string(writer, value.name.as_deref())?;
        write_option_string(writer, value.exclude.as_deref())?;
        write_override_sub_targets(writer, &value.sub_materials)?;
        write_public_params(writer, &value.extra_attributes)?;
    }
    Ok(())
}

fn read_override_targets(
    reader: &mut impl Read,
) -> Result<Vec<MaterialOverrideTarget>, MaterialAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideTarget {
            name: read_option_string(reader)?,
            exclude: read_option_string(reader)?,
            sub_materials: read_override_sub_targets(reader)?,
            extra_attributes: read_public_params(reader)?,
        });
    }
    Ok(values)
}

async fn read_async_override_targets(
    reader: &mut dyn Reader,
) -> Result<Vec<MaterialOverrideTarget>, MaterialAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideTarget {
            name: read_async_option_string(reader).await?,
            exclude: read_async_option_string(reader).await?,
            sub_materials: read_async_override_sub_targets(reader).await?,
            extra_attributes: read_async_public_params(reader).await?,
        });
    }
    Ok(values)
}

fn write_override_sub_targets(
    writer: &mut impl Write,
    values: &[MaterialOverrideSubTarget],
) -> Result<(), MaterialAssetFormatError> {
    write_u32(
        writer,
        checked_u32(values.len(), "material override sub targets")?,
    )?;
    for value in values {
        write_option_string(writer, value.name.as_deref())?;
        write_override_switches(writer, &value.shader_generation_params)?;
        write_override_param_blocks(writer, &value.texture_maps)?;
        write_override_param_blocks(writer, &value.shader_params)?;
        write_public_params(writer, &value.extra_attributes)?;
    }
    Ok(())
}

fn read_override_sub_targets(
    reader: &mut impl Read,
) -> Result<Vec<MaterialOverrideSubTarget>, MaterialAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideSubTarget {
            name: read_option_string(reader)?,
            shader_generation_params: read_override_switches(reader)?,
            texture_maps: read_override_param_blocks(reader)?,
            shader_params: read_override_param_blocks(reader)?,
            extra_attributes: read_public_params(reader)?,
        });
    }
    Ok(values)
}

async fn read_async_override_sub_targets(
    reader: &mut dyn Reader,
) -> Result<Vec<MaterialOverrideSubTarget>, MaterialAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideSubTarget {
            name: read_async_option_string(reader).await?,
            shader_generation_params: read_async_override_switches(reader).await?,
            texture_maps: read_async_override_param_blocks(reader).await?,
            shader_params: read_async_override_param_blocks(reader).await?,
            extra_attributes: read_async_public_params(reader).await?,
        });
    }
    Ok(values)
}

fn write_override_switches(
    writer: &mut impl Write,
    values: &[MaterialOverrideSwitch],
) -> Result<(), MaterialAssetFormatError> {
    write_u32(
        writer,
        checked_u32(values.len(), "material override switches")?,
    )?;
    for value in values {
        write_string(writer, &value.name)?;
        write_bool(writer, value.enabled)?;
        write_public_params(writer, &value.extra_attributes)?;
    }
    Ok(())
}

fn read_override_switches(
    reader: &mut impl Read,
) -> Result<Vec<MaterialOverrideSwitch>, MaterialAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideSwitch {
            name: read_string(reader)?,
            enabled: read_bool(reader)?,
            extra_attributes: read_public_params(reader)?,
        });
    }
    Ok(values)
}

async fn read_async_override_switches(
    reader: &mut dyn Reader,
) -> Result<Vec<MaterialOverrideSwitch>, MaterialAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideSwitch {
            name: read_async_string(reader).await?,
            enabled: read_async_bool(reader).await?,
            extra_attributes: read_async_public_params(reader).await?,
        });
    }
    Ok(values)
}

fn write_override_param_blocks(
    writer: &mut impl Write,
    values: &[MaterialOverrideParamBlock],
) -> Result<(), MaterialAssetFormatError> {
    write_u32(
        writer,
        checked_u32(values.len(), "material override param blocks")?,
    )?;
    for value in values {
        write_string(writer, &value.name)?;
        write_override_params(writer, &value.params)?;
        write_public_params(writer, &value.extra_attributes)?;
    }
    Ok(())
}

fn read_override_param_blocks(
    reader: &mut impl Read,
) -> Result<Vec<MaterialOverrideParamBlock>, MaterialAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideParamBlock {
            name: read_string(reader)?,
            params: read_override_params(reader)?,
            extra_attributes: read_public_params(reader)?,
        });
    }
    Ok(values)
}

async fn read_async_override_param_blocks(
    reader: &mut dyn Reader,
) -> Result<Vec<MaterialOverrideParamBlock>, MaterialAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideParamBlock {
            name: read_async_string(reader).await?,
            params: read_async_override_params(reader).await?,
            extra_attributes: read_async_public_params(reader).await?,
        });
    }
    Ok(values)
}

fn write_override_params(
    writer: &mut impl Write,
    values: &[MaterialOverrideParam],
) -> Result<(), MaterialAssetFormatError> {
    write_u32(
        writer,
        checked_u32(values.len(), "material override params")?,
    )?;
    for value in values {
        write_string(writer, &value.name)?;
        write_override_value_kind(writer, &value.value_kind)?;
        write_string(writer, &value.value)?;
        write_public_params(writer, &value.extra_attributes)?;
    }
    Ok(())
}

fn read_override_params(
    reader: &mut impl Read,
) -> Result<Vec<MaterialOverrideParam>, MaterialAssetFormatError> {
    let count = read_u32(reader)? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideParam {
            name: read_string(reader)?,
            value_kind: read_override_value_kind(reader)?,
            value: read_string(reader)?,
            extra_attributes: read_public_params(reader)?,
        });
    }
    Ok(values)
}

async fn read_async_override_params(
    reader: &mut dyn Reader,
) -> Result<Vec<MaterialOverrideParam>, MaterialAssetFormatError> {
    let count = read_async_u32(reader).await? as usize;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(MaterialOverrideParam {
            name: read_async_string(reader).await?,
            value_kind: read_async_override_value_kind(reader).await?,
            value: read_async_string(reader).await?,
            extra_attributes: read_async_public_params(reader).await?,
        });
    }
    Ok(values)
}

fn write_override_value_kind(
    writer: &mut impl Write,
    value: &MaterialOverrideValueKind,
) -> Result<(), MaterialAssetFormatError> {
    let code = match value {
        MaterialOverrideValueKind::String => 0,
        MaterialOverrideValueKind::Float => 1,
        MaterialOverrideValueKind::Color => 2,
        MaterialOverrideValueKind::Bool => 3,
        MaterialOverrideValueKind::Int => 4,
        MaterialOverrideValueKind::Vector => 5,
        MaterialOverrideValueKind::Unknown(value) => {
            writer.write_all(&[255])?;
            write_string(writer, value)?;
            return Ok(());
        }
    };
    writer.write_all(&[code])?;
    Ok(())
}

fn read_override_value_kind(
    reader: &mut impl Read,
) -> Result<MaterialOverrideValueKind, MaterialAssetFormatError> {
    let code = read_u8(reader)?;
    Ok(match code {
        0 => MaterialOverrideValueKind::String,
        1 => MaterialOverrideValueKind::Float,
        2 => MaterialOverrideValueKind::Color,
        3 => MaterialOverrideValueKind::Bool,
        4 => MaterialOverrideValueKind::Int,
        5 => MaterialOverrideValueKind::Vector,
        255 => MaterialOverrideValueKind::Unknown(read_string(reader)?),
        _ => {
            return Err(MaterialAssetFormatError::InvalidData(
                "material override value kind",
            ));
        }
    })
}

async fn read_async_override_value_kind(
    reader: &mut dyn Reader,
) -> Result<MaterialOverrideValueKind, MaterialAssetFormatError> {
    let code = read_async_u8(reader).await?;
    Ok(match code {
        0 => MaterialOverrideValueKind::String,
        1 => MaterialOverrideValueKind::Float,
        2 => MaterialOverrideValueKind::Color,
        3 => MaterialOverrideValueKind::Bool,
        4 => MaterialOverrideValueKind::Int,
        5 => MaterialOverrideValueKind::Vector,
        255 => MaterialOverrideValueKind::Unknown(read_async_string(reader).await?),
        _ => {
            return Err(MaterialAssetFormatError::InvalidData(
                "material override value kind",
            ));
        }
    })
}

fn write_option_string(
    writer: &mut impl Write,
    value: Option<&str>,
) -> Result<(), MaterialAssetFormatError> {
    match value {
        Some(value) => {
            writer.write_all(&[1])?;
            write_string(writer, value)?;
        }
        None => writer.write_all(&[0])?,
    }
    Ok(())
}

fn read_option_string(reader: &mut impl Read) -> Result<Option<String>, MaterialAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(read_string(reader)?))
}

async fn read_async_option_string(
    reader: &mut dyn Reader,
) -> Result<Option<String>, MaterialAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(read_async_string(reader).await?))
}

fn write_string(writer: &mut impl Write, value: &str) -> Result<(), MaterialAssetFormatError> {
    write_u32(writer, checked_u32(value.len(), "string bytes")?)?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn read_string(reader: &mut impl Read) -> Result<String, MaterialAssetFormatError> {
    let len = read_u32(reader)? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}

async fn read_async_string(reader: &mut dyn Reader) -> Result<String, MaterialAssetFormatError> {
    let len = read_async_u32(reader).await? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes).await?;
    Ok(String::from_utf8(bytes)?)
}

fn write_option_color(
    writer: &mut impl Write,
    value: Option<Srgba>,
) -> Result<(), MaterialAssetFormatError> {
    match value {
        Some(value) => {
            writer.write_all(&[1])?;
            write_f32(writer, value.red)?;
            write_f32(writer, value.green)?;
            write_f32(writer, value.blue)?;
            write_f32(writer, value.alpha)?;
        }
        None => writer.write_all(&[0])?,
    }
    Ok(())
}

fn read_option_color(reader: &mut impl Read) -> Result<Option<Srgba>, MaterialAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(Srgba::new(
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
        read_f32(reader)?,
    )))
}

async fn read_async_option_color(
    reader: &mut dyn Reader,
) -> Result<Option<Srgba>, MaterialAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(Srgba::new(
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
        read_async_f32(reader).await?,
    )))
}

fn write_option_linear_rgba(
    writer: &mut impl Write,
    value: Option<LinearRgba>,
) -> Result<(), MaterialAssetFormatError> {
    match value {
        Some(value) => {
            writer.write_all(&[1])?;
            write_f32(writer, value.red)?;
            write_f32(writer, value.green)?;
            write_f32(writer, value.blue)?;
            write_f32(writer, value.alpha)?;
        }
        None => writer.write_all(&[0])?,
    }
    Ok(())
}

fn read_option_linear_rgba(
    reader: &mut impl Read,
) -> Result<Option<LinearRgba>, MaterialAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(LinearRgba {
        red: read_f32(reader)?,
        green: read_f32(reader)?,
        blue: read_f32(reader)?,
        alpha: read_f32(reader)?,
    }))
}

async fn read_async_option_linear_rgba(
    reader: &mut dyn Reader,
) -> Result<Option<LinearRgba>, MaterialAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(LinearRgba {
        red: read_async_f32(reader).await?,
        green: read_async_f32(reader).await?,
        blue: read_async_f32(reader).await?,
        alpha: read_async_f32(reader).await?,
    }))
}

fn write_option_f32(
    writer: &mut impl Write,
    value: Option<f32>,
) -> Result<(), MaterialAssetFormatError> {
    match value {
        Some(value) => {
            writer.write_all(&[1])?;
            write_f32(writer, value)?;
        }
        None => writer.write_all(&[0])?,
    }
    Ok(())
}

fn read_option_f32(reader: &mut impl Read) -> Result<Option<f32>, MaterialAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(read_f32(reader)?))
}

async fn read_async_option_f32(
    reader: &mut dyn Reader,
) -> Result<Option<f32>, MaterialAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(read_async_f32(reader).await?))
}

fn write_option_i32(
    writer: &mut impl Write,
    value: Option<i32>,
) -> Result<(), MaterialAssetFormatError> {
    match value {
        Some(value) => {
            writer.write_all(&[1])?;
            write_i32(writer, value)?;
        }
        None => writer.write_all(&[0])?,
    }
    Ok(())
}

fn read_option_i32(reader: &mut impl Read) -> Result<Option<i32>, MaterialAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(read_i32(reader)?))
}

async fn read_async_option_i32(
    reader: &mut dyn Reader,
) -> Result<Option<i32>, MaterialAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(read_async_i32(reader).await?))
}

fn write_option_u64(
    writer: &mut impl Write,
    value: Option<u64>,
) -> Result<(), MaterialAssetFormatError> {
    match value {
        Some(value) => {
            writer.write_all(&[1])?;
            write_u64(writer, value)?;
        }
        None => writer.write_all(&[0])?,
    }
    Ok(())
}

fn read_option_u64(reader: &mut impl Read) -> Result<Option<u64>, MaterialAssetFormatError> {
    if !read_bool(reader)? {
        return Ok(None);
    }
    Ok(Some(read_u64(reader)?))
}

async fn read_async_option_u64(
    reader: &mut dyn Reader,
) -> Result<Option<u64>, MaterialAssetFormatError> {
    if !read_async_bool(reader).await? {
        return Ok(None);
    }
    Ok(Some(read_async_u64(reader).await?))
}

fn write_bool(writer: &mut impl Write, value: bool) -> Result<(), std::io::Error> {
    writer.write_all(&[u8::from(value)])
}

fn read_bool(reader: &mut impl Read) -> Result<bool, MaterialAssetFormatError> {
    match read_u8(reader)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(MaterialAssetFormatError::InvalidData("boolean value")),
    }
}

async fn read_async_bool(reader: &mut dyn Reader) -> Result<bool, MaterialAssetFormatError> {
    match read_async_u8(reader).await? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(MaterialAssetFormatError::InvalidData("boolean value")),
    }
}

fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_i32(writer: &mut impl Write, value: i32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u64(writer: &mut impl Write, value: u64) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn write_f32(writer: &mut impl Write, value: f32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn read_u8(reader: &mut impl Read) -> Result<u8, std::io::Error> {
    let mut byte = [0u8; 1];
    reader.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn read_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_i32(reader: &mut impl Read) -> Result<i32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(i32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Result<u64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_f32(reader: &mut impl Read) -> Result<f32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

async fn read_async_u8(reader: &mut dyn Reader) -> Result<u8, std::io::Error> {
    let mut byte = [0u8; 1];
    reader.read_exact(&mut byte).await?;
    Ok(byte[0])
}

async fn read_async_u32(reader: &mut dyn Reader) -> Result<u32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(u32::from_le_bytes(bytes))
}

async fn read_async_i32(reader: &mut dyn Reader) -> Result<i32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(i32::from_le_bytes(bytes))
}

async fn read_async_u64(reader: &mut dyn Reader) -> Result<u64, std::io::Error> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes).await?;
    Ok(u64::from_le_bytes(bytes))
}

async fn read_async_f32(reader: &mut dyn Reader) -> Result<f32, std::io::Error> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).await?;
    Ok(f32::from_le_bytes(bytes))
}

fn checked_u32(count: usize, what: &'static str) -> Result<u32, MaterialAssetFormatError> {
    u32::try_from(count).map_err(|_| MaterialAssetFormatError::TooManyItems { what, count })
}
