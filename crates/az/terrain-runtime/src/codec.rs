use std::io::{self, Write};

use glam::Vec2;
use thiserror::Error;

use crate::{
    SurfaceTag, TERRAIN_HEIGHTMAP_MAGIC, TERRAIN_HEIGHTMAP_PRODUCT_VERSION,
    TERRAIN_LAYER_SET_MAGIC, TERRAIN_LAYER_SET_PRODUCT_VERSION, TERRAIN_REGION_MAGIC,
    TERRAIN_REGION_PRODUCT_VERSION, TERRAIN_WORLD_MAGIC, TERRAIN_WORLD_PRODUCT_VERSION,
    TerrainBounds, TerrainConstantHeightSource, TerrainCoord, TerrainHeightGraphSource,
    TerrainHeightImageSource, TerrainHeightRange, TerrainHeightSource, TerrainHeightTilesSource,
    TerrainHeightmapAsset, TerrainImageChannel, TerrainLayer, TerrainLayerSetAsset,
    TerrainRegionAsset, TerrainRegionRef, TerrainResolution, TerrainSurfaceChannel,
    TerrainSurfaceGraphSource, TerrainSurfaceImageSource, TerrainSurfaceSource,
    TerrainSurfaceWeightsSource, TerrainWorldAsset,
};

#[derive(Debug, Error)]
pub enum TerrainAssetCodecError {
    #[error("terrain product ended while reading {field}")]
    UnexpectedEof { field: &'static str },
    #[error("bad terrain product magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported terrain product version {version}")]
    UnsupportedVersion { version: u32 },
    #[error("terrain product text field {field} is not UTF-8: {source}")]
    InvalidUtf8 {
        field: &'static str,
        source: std::str::Utf8Error,
    },
    #[error("unknown terrain product tag {tag} while reading {field}")]
    InvalidTag { field: &'static str, tag: u8 },
    #[error("terrain product count for {field} is too large: {count}")]
    CountTooLarge { field: &'static str, count: u64 },
    #[error(
        "terrain heightmap sample count mismatch for {width} x {height}: expected {expected}, found {found}"
    )]
    HeightmapSampleCountMismatch {
        width: u32,
        height: u32,
        expected: usize,
        found: usize,
    },
    #[error("terrain product has {remaining} trailing bytes")]
    TrailingBytes { remaining: usize },
    #[error("failed to write terrain product: {0}")]
    Io(#[from] io::Error),
}

/// Encode a terrain world product into bytes.
///
/// # Errors
///
/// Returns an error if a field cannot be written or a collection/string length
/// exceeds the product format's `u32` length limit.
pub fn encode_terrain_world_asset(
    asset: &TerrainWorldAsset,
) -> Result<Vec<u8>, TerrainAssetCodecError> {
    let mut bytes = Vec::new();
    write_terrain_world_asset(asset, &mut bytes)?;
    Ok(bytes)
}

/// Encode a terrain region product into bytes.
///
/// # Errors
///
/// Returns an error if a field cannot be written or a collection/string length
/// exceeds the product format's `u32` length limit.
pub fn encode_terrain_region_asset(
    asset: &TerrainRegionAsset,
) -> Result<Vec<u8>, TerrainAssetCodecError> {
    let mut bytes = Vec::new();
    write_terrain_region_asset(asset, &mut bytes)?;
    Ok(bytes)
}

/// Encode a terrain layer-set product into bytes.
///
/// # Errors
///
/// Returns an error if a field cannot be written or a collection/string length
/// exceeds the product format's `u32` length limit.
pub fn encode_terrain_layer_set_asset(
    asset: &TerrainLayerSetAsset,
) -> Result<Vec<u8>, TerrainAssetCodecError> {
    let mut bytes = Vec::new();
    write_terrain_layer_set_asset(asset, &mut bytes)?;
    Ok(bytes)
}

/// Encode a terrain heightmap product into bytes.
///
/// # Errors
///
/// Returns an error if a field cannot be written, a collection/string length
/// exceeds the product format's `u32` length limit, or the sample count does
/// not match `width * height`.
pub fn encode_terrain_heightmap_asset(
    asset: &TerrainHeightmapAsset,
) -> Result<Vec<u8>, TerrainAssetCodecError> {
    let mut bytes = Vec::new();
    write_terrain_heightmap_asset(asset, &mut bytes)?;
    Ok(bytes)
}

/// Write a terrain world product.
///
/// # Errors
///
/// Returns an error if the writer fails or a collection/string length exceeds
/// the product format's `u32` length limit.
pub fn write_terrain_world_asset(
    asset: &TerrainWorldAsset,
    writer: &mut impl Write,
) -> Result<(), TerrainAssetCodecError> {
    writer.write_all(TERRAIN_WORLD_MAGIC)?;
    write_u32(writer, TERRAIN_WORLD_PRODUCT_VERSION)?;
    write_string(writer, "name", &asset.name)?;
    write_bounds(writer, &asset.bounds)?;
    write_height_range(writer, asset.height_range)?;
    write_resolution(writer, asset.resolution)?;
    write_string(writer, "layers", &asset.layers)?;
    write_len(writer, "regions", asset.regions.len())?;
    for region in &asset.regions {
        write_region_ref(writer, region)?;
    }
    Ok(())
}

/// Write a terrain region product.
///
/// # Errors
///
/// Returns an error if the writer fails or a collection/string length exceeds
/// the product format's `u32` length limit.
pub fn write_terrain_region_asset(
    asset: &TerrainRegionAsset,
    writer: &mut impl Write,
) -> Result<(), TerrainAssetCodecError> {
    writer.write_all(TERRAIN_REGION_MAGIC)?;
    write_u32(writer, TERRAIN_REGION_PRODUCT_VERSION)?;
    write_string(writer, "name", &asset.name)?;
    write_height_source(writer, &asset.height)?;
    write_option(writer, asset.surface.as_ref(), write_surface_source)?;
    write_option(writer, asset.water.as_deref(), |writer, value| {
        write_string(writer, "water", value)
    })?;
    write_option(writer, asset.layers.as_deref(), |writer, value| {
        write_string(writer, "layers", value)
    })?;
    Ok(())
}

/// Write a terrain layer-set product.
///
/// # Errors
///
/// Returns an error if the writer fails or a collection/string length exceeds
/// the product format's `u32` length limit.
pub fn write_terrain_layer_set_asset(
    asset: &TerrainLayerSetAsset,
    writer: &mut impl Write,
) -> Result<(), TerrainAssetCodecError> {
    writer.write_all(TERRAIN_LAYER_SET_MAGIC)?;
    write_u32(writer, TERRAIN_LAYER_SET_PRODUCT_VERSION)?;
    write_string(writer, "name", &asset.name)?;
    write_len(writer, "layers", asset.layers.len())?;
    for layer in &asset.layers {
        write_layer(writer, layer)?;
    }
    Ok(())
}

/// Write a terrain heightmap product.
///
/// # Errors
///
/// Returns an error if the writer fails, a collection/string length exceeds
/// the product format's `u32` length limit, or the sample count does not match
/// `width * height`.
pub fn write_terrain_heightmap_asset(
    asset: &TerrainHeightmapAsset,
    writer: &mut impl Write,
) -> Result<(), TerrainAssetCodecError> {
    validate_heightmap_sample_count(asset.width, asset.height, asset.samples.len())?;

    writer.write_all(TERRAIN_HEIGHTMAP_MAGIC)?;
    write_u32(writer, TERRAIN_HEIGHTMAP_PRODUCT_VERSION)?;
    write_string(writer, "name", &asset.name)?;
    write_u32(writer, asset.width)?;
    write_u32(writer, asset.height)?;
    write_len(writer, "height samples", asset.samples.len())?;
    for sample in &asset.samples {
        write_u16(writer, *sample)?;
    }
    Ok(())
}

/// Decode a terrain world product.
///
/// # Errors
///
/// Returns an error if the byte stream is truncated, has a bad magic/version,
/// contains unknown tags, invalid UTF-8, or trailing bytes.
pub fn decode_terrain_world_asset(
    bytes: &[u8],
) -> Result<TerrainWorldAsset, TerrainAssetCodecError> {
    let mut reader = TerrainReader::new(bytes);
    read_header(
        &mut reader,
        *TERRAIN_WORLD_MAGIC,
        TERRAIN_WORLD_PRODUCT_VERSION,
    )?;
    let asset = TerrainWorldAsset {
        name: reader.read_string("name")?,
        bounds: read_bounds(&mut reader)?,
        height_range: read_height_range(&mut reader)?,
        resolution: read_resolution(&mut reader)?,
        layers: reader.read_string("layers")?,
        regions: read_vec(&mut reader, "regions", read_region_ref)?,
    };
    reader.finish()?;
    Ok(asset)
}

/// Decode a terrain region product.
///
/// # Errors
///
/// Returns an error if the byte stream is truncated, has a bad magic/version,
/// contains unknown tags, invalid UTF-8, or trailing bytes.
pub fn decode_terrain_region_asset(
    bytes: &[u8],
) -> Result<TerrainRegionAsset, TerrainAssetCodecError> {
    let mut reader = TerrainReader::new(bytes);
    read_header(
        &mut reader,
        *TERRAIN_REGION_MAGIC,
        TERRAIN_REGION_PRODUCT_VERSION,
    )?;
    let asset = TerrainRegionAsset {
        name: reader.read_string("name")?,
        height: read_height_source(&mut reader)?,
        surface: read_option(&mut reader, "surface", read_surface_source)?,
        water: read_option(&mut reader, "water", |reader| reader.read_string("water"))?,
        layers: read_option(&mut reader, "layers", |reader| reader.read_string("layers"))?,
    };
    reader.finish()?;
    Ok(asset)
}

/// Decode a terrain layer-set product.
///
/// # Errors
///
/// Returns an error if the byte stream is truncated, has a bad magic/version,
/// contains unknown tags, invalid UTF-8, or trailing bytes.
pub fn decode_terrain_layer_set_asset(
    bytes: &[u8],
) -> Result<TerrainLayerSetAsset, TerrainAssetCodecError> {
    let mut reader = TerrainReader::new(bytes);
    read_header(
        &mut reader,
        *TERRAIN_LAYER_SET_MAGIC,
        TERRAIN_LAYER_SET_PRODUCT_VERSION,
    )?;
    let asset = TerrainLayerSetAsset {
        name: reader.read_string("name")?,
        layers: read_vec(&mut reader, "layers", read_layer)?,
    };
    reader.finish()?;
    Ok(asset)
}

/// Decode a terrain heightmap product.
///
/// # Errors
///
/// Returns an error if the byte stream is truncated, has a bad magic/version,
/// invalid UTF-8, an invalid sample count, or trailing bytes.
pub fn decode_terrain_heightmap_asset(
    bytes: &[u8],
) -> Result<TerrainHeightmapAsset, TerrainAssetCodecError> {
    let mut reader = TerrainReader::new(bytes);
    read_header(
        &mut reader,
        *TERRAIN_HEIGHTMAP_MAGIC,
        TERRAIN_HEIGHTMAP_PRODUCT_VERSION,
    )?;
    let asset = TerrainHeightmapAsset {
        name: reader.read_string("name")?,
        width: reader.read_u32("heightmap width")?,
        height: reader.read_u32("heightmap height")?,
        samples: read_vec(&mut reader, "height samples", |reader| {
            reader.read_u16("height sample")
        })?,
    };
    validate_heightmap_sample_count(asset.width, asset.height, asset.samples.len())?;
    reader.finish()?;
    Ok(asset)
}

fn write_region_ref<W: Write + ?Sized>(
    writer: &mut W,
    region: &TerrainRegionRef,
) -> Result<(), TerrainAssetCodecError> {
    write_string(writer, "region asset", &region.asset)?;
    write_option(writer, region.coord, write_coord)?;
    write_bounds(writer, &region.bounds)?;
    write_i32(writer, region.priority)?;
    Ok(())
}

fn read_region_ref(
    reader: &mut TerrainReader<'_>,
) -> Result<TerrainRegionRef, TerrainAssetCodecError> {
    Ok(TerrainRegionRef {
        asset: reader.read_string("region asset")?,
        coord: read_option(reader, "region coord", read_coord)?,
        bounds: read_bounds(reader)?,
        priority: reader.read_i32("region priority")?,
    })
}

fn write_layer<W: Write + ?Sized>(
    writer: &mut W,
    layer: &TerrainLayer,
) -> Result<(), TerrainAssetCodecError> {
    write_surface_tag(writer, &layer.tag)?;
    write_i32(writer, layer.priority)?;
    write_option(writer, layer.material.as_deref(), |writer, value| {
        write_string(writer, "material", value)
    })?;
    write_option(
        writer,
        layer.physics_material.as_deref(),
        |writer, value| write_string(writer, "physics material", value),
    )?;
    write_f32(writer, layer.texture_scale)?;
    Ok(())
}

fn read_layer(reader: &mut TerrainReader<'_>) -> Result<TerrainLayer, TerrainAssetCodecError> {
    Ok(TerrainLayer {
        tag: read_surface_tag(reader)?,
        priority: reader.read_i32("layer priority")?,
        material: read_option(reader, "material", |reader| reader.read_string("material"))?,
        physics_material: read_option(reader, "physics material", |reader| {
            reader.read_string("physics material")
        })?,
        texture_scale: reader.read_f32("texture scale")?,
    })
}

fn write_height_source<W: Write + ?Sized>(
    writer: &mut W,
    source: &TerrainHeightSource,
) -> Result<(), TerrainAssetCodecError> {
    match source {
        TerrainHeightSource::Image(source) => {
            write_u8(writer, 0)?;
            write_string(writer, "height image", &source.image)?;
            write_image_channel(writer, source.channel)?;
            write_u32(writer, source.mip)?;
            write_vec2(writer, source.tiling)?;
        }
        TerrainHeightSource::Tiled(source) => {
            write_u8(writer, 1)?;
            write_string(writer, "height asset", &source.asset)?;
        }
        TerrainHeightSource::Graph(source) => {
            write_u8(writer, 2)?;
            write_string(writer, "height graph", &source.graph)?;
        }
        TerrainHeightSource::Constant(source) => {
            write_u8(writer, 3)?;
            write_f32(writer, source.value)?;
        }
    }
    Ok(())
}

fn read_height_source(
    reader: &mut TerrainReader<'_>,
) -> Result<TerrainHeightSource, TerrainAssetCodecError> {
    let tag = reader.read_u8("height source tag")?;
    match tag {
        0 => Ok(TerrainHeightSource::Image(TerrainHeightImageSource {
            image: reader.read_string("height image")?,
            channel: read_image_channel(reader)?,
            mip: reader.read_u32("height image mip")?,
            tiling: read_vec2(reader)?,
        })),
        1 => Ok(TerrainHeightSource::Tiled(TerrainHeightTilesSource {
            asset: reader.read_string("height asset")?,
        })),
        2 => Ok(TerrainHeightSource::Graph(TerrainHeightGraphSource {
            graph: reader.read_string("height graph")?,
        })),
        3 => Ok(TerrainHeightSource::Constant(TerrainConstantHeightSource {
            value: reader.read_f32("constant height")?,
        })),
        tag => Err(TerrainAssetCodecError::InvalidTag {
            field: "height source",
            tag,
        }),
    }
}

fn write_surface_source<W: Write + ?Sized>(
    writer: &mut W,
    source: &TerrainSurfaceSource,
) -> Result<(), TerrainAssetCodecError> {
    match source {
        TerrainSurfaceSource::Image(source) => {
            write_u8(writer, 0)?;
            write_string(writer, "surface image", &source.image)?;
            write_u32(writer, source.mip)?;
            write_vec2(writer, source.tiling)?;
            write_len(writer, "surface channels", source.channels.len())?;
            for channel in &source.channels {
                write_surface_channel(writer, channel)?;
            }
        }
        TerrainSurfaceSource::Weights(source) => {
            write_u8(writer, 1)?;
            write_string(writer, "surface weights", &source.asset)?;
        }
        TerrainSurfaceSource::Graph(source) => {
            write_u8(writer, 2)?;
            write_string(writer, "surface graph", &source.graph)?;
        }
    }
    Ok(())
}

fn read_surface_source(
    reader: &mut TerrainReader<'_>,
) -> Result<TerrainSurfaceSource, TerrainAssetCodecError> {
    let tag = reader.read_u8("surface source tag")?;
    match tag {
        0 => Ok(TerrainSurfaceSource::Image(TerrainSurfaceImageSource {
            image: reader.read_string("surface image")?,
            mip: reader.read_u32("surface image mip")?,
            tiling: read_vec2(reader)?,
            channels: read_vec(reader, "surface channels", read_surface_channel)?,
        })),
        1 => Ok(TerrainSurfaceSource::Weights(TerrainSurfaceWeightsSource {
            asset: reader.read_string("surface weights")?,
        })),
        2 => Ok(TerrainSurfaceSource::Graph(TerrainSurfaceGraphSource {
            graph: reader.read_string("surface graph")?,
        })),
        tag => Err(TerrainAssetCodecError::InvalidTag {
            field: "surface source",
            tag,
        }),
    }
}

fn write_surface_channel<W: Write + ?Sized>(
    writer: &mut W,
    channel: &TerrainSurfaceChannel,
) -> Result<(), TerrainAssetCodecError> {
    write_image_channel(writer, channel.channel)?;
    write_surface_tag(writer, &channel.tag)
}

fn read_surface_channel(
    reader: &mut TerrainReader<'_>,
) -> Result<TerrainSurfaceChannel, TerrainAssetCodecError> {
    Ok(TerrainSurfaceChannel {
        channel: read_image_channel(reader)?,
        tag: read_surface_tag(reader)?,
    })
}

fn write_image_channel<W: Write + ?Sized>(
    writer: &mut W,
    channel: TerrainImageChannel,
) -> Result<(), TerrainAssetCodecError> {
    let tag = match channel {
        TerrainImageChannel::Red => 0,
        TerrainImageChannel::Green => 1,
        TerrainImageChannel::Blue => 2,
        TerrainImageChannel::Alpha => 3,
        TerrainImageChannel::Luminance => 4,
    };
    write_u8(writer, tag)
}

fn read_image_channel(
    reader: &mut TerrainReader<'_>,
) -> Result<TerrainImageChannel, TerrainAssetCodecError> {
    let tag = reader.read_u8("image channel tag")?;
    match tag {
        0 => Ok(TerrainImageChannel::Red),
        1 => Ok(TerrainImageChannel::Green),
        2 => Ok(TerrainImageChannel::Blue),
        3 => Ok(TerrainImageChannel::Alpha),
        4 => Ok(TerrainImageChannel::Luminance),
        tag => Err(TerrainAssetCodecError::InvalidTag {
            field: "image channel",
            tag,
        }),
    }
}

fn write_coord<W: Write + ?Sized>(
    writer: &mut W,
    coord: TerrainCoord,
) -> Result<(), TerrainAssetCodecError> {
    write_i32(writer, coord.x)?;
    write_i32(writer, coord.y)
}

fn read_coord(reader: &mut TerrainReader<'_>) -> Result<TerrainCoord, TerrainAssetCodecError> {
    Ok(TerrainCoord {
        x: reader.read_i32("coord x")?,
        y: reader.read_i32("coord y")?,
    })
}

fn write_bounds<W: Write + ?Sized>(
    writer: &mut W,
    bounds: &TerrainBounds,
) -> Result<(), TerrainAssetCodecError> {
    write_vec2(writer, bounds.min)?;
    write_vec2(writer, bounds.max)
}

fn read_bounds(reader: &mut TerrainReader<'_>) -> Result<TerrainBounds, TerrainAssetCodecError> {
    Ok(TerrainBounds {
        min: read_vec2(reader)?,
        max: read_vec2(reader)?,
    })
}

fn write_height_range<W: Write + ?Sized>(
    writer: &mut W,
    range: TerrainHeightRange,
) -> Result<(), TerrainAssetCodecError> {
    write_f32(writer, range.min)?;
    write_f32(writer, range.max)
}

fn read_height_range(
    reader: &mut TerrainReader<'_>,
) -> Result<TerrainHeightRange, TerrainAssetCodecError> {
    Ok(TerrainHeightRange {
        min: reader.read_f32("height range min")?,
        max: reader.read_f32("height range max")?,
    })
}

fn write_resolution<W: Write + ?Sized>(
    writer: &mut W,
    resolution: TerrainResolution,
) -> Result<(), TerrainAssetCodecError> {
    write_f32(writer, resolution.height_spacing)?;
    write_f32(writer, resolution.surface_spacing)
}

fn read_resolution(
    reader: &mut TerrainReader<'_>,
) -> Result<TerrainResolution, TerrainAssetCodecError> {
    Ok(TerrainResolution {
        height_spacing: reader.read_f32("height spacing")?,
        surface_spacing: reader.read_f32("surface spacing")?,
    })
}

fn write_vec2<W: Write + ?Sized>(
    writer: &mut W,
    value: Vec2,
) -> Result<(), TerrainAssetCodecError> {
    write_f32(writer, value.x)?;
    write_f32(writer, value.y)
}

fn read_vec2(reader: &mut TerrainReader<'_>) -> Result<Vec2, TerrainAssetCodecError> {
    Ok(Vec2::new(
        reader.read_f32("vec2 x")?,
        reader.read_f32("vec2 y")?,
    ))
}

fn write_surface_tag<W: Write + ?Sized>(
    writer: &mut W,
    tag: &SurfaceTag,
) -> Result<(), TerrainAssetCodecError> {
    write_string(writer, "surface tag", &tag.name)
}

fn read_surface_tag(reader: &mut TerrainReader<'_>) -> Result<SurfaceTag, TerrainAssetCodecError> {
    Ok(SurfaceTag {
        name: reader.read_string("surface tag")?,
    })
}

fn write_option<W, T>(
    writer: &mut W,
    value: Option<T>,
    write_value: impl FnOnce(&mut W, T) -> Result<(), TerrainAssetCodecError>,
) -> Result<(), TerrainAssetCodecError>
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
    reader: &mut TerrainReader<'_>,
    field: &'static str,
    read_value: impl FnOnce(&mut TerrainReader<'_>) -> Result<T, TerrainAssetCodecError>,
) -> Result<Option<T>, TerrainAssetCodecError> {
    let tag = reader.read_u8(field)?;
    match tag {
        0 => Ok(None),
        1 => read_value(reader).map(Some),
        tag => Err(TerrainAssetCodecError::InvalidTag { field, tag }),
    }
}

fn read_vec<T>(
    reader: &mut TerrainReader<'_>,
    field: &'static str,
    mut read_value: impl FnMut(&mut TerrainReader<'_>) -> Result<T, TerrainAssetCodecError>,
) -> Result<Vec<T>, TerrainAssetCodecError> {
    let len = reader.read_len(field)?;
    let mut values = Vec::with_capacity(len);
    for _ in 0..len {
        values.push(read_value(reader)?);
    }
    Ok(values)
}

fn read_header(
    reader: &mut TerrainReader<'_>,
    expected_magic: [u8; 8],
    expected_version: u32,
) -> Result<(), TerrainAssetCodecError> {
    let magic = reader.read_array::<8>("magic")?;
    if magic != expected_magic {
        return Err(TerrainAssetCodecError::BadMagic { found: magic });
    }

    let version = reader.read_u32("version")?;
    if version != expected_version {
        return Err(TerrainAssetCodecError::UnsupportedVersion { version });
    }
    Ok(())
}

fn write_u8<W: Write + ?Sized>(writer: &mut W, value: u8) -> Result<(), TerrainAssetCodecError> {
    writer.write_all(&[value])?;
    Ok(())
}

fn write_u16<W: Write + ?Sized>(writer: &mut W, value: u16) -> Result<(), TerrainAssetCodecError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_i32<W: Write + ?Sized>(writer: &mut W, value: i32) -> Result<(), TerrainAssetCodecError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_u32<W: Write + ?Sized>(writer: &mut W, value: u32) -> Result<(), TerrainAssetCodecError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_f32<W: Write + ?Sized>(writer: &mut W, value: f32) -> Result<(), TerrainAssetCodecError> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_string<W: Write + ?Sized>(
    writer: &mut W,
    field: &'static str,
    value: &str,
) -> Result<(), TerrainAssetCodecError> {
    write_len(writer, field, value.len())?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn write_len<W: Write + ?Sized>(
    writer: &mut W,
    field: &'static str,
    len: usize,
) -> Result<(), TerrainAssetCodecError> {
    let len = u32::try_from(len).map_err(|_| TerrainAssetCodecError::CountTooLarge {
        field,
        count: len as u64,
    })?;
    write_u32(writer, len)
}

struct TerrainReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> TerrainReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    const fn finish(&self) -> Result<(), TerrainAssetCodecError> {
        let remaining = self.bytes.len().saturating_sub(self.cursor);
        if remaining == 0 {
            Ok(())
        } else {
            Err(TerrainAssetCodecError::TrailingBytes { remaining })
        }
    }

    fn read_array<const N: usize>(
        &mut self,
        field: &'static str,
    ) -> Result<[u8; N], TerrainAssetCodecError> {
        let end = self
            .cursor
            .checked_add(N)
            .ok_or(TerrainAssetCodecError::UnexpectedEof { field })?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(TerrainAssetCodecError::UnexpectedEof { field })?;
        self.cursor = end;

        let mut out = [0; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    fn read_u8(&mut self, field: &'static str) -> Result<u8, TerrainAssetCodecError> {
        Ok(self.read_array::<1>(field)?[0])
    }

    fn read_u16(&mut self, field: &'static str) -> Result<u16, TerrainAssetCodecError> {
        Ok(u16::from_le_bytes(self.read_array::<2>(field)?))
    }

    fn read_i32(&mut self, field: &'static str) -> Result<i32, TerrainAssetCodecError> {
        Ok(i32::from_le_bytes(self.read_array::<4>(field)?))
    }

    fn read_u32(&mut self, field: &'static str) -> Result<u32, TerrainAssetCodecError> {
        Ok(u32::from_le_bytes(self.read_array::<4>(field)?))
    }

    fn read_f32(&mut self, field: &'static str) -> Result<f32, TerrainAssetCodecError> {
        Ok(f32::from_le_bytes(self.read_array::<4>(field)?))
    }

    fn read_len(&mut self, field: &'static str) -> Result<usize, TerrainAssetCodecError> {
        let count = self.read_u32(field)?;
        usize::try_from(count).map_err(|_| TerrainAssetCodecError::CountTooLarge {
            field,
            count: u64::from(count),
        })
    }

    fn read_string(&mut self, field: &'static str) -> Result<String, TerrainAssetCodecError> {
        let len = self.read_len(field)?;
        let end = self
            .cursor
            .checked_add(len)
            .ok_or(TerrainAssetCodecError::UnexpectedEof { field })?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(TerrainAssetCodecError::UnexpectedEof { field })?;
        self.cursor = end;
        let text = std::str::from_utf8(bytes)
            .map_err(|source| TerrainAssetCodecError::InvalidUtf8 { field, source })?;
        Ok(text.to_string())
    }
}

fn validate_heightmap_sample_count(
    width: u32,
    height: u32,
    found: usize,
) -> Result<(), TerrainAssetCodecError> {
    let expected = usize::try_from(u64::from(width) * u64::from(height)).map_err(|_| {
        TerrainAssetCodecError::CountTooLarge {
            field: "height samples",
            count: u64::from(width) * u64::from(height),
        }
    })?;
    if expected != found {
        return Err(TerrainAssetCodecError::HeightmapSampleCountMismatch {
            width,
            height,
            expected,
            found,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_round_trips() {
        let asset = TerrainWorldAsset {
            name: "north".to_string(),
            bounds: TerrainBounds {
                min: Vec2::new(0.0, 1.0),
                max: Vec2::new(1024.0, 2048.0),
            },
            height_range: TerrainHeightRange {
                min: -64.0,
                max: 2048.0,
            },
            resolution: TerrainResolution {
                height_spacing: 1.0,
                surface_spacing: 2.0,
            },
            layers: "terrain/layers/base.azterrain-layers.ron".to_string(),
            regions: vec![TerrainRegionRef {
                asset: "terrain/regions/r0.azterrain-region.ron".to_string(),
                coord: Some(TerrainCoord { x: 0, y: 1 }),
                bounds: TerrainBounds {
                    min: Vec2::ZERO,
                    max: Vec2::splat(512.0),
                },
                priority: 7,
            }],
        };

        let bytes = encode_terrain_world_asset(&asset).unwrap();
        assert_eq!(decode_terrain_world_asset(&bytes).unwrap(), asset);
    }

    #[test]
    fn region_round_trips() {
        let asset = TerrainRegionAsset {
            name: "r0".to_string(),
            height: TerrainHeightSource::Image(TerrainHeightImageSource {
                image: "textures/terrain/height.tif".to_string(),
                channel: TerrainImageChannel::Red,
                mip: 0,
                tiling: Vec2::ONE,
            }),
            surface: Some(TerrainSurfaceSource::Image(TerrainSurfaceImageSource {
                image: "textures/terrain/surface.tif".to_string(),
                mip: 1,
                tiling: Vec2::splat(2.0),
                channels: vec![TerrainSurfaceChannel {
                    channel: TerrainImageChannel::Green,
                    tag: SurfaceTag {
                        name: "grass".to_string(),
                    },
                }],
            })),
            water: Some("terrain/water/default.water.ron".to_string()),
            layers: None,
        };

        let bytes = encode_terrain_region_asset(&asset).unwrap();
        assert_eq!(decode_terrain_region_asset(&bytes).unwrap(), asset);
    }

    #[test]
    fn layer_set_round_trips() {
        let asset = TerrainLayerSetAsset {
            name: "base".to_string(),
            layers: vec![TerrainLayer {
                tag: SurfaceTag {
                    name: "rock".to_string(),
                },
                priority: 10,
                material: Some("materials/terrain/rock.azmaterial.ron".to_string()),
                physics_material: None,
                texture_scale: 4.0,
            }],
        };

        let bytes = encode_terrain_layer_set_asset(&asset).unwrap();
        assert_eq!(decode_terrain_layer_set_asset(&bytes).unwrap(), asset);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = encode_terrain_layer_set_asset(&TerrainLayerSetAsset {
            name: "empty".to_string(),
            layers: Vec::new(),
        })
        .unwrap();
        bytes[0] = b'X';

        assert!(matches!(
            decode_terrain_layer_set_asset(&bytes),
            Err(TerrainAssetCodecError::BadMagic { .. })
        ));
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut bytes = encode_terrain_world_asset(&TerrainWorldAsset {
            name: "empty".to_string(),
            bounds: TerrainBounds {
                min: Vec2::ZERO,
                max: Vec2::ONE,
            },
            height_range: TerrainHeightRange { min: 0.0, max: 1.0 },
            resolution: TerrainResolution {
                height_spacing: 1.0,
                surface_spacing: 1.0,
            },
            layers: "terrain/layers/default.azterrain-layers.ron".to_string(),
            regions: Vec::new(),
        })
        .unwrap();
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());

        assert!(matches!(
            decode_terrain_world_asset(&bytes),
            Err(TerrainAssetCodecError::UnsupportedVersion { version: 2 })
        ));
    }
}
