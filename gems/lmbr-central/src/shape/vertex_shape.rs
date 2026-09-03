//! Vertex shape asset data.

use std::io::{Cursor, Read, Write};

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AsyncReadExt, LoadContext};
use bevy::math::bounding::Aabb3d;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::{Uuid, uuid};

use super::bounds::aabb_from_min_max;

/// `VertexShapeAsset` asset UUID.
pub const VERTEX_SHAPE_ASSET_TYPE_ID: Uuid = uuid!("EE5F2696-D2A4-4C91-8614-82352BB33D90");

/// `VertexShapeAssetHandler` UUID.
pub const VERTEX_SHAPE_ASSET_HANDLER_TYPE_ID: Uuid = uuid!("464A863C-0ABD-49B5-864E-39AE7E0E71D8");

/// Current vertex shape asset schema version.
pub const VERTEX_SHAPE_ASSET_VERSION: u32 = 2;

/// File extensions claimed by [`VertexShapeAssetLoader`].
///
/// Vertex shapes use the `.vshapec` extension.
pub const VERTEX_SHAPE_ASSET_EXTENSIONS: &[&str] = &["vshapec"];

const MAGIC: &[u8; 8] = b"AZVSHAP\0";

/// Engine vertex shape asset.
#[derive(Asset, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
pub struct VertexShapeAsset {
    pub version: u32,
    pub vertices: Vec<Vec3>,
    pub metadata: Vec<VertexShapeMetadata>,
    pub height: f32,
    pub reserved: VertexShapeReserved,
}

impl Default for VertexShapeAsset {
    fn default() -> Self {
        Self {
            version: VERTEX_SHAPE_ASSET_VERSION,
            vertices: Vec::new(),
            metadata: Vec::new(),
            height: 0.0,
            reserved: VertexShapeReserved::default(),
        }
    }
}

impl VertexShapeAsset {
    #[must_use]
    pub const fn new(
        version: u32,
        vertices: Vec<Vec3>,
        metadata: Vec<VertexShapeMetadata>,
        height: f32,
        reserved: VertexShapeReserved,
    ) -> Self {
        Self {
            version,
            vertices,
            metadata,
            height,
            reserved,
        }
    }

    #[must_use]
    pub const fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    #[must_use]
    pub fn local_bounds(&self) -> Option<Aabb3d> {
        let first = *self.vertices.first()?;
        let first_top = first + Vec3::Z * self.height;
        let (mut min, mut max) = (first.min(first_top), first.max(first_top));

        for vertex in &self.vertices[1..] {
            let top = *vertex + Vec3::Z * self.height;
            min = min.min(*vertex).min(top);
            max = max.max(*vertex).max(top);
        }

        Some(aabb_from_min_max(min, max))
    }
}

/// One vertex shape metadata entry.
#[derive(Debug, Default, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct VertexShapeMetadata {
    pub key: String,
    pub value: String,
}

impl VertexShapeMetadata {
    #[must_use]
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// Reserved values stored with a vertex shape asset.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Reflect, Serialize, Deserialize)]
pub struct VertexShapeReserved {
    pub first: u32,
    pub second: u32,
    pub third: u32,
}

impl VertexShapeReserved {
    #[must_use]
    pub const fn new(first: u32, second: u32, third: u32) -> Self {
        Self {
            first,
            second,
            third,
        }
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.first == 0 && self.second == 0 && self.third == 0
    }
}

/// Write an engine vertex shape asset.
///
/// # Errors
///
/// [`VertexShapeAssetFormatError::Io`] if `writer` rejects a write, or
/// [`VertexShapeAssetFormatError::TooManyItems`] if the asset holds more
/// vertices or metadata entries than the `u32` count the format encodes.
pub fn write_vertex_shape_asset(
    asset: &VertexShapeAsset,
    writer: impl Write,
) -> Result<(), VertexShapeAssetFormatError> {
    format::write_vertex_shape_asset(asset, writer)
}

/// Read an engine vertex shape asset.
///
/// # Errors
///
/// [`VertexShapeAssetFormatError::BadMagic`] if `bytes` does not open with the
/// vertex-shape magic, [`VertexShapeAssetFormatError::UnsupportedVersion`] for
/// a version this build cannot decode,
/// [`VertexShapeAssetFormatError::InvalidData`] for a metadata string that is not valid UTF-8, or
/// [`VertexShapeAssetFormatError::Io`] if `bytes` ends mid-record.
pub fn read_vertex_shape_asset(
    bytes: &[u8],
) -> Result<VertexShapeAsset, VertexShapeAssetFormatError> {
    format::read_vertex_shape_asset(bytes)
}

/// Read an engine vertex shape asset from a stream.
///
/// # Errors
///
/// Returns the same errors as [`read_vertex_shape_asset`], with
/// [`VertexShapeAssetFormatError::Io`] additionally covering a `reader`
/// failure.
pub fn read_vertex_shape_asset_from_reader(
    reader: impl Read,
) -> Result<VertexShapeAsset, VertexShapeAssetFormatError> {
    format::read_vertex_shape_asset_from_reader(reader)
}

/// Bevy asset loader for engine vertex shape assets.
#[derive(Default, TypePath)]
pub struct VertexShapeAssetLoader;

impl AssetLoader for VertexShapeAssetLoader {
    type Asset = VertexShapeAsset;
    type Settings = ();
    type Error = VertexShapeAssetFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        format::read_vertex_shape_asset_from_bevy_reader(reader).await
    }

    fn extensions(&self) -> &[&str] {
        VERTEX_SHAPE_ASSET_EXTENSIONS
    }
}

/// Engine vertex shape asset format errors.
#[derive(Debug, Error)]
pub enum VertexShapeAssetFormatError {
    #[error("bad vertex shape asset magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported vertex shape asset version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("too many vertex shape {what}: {count}")]
    TooManyItems { what: &'static str, count: usize },
    #[error("invalid vertex shape asset data: {0}")]
    InvalidData(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

mod format {
    use super::{
        AsyncReadExt, Cursor, MAGIC, Read, Reader, Result, String, VERTEX_SHAPE_ASSET_VERSION, Vec,
        Vec3, VertexShapeAsset, VertexShapeAssetFormatError, VertexShapeMetadata,
        VertexShapeReserved, Write, vec,
    };

    pub(super) fn write_vertex_shape_asset(
        asset: &VertexShapeAsset,
        mut writer: impl Write,
    ) -> Result<(), VertexShapeAssetFormatError> {
        writer.write_all(MAGIC)?;
        write_u32(&mut writer, VERTEX_SHAPE_ASSET_VERSION)?;
        write_u32(&mut writer, asset.version)?;
        write_vec3s(&mut writer, &asset.vertices)?;
        write_metadata(&mut writer, &asset.metadata)?;
        write_f32(&mut writer, asset.height)?;
        write_u32(&mut writer, asset.reserved.first)?;
        write_u32(&mut writer, asset.reserved.second)?;
        write_u32(&mut writer, asset.reserved.third)?;
        Ok(())
    }

    pub(super) fn read_vertex_shape_asset(
        bytes: &[u8],
    ) -> Result<VertexShapeAsset, VertexShapeAssetFormatError> {
        read_vertex_shape_asset_from_reader(Cursor::new(bytes))
    }

    pub(super) fn read_vertex_shape_asset_from_reader(
        mut reader: impl Read,
    ) -> Result<VertexShapeAsset, VertexShapeAssetFormatError> {
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(VertexShapeAssetFormatError::BadMagic { found: magic });
        }
        let schema_version = read_u32(&mut reader)?;
        if schema_version != VERTEX_SHAPE_ASSET_VERSION {
            return Err(VertexShapeAssetFormatError::UnsupportedVersion {
                version: schema_version,
                expected: VERTEX_SHAPE_ASSET_VERSION,
            });
        }
        Ok(VertexShapeAsset::new(
            read_u32(&mut reader)?,
            read_vec3s(&mut reader)?,
            read_metadata(&mut reader)?,
            read_f32(&mut reader)?,
            VertexShapeReserved::new(
                read_u32(&mut reader)?,
                read_u32(&mut reader)?,
                read_u32(&mut reader)?,
            ),
        ))
    }

    pub(super) async fn read_vertex_shape_asset_from_bevy_reader(
        reader: &mut dyn Reader,
    ) -> Result<VertexShapeAsset, VertexShapeAssetFormatError> {
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic).await?;
        if &magic != MAGIC {
            return Err(VertexShapeAssetFormatError::BadMagic { found: magic });
        }
        let schema_version = read_async_u32(reader).await?;
        if schema_version != VERTEX_SHAPE_ASSET_VERSION {
            return Err(VertexShapeAssetFormatError::UnsupportedVersion {
                version: schema_version,
                expected: VERTEX_SHAPE_ASSET_VERSION,
            });
        }
        Ok(VertexShapeAsset::new(
            read_async_u32(reader).await?,
            read_async_vec3s(reader).await?,
            read_async_metadata(reader).await?,
            read_async_f32(reader).await?,
            VertexShapeReserved::new(
                read_async_u32(reader).await?,
                read_async_u32(reader).await?,
                read_async_u32(reader).await?,
            ),
        ))
    }

    fn write_vec3s(
        writer: &mut impl Write,
        values: &[Vec3],
    ) -> Result<(), VertexShapeAssetFormatError> {
        write_u32(writer, checked_u32(values.len(), "vertices")?)?;
        for value in values {
            write_f32(writer, value.x)?;
            write_f32(writer, value.y)?;
            write_f32(writer, value.z)?;
        }
        Ok(())
    }

    fn read_vec3s(reader: &mut impl Read) -> Result<Vec<Vec3>, VertexShapeAssetFormatError> {
        let count = read_u32(reader)? as usize;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(Vec3::new(
                read_f32(reader)?,
                read_f32(reader)?,
                read_f32(reader)?,
            ));
        }
        Ok(values)
    }

    async fn read_async_vec3s(
        reader: &mut dyn Reader,
    ) -> Result<Vec<Vec3>, VertexShapeAssetFormatError> {
        let count = read_async_u32(reader).await? as usize;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(Vec3::new(
                read_async_f32(reader).await?,
                read_async_f32(reader).await?,
                read_async_f32(reader).await?,
            ));
        }
        Ok(values)
    }

    fn write_metadata(
        writer: &mut impl Write,
        values: &[VertexShapeMetadata],
    ) -> Result<(), VertexShapeAssetFormatError> {
        write_u32(writer, checked_u32(values.len(), "metadata entries")?)?;
        for value in values {
            write_string(writer, &value.key)?;
            write_string(writer, &value.value)?;
        }
        Ok(())
    }

    fn read_metadata(
        reader: &mut impl Read,
    ) -> Result<Vec<VertexShapeMetadata>, VertexShapeAssetFormatError> {
        let count = read_u32(reader)? as usize;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(VertexShapeMetadata::new(
                read_string(reader)?,
                read_string(reader)?,
            ));
        }
        Ok(values)
    }

    async fn read_async_metadata(
        reader: &mut dyn Reader,
    ) -> Result<Vec<VertexShapeMetadata>, VertexShapeAssetFormatError> {
        let count = read_async_u32(reader).await? as usize;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(VertexShapeMetadata::new(
                read_async_string(reader).await?,
                read_async_string(reader).await?,
            ));
        }
        Ok(values)
    }

    fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), std::io::Error> {
        writer.write_all(&value.to_le_bytes())
    }

    fn write_string(
        writer: &mut impl Write,
        value: &str,
    ) -> Result<(), VertexShapeAssetFormatError> {
        write_u32(writer, checked_u32(value.len(), "metadata string bytes")?)?;
        writer.write_all(value.as_bytes())?;
        Ok(())
    }

    fn read_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
        let mut bytes = [0u8; 4];
        reader.read_exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_string(reader: &mut impl Read) -> Result<String, VertexShapeAssetFormatError> {
        let len = read_u32(reader)? as usize;
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes)?;
        String::from_utf8(bytes).map_err(|_| {
            VertexShapeAssetFormatError::InvalidData("metadata string is not valid UTF-8")
        })
    }

    async fn read_async_u32(reader: &mut dyn Reader) -> Result<u32, std::io::Error> {
        let mut bytes = [0u8; 4];
        reader.read_exact(&mut bytes).await?;
        Ok(u32::from_le_bytes(bytes))
    }

    async fn read_async_string(
        reader: &mut dyn Reader,
    ) -> Result<String, VertexShapeAssetFormatError> {
        let len = read_async_u32(reader).await? as usize;
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes).await?;
        String::from_utf8(bytes).map_err(|_| {
            VertexShapeAssetFormatError::InvalidData("metadata string is not valid UTF-8")
        })
    }

    fn write_f32(writer: &mut impl Write, value: f32) -> Result<(), std::io::Error> {
        writer.write_all(&value.to_le_bytes())
    }

    fn read_f32(reader: &mut impl Read) -> Result<f32, std::io::Error> {
        let mut bytes = [0u8; 4];
        reader.read_exact(&mut bytes)?;
        Ok(f32::from_le_bytes(bytes))
    }

    async fn read_async_f32(reader: &mut dyn Reader) -> Result<f32, std::io::Error> {
        let mut bytes = [0u8; 4];
        reader.read_exact(&mut bytes).await?;
        Ok(f32::from_le_bytes(bytes))
    }

    fn checked_u32(count: usize, what: &'static str) -> Result<u32, VertexShapeAssetFormatError> {
        u32::try_from(count).map_err(|_| VertexShapeAssetFormatError::TooManyItems { what, count })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_shape_bounds_extrude_by_height() {
        let asset = VertexShapeAsset::new(
            VERTEX_SHAPE_ASSET_VERSION,
            vec![Vec3::new(-1.0, 2.0, 0.0), Vec3::new(3.0, -4.0, 1.0)],
            Vec::new(),
            10.0,
            VertexShapeReserved::default(),
        );

        let bounds = asset.local_bounds().unwrap();
        assert_eq!(bounds.min, Vec3::new(-1.0, -4.0, 0.0).into());
        assert_eq!(bounds.max, Vec3::new(3.0, 2.0, 11.0).into());
    }

    #[test]
    fn vertex_shape_asset_round_trips_binary_format() {
        let asset = VertexShapeAsset::new(
            VERTEX_SHAPE_ASSET_VERSION,
            vec![Vec3::new(1.0, 2.0, 0.0), Vec3::new(3.0, 4.0, 0.0)],
            vec![VertexShapeMetadata::new("RegionId", "14:@Example")],
            128.0,
            VertexShapeReserved::new(1, 2, 3),
        );

        let mut bytes = Vec::new();
        write_vertex_shape_asset(&asset, &mut bytes).unwrap();
        let decoded = read_vertex_shape_asset(&bytes).unwrap();

        assert_eq!(decoded, asset);
    }
}
