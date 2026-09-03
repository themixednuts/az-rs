//! Versioned little-endian codec for `AZMESH` products.

use std::io::{self, Write};

use thiserror::Error;

use crate::{MESH_MAGIC, MESH_PRODUCT_VERSION, MeshAsset, MeshMaterialSlot, MeshPrimitive};

#[derive(Debug, Error)]
pub enum MeshCodecError {
    #[error("mesh product ended while reading {field}")]
    UnexpectedEof { field: &'static str },
    #[error("bad mesh product magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported mesh product version {version}")]
    UnsupportedVersion { version: u32 },
    #[error("mesh product text field {field} is not UTF-8: {source}")]
    InvalidUtf8 {
        field: &'static str,
        source: std::str::Utf8Error,
    },
    #[error("mesh product count for {field} is too large: {count}")]
    CountTooLarge { field: &'static str, count: u64 },
    #[error(
        "mesh product primitive {primitive} has {actual} {attribute} values for {vertices} vertices"
    )]
    AttributeCount {
        primitive: usize,
        attribute: &'static str,
        actual: usize,
        vertices: usize,
    },
    #[error("mesh product has {remaining} trailing bytes")]
    TrailingBytes { remaining: usize },
    #[error("failed to write mesh product: {0}")]
    Io(#[from] io::Error),
}

/// Encode a processed mesh product.
///
/// # Errors
///
/// Returns [`MeshCodecError::AttributeCount`] if a primitive's `normals`,
/// `tangents` or `uv0` list is non-empty but does not match its position count,
/// [`MeshCodecError::CountTooLarge`] if a slot, primitive or attribute list is
/// longer than [`u32::MAX`], or [`MeshCodecError::Io`] if writing into the
/// output buffer fails.
pub fn encode_mesh_asset(asset: &MeshAsset) -> Result<Vec<u8>, MeshCodecError> {
    validate(asset)?;
    let mut bytes = Vec::new();
    bytes.write_all(MESH_MAGIC)?;
    write_u32(&mut bytes, MESH_PRODUCT_VERSION)?;
    write_string(&mut bytes, "name", &asset.name)?;
    write_vec3(&mut bytes, asset.bounds_min)?;
    write_vec3(&mut bytes, asset.bounds_max)?;
    write_len(&mut bytes, "material slots", asset.material_slots.len())?;
    for slot in &asset.material_slots {
        write_u32(&mut bytes, slot.id)?;
        write_string(&mut bytes, "slot label", &slot.label)?;
        write_option_string(
            &mut bytes,
            "slot default material",
            slot.default_material.as_deref(),
        )?;
    }
    write_len(&mut bytes, "primitives", asset.primitives.len())?;
    for primitive in &asset.primitives {
        write_string(&mut bytes, "primitive label", &primitive.label)?;
        write_u32(&mut bytes, primitive.material_slot)?;
        write_vec(
            &mut bytes,
            "positions",
            &primitive.positions,
            |writer, value| write_vec3(writer, *value),
        )?;
        write_vec(
            &mut bytes,
            "normals",
            &primitive.normals,
            |writer, value| write_vec3(writer, *value),
        )?;
        write_vec(
            &mut bytes,
            "tangents",
            &primitive.tangents,
            |writer, value| {
                for component in value {
                    write_f32(writer, *component)?;
                }
                Ok(())
            },
        )?;
        write_vec(&mut bytes, "uv0", &primitive.uv0, |writer, value| {
            write_f32(writer, value[0])?;
            write_f32(writer, value[1])
        })?;
        write_vec(
            &mut bytes,
            "indices",
            &primitive.indices,
            |writer, value| write_u32(writer, *value),
        )?;
    }
    Ok(bytes)
}

/// Decode and validate a processed mesh product.
///
/// # Errors
///
/// Returns [`MeshCodecError::BadMagic`] if `bytes` does not open with
/// [`MESH_MAGIC`], [`MeshCodecError::UnsupportedVersion`] if the version word is
/// not [`MESH_PRODUCT_VERSION`], [`MeshCodecError::UnexpectedEof`] if the stream
/// ends mid-field, [`MeshCodecError::InvalidUtf8`] if a text field is not UTF-8,
/// [`MeshCodecError::CountTooLarge`] if a length prefix exceeds `usize`,
/// [`MeshCodecError::TrailingBytes`] if bytes remain after the last primitive,
/// or [`MeshCodecError::AttributeCount`] if the decoded primitives fail the same
/// attribute-length check [`encode_mesh_asset`] applies.
pub fn decode_mesh_asset(bytes: &[u8]) -> Result<MeshAsset, MeshCodecError> {
    let mut reader = Reader::new(bytes);
    let magic = reader.array::<8>("magic")?;
    if &magic != MESH_MAGIC {
        return Err(MeshCodecError::BadMagic { found: magic });
    }
    let version = reader.u32("version")?;
    if version != MESH_PRODUCT_VERSION {
        return Err(MeshCodecError::UnsupportedVersion { version });
    }
    let name = reader.string("name")?;
    let bounds_min = reader.vec3("bounds min")?;
    let bounds_max = reader.vec3("bounds max")?;
    let material_slots = reader.vec("material slots", |reader| {
        Ok(MeshMaterialSlot {
            id: reader.u32("slot id")?,
            label: reader.string("slot label")?,
            default_material: reader.option_string("slot default material")?,
        })
    })?;
    let primitives = reader.vec("primitives", |reader| {
        Ok(MeshPrimitive {
            label: reader.string("primitive label")?,
            material_slot: reader.u32("primitive material slot")?,
            positions: reader.vec("positions", |reader| reader.vec3("position"))?,
            normals: reader.vec("normals", |reader| reader.vec3("normal"))?,
            tangents: reader.vec("tangents", |reader| {
                Ok([
                    reader.f32("tangent x")?,
                    reader.f32("tangent y")?,
                    reader.f32("tangent z")?,
                    reader.f32("tangent w")?,
                ])
            })?,
            uv0: reader.vec("uv0", |reader| {
                Ok([reader.f32("uv x")?, reader.f32("uv y")?])
            })?,
            indices: reader.vec("indices", |reader| reader.u32("index"))?,
        })
    })?;
    reader.finish()?;
    let asset = MeshAsset {
        name,
        bounds_min,
        bounds_max,
        material_slots,
        primitives,
    };
    validate(&asset)?;
    Ok(asset)
}

fn validate(asset: &MeshAsset) -> Result<(), MeshCodecError> {
    for (primitive_index, primitive) in asset.primitives.iter().enumerate() {
        let vertices = primitive.positions.len();
        for (attribute, actual) in [
            ("normals", primitive.normals.len()),
            ("tangents", primitive.tangents.len()),
            ("uv0", primitive.uv0.len()),
        ] {
            if actual != 0 && actual != vertices {
                return Err(MeshCodecError::AttributeCount {
                    primitive: primitive_index,
                    attribute,
                    actual,
                    vertices,
                });
            }
        }
    }
    Ok(())
}

fn write_vec<W: Write + ?Sized, T>(
    writer: &mut W,
    field: &'static str,
    values: &[T],
    mut write_value: impl FnMut(&mut W, &T) -> Result<(), MeshCodecError>,
) -> Result<(), MeshCodecError> {
    write_len(writer, field, values.len())?;
    for value in values {
        write_value(writer, value)?;
    }
    Ok(())
}

fn write_option_string<W: Write + ?Sized>(
    writer: &mut W,
    field: &'static str,
    value: Option<&str>,
) -> Result<(), MeshCodecError> {
    if let Some(value) = value {
        writer.write_all(&[1])?;
        write_string(writer, field, value)
    } else {
        writer.write_all(&[0])?;
        Ok(())
    }
}

fn write_vec3<W: Write + ?Sized>(writer: &mut W, value: [f32; 3]) -> Result<(), MeshCodecError> {
    for component in value {
        write_f32(writer, component)?;
    }
    Ok(())
}

fn write_f32<W: Write + ?Sized>(writer: &mut W, value: f32) -> Result<(), MeshCodecError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_u32<W: Write + ?Sized>(writer: &mut W, value: u32) -> Result<(), MeshCodecError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_string<W: Write + ?Sized>(
    writer: &mut W,
    field: &'static str,
    value: &str,
) -> Result<(), MeshCodecError> {
    write_len(writer, field, value.len())?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn write_len<W: Write + ?Sized>(
    writer: &mut W,
    field: &'static str,
    len: usize,
) -> Result<(), MeshCodecError> {
    let value = u32::try_from(len).map_err(|_| MeshCodecError::CountTooLarge {
        field,
        count: len as u64,
    })?;
    write_u32(writer, value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    const fn finish(&self) -> Result<(), MeshCodecError> {
        let remaining = self.bytes.len().saturating_sub(self.cursor);
        if remaining == 0 {
            Ok(())
        } else {
            Err(MeshCodecError::TrailingBytes { remaining })
        }
    }

    fn array<const N: usize>(&mut self, field: &'static str) -> Result<[u8; N], MeshCodecError> {
        let end = self
            .cursor
            .checked_add(N)
            .ok_or(MeshCodecError::UnexpectedEof { field })?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(MeshCodecError::UnexpectedEof { field })?;
        self.cursor = end;
        let mut value = [0; N];
        value.copy_from_slice(bytes);
        Ok(value)
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, MeshCodecError> {
        Ok(u32::from_le_bytes(self.array::<4>(field)?))
    }

    fn f32(&mut self, field: &'static str) -> Result<f32, MeshCodecError> {
        Ok(f32::from_le_bytes(self.array::<4>(field)?))
    }

    fn vec3(&mut self, field: &'static str) -> Result<[f32; 3], MeshCodecError> {
        Ok([self.f32(field)?, self.f32(field)?, self.f32(field)?])
    }

    fn len(&mut self, field: &'static str) -> Result<usize, MeshCodecError> {
        let count = self.u32(field)?;
        usize::try_from(count).map_err(|_| MeshCodecError::CountTooLarge {
            field,
            count: u64::from(count),
        })
    }

    fn string(&mut self, field: &'static str) -> Result<String, MeshCodecError> {
        let len = self.len(field)?;
        let end = self
            .cursor
            .checked_add(len)
            .ok_or(MeshCodecError::UnexpectedEof { field })?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(MeshCodecError::UnexpectedEof { field })?;
        self.cursor = end;
        Ok(std::str::from_utf8(bytes)
            .map_err(|source| MeshCodecError::InvalidUtf8 { field, source })?
            .to_owned())
    }

    fn option_string(&mut self, field: &'static str) -> Result<Option<String>, MeshCodecError> {
        match self.array::<1>(field)?[0] {
            0 => Ok(None),
            1 => self.string(field).map(Some),
            _ => Err(MeshCodecError::UnexpectedEof { field }),
        }
    }

    fn vec<T>(
        &mut self,
        field: &'static str,
        mut read_value: impl FnMut(&mut Self) -> Result<T, MeshCodecError>,
    ) -> Result<Vec<T>, MeshCodecError> {
        let len = self.len(field)?;
        let mut values = Vec::with_capacity(len.min(4096));
        for _ in 0..len {
            values.push(read_value(self)?);
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> MeshAsset {
        MeshAsset {
            name: "triangle".to_owned(),
            bounds_min: [-1.0, 0.0, 0.0],
            bounds_max: [1.0, 2.0, 0.0],
            material_slots: vec![MeshMaterialSlot {
                id: 7,
                label: "body".to_owned(),
                default_material: Some("materials/red.azmaterial".to_owned()),
            }],
            primitives: vec![MeshPrimitive {
                label: "triangle".to_owned(),
                material_slot: 7,
                positions: vec![[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                tangents: Vec::new(),
                uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
                indices: vec![0, 1, 2],
            }],
        }
    }

    #[test]
    fn mesh_product_round_trips() {
        let asset = fixture();
        let bytes = encode_mesh_asset(&asset).unwrap();
        assert_eq!(decode_mesh_asset(&bytes).unwrap(), asset);
    }

    #[test]
    fn mesh_product_rejects_bad_magic_and_trailing_bytes() {
        let mut bytes = encode_mesh_asset(&fixture()).unwrap();
        bytes[0] = b'X';
        assert!(matches!(
            decode_mesh_asset(&bytes),
            Err(MeshCodecError::BadMagic { .. })
        ));

        let mut bytes = encode_mesh_asset(&fixture()).unwrap();
        bytes.push(1);
        assert!(matches!(
            decode_mesh_asset(&bytes),
            Err(MeshCodecError::TrailingBytes { remaining: 1 })
        ));
    }
}
