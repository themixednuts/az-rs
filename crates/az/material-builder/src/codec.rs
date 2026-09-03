//! Byte codecs for compiled material products.
//!
//! Same shape as `az-terrain-runtime`: magic constant + separate header
//! version, little-endian scalars, `u32`-length-prefixed strings/collections,
//! strict trailing-byte checks.

use std::io::{self, Write};
use std::str::FromStr;

use az_material::{
    BlendMode, CullMode, MaterialColor, MaterialDomain, MaterialPropertyBinding,
    MaterialPropertyDefinition, MaterialPropertyGroup, MaterialPropertyValue, MaterialTexture,
    ShadingModel,
};
use glam::{Vec2, Vec3, Vec4};
use thiserror::Error;

use crate::{
    MATERIAL_MAGIC, MATERIAL_PRODUCT_VERSION, MATERIAL_TYPE_MAGIC, MATERIAL_TYPE_PRODUCT_VERSION,
    MaterialAsset, MaterialTypeAsset,
};

#[derive(Debug, Error)]
pub enum MaterialAssetCodecError {
    #[error("material product ended while reading {field}")]
    UnexpectedEof { field: &'static str },
    #[error("bad material product magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported material product version {version}")]
    UnsupportedVersion { version: u32 },
    #[error("material product text field {field} is not UTF-8: {source}")]
    InvalidUtf8 {
        field: &'static str,
        source: std::str::Utf8Error,
    },
    #[error("unknown material product tag {tag} while reading {field}")]
    InvalidTag { field: &'static str, tag: u8 },
    #[error("material product count for {field} is too large: {count}")]
    CountTooLarge { field: &'static str, count: u64 },
    #[error("material product asset path {field} is invalid: {reason}")]
    InvalidAssetPath { field: &'static str, reason: String },
    #[error("material product has {remaining} trailing bytes")]
    TrailingBytes { remaining: usize },
    #[error("failed to write material product: {0}")]
    Io(#[from] io::Error),
}

/// Encode a compiled material-type product into bytes.
///
/// # Errors
///
/// Returns an error if a field cannot be written or a collection/string length
/// exceeds the product format's `u32` length limit.
pub fn encode_material_type_asset(
    asset: &MaterialTypeAsset,
) -> Result<Vec<u8>, MaterialAssetCodecError> {
    let mut bytes = Vec::new();
    bytes.write_all(MATERIAL_TYPE_MAGIC)?;
    write_u32(&mut bytes, MATERIAL_TYPE_PRODUCT_VERSION)?;
    write_string(&mut bytes, "name", &asset.name)?;
    write_string(&mut bytes, "description", &asset.description)?;
    write_domain(&mut bytes, asset.domain)?;
    write_blend_mode(&mut bytes, asset.blend_mode)?;
    write_cull_mode(&mut bytes, asset.cull_mode)?;
    write_shading_model(&mut bytes, asset.shading_model)?;
    write_string(&mut bytes, "shader graph", &asset.shader_graph)?;
    write_len(&mut bytes, "property groups", asset.property_groups.len())?;
    for group in &asset.property_groups {
        write_property_group(&mut bytes, group)?;
    }
    Ok(bytes)
}

/// Decode a compiled material-type product.
///
/// # Errors
///
/// Returns an error if the byte stream is truncated, has a bad magic/version,
/// contains unknown tags, invalid UTF-8, or trailing bytes.
pub fn decode_material_type_asset(
    bytes: &[u8],
) -> Result<MaterialTypeAsset, MaterialAssetCodecError> {
    let mut reader = MaterialReader::new(bytes);
    read_header(
        &mut reader,
        *MATERIAL_TYPE_MAGIC,
        MATERIAL_TYPE_PRODUCT_VERSION,
    )?;
    let asset = MaterialTypeAsset {
        name: reader.read_string("name")?,
        description: reader.read_string("description")?,
        domain: read_domain(&mut reader)?,
        blend_mode: read_blend_mode(&mut reader)?,
        cull_mode: read_cull_mode(&mut reader)?,
        shading_model: read_shading_model(&mut reader)?,
        shader_graph: reader.read_string("shader graph")?,
        property_groups: read_vec(&mut reader, "property groups", read_property_group)?,
    };
    reader.finish()?;
    Ok(asset)
}

/// Encode a compiled material product into bytes.
///
/// # Errors
///
/// Returns an error if a field cannot be written or a collection/string length
/// exceeds the product format's `u32` length limit.
pub fn encode_material_asset(asset: &MaterialAsset) -> Result<Vec<u8>, MaterialAssetCodecError> {
    let mut bytes = Vec::new();
    bytes.write_all(MATERIAL_MAGIC)?;
    write_u32(&mut bytes, MATERIAL_PRODUCT_VERSION)?;
    write_string(&mut bytes, "name", &asset.name)?;
    write_string(&mut bytes, "material type", &asset.material_type)?;
    write_option(&mut bytes, asset.parent.as_deref(), |writer, value| {
        write_string(writer, "parent", value)
    })?;
    write_len(&mut bytes, "property values", asset.property_values.len())?;
    for binding in &asset.property_values {
        write_property_binding(&mut bytes, binding)?;
    }
    Ok(bytes)
}

/// Decode a compiled material product.
///
/// # Errors
///
/// Returns an error if the byte stream is truncated, has a bad magic/version,
/// contains unknown tags, invalid UTF-8, or trailing bytes.
pub fn decode_material_asset(bytes: &[u8]) -> Result<MaterialAsset, MaterialAssetCodecError> {
    let mut reader = MaterialReader::new(bytes);
    read_header(&mut reader, *MATERIAL_MAGIC, MATERIAL_PRODUCT_VERSION)?;
    let asset = MaterialAsset {
        name: reader.read_string("name")?,
        material_type: reader.read_string("material type")?,
        parent: read_option(&mut reader, "parent", |reader| reader.read_string("parent"))?,
        property_values: read_vec(&mut reader, "property values", read_property_binding)?,
    };
    reader.finish()?;
    Ok(asset)
}

fn write_property_group<W: Write + ?Sized>(
    writer: &mut W,
    group: &MaterialPropertyGroup,
) -> Result<(), MaterialAssetCodecError> {
    write_string(writer, "group id", &group.id)?;
    write_string(writer, "group display name", &group.display_name)?;
    write_string(writer, "group description", &group.description)?;
    write_len(writer, "group properties", group.properties.len())?;
    for property in &group.properties {
        write_property_definition(writer, property)?;
    }
    Ok(())
}

fn read_property_group(
    reader: &mut MaterialReader<'_>,
) -> Result<MaterialPropertyGroup, MaterialAssetCodecError> {
    Ok(MaterialPropertyGroup {
        id: reader.read_string("group id")?,
        display_name: reader.read_string("group display name")?,
        description: reader.read_string("group description")?,
        properties: read_vec(reader, "group properties", read_property_definition)?,
    })
}

fn write_property_definition<W: Write + ?Sized>(
    writer: &mut W,
    definition: &MaterialPropertyDefinition,
) -> Result<(), MaterialAssetCodecError> {
    write_string(writer, "property id", &definition.id)?;
    write_string(writer, "property display name", &definition.display_name)?;
    write_string(writer, "property description", &definition.description)?;
    write_property_value(writer, &definition.default_value)?;
    write_option(writer, definition.connection.as_deref(), |writer, value| {
        write_string(writer, "property connection", value)
    })?;
    write_len(writer, "property enum values", definition.enum_values.len())?;
    for value in &definition.enum_values {
        write_string(writer, "property enum value", value)?;
    }
    Ok(())
}

fn read_property_definition(
    reader: &mut MaterialReader<'_>,
) -> Result<MaterialPropertyDefinition, MaterialAssetCodecError> {
    Ok(MaterialPropertyDefinition {
        id: reader.read_string("property id")?,
        display_name: reader.read_string("property display name")?,
        description: reader.read_string("property description")?,
        default_value: read_property_value(reader)?,
        connection: read_option(reader, "property connection", |reader| {
            reader.read_string("property connection")
        })?,
        enum_values: read_vec(reader, "property enum values", |reader| {
            reader.read_string("property enum value")
        })?,
    })
}

fn write_property_binding<W: Write + ?Sized>(
    writer: &mut W,
    binding: &MaterialPropertyBinding,
) -> Result<(), MaterialAssetCodecError> {
    write_string(writer, "binding property", &binding.property)?;
    write_property_value(writer, &binding.value)
}

fn read_property_binding(
    reader: &mut MaterialReader<'_>,
) -> Result<MaterialPropertyBinding, MaterialAssetCodecError> {
    Ok(MaterialPropertyBinding {
        property: reader.read_string("binding property")?,
        value: read_property_value(reader)?,
    })
}

fn write_property_value<W: Write + ?Sized>(
    writer: &mut W,
    value: &MaterialPropertyValue,
) -> Result<(), MaterialAssetCodecError> {
    match value {
        MaterialPropertyValue::Bool(value) => {
            write_u8(writer, 0)?;
            write_u8(writer, u8::from(*value))?;
        }
        MaterialPropertyValue::Int(value) => {
            write_u8(writer, 1)?;
            writer.write_all(&value.to_le_bytes())?;
        }
        MaterialPropertyValue::UInt(value) => {
            write_u8(writer, 2)?;
            write_u32(writer, *value)?;
        }
        MaterialPropertyValue::Float(value) => {
            write_u8(writer, 3)?;
            write_f32(writer, *value)?;
        }
        MaterialPropertyValue::Vec2(value) => {
            write_u8(writer, 4)?;
            write_f32(writer, value.x)?;
            write_f32(writer, value.y)?;
        }
        MaterialPropertyValue::Vec3(value) => {
            write_u8(writer, 5)?;
            write_f32(writer, value.x)?;
            write_f32(writer, value.y)?;
            write_f32(writer, value.z)?;
        }
        MaterialPropertyValue::Vec4(value) => {
            write_u8(writer, 6)?;
            write_vec4(writer, *value)?;
        }
        MaterialPropertyValue::Color(value) => {
            write_u8(writer, 7)?;
            write_vec4(writer, value.rgba)?;
        }
        MaterialPropertyValue::Texture(value) => {
            write_u8(writer, 8)?;
            write_string(writer, "texture asset", value.asset.as_str())?;
        }
        MaterialPropertyValue::String(value) => {
            write_u8(writer, 9)?;
            write_string(writer, "string value", value)?;
        }
    }
    Ok(())
}

fn read_property_value(
    reader: &mut MaterialReader<'_>,
) -> Result<MaterialPropertyValue, MaterialAssetCodecError> {
    let tag = reader.read_u8("property value tag")?;
    match tag {
        0 => match reader.read_u8("bool value")? {
            0 => Ok(MaterialPropertyValue::Bool(false)),
            1 => Ok(MaterialPropertyValue::Bool(true)),
            tag => Err(MaterialAssetCodecError::InvalidTag {
                field: "bool value",
                tag,
            }),
        },
        1 => Ok(MaterialPropertyValue::Int(i32::from_le_bytes(
            reader.read_array::<4>("int value")?,
        ))),
        2 => Ok(MaterialPropertyValue::UInt(reader.read_u32("uint value")?)),
        3 => Ok(MaterialPropertyValue::Float(
            reader.read_f32("float value")?,
        )),
        4 => Ok(MaterialPropertyValue::Vec2(Vec2::new(
            reader.read_f32("vec2 x")?,
            reader.read_f32("vec2 y")?,
        ))),
        5 => Ok(MaterialPropertyValue::Vec3(Vec3::new(
            reader.read_f32("vec3 x")?,
            reader.read_f32("vec3 y")?,
            reader.read_f32("vec3 z")?,
        ))),
        6 => Ok(MaterialPropertyValue::Vec4(read_vec4(reader)?)),
        7 => Ok(MaterialPropertyValue::Color(MaterialColor {
            rgba: read_vec4(reader)?,
        })),
        8 => {
            let path = reader.read_string("texture asset")?;
            Ok(MaterialPropertyValue::Texture(MaterialTexture {
                asset: parse_asset_path(&path, "texture asset")?,
            }))
        }
        9 => Ok(MaterialPropertyValue::String(
            reader.read_string("string value")?,
        )),
        tag => Err(MaterialAssetCodecError::InvalidTag {
            field: "property value",
            tag,
        }),
    }
}

fn parse_asset_path<T>(path: &str, field: &'static str) -> Result<T, MaterialAssetCodecError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    path.parse()
        .map_err(|error: T::Err| MaterialAssetCodecError::InvalidAssetPath {
            field,
            reason: error.to_string(),
        })
}

fn write_domain<W: Write + ?Sized>(
    writer: &mut W,
    domain: MaterialDomain,
) -> Result<(), MaterialAssetCodecError> {
    let tag = match domain {
        MaterialDomain::Surface => 0,
        MaterialDomain::Decal => 1,
        MaterialDomain::PostProcess => 2,
    };
    write_u8(writer, tag)
}

fn read_domain(reader: &mut MaterialReader<'_>) -> Result<MaterialDomain, MaterialAssetCodecError> {
    match reader.read_u8("domain")? {
        0 => Ok(MaterialDomain::Surface),
        1 => Ok(MaterialDomain::Decal),
        2 => Ok(MaterialDomain::PostProcess),
        tag => Err(MaterialAssetCodecError::InvalidTag {
            field: "domain",
            tag,
        }),
    }
}

fn write_blend_mode<W: Write + ?Sized>(
    writer: &mut W,
    blend_mode: BlendMode,
) -> Result<(), MaterialAssetCodecError> {
    let tag = match blend_mode {
        BlendMode::Opaque => 0,
        BlendMode::Masked => 1,
        BlendMode::Translucent => 2,
        BlendMode::Additive => 3,
    };
    write_u8(writer, tag)
}

fn read_blend_mode(reader: &mut MaterialReader<'_>) -> Result<BlendMode, MaterialAssetCodecError> {
    match reader.read_u8("blend mode")? {
        0 => Ok(BlendMode::Opaque),
        1 => Ok(BlendMode::Masked),
        2 => Ok(BlendMode::Translucent),
        3 => Ok(BlendMode::Additive),
        tag => Err(MaterialAssetCodecError::InvalidTag {
            field: "blend mode",
            tag,
        }),
    }
}

fn write_cull_mode<W: Write + ?Sized>(
    writer: &mut W,
    cull_mode: CullMode,
) -> Result<(), MaterialAssetCodecError> {
    let tag = match cull_mode {
        CullMode::Back => 0,
        CullMode::Front => 1,
        CullMode::None => 2,
    };
    write_u8(writer, tag)
}

fn read_cull_mode(reader: &mut MaterialReader<'_>) -> Result<CullMode, MaterialAssetCodecError> {
    match reader.read_u8("cull mode")? {
        0 => Ok(CullMode::Back),
        1 => Ok(CullMode::Front),
        2 => Ok(CullMode::None),
        tag => Err(MaterialAssetCodecError::InvalidTag {
            field: "cull mode",
            tag,
        }),
    }
}

fn write_shading_model<W: Write + ?Sized>(
    writer: &mut W,
    shading_model: ShadingModel,
) -> Result<(), MaterialAssetCodecError> {
    let tag = match shading_model {
        ShadingModel::Standard => 0,
        ShadingModel::Unlit => 1,
    };
    write_u8(writer, tag)
}

fn read_shading_model(
    reader: &mut MaterialReader<'_>,
) -> Result<ShadingModel, MaterialAssetCodecError> {
    match reader.read_u8("shading model")? {
        0 => Ok(ShadingModel::Standard),
        1 => Ok(ShadingModel::Unlit),
        tag => Err(MaterialAssetCodecError::InvalidTag {
            field: "shading model",
            tag,
        }),
    }
}

fn write_vec4<W: Write + ?Sized>(
    writer: &mut W,
    value: Vec4,
) -> Result<(), MaterialAssetCodecError> {
    write_f32(writer, value.x)?;
    write_f32(writer, value.y)?;
    write_f32(writer, value.z)?;
    write_f32(writer, value.w)
}

fn read_vec4(reader: &mut MaterialReader<'_>) -> Result<Vec4, MaterialAssetCodecError> {
    Ok(Vec4::new(
        reader.read_f32("vec4 x")?,
        reader.read_f32("vec4 y")?,
        reader.read_f32("vec4 z")?,
        reader.read_f32("vec4 w")?,
    ))
}

fn write_option<W, T>(
    writer: &mut W,
    value: Option<T>,
    write_value: impl FnOnce(&mut W, T) -> Result<(), MaterialAssetCodecError>,
) -> Result<(), MaterialAssetCodecError>
where
    W: Write + ?Sized,
{
    match value {
        Some(value) => {
            write_u8(writer, 1)?;
            write_value(writer, value)?;
        }
        None => write_u8(writer, 0)?,
    }
    Ok(())
}

fn read_option<T>(
    reader: &mut MaterialReader<'_>,
    field: &'static str,
    read_value: impl FnOnce(&mut MaterialReader<'_>) -> Result<T, MaterialAssetCodecError>,
) -> Result<Option<T>, MaterialAssetCodecError> {
    let tag = reader.read_u8(field)?;
    match tag {
        0 => Ok(None),
        1 => read_value(reader).map(Some),
        tag => Err(MaterialAssetCodecError::InvalidTag { field, tag }),
    }
}

fn read_vec<T>(
    reader: &mut MaterialReader<'_>,
    field: &'static str,
    mut read_value: impl FnMut(&mut MaterialReader<'_>) -> Result<T, MaterialAssetCodecError>,
) -> Result<Vec<T>, MaterialAssetCodecError> {
    let len = reader.read_len(field)?;
    let mut values = Vec::with_capacity(len.min(1024));
    for _ in 0..len {
        values.push(read_value(reader)?);
    }
    Ok(values)
}

fn read_header(
    reader: &mut MaterialReader<'_>,
    expected_magic: [u8; 8],
    expected_version: u32,
) -> Result<(), MaterialAssetCodecError> {
    let magic = reader.read_array::<8>("magic")?;
    if magic != expected_magic {
        return Err(MaterialAssetCodecError::BadMagic { found: magic });
    }

    let version = reader.read_u32("version")?;
    if version != expected_version {
        return Err(MaterialAssetCodecError::UnsupportedVersion { version });
    }
    Ok(())
}

fn write_u8<W: Write + ?Sized>(writer: &mut W, value: u8) -> Result<(), MaterialAssetCodecError> {
    writer.write_all(&[value])?;
    Ok(())
}

fn write_u32<W: Write + ?Sized>(writer: &mut W, value: u32) -> Result<(), MaterialAssetCodecError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_f32<W: Write + ?Sized>(writer: &mut W, value: f32) -> Result<(), MaterialAssetCodecError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_string<W: Write + ?Sized>(
    writer: &mut W,
    field: &'static str,
    value: &str,
) -> Result<(), MaterialAssetCodecError> {
    write_len(writer, field, value.len())?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn write_len<W: Write + ?Sized>(
    writer: &mut W,
    field: &'static str,
    len: usize,
) -> Result<(), MaterialAssetCodecError> {
    let len = u32::try_from(len).map_err(|_| MaterialAssetCodecError::CountTooLarge {
        field,
        count: len as u64,
    })?;
    write_u32(writer, len)
}

struct MaterialReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> MaterialReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    const fn finish(&self) -> Result<(), MaterialAssetCodecError> {
        let remaining = self.bytes.len().saturating_sub(self.cursor);
        if remaining == 0 {
            Ok(())
        } else {
            Err(MaterialAssetCodecError::TrailingBytes { remaining })
        }
    }

    fn read_array<const N: usize>(
        &mut self,
        field: &'static str,
    ) -> Result<[u8; N], MaterialAssetCodecError> {
        let end = self
            .cursor
            .checked_add(N)
            .ok_or(MaterialAssetCodecError::UnexpectedEof { field })?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(MaterialAssetCodecError::UnexpectedEof { field })?;
        self.cursor = end;

        let mut out = [0; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    fn read_u8(&mut self, field: &'static str) -> Result<u8, MaterialAssetCodecError> {
        Ok(self.read_array::<1>(field)?[0])
    }

    fn read_u32(&mut self, field: &'static str) -> Result<u32, MaterialAssetCodecError> {
        Ok(u32::from_le_bytes(self.read_array::<4>(field)?))
    }

    fn read_f32(&mut self, field: &'static str) -> Result<f32, MaterialAssetCodecError> {
        Ok(f32::from_le_bytes(self.read_array::<4>(field)?))
    }

    fn read_len(&mut self, field: &'static str) -> Result<usize, MaterialAssetCodecError> {
        let count = self.read_u32(field)?;
        usize::try_from(count).map_err(|_| MaterialAssetCodecError::CountTooLarge {
            field,
            count: u64::from(count),
        })
    }

    fn read_string(&mut self, field: &'static str) -> Result<String, MaterialAssetCodecError> {
        let len = self.read_len(field)?;
        let end = self
            .cursor
            .checked_add(len)
            .ok_or(MaterialAssetCodecError::UnexpectedEof { field })?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(MaterialAssetCodecError::UnexpectedEof { field })?;
        self.cursor = end;
        let text = std::str::from_utf8(bytes)
            .map_err(|source| MaterialAssetCodecError::InvalidUtf8 { field, source })?;
        Ok(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_material_type() -> MaterialTypeAsset {
        MaterialTypeAsset {
            name: "Standard PBR".to_string(),
            description: "Base opaque surface".to_string(),
            domain: MaterialDomain::Surface,
            blend_mode: BlendMode::Opaque,
            cull_mode: CullMode::Back,
            shading_model: ShadingModel::Standard,
            shader_graph: "materials/graphs/standard.azmat.ron".to_string(),
            property_groups: vec![MaterialPropertyGroup {
                id: "base".to_string(),
                display_name: "Base".to_string(),
                description: "Base color inputs".to_string(),
                properties: vec![
                    MaterialPropertyDefinition {
                        id: "base.color".to_string(),
                        display_name: "Color".to_string(),
                        description: "Albedo tint".to_string(),
                        default_value: MaterialPropertyValue::Color(MaterialColor {
                            rgba: Vec4::new(1.0, 0.5, 0.25, 1.0),
                        }),
                        connection: Some("base_color".to_string()),
                        enum_values: Vec::new(),
                    },
                    MaterialPropertyDefinition {
                        id: "base.texture".to_string(),
                        display_name: "Texture".to_string(),
                        description: String::new(),
                        default_value: MaterialPropertyValue::Texture(MaterialTexture {
                            asset: "textures/default.png".parse().unwrap(),
                        }),
                        connection: None,
                        enum_values: vec!["a".to_string(), "b".to_string()],
                    },
                ],
            }],
        }
    }

    fn sample_material() -> MaterialAsset {
        MaterialAsset {
            name: "wood".to_string(),
            material_type: "materials/types/standard.azmaterialtype".to_string(),
            parent: Some("materials/base-wood.azmaterial".to_string()),
            property_values: vec![
                MaterialPropertyBinding {
                    property: "base.color".to_string(),
                    value: MaterialPropertyValue::Vec3(Vec3::new(0.6, 0.4, 0.2)),
                },
                MaterialPropertyBinding {
                    property: "base.roughness".to_string(),
                    value: MaterialPropertyValue::Float(0.8),
                },
                MaterialPropertyBinding {
                    property: "detail.tiling".to_string(),
                    value: MaterialPropertyValue::Vec2(Vec2::splat(4.0)),
                },
                MaterialPropertyBinding {
                    property: "detail.enabled".to_string(),
                    value: MaterialPropertyValue::Bool(true),
                },
                MaterialPropertyBinding {
                    property: "detail.layer".to_string(),
                    value: MaterialPropertyValue::Int(-2),
                },
                MaterialPropertyBinding {
                    property: "detail.mask".to_string(),
                    value: MaterialPropertyValue::UInt(7),
                },
                MaterialPropertyBinding {
                    property: "detail.mode".to_string(),
                    value: MaterialPropertyValue::String("overlay".to_string()),
                },
                MaterialPropertyBinding {
                    property: "detail.plane".to_string(),
                    value: MaterialPropertyValue::Vec4(Vec4::new(0.0, 1.0, 0.0, 0.5)),
                },
            ],
        }
    }

    #[test]
    fn material_type_round_trips() {
        let asset = sample_material_type();

        let bytes = encode_material_type_asset(&asset).unwrap();
        assert_eq!(decode_material_type_asset(&bytes).unwrap(), asset);
    }

    #[test]
    fn material_round_trips() {
        let asset = sample_material();

        let bytes = encode_material_asset(&asset).unwrap();
        assert_eq!(decode_material_asset(&bytes).unwrap(), asset);
    }

    #[test]
    fn material_type_rejects_bad_magic() {
        let mut bytes = encode_material_type_asset(&sample_material_type()).unwrap();
        bytes[0] = b'X';

        assert!(matches!(
            decode_material_type_asset(&bytes),
            Err(MaterialAssetCodecError::BadMagic { .. })
        ));
    }

    #[test]
    fn material_type_rejects_unsupported_version() {
        let mut bytes = encode_material_type_asset(&sample_material_type()).unwrap();
        bytes[8..12].copy_from_slice(&5u32.to_le_bytes());

        assert!(matches!(
            decode_material_type_asset(&bytes),
            Err(MaterialAssetCodecError::UnsupportedVersion { version: 5 })
        ));
    }

    #[test]
    fn material_rejects_bad_magic() {
        let mut bytes = encode_material_asset(&sample_material()).unwrap();
        bytes[0] = b'X';

        assert!(matches!(
            decode_material_asset(&bytes),
            Err(MaterialAssetCodecError::BadMagic { .. })
        ));
    }

    #[test]
    fn material_rejects_trailing_bytes() {
        let mut bytes = encode_material_asset(&sample_material()).unwrap();
        bytes.push(9);

        assert!(matches!(
            decode_material_asset(&bytes),
            Err(MaterialAssetCodecError::TrailingBytes { remaining: 1 })
        ));
    }

    #[test]
    fn material_rejects_truncation() {
        let bytes = encode_material_asset(&sample_material()).unwrap();

        assert!(matches!(
            decode_material_asset(&bytes[..bytes.len() - 2]),
            Err(MaterialAssetCodecError::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn material_rejects_unknown_property_value_tag() {
        let mut bytes = encode_material_asset(&MaterialAsset {
            name: "m".to_string(),
            material_type: "materials/types/t.azmaterialtype".to_string(),
            parent: None,
            property_values: vec![MaterialPropertyBinding {
                property: "p".to_string(),
                value: MaterialPropertyValue::Bool(false),
            }],
        })
        .unwrap();
        // The property value tag is the last two bytes (tag + bool payload).
        let tag_index = bytes.len() - 2;
        bytes[tag_index] = 200;

        assert!(matches!(
            decode_material_asset(&bytes),
            Err(MaterialAssetCodecError::InvalidTag { tag: 200, .. })
        ));
    }
}
